//! Lantern Terminal: shells in tabs and panes, on Lantern UI 2 with
//! `lntrn-kit`'s look. The terminal itself (the pty, the screen, the
//! escape codes, the drawing) is Lantern UI's `lntrn-term`, the same one
//! Lantern Code has; this is the window around it.
//!
//! ## Layout
//!
//! - `app.rs` — the [`lntrn_ui::Host`]: one editor, a terminal, in the
//!   shell's own panes and tabs. A tab's terminal lives in the tab.
//! - `ops.rs` — what happens once a frame is built: panes and tabs
//!   changed, every terminal's output taken in, tabs named and closed.
//! - `menu.rs` — the right-click menu, the title bar's menus, the
//!   palette's commands, and what they all do.
//! - `settings.rs` — the `[terminal]` section of `lantern.toml`.
//! - `look.rs` — the colours.

mod app;
mod look;
mod menu;
mod ops;
mod settings;
#[cfg(test)]
mod smoke;

use std::ffi::OsString;

use lntrn_app::{AppConfig, run};
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};

/// `lntrn-terminal -e <program> [args...]`: run this in the window's own
/// tab instead of a shell. Everything after `-e` is the program's.
fn command() -> Option<Vec<OsString>> {
    let mut args = std::env::args_os().skip(1);
    args.find(|arg| arg == "-e")?;
    let command: Vec<OsString> = args.collect();
    (!command.is_empty()).then_some(command)
}

fn main() {
    lntrn_kit::startup::log_panics(APP_ID);
    let mut app = App::new();
    let mut shell = Shell::new(Editor::Terminal);
    app.open_first(&mut shell, command());
    let desktop = app.follow.desktop().clone();
    let config = AppConfig {
        title: "Terminal".into(),
        app_id: APP_ID.into(),
        size: (1200.0, 760.0),
        min_size: (480.0, 300.0),
        maximized: false,
        title_bar: !app.bar_hidden,
        sans: desktop.font,
        opacity: desktop.opacity,
        transparent: true,
        // The tabs are this run's shells: nothing of them to bring back.
        persist: false,
        ..AppConfig::default()
    };
    run(config, app, shell);
    std::process::exit(0);
}
