//! # ⚠️  DEPRECATED — lntrn-bar is no longer the Lantern shell bar.
//!
//! The **command-center** (`lntrn-command-center`) replaced this component.
//! It is launched by the compositor (Super-tap / hot corners) and handles the
//! clock, app tray, audio, system info, and everything this crate used to do.
//!
//! `lntrn-bar` is **not** auto-started by anything anymore — no XDG autostart
//! entry, no session-manager spawn. It only runs if launched by hand, and when
//! it does it defaults to the bottom edge.
//!
//! Kept in the workspace for reference / salvageable widgets, but do not wire it
//! back into the session. If you're sure it's dead, delete the crate (recover
//! via git). See the loud warning printed by `main()` on launch.

mod appmenu;
mod bar_settings;
mod apptray;
mod audio;
mod battery;
mod bluetooth;
mod bluetooth_worker;
mod bluetooth_send;
mod bluetooth_transfer;
mod bluetooth_ui;
mod clock;
mod dbusmenu;
mod desktop;
mod hover;
mod lava;
mod mpris;
mod layershell;
mod sni;
mod svg_icon;
mod temperature;
mod theme_state;
mod toplevel;
mod tray;
mod wifi;
mod workspaces;

use std::path::PathBuf;

/// Returns `~/.lantern`, the root of the Lantern home directory.
pub(crate) fn lantern_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern")
}

/// Returns `~/.lantern/config/bar/` — bar-specific config directory.
pub(crate) fn bar_config_dir() -> PathBuf {
    lantern_home().join("config/bar")
}

/// Returns `~/.lantern/icons/` — shared icon directory.
pub(crate) fn lantern_icons_dir() -> PathBuf {
    lantern_home().join("icons")
}

fn main() -> anyhow::Result<()> {
    // ⚠️ Loud deprecation notice — lntrn-bar was replaced by the command-center.
    // Printed to stderr so it shows up no matter how the binary was launched.
    eprintln!("\n\x1b[1;33m╔══════════════════════════════════════════════════════════════╗");
    eprintln!("║  ⚠️  lntrn-bar is DEPRECATED                                  ║");
    eprintln!("║  The command-center replaced it (Super-tap / hot corners).   ║");
    eprintln!("║  Nothing auto-starts this anymore. You launched it by hand.  ║");
    eprintln!("╚══════════════════════════════════════════════════════════════╝\x1b[0m\n");

    // Write logs to ~/.lantern/log/lntrn-bar.log (truncated each session)
    // so we can diagnose startup hangs even when stdout is unattached.
    let log_dir = lantern_home().join("log");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("lntrn-bar.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .ok();

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if let Some(file) = log_file {
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .with_env_filter(env_filter)
            .try_init()
            .ok();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .try_init()
            .ok();
    }

    tracing::warn!("lntrn-bar is DEPRECATED — replaced by the command-center. Nothing auto-starts it.");

    layershell::run()
}
