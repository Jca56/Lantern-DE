use nix::sys::signal::{self, Signal, SigHandler, SigAction, SaFlags, SigSet};
use nix::sys::wait::{self, WaitStatus};
use nix::unistd::Pid;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".lantern").join("log").join("session.log")
}

extern "C" fn handle_signal(_: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

// ── Child process tracking ───────────────────────────────────────────────────

struct ManagedProcess {
    #[allow(dead_code)]
    name: &'static str,
    child: Child,
}

impl ManagedProcess {
    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        let pid = Pid::from_raw(self.child.id() as i32);
        let _ = signal::kill(pid, Signal::SIGTERM);
        std::thread::sleep(std::time::Duration::from_millis(500));
        if self.is_running() {
            let _ = signal::kill(pid, Signal::SIGKILL);
        }
    }

    fn spawn_wayland(
        name: &'static str,
        cmd: &str,
        wayland_display: &str,
        x11_display: Option<&str>,
    ) -> Result<Self, String> {
        let mut command = Command::new(cmd);
        command.env("WAYLAND_DISPLAY", wayland_display);
        if let Some(d) = x11_display {
            command.env("DISPLAY", d);
        }
        let child = command
            .spawn()
            .map_err(|e| format!("Failed to start {name}: {e}"))?;
        log(&format!("🏮 Started {name} (pid {})", child.id()));
        Ok(Self { name, child })
    }
}

// ── XDG Autostart ────────────────────────────────────────────────────────────

fn run_xdg_autostart() {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            PathBuf::from(home).join(".config")
        });

    let dirs = [
        config_home.join("autostart"),
        PathBuf::from("/etc/xdg/autostart"),
    ];

    let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();

    for dir in &dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            if let Some(exec) = parse_autostart_entry(&path, &current_desktop) {
                log(&format!("🏮 Autostart: {exec}"));
                use std::process::Stdio;
                let _ = Command::new("sh")
                    .args(["-c", &exec])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
    }
}

fn parse_autostart_entry(path: &std::path::Path, current_desktop: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut exec = None;
    let mut is_application = false;
    let mut hidden = false;
    let mut only_show_in: Option<Vec<&str>> = None;
    let mut not_show_in: Option<Vec<&str>> = None;
    let mut try_exec = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line != "[Desktop Entry]" {
            break;
        }
        if let Some(val) = line.strip_prefix("Exec=") {
            exec = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("Type=") {
            is_application = val.trim() == "Application";
        } else if let Some(val) = line.strip_prefix("Hidden=") {
            hidden = val.trim().eq_ignore_ascii_case("true");
        } else if let Some(val) = line.strip_prefix("OnlyShowIn=") {
            only_show_in = Some(val.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()).collect());
        } else if let Some(val) = line.strip_prefix("NotShowIn=") {
            not_show_in = Some(val.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()).collect());
        } else if let Some(val) = line.strip_prefix("TryExec=") {
            try_exec = Some(val.trim().to_string());
        }
    }

    if !is_application || hidden {
        return None;
    }

    if let Some(ref only) = only_show_in {
        if !only.iter().any(|d| d.eq_ignore_ascii_case(current_desktop)) {
            return None;
        }
    }
    if let Some(ref not) = not_show_in {
        if not.iter().any(|d| d.eq_ignore_ascii_case(current_desktop)) {
            return None;
        }
    }

    if let Some(ref te) = try_exec {
        let found = Command::new("which").arg(te)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_or(false, |s| s.success());
        if !found {
            return None;
        }
    }

    exec
}

// ── Session ──────────────────────────────────────────────────────────────────

fn log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        use std::io::Write;
        let _ = writeln!(f, "{msg}");
    }
    println!("{msg}");
}

