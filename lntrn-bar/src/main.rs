mod appmenu;
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
