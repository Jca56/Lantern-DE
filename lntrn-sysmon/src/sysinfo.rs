use std::fs;
use std::path::Path;

pub struct SystemInfo {
    pub entries: Vec<(&'static str, String)>,
}

impl SystemInfo {
    pub fn gather() -> Self {
        let mut e = Vec::new();

        e.push(("Hostname", read_line("/etc/hostname")));
        e.push(("OS", os_name()));
        e.push(("Kernel", read_line("/proc/sys/kernel/osrelease")));
        e.push(("Uptime", uptime()));
        e.push(("Shell", shell()));
        e.push(("DE/WM", "Lantern".into()));
        e.push(("Display", "Wayland".into()));
        e.push(("Resolution", resolution()));
        e.push(("CPU", cpu_model()));
        e.push(("Cores", cpu_cores()));
        e.push(("GPU", gpu_name()));
        e.push(("Memory", memory()));
        e.push(("Swap", swap()));
        e.push(("Disk", disk_usage()));
        e.push(("Battery", battery()));
        e.push(("Packages", packages()));
        e.push(("Motherboard", motherboard()));

        Self { entries: e }
    }
}

fn read_line(path: &str) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn os_name() -> String {
    let release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    for line in release.lines() {
        if let Some(name) = line.strip_prefix("PRETTY_NAME=") {
            return name.trim_matches('"').to_string();
        }
    }
    "Linux".into()
}

fn uptime() -> String {
    let secs: f64 = read_line("/proc/uptime")
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let h = (secs / 3600.0) as u64;
    let m = ((secs % 3600.0) / 60.0) as u64;
    if h > 24 {
        let d = h / 24;
        format!("{}d {}h {}m", d, h % 24, m)
    } else {
        format!("{}h {}m", h, m)
    }
}

fn shell() -> String {
    std::env::var("SHELL")
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

fn resolution() -> String {
    // Try to read from DRM
    let drm = Path::new("/sys/class/drm");
    if let Ok(entries) = fs::read_dir(drm) {
        for entry in entries.flatten() {
            let modes = entry.path().join("modes");
            if modes.exists() {
                if let Ok(content) = fs::read_to_string(&modes) {
                    if let Some(first) = content.lines().next() {
                        if !first.is_empty() {
                            return first.to_string();
                        }
                    }
                }
            }
        }
    }
    "unknown".into()
}

fn cpu_model() -> String {
    let info = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    for line in info.lines() {
        if line.starts_with("model name") {
            if let Some((_, val)) = line.split_once(':') {
                let name = val.trim().to_string();
                // Shorten common prefixes
                return name
                    .replace("Intel(R) Core(TM) ", "Intel ")
                    .replace("AMD Ryzen ", "Ryzen ");
            }
        }
    }
    "unknown".into()
}

fn cpu_cores() -> String {
    let info = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let logical = info.lines().filter(|l| l.starts_with("processor")).count();
    // Try to get physical cores
    let mut physical = 0u32;
    for line in info.lines() {
        if line.starts_with("cpu cores") {
            if let Some((_, val)) = line.split_once(':') {
                physical = val.trim().parse().unwrap_or(0);
                break;
            }
        }
    }
    if physical > 0 && logical > 0 {
        format!("{} cores / {} threads", physical, logical)
    } else {
        format!("{} threads", logical)
    }
}

fn gpu_name() -> String {
    let drm = Path::new("/sys/class/drm");
    if let Ok(entries) = fs::read_dir(drm) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Match "card0", "card1", etc. (not "card0-DP-1")
            if !name_str.starts_with("card") || name_str.contains('-') { continue; }
            let dev = entry.path().join("device");
            if let Ok(uevent) = fs::read_to_string(dev.join("uevent")) {
                let mut driver = "";
                let mut pci_id = "";
                for line in uevent.lines() {
                    if let Some(d) = line.strip_prefix("DRIVER=") { driver = d.trim(); }
                    if let Some(p) = line.strip_prefix("PCI_ID=") { pci_id = p.trim(); }
                }
                if !driver.is_empty() {
                    let vendor = match pci_id.split(':').next().unwrap_or("") {
                        "8086" => "Intel",
                        "1002" => "AMD",
                        "10DE" | "10de" => "NVIDIA",
                        _ => "GPU",
                    };
                    return format!("{} ({})", vendor, driver);
                }
            }
        }
    }
    "unknown".into()
}

fn memory() -> String {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total_kb = 0u64;
    let mut avail_kb = 0u64;
    for line in info.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = parse_meminfo_kb(line);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = parse_meminfo_kb(line);
        }
    }
    let used_mb = (total_kb - avail_kb) / 1024;
    let total_mb = total_kb / 1024;
    format!("{} MiB / {} MiB", used_mb, total_mb)
}

fn swap() -> String {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total_kb = 0u64;
    let mut free_kb = 0u64;
    for line in info.lines() {
        if line.starts_with("SwapTotal:") {
            total_kb = parse_meminfo_kb(line);
        } else if line.starts_with("SwapFree:") {
            free_kb = parse_meminfo_kb(line);
        }
    }
    if total_kb == 0 { return "None".into(); }
    let used_mb = (total_kb - free_kb) / 1024;
    let total_mb = total_kb / 1024;
    format!("{} MiB / {} MiB", used_mb, total_mb)
}

fn parse_meminfo_kb(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn disk_usage() -> String {
    // Read /proc/mounts to find root filesystem, then statvfs
    let stat = fs::read_to_string("/proc/mounts").unwrap_or_default();
    for line in stat.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "/" && !parts[0].starts_with("rootfs") {
            // Use nix/libc statvfs
            unsafe {
                let mut buf: libc::statvfs = std::mem::zeroed();
                let path = std::ffi::CString::new("/").unwrap();
                if libc::statvfs(path.as_ptr(), &mut buf) == 0 {
                    let total = buf.f_blocks * buf.f_frsize as u64;
                    let avail = buf.f_bavail * buf.f_frsize as u64;
                    let used = total - avail;
                    let total_gb = total as f64 / 1_073_741_824.0;
                    let used_gb = used as f64 / 1_073_741_824.0;
                    return format!("{:.1} GiB / {:.1} GiB", used_gb, total_gb);
                }
            }
        }
    }
    "unknown".into()
}

fn battery() -> String {
    let bat = Path::new("/sys/class/power_supply/BAT0");
    if !bat.exists() { return "N/A".into(); }
    let cap = read_line(&bat.join("capacity").to_string_lossy());
    let status = read_line(&bat.join("status").to_string_lossy());
    format!("{}% ({})", cap, status)
}

fn packages() -> String {
    // Count pacman packages
    let pacman_db = Path::new("/var/lib/pacman/local");
    if let Ok(entries) = fs::read_dir(pacman_db) {
        let count = entries.count();
        return format!("{} (pacman)", count);
    }
    "unknown".into()
}

fn motherboard() -> String {
    let name = read_line("/sys/devices/virtual/dmi/id/board_name");
    let vendor = read_line("/sys/devices/virtual/dmi/id/board_vendor");
    if name.is_empty() { return "unknown".into(); }
    if vendor.is_empty() { return name; }
    format!("{} {}", vendor, name)
}
