#![allow(irrefutable_let_patterns)]

mod animation;
mod animations;
mod blur;
pub mod cc_thumbs;
pub mod clipboard_ipc;
pub mod clipboard_manager;
mod cursor;
mod cursor_click;
mod cursor_loading;
mod easing;
mod gestures;
mod grabs;
mod handlers;
mod hot_corners;
pub mod hover_preview;
mod input;
mod keyboard_focus;
mod layer_position;
mod minimize_anim;
mod rect_anim;
mod render;
mod window_state_anim;
mod rounded_element;
mod screencopy_render;
mod security;
mod shaders;
mod snap;
pub mod ssd;
mod state;
mod switcher;
mod workspace_anim;
mod workspace_ipc;
mod workspaces;
mod vrr;
pub mod udev;
mod udev_device;
mod power;
mod wallpaper;
mod window_ext;
mod window_management;
mod window_state;
mod winit;
mod xcursor_export;
mod xwayland;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::Lantern;

/// Returns `~/.lantern`, the root of the Lantern home directory.
pub(crate) fn lantern_home() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".lantern")
}

/// Returns the path to the shared DE config, with old-path fallback.
pub(crate) fn lantern_config_path() -> std::path::PathBuf {
    let new_path = lantern_home().join("config/lantern.toml");
    if new_path.exists() { return new_path; }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let old_path = std::path::PathBuf::from(home).join(".config/lantern/lantern.toml");
    if old_path.exists() { return old_path; }
    new_path
}

/// Cached lantern.toml contents, invalidated by mtime. Avoids re-reading the
/// file on every read_config call — periodic reloads in the render path were
/// causing 33ms stalls every 300 frames (visible as cursor/animation lag).
pub(crate) fn cached_lantern_toml() -> std::sync::Arc<String> {
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::SystemTime;
    static CACHE: OnceLock<Mutex<(Option<SystemTime>, Arc<String>)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new((None, Arc::new(String::new()))));

    let path = lantern_config_path();
    let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());

    let mut guard = cache.lock().unwrap();
    if guard.0 != mtime {
        match std::fs::read_to_string(&path) {
            Ok(c) => { guard.0 = mtime; guard.1 = Arc::new(c); }
            Err(_) => { /* keep previous cache on read error */ }
        }
    }
    Arc::clone(&guard.1)
}

/// Read a string setting from a given [section] in lantern.toml.
pub(crate) fn read_config(section: &str, key: &str, default: &str) -> String {
    let contents = cached_lantern_toml();
    if contents.is_empty() { return default.to_string(); }
    let section_header = format!("[{}]", section);
    let mut in_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == section_header;
            continue;
        }
        if in_section {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == key {
                    return v.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    default.to_string()
}

/// This machine's hostname, cached for the process lifetime. Read from
/// `/etc/hostname` (trimmed), falling back to `$HOSTNAME`, then "unknown".
/// Used to scope keybinds per-machine so the Gentoo PC and Arch laptop can
/// diverge — see `read_keybind`.
pub(crate) fn machine_hostname() -> &'static str {
    use std::sync::OnceLock;
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(read_hostname)
}

