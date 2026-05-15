mod assets;
mod audio;
mod clock;
mod visualizer;
mod context_menu;
mod dispatch;
mod fs_watch;
mod icons;
mod input;
mod keyboard;
mod layout;
mod rainbow;
mod render;
mod state;
mod wayland;
mod widgets_config;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LNTRN_DESKTOP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Daemonize so the desktop survives terminal close.
    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            std::process::exit(1);
        }
        if pid > 0 {
            std::process::exit(0);
        }
        libc::setsid();
    }

    if let Err(e) = wayland::run() {
        tracing::error!("[desktop] fatal: {e:#}");
        std::process::exit(1);
    }
}
