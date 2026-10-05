//! Lantern System Settings: the desktop's `lantern.toml` with a face, on
//! Lantern UI 2.
//!
//! ## Layout
//!
//! - `app.rs` — the [`lntrn_ui::Host`]: one editor, the sidebar on the
//!   left and the chosen page on the right, live saving.
//! - `nav.rs`, `sidebar.rs`, `glyphs.rs` — the pages, the groups the
//!   sidebar lists them under, and their pictures.
//! - `look.rs` — the Lantern palette Settings wears, with the desktop's
//!   accent on top.
//! - `kit/` — the widgets pages are made of: cards, rows and controls.
//! - `pages/` — one module per page.
//! - `config/` — every section we own as a `props!` struct, loaded from
//!   and merged back into `lantern.toml` without touching what we don't.
//! - `thumbs.rs` — wallpaper thumbnails, made off the UI thread.
//! - `machine.rs`, `fonts.rs` — what the hardware has, what fonts exist.

mod app;
mod config;
mod fonts;
mod glyphs;
mod kit;
mod look;
mod machine;
mod nav;
mod pages;
mod sidebar;
#[cfg(test)]
mod smoke;
mod thumbs;

use std::path::PathBuf;

use lntrn_app::{AppConfig, run};
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};
use crate::config::Config;

unsafe extern "C" {
    fn isatty(fd: i32) -> i32;
    fn dup2(from: i32, to: i32) -> i32;
}

/// Panics go to `~/.lantern/log/lntrn-system-settings.log` with a
/// backtrace, and so does everything else written to stderr when no
/// terminal is attached, since an app launched from the desktop has
/// nowhere else to print.
fn log_panics() {
    let path = std::env::var_os("HOME").map(PathBuf::from).map(|h| h.join(".lantern/log").join(format!("{APP_ID}.log")));
    // SAFETY: plain libc calls on the standard descriptors.
    if let Some(p) = &path
        && unsafe { isatty(2) } == 0
        && let Some(dir) = p.parent()
        && std::fs::create_dir_all(dir).is_ok()
        && let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(p)
    {
        use std::os::fd::AsRawFd;
        unsafe {
            dup2(f.as_raw_fd(), 2);
        }
        std::mem::forget(f);
        eprintln!("---- {APP_ID} started, pid {} ----", std::process::id());
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(p) = &path {
            let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let text = format!("[{now}] {info}\n{}\n\n", std::backtrace::Backtrace::force_capture());
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        default(info);
    }));
}

fn main() {
    log_panics();
    let config = Config::load();
    let sans = fonts::effective_family(&config.appearance.font_family);
    let opacity = config.windows.background_opacity.clamp(0.05, 1.0);
    let app = App::new(config);
    let app_config = AppConfig {
        title: "System Settings".into(),
        app_id: APP_ID.into(),
        size: (1500.0, 1000.0),
        min_size: (1100.0, 700.0),
        maximized: false,
        sans,
        opacity,
        transparent: true,
        ..AppConfig::default()
    };
    run(app_config, app, Shell::new(Editor::Settings));
    std::process::exit(0);
}
