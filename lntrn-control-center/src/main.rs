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
//! - `calendar/` — the clock as the calendar's face: its popup (the month,
//!   the day's events, the form making them), the events' file, and the
//!   thread saying their reminders.
//! - `tray/` — the system tray before the clock: battery, WiFi,
//!   Bluetooth, sound, brightness, the workspace's number and Game Mode,
//!   and their popups (`lntrn-status` keeps them current).
//! - `game_mode.rs` — the line to the compositor's Game Mode.
//! - `launcher/` — the app launcher: its button at the bar's left end and
//!   the menu it opens above the bar.
//! - `monitors/` — the mini monitors after it: the processors, the
//!   graphics card and the memory graphed, and their popups (`lntrn-status`
//!   looks at the machine).
//! - `dock/` — the dock in the middle: pinned and running apps, their
//!   cards with live thumbnails, their menus.
//! - `compositor.rs` — the line to the compositor: whether a window fills
//!   our monitor (to dock), the open windows, our monitor's workspaces,
//!   and the dock's and the tray's commands.
//! - `idlist.rs` — the favorites' and the pins' files.
//! - `hide.rs` — auto hide: sliding out of sight and back, the windows
//!   giving way to as much of the bar as shows.
//! - `morph.rs` — the plate's glides: floating and docked, hidden and
//!   shown.
//! - `glass.rs` — where the popups are painted see-through, for the
//!   compositor to blur behind.
//! - `settings.rs` — what it reads from `lantern.toml`, and the thread
//!   that watches the file.
//! - `instance.rs` — one bar at a time.
//!
//! The session manager keeps it running: a crash, and it is started
//! again. The compositor taking the bar away with its monitor is no crash:
//! it waits for a monitor, then starts over in place (the same pid), so
//! no monitor for a long while never uses up the session's restarts.

mod app;
mod bar;
mod calendar;
mod clipboard;
mod clock;
mod compositor;
mod dock;
mod game_mode;
mod glass;
mod hide;
mod idlist;
mod instance;
mod launcher;
mod monitors;
mod morph;
mod settings;
mod thumbs;
#[cfg(test)]
mod smoke;
mod tray;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use lntrn_app::{AppConfig, run};
use lntrn_core::{log_error, log_info, log_warn};
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};
use crate::settings::Settings;

/// Between looks for a monitor while there is none; at most, the wait
/// before starting over after being taken away again soon after starting.
const LOOK: Duration = Duration::from_secs(2);
const MOST: Duration = Duration::from_secs(30);
/// Up this long, a bar counts as one that stayed.
const STAYED: Duration = Duration::from_secs(60);
/// How many times in a row the bar was taken away soon after it started,
/// handed to the next start.
const AWAY: &str = "LNTRN_CC_TAKEN_AWAY";

fn main() {
    lntrn_kit::startup::log_panics(APP_ID);
    let Some(lock) = instance::claim(APP_ID) else {
        log_info!("{APP_ID}: already running");
        return;
    };
    let settings = Settings::read().unwrap_or_default();
    let output = bar::home(settings.primary.as_deref());
    log_info!("{APP_ID}: on {}", output.as_deref().unwrap_or("the compositor's pick"));
    lntrn_kit::look::wear(bar::PALETTE);
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
    let started = Instant::now();
    run(config, App::new(settings, output), shell);
    if app::restart_wanted() {
        drop(lock);
        restart(None);
    }
    if app::taken_away() {
        // In a row: since it last stayed up a while. Each wait is longer.
        let times = if started.elapsed() > STAYED { 0 } else { std::env::var(AWAY).ok().and_then(|v| v.parse().ok()).unwrap_or(0) };
        let wait = (LOOK * 2u32.pow(times.min(4))).min(MOST);
        log_warn!("{APP_ID}: the compositor took the bar away; starting over in {}s, once there's a monitor", wait.as_secs());
        std::thread::sleep(wait);
        while !bar::any_monitor() {
            std::thread::sleep(LOOK);
        }
        drop(lock);
        restart(Some(times + 1));
    }
    std::process::exit(0);
}

/// Start over as a fresh process with the same arguments: a layer
/// surface's monitor is fixed when it is made, so moving to a new primary
/// monitor takes a new surface; Quick Kill's fresh start, the newest
/// build; and a bar the compositor took away. The pid stays the same, so
/// the session manager keeps it. `away`: how many times in a row it was
/// taken away.
fn restart(away: Option<u32>) -> ! {
    // A binary replaced since it started reads as `… (deleted)`; the new
    // one is at the same path.
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_ID));
    let exe = exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")).map(PathBuf::from).unwrap_or(exe);
    let mut command = std::process::Command::new(&exe);
    command.args(std::env::args_os().skip(1));
    match away {
        Some(times) => command.env(AWAY, times.to_string()),
        None => command.env_remove(AWAY),
    };
    let err = command.exec();
    log_error!("{APP_ID}: can't start over as {}: {err}", exe.display());
    std::process::exit(1);
}
