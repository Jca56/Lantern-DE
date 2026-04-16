#![allow(irrefutable_let_patterns)]

mod animation;
mod blur;
mod canvas;
mod cursor;
mod easing;
mod gestures;
mod grabs;
mod handlers;
mod hot_corners;
pub mod hover_preview;
mod input;
mod layer_position;
mod render;
mod rounded_element;
mod screencopy_render;
mod shaders;
mod snap;
pub mod ssd;
mod state;
mod switcher;
mod tiling;
mod tiling_anim;
pub mod udev;
mod udev_device;
mod wallpaper;
mod window_ext;
mod window_management;
mod winit;
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

/// Read a string setting from a given [section] in lantern.toml.
pub(crate) fn read_config(section: &str, key: &str, default: &str) -> String {
    let path = lantern_config_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return default.to_string(),
    };
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

/// Read a float setting from a given [section] in lantern.toml.
pub(crate) fn read_config_f32(key: &str, default: f32) -> f32 {
    let s = read_config("windows", key, "");
    if s.is_empty() { return default; }
    s.parse::<f32>().unwrap_or(default)
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

    let mut flush = |name: &mut String, x: &mut Option<i32>, y: &mut Option<i32>,
                     resolution: &mut Option<String>, refresh_rate: &mut Option<u32>,
                     scale: &mut Option<f64>, wallpaper: &mut Option<String>,
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
            });
        }
    };

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "[[monitors]]" {
            flush(&mut name, &mut x, &mut y, &mut resolution, &mut refresh_rate, &mut scale, &mut wallpaper, &mut monitors);
            in_monitors = true;
            continue;
        }
        if trimmed.starts_with('[') {
            if in_monitors {
                flush(&mut name, &mut x, &mut y, &mut resolution, &mut refresh_rate, &mut scale, &mut wallpaper, &mut monitors);
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
                    _ => {}
                }
            }
        }
    }
    // Flush last entry
    if in_monitors {
        flush(&mut name, &mut x, &mut y, &mut resolution, &mut refresh_rate, &mut scale, &mut wallpaper, &mut monitors);
    }

    monitors
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_child_reaper();
    let log_file = setup_persistent_log();
    init_logging(log_file);
    setup_panic_hook();

    let backend = parse_backend();
    tracing::info!("Starting Lantern compositor with {:?} backend", backend);

    let mut event_loop: EventLoop<Lantern> = EventLoop::try_new()?;
    let display: Display<Lantern> = Display::new()?;
    let mut state = Lantern::new(&mut event_loop, display);

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
            crate::xwayland::start_xwayland(&mut state);
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
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        sa.sa_flags = libc::SA_NOCLDWAIT;
        libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut());
    }
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

