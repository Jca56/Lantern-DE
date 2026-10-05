//! Lantern System Settings: the desktop's `lantern.toml` with a face, on
//! Lantern UI 2.
//!
//! ## Layout
//!
//! - `app.rs` — the [`lntrn_ui::Host`]: one editor, the sidebar on the
//!   left and the chosen page on the right, live saving.
//! - `nav.rs`, `sidebar.rs`, `glyphs.rs` — the pages, the groups the
//!   sidebar lists them under, and their pictures.
//! - `pages/` — one module per page, made of `lntrn-kit`'s cards, rows and
//!   controls, in its Lantern palette.
//! - `config/` — every section we own as a `props!` struct, loaded from
//!   and merged back into `lantern.toml` without touching what we don't.
//! - `thumbs.rs` — wallpaper thumbnails, made off the UI thread.
//! - `machine.rs`, `fonts.rs` — what the hardware has, what fonts exist.

mod app;
mod config;
mod fonts;
mod glyphs;
mod machine;
mod nav;
mod pages;
mod sidebar;
#[cfg(test)]
mod smoke;
mod thumbs;

use lntrn_app::{AppConfig, run};
use lntrn_kit as kit;
use lntrn_kit::look;
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};
use crate::config::Config;

fn main() {
    kit::startup::log_panics(APP_ID);
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
