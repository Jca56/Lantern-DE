use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub selected: bool,
}

impl FileEntry {
    /// File extension (lowercase), or empty string for dirs / no extension.
    pub fn extension(&self) -> String {
        if self.is_dir { return String::new(); }
        self.path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortBy {
    Name,
    Size,
    Date,
    Type,
}

/// List a directory, returning sorted entries (dirs first, then files).
pub fn list_directory(path: &Path, show_hidden: bool, sort_by: SortBy) -> Vec<FileEntry> {
    let Ok(read_dir) = std::fs::read_dir(path) else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();

        if !show_hidden && name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        let metadata = entry.metadata().ok();
        let is_dir = metadata.as_ref().map_or(false, |m| m.is_dir());
        let size = metadata.as_ref().map_or(0, |m| m.len());
        let modified = metadata.as_ref().and_then(|m| m.modified().ok());

        let fe = FileEntry {
            name,
            path,
            is_dir,
            size,
            modified,
            selected: false,
        };

        if is_dir {
            dirs.push(fe);
        } else {
            files.push(fe);
        }
    }

    let sort_fn = |a: &FileEntry, b: &FileEntry| -> std::cmp::Ordering {
        match sort_by {
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Size => a.size.cmp(&b.size).then_with(|| {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            }),
            SortBy::Date => {
                let at = a.modified.unwrap_or(SystemTime::UNIX_EPOCH);
                let bt = b.modified.unwrap_or(SystemTime::UNIX_EPOCH);
                bt.cmp(&at).then_with(|| { // newest first
                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                })
            }
            SortBy::Type => {
                a.extension().cmp(&b.extension()).then_with(|| {
                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                })
            }
        }
    };

    dirs.sort_by(sort_fn);
    files.sort_by(sort_fn);

    dirs.extend(files);
    dirs
}

// ── Drive / mount detection ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Drive {
    pub name: String,
    pub mount_point: PathBuf,
    pub device: String,
    pub fstype: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
}

impl Drive {
    pub fn usage_fraction(&self) -> f32 {
        if self.total_bytes == 0 { return 0.0; }
        self.used_bytes as f32 / self.total_bytes as f32
    }

    pub fn total_display(&self) -> String {
        format_size(self.total_bytes)
    }

    pub fn free_display(&self) -> String {
        format_size(self.free_bytes)
    }
}

fn format_size(bytes: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / MB)
    }
}

/// Detect mounted drives by parsing /proc/mounts and calling statvfs.
/// Deduplicates by device path (keeps the shortest mount point).
pub fn detect_drives() -> Vec<Drive> {
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };

    // Collect all real device mounts, dedup by device (keep shortest mount)
    let mut by_device: HashMap<String, (String, String)> = HashMap::new(); // device -> (mount, fstype)

    for line in contents.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 { continue; }
        let device = parts[0];
        let mount = parts[1];
        let fstype = parts[2];

        // Only real block devices
        if !device.starts_with("/dev/") { continue; }
        // Skip snap/loop
        if device.contains("loop") { continue; }

        let entry = by_device.entry(device.to_string()).or_insert_with(|| {
            (mount.to_string(), fstype.to_string())
        });
        // Keep the shortest mount point (usually the "main" one)
        if mount.len() < entry.0.len() {
            *entry = (mount.to_string(), fstype.to_string());
        }
    }

    let mut drives: Vec<Drive> = by_device
        .into_iter()
        .filter_map(|(device, (mount, fstype))| {
            let stat = statvfs(&mount)?;
            let total = stat.blocks * stat.block_size;
            let free = stat.blocks_free * stat.block_size;
            let used = total.saturating_sub(free);

            // Derive a friendly name
            let name = if mount == "/" {
                "System".to_string()
            } else if mount == "/boot" {
                "Boot".to_string()
            } else if mount.starts_with("/media/") || mount.starts_with("/mnt/") || mount.starts_with("/run/media/") {
                mount.rsplit('/').next().unwrap_or("Drive").to_string()
            } else if mount == "/home" {
                // Skip /home if it's on the same device as /
                return None;
            } else {
                // Skip internal btrfs subvolumes etc.
                return None;
            };

            Some(Drive {
                name,
                mount_point: PathBuf::from(mount),
                device,
                fstype,
                total_bytes: total,
                used_bytes: used,
                free_bytes: free,
            })
        })
        .collect();

    // Sort: System first, Boot second, then alphabetical
    drives.sort_by(|a, b| {
        let ord = |d: &Drive| -> u8 {
            match d.name.as_str() {
                "System" => 0,
                "Boot" => 1,
                _ => 2,
            }
        };
        ord(a).cmp(&ord(b)).then_with(|| a.name.cmp(&b.name))
    });

    drives
}

struct StatVfs {
    block_size: u64,
    blocks: u64,
    blocks_free: u64,
}

// ── Phone (MTP) detection ───────────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Phone {
    pub name: String,
    pub manufacturer: String,
    pub product: String,
    pub vendor_id: String,
    pub product_id: String,
    pub serial: String,
    pub mount_point: PathBuf,
}

