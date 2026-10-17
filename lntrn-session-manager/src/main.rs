mod autostart;
mod backoff;
mod children;
mod env;
mod exit;
mod log;
mod panel;
mod paths;
mod procfs;
mod sockets;
#[cfg(test)]
mod testdir;

use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use children::Children;
use log::log;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// The shell daemons, in the order they start. Each one stays in the
/// foreground, so the process started here is the daemon itself: that is what
/// gets restarted if it dies and stopped at logout.
///
/// The panel, the Control Center or the Command Center, starts after them,
/// whichever `[session] panel` chooses (`panel.rs`), and the other takes its
/// place when that changes. The Control Center (the bar) exits uncleanly
/// when the compositor takes its surface away with its monitor, so it is
/// started again too; its own restarts (a new primary monitor, Quick Kill)
/// exec in place, keeping its pid.
const DAEMONS: &[(&str, &[&str])] = &[
    ("lntrn-desktop", &["--foreground"]),
    ("lntrn-portal", &[]),
    ("lntrn-notifyd", &[]),
];

/// How long the compositor gets to open its Wayland socket. It does so first
/// thing, before it touches the GPU; a compositor that dies ends the wait at
/// once, so this only bounds one that hangs.
const WAYLAND_WAIT: Duration = Duration::from_secs(10);
/// How long XWayland's display gets to appear after that.
const X11_WAIT: Duration = Duration::from_secs(5);
/// How long the daemons, then the compositor, get to quit when asked before
/// they are killed.
const DAEMON_GRACE: Duration = Duration::from_secs(2);
const COMPOSITOR_GRACE: Duration = Duration::from_secs(5);