/// Resolve this machine's hostname robustly. The kernel hostname
/// (`/proc/sys/kernel/hostname`) is the source of truth and is always present
/// on Linux — `/etc/hostname` is absent on some setups (e.g. Gentoo stores it
/// in `/etc/conf.d/hostname`). Falls back through `/etc/hostname`, `$HOSTNAME`,
/// then "unknown".
fn read_hostname() -> String {
    let try_file = |p: &str| {
        std::fs::read_to_string(p)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    try_file("/proc/sys/kernel/hostname")
        .or_else(|| try_file("/etc/hostname"))
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown".into())
}

/// Read a keybind setting with per-machine override. Looks first in the
/// machine-scoped section `[keybinds.machine.<hostname>]`, then falls back to
/// the shared `[keybinds]` section, then `default`. This is how a setting can
/// apply to only the desktop without touching the laptop.
pub(crate) fn read_keybind(key: &str, default: &str) -> String {
    let scoped = format!("keybinds.machine.{}", machine_hostname());
    let v = read_config(&scoped, key, "");
    if !v.is_empty() {
        return v;
    }
    read_config("keybinds", key, default)
}

/// Read a float setting from a given [section] in lantern.toml.
pub(crate) fn read_config_f32(key: &str, default: f32) -> f32 {
    let s = read_config("windows", key, "");
    if s.is_empty() { return default; }
    s.parse::<f32>().unwrap_or(default)
}

/// Fallback output scale — read from `[display] scale =` in lantern.toml.
/// Used when no per-monitor entry exists. Cached for process lifetime.
pub(crate) fn output_scale() -> f64 {
    use std::sync::OnceLock;
    static SCALE: OnceLock<f64> = OnceLock::new();
    *SCALE.get_or_init(|| {
        let s = read_config("display", "scale", "1.0");
        s.parse::<f64>().unwrap_or(1.0)
    })
}

/// Per-monitor scale — looks up `[[monitors]] scale` by output name in
/// lantern.toml. Falls back to [display] scale, then 1.0. Re-reads the config
/// each call (cheap — mtime-cached) so a system-settings write is picked up
/// on the next output add or scale-change request.
pub(crate) fn monitor_scale(name: &str) -> f64 {
    read_monitor_configs()
        .into_iter()
        .find(|c| c.name == name)
        .and_then(|c| c.scale)
        .unwrap_or_else(output_scale)
}

/// Read a string-list setting from [section] in lantern.toml.
/// Expects TOML array syntax: `key = ["a", "b", "c"]`
pub(crate) fn read_config_list(section: &str, key: &str) -> Vec<String> {
    let raw = read_config(section, key, "");
    if raw.is_empty() { return Vec::new(); }
    // Strip surrounding brackets and split on commas
    let inner = raw.trim().trim_start_matches('[').trim_end_matches(']');
    inner.split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a hex color string into a linear [R, G, B, A] array for the glow shader.
/// Alpha defaults to 1.0 — callers override it with the user's glow intensity.
pub(crate) fn parse_glow_color(hex: &str) -> [f32; 4] {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    let (r, g, b) = if hex.len() >= 6 {
        (
            u8::from_str_radix(&hex[0..2], 16).unwrap_or(74),
            u8::from_str_radix(&hex[2..4], 16).unwrap_or(158),
            u8::from_str_radix(&hex[4..6], 16).unwrap_or(255),
        )
    } else {
        (74, 158, 255) // default blue
    };
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// A configured monitor position from `[[monitors]]` in lantern.toml.
#[derive(Debug, Clone)]
pub(crate) struct MonitorConfig {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub resolution: Option<String>,
    pub refresh_rate: Option<u32>,
    pub scale: Option<f64>,
    pub wallpaper: Option<String>,
    /// User-designated "main" monitor — UI surfaces that should always
    /// land on one specific output (e.g. command-center) anchor here.
    pub primary: bool,
    /// Allow Variable Refresh Rate (adaptive sync) on this output. Honored
    /// on demand — only while a fullscreen app owns the output. See `vrr.rs`.
    pub vrr: bool,
}

/// A per-app window sizing rule from `[[window_rules]]` in lantern.toml.
#[derive(Debug, Clone)]
pub(crate) struct WindowRule {
    pub app_id: String,
    pub width: i32,
    pub height: i32,
}

/// Default window size from `[windows] default_width/default_height`.
/// Returns None if either is unset.
pub(crate) fn default_window_size() -> Option<(i32, i32)> {
    let w = read_config("windows", "default_width", "");
    let h = read_config("windows", "default_height", "");
    if w.is_empty() || h.is_empty() { return None; }
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Percentage of an output's work area to use as a window's initial / Tiny /
/// Middle / SoloTile size — read from `[windows]` in lantern.toml. Returns a
/// clamped fraction in `0.05..=1.0`. Sentinel `default` (5..=100 int %)
/// applied when the config value is missing or out of range.
fn window_size_pct(key: &str, default: u32) -> f32 {
    let raw = read_config("windows", key, "");
    let parsed: u32 = raw.parse().unwrap_or(default);
    parsed.clamp(5, 100) as f32 / 100.0
}

pub(crate) fn default_size_pct() -> f32 { window_size_pct("default_size_pct", 60) }
pub(crate) fn size_small_pct()    -> f32 { window_size_pct("size_small_pct",   30) }
pub(crate) fn size_medium_pct()   -> f32 { window_size_pct("size_medium_pct",  60) }
pub(crate) fn size_large_pct()    -> f32 { window_size_pct("size_large_pct",   85) }
pub(crate) fn size_xlarge_pct()   -> f32 { window_size_pct("size_xlarge_pct", 100) }

/// Inner gap (formerly the tiling gap) — read from `[window_manager].gap`,
/// default 8, clamped to 0..=32. Still used by snap zones, zone-move, and
/// axis-resize to give windows breathing room from screen edges.
pub(crate) fn default_gap() -> i32 {
    read_config("window_manager", "gap", "8")
        .parse::<i32>()
        .unwrap_or(8)
        .clamp(0, 32)
}

/// Outer gap (between a window and the screen edge). Matches the inner gap
/// so users see one consistent value.
pub(crate) fn default_outer_gap() -> i32 {
    default_gap()
}

/// Larger outer gap used when a single window is "solo-tiled" (puffed up to
/// fill the work area), so it doesn't sit flush against the screen edges.
pub(crate) const SINGLE_WINDOW_OUTER_GAP: i32 = 40;

/// Read all `[[window_rules]]` entries from lantern.toml.
pub(crate) fn read_window_rules() -> Vec<WindowRule> {
    let contents = cached_lantern_toml();
    if contents.is_empty() { return Vec::new(); }

    let mut rules = Vec::new();
    let mut in_rule = false;
    let mut app_id = String::new();
    let mut width: Option<i32> = None;
    let mut height: Option<i32> = None;

    let flush = |app_id: &mut String, width: &mut Option<i32>, height: &mut Option<i32>, rules: &mut Vec<WindowRule>| {
        if !app_id.is_empty() {
            if let (Some(w), Some(h)) = (width.take(), height.take()) {
                rules.push(WindowRule { app_id: std::mem::take(app_id), width: w, height: h });
            } else {
                app_id.clear();
            }
        }
    };

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "[[window_rules]]" {
            flush(&mut app_id, &mut width, &mut height, &mut rules);
            in_rule = true;
            continue;
        }
        if trimmed.starts_with('[') {
            if in_rule {
                flush(&mut app_id, &mut width, &mut height, &mut rules);
            }
            in_rule = false;
            continue;
        }
        if in_rule {
            if let Some((k, v)) = trimmed.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "app_id" => app_id = v.to_string(),
                    "width" => width = v.parse().ok(),
                    "height" => height = v.parse().ok(),
                    _ => {}
                }
            }
        }
    }
    if in_rule {
        flush(&mut app_id, &mut width, &mut height, &mut rules);
    }

    rules
}

/// Read all `[[monitors]]` entries from lantern.toml.
pub(crate) fn read_monitor_configs() -> Vec<MonitorConfig> {
    let path = lantern_config_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut monitors = Vec::new();
    let mut in_monitors = false;
    let mut name = String::new();
    let mut x: Option<i32> = None;
    let mut y: Option<i32> = None;
    let mut resolution: Option<String> = None;
    let mut refresh_rate: Option<u32> = None;
    let mut scale: Option<f64> = None;
    let mut wallpaper: Option<String> = None;
    let mut primary = false;
    let mut vrr = false;

    let mut flush = |name: &mut String, x: &mut Option<i32>, y: &mut Option<i32>,
                     resolution: &mut Option<String>, refresh_rate: &mut Option<u32>,
                     scale: &mut Option<f64>, wallpaper: &mut Option<String>,
                     primary: &mut bool, vrr: &mut bool,
                     monitors: &mut Vec<MonitorConfig>| {
        if !name.is_empty() {
            monitors.push(MonitorConfig {
                name: std::mem::take(name),
                x: x.take().unwrap_or(0),
                y: y.take().unwrap_or(0),
                resolution: resolution.take(),
                refresh_rate: refresh_rate.take(),
                scale: scale.take(),
                wallpaper: wallpaper.take(),
                primary: std::mem::take(primary),
                vrr: std::mem::take(vrr),
            });
        }
    };

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "[[monitors]]" {
            flush(&mut name, &mut x, &mut y, &mut resolution, &mut refresh_rate, &mut scale, &mut wallpaper, &mut primary, &mut vrr, &mut monitors);
            in_monitors = true;
            continue;
        }
        if trimmed.starts_with('[') {
            if in_monitors {
                flush(&mut name, &mut x, &mut y, &mut resolution, &mut refresh_rate, &mut scale, &mut wallpaper, &mut primary, &mut vrr, &mut monitors);
            }
            in_monitors = false;
            continue;
        }
        if in_monitors {
            if let Some((k, v)) = trimmed.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "name" => name = v.to_string(),
                    "x" => x = v.parse().ok(),
                    "y" => y = v.parse().ok(),
                    "resolution" => resolution = Some(v.to_string()),
                    "refresh_rate" => refresh_rate = v.parse().ok(),
                    "scale" => scale = v.parse().ok(),
                    "wallpaper" => wallpaper = Some(v.to_string()),
                    "primary" => primary = v == "true",
                    "vrr" => vrr = v == "true",
                    _ => {}
                }
            }
        }
    }
    // Flush last entry
    if in_monitors {
        flush(&mut name, &mut x, &mut y, &mut resolution, &mut refresh_rate, &mut scale, &mut wallpaper, &mut primary, &mut vrr, &mut monitors);
    }

    monitors
}

