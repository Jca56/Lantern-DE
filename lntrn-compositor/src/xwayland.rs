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

    // Resolve the cursor theme + size from env (falls back to compositor config
    // / default) so XWayland and its child X11 clients use our cursor theme
    // instead of the tiny black X11 root cursor.
    // Use the Lantern theme written to disk by xcursor_export so X11
    // clients render our actual SVG cursors. Honors a user-set XCURSOR_THEME
    // override if they want to point at a system theme instead.
    let xcursor_theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Lantern".to_string());
    let xcursor_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .or_else(|| {
            Some(
                crate::input::read_input_setting_f64("cursor_size", 24.0)
                    .round()
                    .to_string(),
            )
        })
        .unwrap_or_else(|| "24".to_string());
    // libxcursor's compiled-in search path on Gentoo (and most non-XDG-aware
    // builds) is `~/.icons:/usr/share/icons:/usr/share/pixmaps` — it does NOT
    // include `$XDG_DATA_HOME/icons`, which is exactly where we wrote the
    // Lantern theme. Build an explicit XCURSOR_PATH so XWayland can find it.
    let xcursor_path = build_xcursor_path();
    std::env::set_var("XCURSOR_THEME", &xcursor_theme);
    std::env::set_var("XCURSOR_SIZE", &xcursor_size);
    std::env::set_var("XCURSOR_PATH", &xcursor_path);

    let xwayland_env = vec![
        ("XCURSOR_THEME".to_string(), xcursor_theme),
        ("XCURSOR_SIZE".to_string(), xcursor_size),
        ("XCURSOR_PATH".to_string(), xcursor_path),
    ];

    let (xwayland, client) = match XWayland::spawn(
        &dh,
        None::<u32>, // auto-pick display number
        xwayland_env,
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
            if state.debug_counters.enabled {
                state.debug_counters.xwayland_fires += 1;
            }
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

    // Push DISPLAY (and friends) into the dbus + systemd --user activation
    // environment so Flatpak / dbus-activated apps see it. Without this,
    // Steam Flatpak refuses to start with a "DISPLAY not in activation env" error.
    push_activation_environment();

    // Spawn any -c/--command client now that DISPLAY is set.
    spawn_client();
}

/// Push session vars into the dbus + systemd --user activation environment.
/// Best-effort: logs on failure but never blocks compositor startup.
fn push_activation_environment() {
    let vars = [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_TYPE",
        "XCURSOR_THEME",
        "XCURSOR_SIZE",
        "XCURSOR_PATH",
    ];
    let mut cmd = std::process::Command::new("dbus-update-activation-environment");
    cmd.arg("--systemd").args(vars);
    match cmd.status() {
        Ok(status) if status.success() => {
            tracing::info!("dbus-update-activation-environment: pushed session vars");
        }
        Ok(status) => {
            tracing::warn!("dbus-update-activation-environment exited with {status}");
        }
        Err(err) => {
            tracing::warn!("dbus-update-activation-environment not available: {err}");
        }
    }
}

/// Build a colon-separated XCURSOR_PATH that starts with our generated
/// Lantern theme directory and includes the conventional system fallbacks.
/// Respects $XDG_DATA_HOME and $XDG_DATA_DIRS for systems where they differ.
fn build_xcursor_path() -> String {
    let mut entries: Vec<String> = Vec::new();
    let mut push = |p: String| {
        if !p.is_empty() && !entries.contains(&p) {
            entries.push(p);
        }
    };

    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        push(home.join(".icons").display().to_string());
    }

    let xdg_data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".local").join("share"))
        });
    if let Some(p) = xdg_data_home {
        push(p.join("icons").display().to_string());
    }

    let xdg_data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for dir in xdg_data_dirs.split(':') {
        if dir.is_empty() {
            continue;
        }
        push(format!("{dir}/icons"));
    }

    push("/usr/share/pixmaps".to_string());

    entries.join(":")
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
