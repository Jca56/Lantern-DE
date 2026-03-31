mod appmenu;
mod bar_settings;
mod apptray;
mod audio;
mod battery;
mod bluetooth;
mod clock;
mod dbus;
mod dbusmenu;
mod desktop;
mod hover;
mod lava;
mod layershell;
mod sni;
mod svg_icon;
mod temperature;
mod toplevel;
mod tray;
mod wifi;

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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init()
        .ok();

    layershell::run()
}
