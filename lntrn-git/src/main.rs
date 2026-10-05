//! Lantern Git: the repos under `~/Projects` down the left and the open
//! one beside them, on Lantern UI 2 with `lntrn-kit`'s look.
//!
//! ## Layout
//!
//! - `app.rs` — the [`lntrn_ui::Host`]: menus, the palette, what the
//!   actions do, and taking in what the worker sends back.
//! - `state.rs` — the open repo and what the forms hold.
//! - `git.rs`, `diff.rs`, `github.rs` — git and GitHub through their
//!   command lines. All of it blocks, so all of it runs on
//! - `worker.rs` — the one thread that does.
//! - `history.rs` — the commit graph's layout.
//! - `sidebar.rs`, `views/`, `dialogs.rs` — what is drawn.
//! - `glyphs.rs` — the app's pictures.

mod app;
mod dialogs;
mod diff;
mod git;
mod github;
mod glyphs;
mod history;
mod sidebar;
#[cfg(test)]
mod smoke;
mod state;
mod views;
mod worker;

use lntrn_app::{AppConfig, run};
use lntrn_ui::Shell;

use crate::app::{App, Editor};
use crate::state::APP_ID;

fn main() {
    lntrn_kit::startup::log_panics(APP_ID);
    let app = App::new(state::last_repo());
    let desktop = app.desktop().clone();
    let config = AppConfig {
        title: "Lantern Git".into(),
        app_id: APP_ID.into(),
        size: (1500.0, 1000.0),
        min_size: (1100.0, 700.0),
        maximized: false,
        sans: desktop.font,
        opacity: desktop.opacity,
        transparent: true,
        ..AppConfig::default()
    };
    run(config, app, Shell::new(Editor::Git));
    std::process::exit(0);
}
