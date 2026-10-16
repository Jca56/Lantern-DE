//! Lantern Control Center: the bar along the bottom of the screen, the
//! Command Center's sibling (a Plasma panel beside its drop-down). The
//! two run side by side until this one can stand on its own; then System
//! Settings chooses between them. On Lantern UI 2.
//!
//! ## Layout
//!
//! - `app.rs` — the [`lntrn_ui::Host`]: one editor, the bar, following
//!   `lantern.toml` as it changes.
//! - `bar.rs` — where the bar sits (its layer surface, its monitor) and
//!   how it is painted, floating or docked.
//! - `dock.rs` — when to dock: the compositor's word on whether a window
//!   fills our monitor, read on a thread of its own.
//! - `morph.rs` — the plate's glide between floating and docked.
//! - `settings.rs` — what it reads from `lantern.toml`, and the thread
//!   that watches the file.
//! - `instance.rs` — one bar at a time.

mod app;
mod bar;
mod dock;
mod instance;
mod morph;
mod settings;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use lntrn_app::{AppConfig, run};
use lntrn_core::{log_error, log_info};
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};
use crate::settings::Settings;

fn main() {
    lntrn_kit::startup::log_panics(APP_ID);
    let Some(lock) = instance::claim(APP_ID) else {
        log_info!("{APP_ID}: already running");
        return;
    };
    let settings = Settings::read().unwrap_or_default();
    let output = bar::home(settings.primary.as_deref());
    log_info!("{APP_ID}: on {}", output.as_deref().unwrap_or("the compositor's pick"));
    let mut shell = Shell::new(Editor::Bar);
    shell.prefs.theme = bar::theme(&settings.desktop);
    let config = AppConfig {
        title: "Control Center".into(),
        app_id: APP_ID.into(),
        layer: Some(bar::layer(settings.shape, false, output.as_deref())),
        title_bar: false,
        sans: settings.desktop.font.clone(),
        // See-through past the plate.
        transparent: true,
        // Nothing of the shell's to keep: the layout is one bar.
        persist: false,
        ..AppConfig::default()
    };
    run(config, App::new(settings, output), shell);
    if app::restart_wanted() {
        drop(lock);
        restart();
    }
    std::process::exit(0);
}

/// Start over as a fresh process with the same arguments: a layer
/// surface's monitor is fixed when it is made, so moving to a new primary
/// monitor takes a new surface.
fn restart() -> ! {
    // A binary replaced since it started reads as `… (deleted)`; the new
    // one is at the same path.
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_ID));
    let exe = exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")).map(PathBuf::from).unwrap_or(exe);
    let err = std::process::Command::new(&exe).args(std::env::args_os().skip(1)).exec();
    log_error!("{APP_ID}: can't start over as {}: {err}", exe.display());
    std::process::exit(1);
}
