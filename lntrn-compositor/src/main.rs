#![allow(irrefutable_let_patterns)]

mod cursor;
mod grabs;
mod handlers;
mod input;
mod state;
pub mod udev;
mod winit;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::Lantern;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let log_file = setup_persistent_log();
    init_logging(log_file);
    setup_panic_hook();

    let backend = parse_backend();
    tracing::info!("Starting Lantern compositor with {:?} backend", backend);

    let mut event_loop: EventLoop<Lantern> = EventLoop::try_new()?;
    let display: Display<Lantern> = Display::new()?;
    let mut state = Lantern::new(&mut event_loop, display);

    match backend {
        Backend::Winit => {
            crate::winit::init_winit(&mut event_loop, &mut state)?;

            std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
            spawn_client();

            event_loop.run(None, &mut state, move |_| {})?;
        }
        Backend::Udev => {
            std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
            spawn_client();

            crate::udev::init_udev(&mut event_loop, &mut state)?;
        }
    }

    Ok(())
}

#[derive(Debug)]
enum Backend {
    Winit,
    Udev,
}

fn parse_backend() -> Backend {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--winit" => return Backend::Winit,
            "--udev" => return Backend::Udev,
            _ => {}
        }
    }
    // Default: udev when running standalone, winit when WAYLAND_DISPLAY is set
    if std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("DISPLAY").is_ok() {
        Backend::Winit
    } else {
        Backend::Udev
    }
}

fn setup_persistent_log() -> Option<std::fs::File> {
    let state_dir = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(home).join(".local/state")
        })
        .join("lantern");

    if std::fs::create_dir_all(&state_dir).is_err() {
        return None;
    }

    std::fs::File::create(state_dir.join("compositor.log")).ok()
}

fn init_logging(log_file: Option<std::fs::File>) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if let Some(file) = log_file {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(file)
            .with_ansi(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }
}

fn setup_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("PANIC: {}", info);
        default_hook(info);
    }));
}

fn spawn_client() {
    let mut args = std::env::args().skip(1);
    let flag = args.next();
    let arg = args.next();

    match (flag.as_deref(), arg) {
        (Some("-c") | Some("--command"), Some(command)) => {
            std::process::Command::new(command).spawn().ok();
        }
        _ => {}
    }
}
