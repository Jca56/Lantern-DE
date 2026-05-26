mod auth;
mod config;
mod dispatch;
mod keyboard;
mod render;
mod wayland;

use std::os::fd::AsRawFd;

use wayland::BgImage;

fn main() {
    // Single-instance guard: if a lock screen is already up, do nothing.
    // Keeps idle/lid/CLI triggers from stacking multiple lock screens.
    let _guard = match acquire_lock() {
        Some(fd) => fd,
        None => {
            // Already locked — silently succeed so triggers are idempotent.
            return;
        }
    };

    let style = config::style();
    let bg = load_background();

    if let Err(err) = wayland::run(bg, style) {
        eprintln!("[lntrn-lockscreen] {err}");
        std::process::exit(1);
    }
}

/// Acquire an exclusive advisory lock so only one lock screen runs at a time.
/// Returns the held file (kept open for the process lifetime) or None if held.
fn acquire_lock() -> Option<std::fs::File> {
    let path = runtime_dir().join("lntrn-lockscreen.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(file)
    } else {
        None
    }
}

fn runtime_dir() -> std::path::PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
}

/// Decode the configured background, or fall back to a solid accent fill.
fn load_background() -> BgImage {
    if let Some(path) = config::background_path() {
        if let Ok(img) = image::open(&path) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            return BgImage { rgba: rgba.into_raw(), w, h };
        }
        eprintln!("[lntrn-lockscreen] failed to decode background: {}", path.display());
    }
    solid_fallback(config::accent())
}

/// A 1x1 solid texture used when no background image is available.
fn solid_fallback(c: lntrn_theme::Rgba) -> BgImage {
    // Darken the accent so foreground text stays readable.
    let rgba = vec![c.r / 3, c.g / 3, c.b / 3, 255];
    BgImage { rgba, w: 1, h: 1 }
}
