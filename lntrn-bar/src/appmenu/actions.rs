//! App actions — launch, uninstall, and related helpers.

use crate::desktop;

pub(crate) fn launch_app(exec: &str) {
    let parts: Vec<&str> = exec.split_whitespace().collect();
    if parts.is_empty() { return; }
    let mut cmd = std::process::Command::new("systemd-run");
    cmd.arg("--user").arg("--scope");

    if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
        cmd.arg(format!("--setenv=WAYLAND_DISPLAY={val}"));
    }
    let display = std::env::var("DISPLAY").ok().or_else(detect_x11_display);
    if let Some(val) = display {
        cmd.arg(format!("--setenv=DISPLAY={val}"));
    }

    cmd.arg("--");
    cmd.args(&parts);
    match cmd.spawn() {
        Ok(_) => tracing::info!("launched: {exec}"),
        Err(e) => tracing::error!("failed to launch {exec}: {e}"),
    }
}

pub(super) fn uninstall_app(app_id: &str) {
    // Collect all .desktop file paths for this app
    let desktop_filename = format!("{app_id}.desktop");
    let mut desktop_paths = Vec::new();
    let user_dir = desktop::dirs::data_home().join("applications");
    let user_path = user_dir.join(&desktop_filename);
    if user_path.exists() {
        desktop_paths.push(user_path);
    }
    for dir in desktop::data_dirs() {
        let p = dir.join("applications").join(&desktop_filename);
        if p.exists() {
            desktop_paths.push(p);
        }
    }

    if desktop_paths.is_empty() {
        tracing::warn!("no .desktop files found for {app_id}");
        return;
    }

    // Check if any desktop file belongs to a pacman package
    let pkg = desktop_paths.iter().find_map(|path| {
        let output = std::process::Command::new("pacman")
            .args(["-Qo", &path.to_string_lossy()])
            .output()
            .ok()?;
        if !output.status.success() { return None; }
        // Output: "/path/file is owned by <package> <version>"
        let out = String::from_utf8_lossy(&output.stdout);
        let words: Vec<&str> = out.split_whitespace().collect();
        words.windows(3).find_map(|w| {
            if w[0] == "owned" && w[1] == "by" { Some(w[2].to_string()) } else { None }
        })
    });

    if let Some(pkg) = pkg {
        // Pacman package — use pkexec for graphical sudo
        tracing::info!("uninstalling pacman package: {pkg}");
        let _ = std::process::Command::new("pkexec")
            .args(["pacman", "-Rs", "--noconfirm", &pkg])
            .spawn();
    } else {
        // Not a package — remove desktop file(s) and binary
        uninstall_unpackaged(app_id, &desktop_paths);
    }
}

/// Remove a non-packaged app: delete .desktop file(s) and the Exec binary.
fn uninstall_unpackaged(app_id: &str, desktop_paths: &[std::path::PathBuf]) {
    // Parse Exec= from the first desktop file to find the binary
    let binary = desktop_paths.first().and_then(|p| {
        let content = std::fs::read_to_string(p).ok()?;
        content.lines().find_map(|line| {
            let val = line.strip_prefix("Exec=")?;
            let bin = val.split_whitespace().next()?;
            let path = std::path::PathBuf::from(bin);
            path.exists().then_some(path)
        })
    });

    // Try removing files as current user first, track failures for pkexec
    let mut need_root = false;
    for path in desktop_paths {
        if std::fs::remove_file(path).is_ok() {
            tracing::info!("removed {}", path.display());
        } else {
            need_root = true;
        }
    }

    if let Some(ref bin) = binary {
        if std::fs::remove_file(bin).is_ok() {
            tracing::info!("removed binary {}", bin.display());
        } else {
            need_root = true;
        }
    }

    // Batch remaining files that need root
    if need_root {
        let mut root_paths: Vec<String> = Vec::new();
        for path in desktop_paths {
            if path.exists() {
                root_paths.push(path.to_string_lossy().to_string());
            }
        }
        if let Some(ref bin) = binary {
            if bin.exists() {
                root_paths.push(bin.to_string_lossy().to_string());
            }
        }
        if !root_paths.is_empty() {
            let mut args = vec!["rm".to_string(), "-f".to_string()];
            args.extend(root_paths);
            tracing::info!("pkexec rm: {:?}", args);
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let _ = std::process::Command::new("pkexec").args(&arg_refs).spawn();
        }
    }

    tracing::info!("uninstalled non-packaged app: {app_id}");
}

fn detect_x11_display() -> Option<String> {
    let dir = std::fs::read_dir("/tmp/.X11-unix/").ok()?;
    let mut best: Option<u32> = None;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name = name.to_str()?;
        if let Some(num_str) = name.strip_prefix('X') {
            if let Ok(n) = num_str.parse::<u32>() {
                best = Some(best.map_or(n, |prev: u32| prev.max(n)));
            }
        }
    }
    best.map(|n| format!(":{n}"))
}