/// Name of the monitor flagged `primary = true` in lantern.toml, if any.
/// On single-output setups (laptop) where no monitor is flagged, returns
/// None and callers should fall back to their normal selection logic.
pub(crate) fn primary_output_name() -> Option<String> {
    read_monitor_configs()
        .into_iter()
        .find(|m| m.primary)
        .map(|m| m.name)
}

/// Whether the named output is allowed to use Variable Refresh Rate
/// (`vrr = true` in its `[[monitors]]` block). Default false.
pub(crate) fn output_vrr_enabled(name: &str) -> bool {
    read_monitor_configs()
        .iter()
        .find(|m| m.name == name)
        .map(|m| m.vrr)
        .unwrap_or(false)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_child_reaper();
    ensure_lantern_bin_on_path();
    let log_file = setup_persistent_log();
    init_logging(log_file);
    setup_panic_hook();

    let backend = parse_backend();
    tracing::info!("Starting Lantern compositor with {:?} backend", backend);

    let mut event_loop: EventLoop<Lantern> = EventLoop::try_new()?;
    let display: Display<Lantern> = Display::new()?;
    let mut state = Lantern::new(&mut event_loop, display);
    if let Err(e) = crate::clipboard_manager::install_recheck_source(&mut state) {
        tracing::warn!("clipboard recheck source install failed: {e}");
    }

    // Generate the Lantern Xcursor theme on disk before XWayland spawns so
    // X11 clients see our cursor instead of falling back to Adwaita / the
    // bare X11 root cursor. Safe to call every startup — cached via a
    // fingerprint file.
    crate::xcursor_export::export();

    match backend {
        Backend::Winit => {
            crate::winit::init_winit(&mut event_loop, &mut state)?;

            std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
            crate::xwayland::start_xwayland(&mut state);
            // Daemons/clients are spawned from handle_xwayland_ready()
            // so DISPLAY is guaranteed to be set for X11 apps.

            event_loop.run(None, &mut state, move |_| {})?;
        }
        Backend::Udev => {
            std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
            let no_xwayland = std::env::var_os("LNTRN_NO_XWAYLAND").is_some()
                || lantern_home().join("log/no-xwayland").exists();
            if no_xwayland {
                tracing::warn!("XWAYLAND DISABLED via LNTRN_NO_XWAYLAND flag");
            } else {
                crate::xwayland::start_xwayland(&mut state);
            }
            // Daemons/clients are spawned from handle_xwayland_ready()
            // so DISPLAY is guaranteed to be set for X11 apps.

            crate::udev::init_udev(&mut event_loop, &mut state)?;
        }
    }

    Ok(())
}

