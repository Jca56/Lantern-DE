//! Audio/brightness OSD spawning, detached-child helpers, and the
//! `AudioRepeat` repeat-key tracker.

use smithay::backend::input::Keycode;

use std::process::Command;
use std::time::Instant;

/// Tracks held audio keys for repeat behavior.
pub struct AudioRepeat {
    pub cmd: &'static str,
    pub key_code: Keycode,
    pub last_fire: Instant,
    pub initial_delay_done: bool,
}

pub(super) const AUDIO_REPEAT_DELAY_MS: u128 = 400;
pub(super) const AUDIO_REPEAT_INTERVAL_MS: u128 = 80;

/// Read a string setting from the [input] section of the Lantern config.
/// Uses the shared lantern.toml cache so this is near-instant when the file
/// hasn't changed (just one stat() syscall to check mtime).
pub fn read_input_setting(key: &str, default: &str) -> String {
    let contents = crate::cached_lantern_toml();
    if contents.is_empty() {
        return default.to_string();
    }
    let mut in_input = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_input = trimmed == "[input]";
            continue;
        }
        if in_input {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == key {
                    return v.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    default.to_string()
}

/// Read a float setting from the [input] section.
pub fn read_input_setting_f64(key: &str, default: f64) -> f64 {
    let s = read_input_setting(key, "");
    if s.is_empty() { return default; }
    s.parse::<f64>().unwrap_or(default)
}

/// Resolve a command to an absolute path, preferring `~/.lantern/bin/<cmd>`
/// so spawns work even when the compositor's PATH is incomplete (e.g. when
/// session-manager was launched directly from a TTY without a login shell).
fn resolve_lantern_bin(cmd: &str) -> std::path::PathBuf {
    if cmd.starts_with('/') || cmd.starts_with("./") {
        return std::path::PathBuf::from(cmd);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let candidate = std::path::PathBuf::from(&home).join(".lantern/bin").join(cmd);
    if candidate.exists() {
        return candidate;
    }
    std::path::PathBuf::from(cmd)
}

pub(super) fn spawn_detached(cmd: &str, wayland_display: &std::ffi::OsStr) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    crate::reap_zombies();
    let resolved = resolve_lantern_bin(cmd);
    tracing::info!("spawn_detached: launching {}", resolved.display());
    match unsafe {
        Command::new(&resolved)
            .env("WAYLAND_DISPLAY", wayland_display)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .pre_exec(|| {
                // New session + process group so child is fully detached and
                // never gets the compositor's controlling-tty signals.
                libc::setsid();
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
    } {
        Ok(child) => tracing::info!("spawn_detached: spawned {} (pid {})", cmd, child.id()),
        Err(e) => tracing::error!("Failed to spawn {}: {}", cmd, e),
    }
}

pub(crate) fn spawn_detached_args(cmd: &str, args: &[&str], wayland_display: &std::ffi::OsStr) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    crate::reap_zombies();
    let resolved = resolve_lantern_bin(cmd);
    match unsafe {
        Command::new(&resolved)
            .args(args)
            .env("WAYLAND_DISPLAY", wayland_display)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .pre_exec(|| {
                libc::setsid();
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
    } {
        Ok(_) => {}
        Err(e) => tracing::error!("Failed to spawn {} {:?}: {}", cmd, args, e),
    }
}

pub(super) fn fire_audio_osd(cmd: &str, wayland_display: &std::ffi::OsStr) {
    let script = format!(
        "{cmd}; \
         out=$(wpctl get-volume @DEFAULT_AUDIO_SINK@); \
         vol=$(echo \"$out\" | awk '{{printf \"%d\", $2 * 100}}'); \
         if echo \"$out\" | grep -q MUTED; then \
           lntrn-osd mute; \
         else \
           lntrn-osd volume $vol; \
         fi"
    );
    spawn_detached_args("sh", &["-c", &script], wayland_display);
}

const BRIGHTNESS_STEP: u32 = 5; // percent

/// Auto-detect the first available backlight device under /sys/class/backlight/.
fn detect_backlight_path() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/backlight/").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        if path.join("brightness").exists() && path.join("max_brightness").exists() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

pub(super) fn fire_brightness_osd(direction: i32, wayland_display: &std::ffi::OsStr) {
    let Some(bl) = detect_backlight_path() else {
        tracing::warn!("No backlight device found in /sys/class/backlight/");
        return;
    };
    let script = format!(
        "max=$(cat {bl}/max_brightness); \
         cur=$(cat {bl}/brightness); \
         step=$((max * {BRIGHTNESS_STEP} / 100)); \
         new=$((cur + step * {direction})); \
         [ $new -lt 1 ] && new=1; \
         [ $new -gt $max ] && new=$max; \
         echo $new > {bl}/brightness; \
         pct=$((new * 100 / max)); \
         lntrn-osd brightness $pct"
    );
    spawn_detached_args("sh", &["-c", &script], wayland_display);
}