fn main() {
    // Ensure ~/.lantern/log/ exists before any logging
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let lantern_log_dir = PathBuf::from(&home).join(".lantern").join("log");
    let _ = std::fs::create_dir_all(&lantern_log_dir);

    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())
        {
            use std::io::Write;
            let _ = writeln!(f, "{msg}");
        }
        eprintln!("{msg}");
    }));

    log("🏮 Lantern Session starting...");

    // Install signal handlers for graceful shutdown
    let sa = SigAction::new(SigHandler::Handler(handle_signal), SaFlags::empty(), SigSet::empty());
    unsafe {
        let _ = signal::sigaction(Signal::SIGTERM, &sa);
        let _ = signal::sigaction(Signal::SIGINT, &sa);
        let _ = signal::sigaction(Signal::SIGHUP, &sa);
    }

    // Set desktop environment variables
    std::env::set_var("XDG_CURRENT_DESKTOP", "Lantern");
    std::env::set_var("DESKTOP_SESSION", "lantern");
    std::env::set_var("XDG_SESSION_TYPE", "wayland");

    // Ensure ~/.lantern/bin is in PATH
    if let Ok(path) = std::env::var("PATH") {
        let home = std::env::var("HOME").unwrap_or_default();
        let lantern_bin = format!("{home}/.lantern/bin");
        if !path.split(':').any(|p| p == lantern_bin) {
            std::env::set_var("PATH", format!("{lantern_bin}:{path}"));
        }
    }

    // Push env vars into systemd user manager so services (like xdg-desktop-portal)
    // can see XDG_CURRENT_DESKTOP=Lantern for portal backend matching
    let _ = Command::new("systemctl")
        .args(["--user", "import-environment",
               "XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "XDG_SESSION_TYPE", "PATH"])
        .status();
    // Also update D-Bus activation environment
    let _ = Command::new("dbus-update-activation-environment")
        .args(["--systemd",
               "XDG_CURRENT_DESKTOP=Lantern", "DESKTOP_SESSION=lantern",
               "XDG_SESSION_TYPE=wayland"])
        .status();
    // Restart xdg-desktop-portal so it picks up the new environment
    let _ = Command::new("systemctl")
        .args(["--user", "restart", "xdg-desktop-portal"])
        .status();

    // Start the Lantern compositor
    log("🏮 Starting lntrn-compositor...");
    let compositor_log = std::fs::File::create(lantern_log_dir.join("compositor.log"))
        .expect("Failed to create compositor log file");
    let compositor_err = compositor_log.try_clone().expect("Failed to clone log file handle");

    let mut compositor = match Command::new("lntrn-compositor")
        .arg("--udev")
        .env("RUST_BACKTRACE", "1")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdout(std::process::Stdio::from(compositor_log))
        .stderr(std::process::Stdio::from(compositor_err))
        .spawn()
    {
        Ok(child) => {
            log(&format!("🏮 Started lntrn-compositor (pid {})", child.id()));
            ManagedProcess { name: "lntrn-compositor", child }
        }
        Err(e) => {
            log(&format!("FATAL: Failed to start compositor: {e}"));
            std::process::exit(1);
        }
    };

    // Wait for the compositor to create the Wayland socket.
    let wayland_socket = {
        let xdg_runtime = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| "/run/user/1000".to_string());
        let mut found = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(entries) = std::fs::read_dir(&xdg_runtime) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("wayland-") && !name_str.ends_with(".lock") {
                        found = Some(name_str.into_owned());
                    }
                }
            }
            if found.is_some() { break; }
        }
        match found {
            Some(s) => {
                log(&format!("🏮 Found Wayland socket: {s}"));
                s
            }
            None => {
                log("🏮 WARNING: No Wayland socket found, using wayland-1");
                "wayland-1".to_string()
            }
        }
    };

    // Export WAYLAND_DISPLAY to systemd/dbus so portals and dbus-activated services work
    std::env::set_var("WAYLAND_DISPLAY", &wayland_socket);
    let _ = Command::new("systemctl")
        .args(["--user", "import-environment", "WAYLAND_DISPLAY"])
        .status();
    let _ = Command::new("dbus-update-activation-environment")
        .args(["--systemd", &format!("WAYLAND_DISPLAY={wayland_socket}")])
        .status();

    // Wait for XWayland to create an X11 socket so we can pass DISPLAY
    let x11_display = {
        let mut found = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(entries) = std::fs::read_dir("/tmp/.X11-unix") {
                let mut best: Option<u32> = None;
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if let Some(num_str) = name.to_str().and_then(|n| n.strip_prefix('X')) {
                        if let Ok(n) = num_str.parse::<u32>() {
                            best = Some(best.map_or(n, |prev| prev.max(n)));
                        }
                    }
                }
                if let Some(n) = best {
                    found = Some(format!(":{n}"));
                    break;
                }
            }
        }
        match found {
            Some(d) => {
                log(&format!("🏮 Found X11 display: {d}"));
                std::env::set_var("DISPLAY", &d);
                // Push DISPLAY into systemd/dbus so launched apps inherit it
                let _ = Command::new("systemctl")
                    .args(["--user", "import-environment", "DISPLAY"])
                    .status();
                let _ = Command::new("dbus-update-activation-environment")
                    .args(["--systemd", &format!("DISPLAY={d}")])
                    .status();
                Some(d)
            }
            None => {
                log("🏮 WARNING: No X11 socket found, DISPLAY will not be set");
                None
            }
        }
    };

    // Start shell components — all tracked for clean shutdown
    let mut children: Vec<ManagedProcess> = Vec::new();
    for &(name, cmd) in &[
        ("lntrn-bar", "lntrn-bar"),
        ("lntrn-portal", "lntrn-portal"),
        ("lntrn-notifyd", "lntrn-notifyd"),
    ] {
        log(&format!("🏮 Starting {name}..."));
        match ManagedProcess::spawn_wayland(name, cmd, &wayland_socket, x11_display.as_deref()) {
            Ok(p) => children.push(p),
            Err(e) => log(&format!("WARNING: {e}")),
        }
    }

    // Run XDG autostart entries
    run_xdg_autostart();

    // Main loop: wait for the compositor to exit or signal received
    log("🏮 Lantern Session running");
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            log("🏮 Received shutdown signal");
            break;
        }

        if !compositor.is_running() {
            log("🏮 Compositor exited, shutting down session");
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    // Cleanup — kill children in reverse order, then compositor
    log("🏮 Cleaning up...");
    for child in children.iter_mut().rev() {
        child.kill();
    }
    compositor.kill();

    // Reap zombies
    loop {
        match wait::waitpid(Pid::from_raw(-1), Some(wait::WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) | Err(_) => break,
            _ => continue,
        }
    }

    println!("🏮 Lantern Session ended");
}