/// Scan /sys/bus/usb/devices/ for devices that expose an MTP/PTP interface
/// (USB class 6 = "Still Image", which covers both PTP cameras and MTP phones).
pub fn detect_phones() -> Vec<Phone> {
    let Ok(read_dir) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return Vec::new();
    };
    let mounts_root = mounts_root();
    let mut phones = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Skip root hubs ("usb1", "usb2", …) and interfaces ("1-1:1.0").
        if name.starts_with("usb") || name.contains(':') {
            continue;
        }
        let dev_path = entry.path();
        if !device_has_image_class(&dev_path, &name) {
            continue;
        }

        let manufacturer = read_trim(&dev_path.join("manufacturer")).unwrap_or_default();
        let product = read_trim(&dev_path.join("product")).unwrap_or_default();
        let serial = read_trim(&dev_path.join("serial")).unwrap_or_default();
        let vendor_id = read_trim(&dev_path.join("idVendor")).unwrap_or_default();
        let product_id = read_trim(&dev_path.join("idProduct")).unwrap_or_default();

        let display = display_name(&manufacturer, &product);
        let slug = slugify(&display, &serial);
        let mount_point = mounts_root.join(slug);

        phones.push(Phone {
            name: display,
            manufacturer,
            product,
            vendor_id,
            product_id,
            serial,
            mount_point,
        });
    }

    phones.sort_by(|a, b| a.name.cmp(&b.name));
    phones
}

fn device_has_image_class(dev_path: &Path, dev_name: &str) -> bool {
    let Ok(read_dir) = std::fs::read_dir(dev_path) else { return false; };
    let prefix = format!("{dev_name}:");
    for entry in read_dir.flatten() {
        let n = entry.file_name();
        let n = n.to_string_lossy();
        if !n.starts_with(&prefix) { continue; }
        if let Some(class) = read_trim(&entry.path().join("bInterfaceClass")) {
            if class.eq_ignore_ascii_case("06") { return true; }
        }
    }
    false
}

fn read_trim(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn display_name(manufacturer: &str, product: &str) -> String {
    let pretty = |s: &str| {
        s.replace('_', " ")
            .split_whitespace()
            .map(title_word)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let m = pretty(manufacturer);
    let p = pretty(product);
    if !m.is_empty() && !p.is_empty() && !p.to_lowercase().contains(&m.to_lowercase()) {
        format!("{m} {p}")
    } else if !p.is_empty() {
        p
    } else if !m.is_empty() {
        m
    } else {
        "Phone".to_string()
    }
}

fn title_word(w: &str) -> String {
    let mut chars = w.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn slugify(name: &str, serial: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = base.trim_matches('-').to_string();
    let short = if serial.len() >= 4 { &serial[..4] } else { serial };
    if short.is_empty() { trimmed } else { format!("{trimmed}-{short}") }
}

fn mounts_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".lantern/mounts")
}

/// Returns true when something is mounted at `path` (per /proc/mounts).
pub fn is_path_mounted(path: &Path) -> bool {
    let Ok(target) = path.canonicalize() else { return false; };
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else { return false; };
    for line in contents.lines() {
        let mut parts = line.split_whitespace();
        let _device = parts.next();
        if let Some(mp) = parts.next() {
            if Path::new(mp) == target { return true; }
        }
    }
    false
}

/// Mount a phone via jmtpfs. Creates the mount directory if needed and waits
/// for the mount to settle. Returns Err with a human-readable message if the
/// jmtpfs binary is missing or the mount fails.
pub fn mount_phone(phone: &Phone) -> Result<(), String> {
    if is_path_mounted(&phone.mount_point) {
        return Ok(());
    }
    if let Err(e) = std::fs::create_dir_all(&phone.mount_point) {
        return Err(format!("create mount dir: {e}"));
    }
    let status = std::process::Command::new("jmtpfs")
        .arg(&phone.mount_point)
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "jmtpfs not installed (run: yay -S jmtpfs)".to_string()
            } else {
                format!("spawn jmtpfs: {e}")
            }
        })?;
    if !status.success() {
        return Err(format!("jmtpfs exited with {status}"));
    }
    // jmtpfs returns once mount is established, but give it a beat.
    for _ in 0..20 {
        if is_path_mounted(&phone.mount_point) { return Ok(()); }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

/// Unmount a phone via fusermount.
#[allow(dead_code)]
pub fn unmount_phone(phone: &Phone) {
    if !is_path_mounted(&phone.mount_point) { return; }
    let _ = std::process::Command::new("fusermount")
        .arg("-u")
        .arg(&phone.mount_point)
        .status();
}

fn statvfs(path: &str) -> Option<StatVfs> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    extern "C" {
        fn statvfs(path: *const i8, buf: *mut libc_statvfs) -> i32;
    }

    #[repr(C)]
    struct libc_statvfs {
        f_bsize: u64,
        f_frsize: u64,
        f_blocks: u64,
        f_bfree: u64,
        f_bavail: u64,
        f_files: u64,
        f_ffree: u64,
        f_favail: u64,
        f_fsid: u64,
        f_flag: u64,
        f_namemax: u64,
        __spare: [i32; 6],
    }

    let c_path = CString::new(path).ok()?;
    let mut buf = MaybeUninit::<libc_statvfs>::uninit();
    let ret = unsafe { statvfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if ret != 0 { return None; }
    let buf = unsafe { buf.assume_init() };
    Some(StatVfs {
        block_size: buf.f_frsize,
        blocks: buf.f_blocks,
        blocks_free: buf.f_bavail,
    })
}
