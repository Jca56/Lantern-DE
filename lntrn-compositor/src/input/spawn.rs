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

/// Same as [`spawn_detached_args`] but routes stdout+stderr to
/// `~/.lantern/log/<log_name>.log` (append). Without this, children
/// spawned with stdio=null silently swallow tracing output, so debugging
/// daemons like `lntrn-command-center` becomes impossible.
pub(crate) fn spawn_detached_args_logged(
    cmd: &str,
    args: &[&str],
    wayland_display: &std::ffi::OsStr,
    log_name: &str,
) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    crate::reap_zombies();
    let resolved = resolve_lantern_bin(cmd);

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let log_dir = std::path::PathBuf::from(&home).join(".lantern/log");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("{log_name}.log"));

    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);
    let stderr = stdout
        .as_ref()
        .ok()
        .and_then(|f| f.try_clone().ok());

    let (stdout_io, stderr_io) = match (stdout, stderr) {
        (Ok(out), Some(err)) => (Stdio::from(out), Stdio::from(err)),
        _ => {
            tracing::warn!("could not open {} — falling back to /dev/null", log_path.display());
            (Stdio::null(), Stdio::null())
        }
    };

    match unsafe {
        Command::new(&resolved)
            .args(args)
            .env("WAYLAND_DISPLAY", wayland_display)
            .stdin(Stdio::null())
            .stdout(stdout_io)
            .stderr(stderr_io)
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

/// Fire the audio OSD for an action tag: `"VOL_UP"`, `"VOL_DOWN"`, or `"MUTE"`.
///
/// Up/Down snap to the nearest 5% boundary instead of doing `wpctl 5%+/5%-`,
/// so a non-aligned starting volume (e.g. 1.04 after boosting past 100) lands
/// on 1.00 going down rather than 0.99 — keeps the OSD digits at multiples of
/// 5. Range is capped to 1.2 (120%) to match `wpctl --limit 1.2`.
pub(super) fn fire_audio_osd(cmd: &str, wayland_display: &std::ffi::OsStr) {
    let action = match cmd {
        "VOL_UP" => {
            // awk: next 5% step above current, capped at 1.20.
            "out=$(wpctl get-volume @DEFAULT_AUDIO_SINK@); \
             cur=$(echo \"$out\" | awk '{print $2}'); \
             next=$(awk -v c=\"$cur\" 'BEGIN{ s=int(c/0.05 + 1e-6); n=(s+1)*0.05; if(n>1.2)n=1.2; printf \"%.2f\", n }'); \
             wpctl set-volume --limit 1.2 @DEFAULT_AUDIO_SINK@ $next"
        }
        "VOL_DOWN" => {
            // awk: largest 5% boundary strictly below current, floored at 0.
            // (subtract ε before flooring so an exact boundary like 1.00 steps
            // down to 0.95 instead of staying at 1.00.)
            "out=$(wpctl get-volume @DEFAULT_AUDIO_SINK@); \
             cur=$(echo \"$out\" | awk '{print $2}'); \
             next=$(awk -v c=\"$cur\" 'BEGIN{ s=int((c-1e-6)/0.05); n=s*0.05; if(n<0)n=0; printf \"%.2f\", n }'); \
             wpctl set-volume --limit 1.2 @DEFAULT_AUDIO_SINK@ $next"
        }
        "MUTE" => "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle",
        _ => return,
    };
    let script = format!(
        "{action}; \
         out=$(wpctl get-volume @DEFAULT_AUDIO_SINK@); \
         vol=$(echo \"$out\" | awk '{{printf \"%d\", ($2 * 100) + 0.5}}'); \
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
