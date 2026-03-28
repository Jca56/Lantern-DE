/// On-demand XWayland spawning and lifecycle management.

use smithay::reexports::wayland_server::Client;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::Lantern;

/// XWayland runtime state stored in the compositor.
pub struct XWaylandState {
    /// The X11 window manager, set once XWayland is ready.
    pub wm: Option<X11Wm>,
    /// The display number (e.g. 0 for `:0`).
    pub display_number: Option<u32>,
}

impl XWaylandState {
    pub fn new() -> Self {
        Self {
            wm: None,
            display_number: None,
        }
    }
}

/// Spawn XWayland and hook it into the event loop.
/// XWayland starts immediately but the X11Wm is only created when
/// XWayland signals readiness via `XWaylandEvent::Ready`.
pub fn start_xwayland(state: &mut Lantern) {
    let dh = state.display_handle.clone();

    let (xwayland, client) = match XWayland::spawn(
        &dh,
        None::<u32>, // auto-pick display number
        std::iter::empty::<(String, String)>(),
        true,  // open abstract socket (Linux)
        std::process::Stdio::null(),
        std::process::Stdio::null(),
        |_user_data| {},
    ) {
        Ok(pair) => pair,
        Err(err) => {
            tracing::error!("Failed to spawn XWayland: {}", err);
            return;
        }
    };

    // We need to move the Client into the Ready callback.
    // Use an Option so we can take() it on first Ready event.
    let client = std::cell::RefCell::new(Some(client));

    let result = state
        .loop_handle
        .insert_source(xwayland, move |event, _, state: &mut Lantern| {
            match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    let Some(client) = client.borrow_mut().take() else {
                        tracing::warn!("XWayland Ready fired more than once");
                        return;
                    };
                    handle_xwayland_ready(state, x11_socket, display_number, client);
                }
                XWaylandEvent::Error => {
                    tracing::error!("XWayland process failed to start");
                }
            }
        });

    if let Err(err) = result {
        tracing::error!("Failed to insert XWayland event source: {}", err);
    } else {
        tracing::info!("XWayland spawned, waiting for readiness...");
    }
}

fn handle_xwayland_ready(
    state: &mut Lantern,
    x11_socket: std::os::unix::net::UnixStream,
    display_number: u32,
    client: Client,
) {
    let wm = match X11Wm::start_wm(state.loop_handle.clone(), x11_socket, client) {
        Ok(wm) => wm,
        Err(err) => {
            tracing::error!("Failed to start X11 window manager: {}", err);
            return;
        }
    };

    state.xwayland_state.wm = Some(wm);
    state.xwayland_state.display_number = Some(display_number);

    // Set DISPLAY so child processes can find XWayland
    let display_str = format!(":{}", display_number);
    std::env::set_var("DISPLAY", &display_str);
    tracing::info!("XWayland ready on {}", display_str);

    // Spawn any -c/--command client now that DISPLAY is set.
    spawn_client();
}

fn spawn_client() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "-c" || args[i] == "--command") && i + 1 < args.len() {
            let command = &args[i + 1];
            tracing::info!("spawning client: {command}");
            std::process::Command::new(command).spawn().ok();
            return;
        }
        i += 1;
    }
}