fn main() {
    env::ensure_session_bus();

    // Ensure ~/.lantern/log/ exists before any logging
    let log_dir = paths::log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    log::init(&log_dir);
    std::panic::set_hook(Box::new(|info| log(&format!("PANIC: {info}"))));

    log("🏮 Lantern Session starting...");

    // Install signal handlers for graceful shutdown
    let sa = SigAction::new(
        SigHandler::Handler(handle_signal),
        SaFlags::empty(),
        SigSet::empty(),
    );
    unsafe {
        let _ = signal::sigaction(Signal::SIGTERM, &sa);
        let _ = signal::sigaction(Signal::SIGINT, &sa);
        let _ = signal::sigaction(Signal::SIGHUP, &sa);
    }

    // Push env vars into the systemd user manager and D-Bus so the services
    // they activate (xdg-desktop-portal etc.) see XDG_CURRENT_DESKTOP=Lantern.
    env::set_session_vars();
    env::export(&["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "XDG_SESSION_TYPE", "PATH"]);

    // Start the Lantern compositor
    let mut children = Children::new(log_dir);
    let compositor_path = paths::bin_dir().join("lntrn-compositor");
    log("🏮 Starting lntrn-compositor...");
    log(&format!("🏮 Compositor binary: {}", compositor_path.display()));
    let compositor = match children.start_compositor(&compositor_path) {
        Ok(pid) => pid,
        Err(e) => {
            log(&format!("FATAL: Failed to start compositor: {e}"));
            std::process::exit(1);
        }
    };
    log(&format!("🏮 Started lntrn-compositor (pid {compositor})"));

    // Wait for the compositor to create the Wayland socket.
    let runtime_dir = paths::runtime_dir();
    let wayland_socket = sockets::wait_for_wayland(&runtime_dir, compositor, WAYLAND_WAIT, &mut || {
        still_starting(&mut children)
    });
    let wayland_socket = match wayland_socket {
        Some(name) => {
            log(&format!("🏮 Found Wayland socket: {name}"));
            name
        }
        None => {
            give_up_if_startup_ended(&mut children);
            // The compositor is up but no socket could be shown to be its
            // own. Guess, as this always used to, rather than lose the login.
            let guess = sockets::wayland_sockets(&runtime_dir)
                .pop()
                .unwrap_or_else(|| "wayland-1".to_string());
            log(&format!(
                "🏮 WARNING: lntrn-compositor has not opened a Wayland socket after {WAYLAND_WAIT:?}, guessing {guess}"
            ));
            guess
        }
    };

    // Export WAYLAND_DISPLAY to systemd/dbus so portals and dbus-activated services work
    std::env::set_var("WAYLAND_DISPLAY", &wayland_socket);
    env::export(&["WAYLAND_DISPLAY"]);

    // Wait for XWayland to create an X11 socket so we can pass DISPLAY
    if sockets::xwayland_disabled() {
        log("🏮 XWayland is switched off, DISPLAY will not be set");
    } else {
        let display = sockets::wait_for_x11(compositor, X11_WAIT, &mut || {
            still_starting(&mut children)
        });
        match display {
            Some(display) => {
                log(&format!("🏮 Found X11 display: {display}"));
                // Push DISPLAY into systemd/dbus so launched apps inherit it
                std::env::set_var("DISPLAY", &display);
                env::export(&["DISPLAY"]);
            }
            None => {
                give_up_if_startup_ended(&mut children);
                log("🏮 WARNING: No X11 socket found, DISPLAY will not be set");
            }
        }
    }

    // xdg-desktop-portal belongs to the systemd user manager, not to this
    // session, and outlives it. One left over from an earlier login still has
    // that login's environment and settings, so it is stopped here: the new
    // environment is exported and nothing is using it yet. D-Bus starts a
    // fresh one the first time a portal is asked for.
    if procfs::terminate_named("xdg-desktop-portal", Duration::from_secs(1)) > 0 {
        log("🏮 Stopped the previous xdg-desktop-portal");
    }

    // Start shell components — all tracked for clean shutdown
    for &(name, args) in DAEMONS {
        log(&format!("🏮 Starting {name}..."));
        children.start_daemon(name, args);
    }
    let mut panel = panel::Follow::new(panel::config_path());
    let (name, args) = panel.panel.daemon();
    log(&format!("🏮 Starting {name} (the panel)..."));
    children.start_daemon(name, args);

    // Run XDG autostart entries
    autostart::run(&mut children);

    // Main loop: wait for the compositor to exit or signal received
    log("🏮 Lantern Session running");
    loop {
        children.reap();
        if SHUTDOWN.load(Ordering::SeqCst) {
            log("🏮 Received shutdown signal");
            break;
        }
        if let Some(exit) = children.compositor_exit() {
            log(&format!("🏮 Compositor ended ({exit}), shutting down session"));
            break;
        }
        children.restart_due(Instant::now());
        if let Some(chosen) = panel.changed() {
            switch_panel(&mut children, chosen);
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    shut_down(&mut children);
}

/// System Settings chose the other panel: stop the one running (and any
/// copy of it the session didn't start, such as a Command Center a Super tap
/// started before it was supervised), then start the chosen one.
fn switch_panel(children: &mut Children, chosen: panel::Panel) {
    let (old, _) = chosen.other().daemon();
    let (new, args) = chosen.daemon();
    log(&format!("🏮 The panel is now {new}: stopping {old}"));
    children.stop_daemon(old, DAEMON_GRACE);
    if procfs::terminate_named(old, DAEMON_GRACE) > 0 {
        log(&format!("🏮 Stopped a {old} the session hadn't started"));
    }
    children.start_daemon(new, args);
}

/// Whether to go on waiting for the compositor to come up: it is still
/// running, and nobody has asked the session to stop.
fn still_starting(children: &mut Children) -> bool {
    children.reap();
    children.compositor_running() && !SHUTDOWN.load(Ordering::SeqCst)
}

/// End the session here if the compositor died while it was starting, or a
/// signal asked for that. Nothing but the compositor has been started yet.
fn give_up_if_startup_ended(children: &mut Children) {
    if let Some(exit) = children.compositor_exit() {
        log(&format!(
            "FATAL: lntrn-compositor ended while starting ({exit}); see {}",
            paths::log_dir().join("compositor.log").display()
        ));
        std::process::exit(1);
    }
    if SHUTDOWN.load(Ordering::SeqCst) {
        log("🏮 Received shutdown signal");
        shut_down(children);
    }
}

/// Stop everything that is still running, the compositor last, and exit.
fn shut_down(children: &mut Children) -> ! {
    log("🏮 Cleaning up...");
    children.stop_daemons(DAEMON_GRACE);
    children.stop_compositor(COMPOSITOR_GRACE);
    log("🏮 Lantern Session ended");
    std::process::exit(0);
}
