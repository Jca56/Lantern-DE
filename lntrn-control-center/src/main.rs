//! Lantern Control Center: the bar along the bottom of the screen, the
//! Command Center's sibling (a Plasma panel beside its drop-down). The
//! two run side by side until this one can stand on its own; then System
//! Settings chooses between them. On Lantern UI 2.
//!
//! ## Layout
//!
//! - `app/` — the [`lntrn_ui::Host`]: the bar, following `lantern.toml`
//!   as it changes, and the overlay above it for the launcher's menu and
//!   the dock's cards and menus.
//! - `bar.rs` — where the bar sits (its layer surface, its monitor) and
//!   how it is painted, floating or docked.
//! - `clock.rs` — the time and date at its right end, and the thread
//!   that turns the minute.
//! - `launcher/` — the app launcher: its button at the bar's left end and
//!   the menu it opens above the bar.
//! - `dock/` — the dock in the middle: pinned and running apps, their
//!   cards with live thumbnails, their menus.
//! - `compositor.rs` — the line to the compositor: whether a window fills
//!   our monitor (to dock), the open windows, and the dock's commands.
//! - `idlist.rs` — the favorites' and the pins' files.
//! - `hide.rs` — auto hide: sliding out of sight and back, the windows
//!   giving way to as much of the bar as shows.
//! - `morph.rs` — the plate's glides: floating and docked, hidden and
//!   shown.
//! - `settings.rs` — what it reads from `lantern.toml`, and the thread
//!   that watches the file.
//! - `instance.rs` — one bar at a time.
//!
//! The session manager keeps it running: a crash, or the compositor taking
//! the bar away with its monitor ([`TAKEN_AWAY`]), and it is started again.

mod app;
mod bar;
mod clock;
mod compositor;
mod dock;
mod hide;
mod idlist;
mod instance;
mod launcher;
mod morph;
mod settings;
#[cfg(test)]
mod smoke;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use lntrn_app::{AppConfig, run};
use lntrn_core::{log_error, log_info, log_warn};
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};
use crate::settings::Settings;

/// The exit status for a bar the compositor took away: not a clean exit,
/// which the session manager leaves alone, so it starts the bar again
/// (wherever the compositor puts it, if its monitor is gone). EX_TEMPFAIL.
const TAKEN_AWAY: i32 = 75;

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
    if app::taken_away() {
        log_warn!("{APP_ID}: the compositor took the bar away; exiting for the session to start it again");
        std::process::exit(TAKEN_AWAY);
    }
    std::process::exit(0);
}

/// Start over as a fresh process with the same arguments: a layer
/// surface's monitor is fixed when it is made, so moving to a new primary
/// monitor takes a new surface; and Quick Kill's fresh start, the newest
/// build. The pid stays the same, so the session manager keeps it.
fn restart() -> ! {
    // A binary replaced since it started reads as `… (deleted)`; the new
    // one is at the same path.
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_ID));
    let exe = exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")).map(PathBuf::from).unwrap_or(exe);
    let err = std::process::Command::new(&exe).args(std::env::args_os().skip(1)).exec();
    log_error!("{APP_ID}: can't start over as {}: {err}", exe.display());
    std::process::exit(1);
}
