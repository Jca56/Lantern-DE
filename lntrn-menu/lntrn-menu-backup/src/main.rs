mod dbus;
mod layershell;
mod sni;
mod tray;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init()
        .ok();

    // Parse cursor position: lntrn-menu <x> <y>
    let args: Vec<String> = std::env::args().collect();
    let (cx, cy) = if args.len() >= 3 {
        let x: f64 = args[1].parse().unwrap_or(0.0);
        let y: f64 = args[2].parse().unwrap_or(0.0);
        (x, y)
    } else {
        // Center of screen fallback
        (0.0, 0.0)
    };

    layershell::run(cx, cy)
}