#[derive(Debug)]
enum Backend {
    Winit,
    Udev,
}

fn parse_backend() -> Backend {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--winit" => return Backend::Winit,
            "--udev" => return Backend::Udev,
            _ => {}
        }
    }
    // Default: udev when running standalone, winit when WAYLAND_DISPLAY is set
    if std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("DISPLAY").is_ok() {
        Backend::Winit
    } else {
        Backend::Udev
    }
}

fn setup_persistent_log() -> Option<std::fs::File> {
    let log_dir = lantern_home().join("log");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return None;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("compositor.log"))
        .ok()
}

fn init_logging(log_file: Option<std::fs::File>) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if let Some(file) = log_file {
        // Use LineWriter to flush after every log line, so we don't lose
        // the last breadcrumb if the compositor freezes on a blocking call.
        let writer = std::io::LineWriter::new(file);
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(std::sync::Mutex::new(writer))
            .with_ansi(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }
}

/// Tell the kernel to automatically reap child processes so they never become
/// zombies. Without this, every `Command::new(...).spawn()` that isn't
/// explicitly `wait()`-ed on leaves a zombie in the process table.
fn install_child_reaper() {
    // Previously set SA_NOCLDWAIT on SIGCHLD so the kernel auto-reaped children.
    // Rust std >=1.77 panics inside Command::spawn() when the kernel has already
    // reaped the child (ECHILD from internal waitid). We now reap zombies
    // explicitly via reap_zombies() at every spawn site instead.
}

/// Prepend ~/.lantern/bin to PATH so `Command::new("lntrn-terminal")` etc. resolve
/// regardless of how the compositor was launched (session manager vs. bare TTY).
fn ensure_lantern_bin_on_path() {
    let bin = lantern_home().join("bin");
    let bin_str = bin.to_string_lossy().to_string();
    let current = std::env::var("PATH").unwrap_or_default();
    if current.split(':').any(|p| p == bin_str) {
        return;
    }
    let new_path = if current.is_empty() {
        bin_str
    } else {
        format!("{}:{}", bin_str, current)
    };
    // Safe: single-threaded at startup, before any spawn or thread creation.
    unsafe { std::env::set_var("PATH", new_path); }
}

/// Actively reap any zombie children. Call this periodically since SA_NOCLDWAIT
/// can be overridden by libraries (e.g. XWayland signal handling).
pub fn reap_zombies() {
    unsafe {
        loop {
            let ret = libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG);
            if ret <= 0 {
                break;
            }
        }
    }
}

fn setup_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("PANIC: {}", info);
        default_hook(info);
    }));
}

