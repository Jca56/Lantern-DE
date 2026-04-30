//! lntrn-command-center — drop-down panel for Lantern DE.
//!
//! Single-instance daemon. First invocation binds the IPC socket and
//! runs the layer-shell loop; subsequent invocations send a one-byte
//! command (`T`/`S`/`H`) and exit. See `ipc.rs`.

mod app;
mod controls;
mod ipc;
mod launcher;
mod layershell;
mod render;
mod search;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mode = parse_args();
    let msg = mode.as_command();

    // Send to existing daemon if there is one.
    match ipc::send(msg) {
        Ok(true) => {
            tracing::info!(?mode, "forwarded to running daemon");
            return;
        }
        Ok(false) => {
            tracing::info!(?mode, "no daemon found — becoming daemon");
        }
        Err(e) => {
            tracing::warn!(?e, "ipc::send failed — becoming daemon anyway");
        }
    }

    // Become the daemon.
    let sock = match ipc::bind_daemon() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(?e, "failed to bind IPC socket");
            eprintln!("error: failed to bind IPC socket: {e}");
            std::process::exit(1);
        }
    };

    // Daemon starts hidden by default; first --show or --toggle opens
    // the panel. (The compositor only ever shells out --toggle.)
    let initial_visible = matches!(mode, Mode::Show | Mode::Toggle);

    if let Err(e) = layershell::run(sock, initial_visible) {
        tracing::error!(?e, "command-center daemon crashed");
        eprintln!("error: {e}");
        ipc::cleanup();
        std::process::exit(1);
    }

    ipc::cleanup();
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Toggle,
    Show,
    Hide,
}

impl Mode {
    fn as_command(&self) -> &'static [u8] {
        match self {
            Mode::Toggle => ipc::cmd::TOGGLE,
            Mode::Show => ipc::cmd::SHOW,
            Mode::Hide => ipc::cmd::HIDE,
        }
    }
}

fn parse_args() -> Mode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--show") => Mode::Show,
        Some("--hide") => Mode::Hide,
        Some("--toggle") | None => Mode::Toggle,
        Some(other) => {
            eprintln!("unknown flag: {other}");
            eprintln!("usage: lntrn-command-center [--toggle|--show|--hide]");
            std::process::exit(2);
        }
    }
}
