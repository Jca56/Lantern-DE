mod appmenu;
mod bar_settings;
mod apptray;
mod audio;
mod battery;
mod bluetooth;
mod bluetooth_worker;
mod bluetooth_transfer;
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

    layershell::run()
}
