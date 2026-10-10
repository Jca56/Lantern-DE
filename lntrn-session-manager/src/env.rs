//! The session's environment, and telling systemd and D-Bus about it.

use std::path::PathBuf;
use std::process::Command;

use crate::log::log;
use crate::paths;

/// If no D-Bus session bus address is set, re-exec ourselves under
/// `dbus-run-session` so a session bus exists for the entire desktop
/// (portals, notifications, Firefox a11y, etc.). Without this, every app that
/// touches GDBus prints "Cannot autolaunch D-Bus without X11 $DISPLAY" and
/// falls back to a per-app autolaunched bus.
pub fn ensure_session_bus() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return;
    }
    use std::os::unix::process::CommandExt;
    let self_path =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("lntrn-session-manager"));
    let err = Command::new("dbus-run-session")
        .arg("--")
        .arg(&self_path)
        .args(std::env::args().skip(1))
        .exec();
    log(&format!("🏮 FATAL: failed to re-exec under dbus-run-session: {err}"));
    std::process::exit(1);
}

/// Set what everything started from this session inherits.
pub fn set_session_vars() {
    std::env::set_var("XDG_CURRENT_DESKTOP", "Lantern");
    std::env::set_var("DESKTOP_SESSION", "lantern");
    std::env::set_var("XDG_SESSION_TYPE", "wayland");

    // NVIDIA hardware video decode for browsers (Firefox, Chromium).
    // Routes VA-API calls → NVDEC via nvidia-vaapi-driver. Autodetected via
    // /proc/driver/nvidia so this stays a no-op on the Intel/AMD laptop.
    // Requires `nvidia-vaapi-driver` package; harmless if not yet installed
    // (Firefox just falls back to software decode like it does today).
    if std::path::Path::new("/proc/driver/nvidia").exists() {
        set_default("LIBVA_DRIVER_NAME", "nvidia");
        set_default("NVD_BACKEND", "direct");
        // RDD sandbox blocks the NVIDIA driver from opening /dev/nvidia* —
        // disabling it lets the video-decode process talk to the GPU.
        set_default("MOZ_DISABLE_RDD_SANDBOX", "1");
    }

    // Ensure ~/.lantern/bin is in PATH
    if let Ok(path) = std::env::var("PATH") {
        let lantern_bin = paths::bin_dir().to_string_lossy().into_owned();
        if !path.split(':').any(|p| p == lantern_bin) {
            std::env::set_var("PATH", format!("{lantern_bin}:{path}"));
        }
    }
}

fn set_default(name: &str, value: &str) {
    if std::env::var_os(name).is_none() {
        std::env::set_var(name, value);
    }
}

/// Hand `vars`, as they are set in this process, to the systemd user manager
/// and to D-Bus, so the services they start on demand (portals, the keychain)
/// see the session they are started for.
pub fn export(vars: &[&str]) {
    // No-ops on non-systemd hosts (Gentoo/OpenRC, Artix, Void, Alpine).
    let _ = Command::new("systemctl")
        .args(["--user", "import-environment"])
        .args(vars)
        .status();
    // D-Bus activation env — no `--systemd` flag so this works on both
    // systemd hosts and plain dbus-daemon / dbus-broker setups.
    let assignments: Vec<String> = vars
        .iter()
        .filter_map(|name| Some(format!("{name}={}", std::env::var(name).ok()?)))
        .collect();
    let _ = Command::new("dbus-update-activation-environment")
        .args(&assignments)
        .status();
}
