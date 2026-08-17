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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn flip(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }
}

/// Default direction per sort column — matches what users expect.
pub fn default_dir(sort_by: SortBy) -> SortDir {
    match sort_by {
        SortBy::Name | SortBy::Type => SortDir::Asc,
        SortBy::Size | SortBy::Date => SortDir::Desc,
    }
}

/// True when `path` is the root of a mount under /run/media/<user>/<X>,
/// /media/<X>, or /mnt/<X>. Used to hide ext4's `lost+found` system folder.
fn is_removable_mount_root(path: &Path) -> bool {
    let comps = |prefix: &str| -> Option<usize> {
        path.strip_prefix(prefix).ok().map(|p| p.components().count())
    };
    matches!(comps("/run/media"), Some(2))
        || matches!(comps("/media"), Some(2))
        || matches!(comps("/mnt"), Some(1))
}

/// List a directory, returning sorted entries (dirs first, then files).
pub fn list_directory(path: &Path, show_hidden: bool, sort_by: SortBy, sort_dir: SortDir) -> Vec<FileEntry> {
    let Ok(read_dir) = std::fs::read_dir(path) else {
        return Vec::new();
    };

    let hide_lost_found = is_removable_mount_root(path);

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();

        if !show_hidden && name.starts_with('.') {
            continue;
        }
        if hide_lost_found && name == "lost+found" {
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

    // Compute "ascending" ordering (smallest/earliest/A first), then flip
    // outside the match if direction is Desc. Name tiebreak stays ascending
    // so equal-rank items are still alphabetical.
    let sort_fn = |a: &FileEntry, b: &FileEntry| -> std::cmp::Ordering {
        let primary = match sort_by {
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Date => {
                let at = a.modified.unwrap_or(SystemTime::UNIX_EPOCH);
                let bt = b.modified.unwrap_or(SystemTime::UNIX_EPOCH);
                at.cmp(&bt)
            }
            SortBy::Type => a.extension().cmp(&b.extension()),
        };
        let primary = match sort_dir {
            SortDir::Asc => primary,
            SortDir::Desc => primary.reverse(),
        };
        primary.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
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
    /// Whole-disk device for this drive (e.g. `/dev/sda` for a partition
    /// `/dev/sda1`). For non-partition devices, equals `device`.
    pub parent_disk: String,
    pub fstype: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub mounted: bool,
    pub removable: bool,
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

pub(crate) fn format_size(bytes: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / MB)
    }
}

/// Detect drives: mounted ones via /proc/mounts + statvfs, plus unmounted
/// removable partitions discovered via lsblk JSON. Removable devices with
/// multiple partitions are collapsed into a single sidebar entry per
/// physical disk so a multi-partition USB doesn't sprawl.
pub fn detect_drives() -> Vec<Drive> {
    let mut drives = detect_mounted_drives();
    let mounted_devices: std::collections::HashSet<String> =
        drives.iter().map(|d| d.device.clone()).collect();

    for d in detect_unmounted_removable() {
        if !mounted_devices.contains(&d.device) {
            drives.push(d);
        }
    }

    // Group removable drives by parent disk: one sidebar entry per physical
    // disk, even if it has multiple partitions. Use vendor+model + whole-disk
    // size for the display.
    drives = group_removable_by_disk(drives);

    // Sort: System, Boot, mounted alphabetical, unmounted last
    drives.sort_by(|a, b| {
        let ord = |d: &Drive| -> u8 {
            if !d.mounted { return 3; }
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

/// Resolve a block device's parent disk. For partitions like `/dev/sda1`
/// returns `/dev/sda`; for whole disks or pseudo-devices returns the device
/// unchanged.
pub fn parent_disk_of(device: &str) -> String {
    let basename = device.trim_start_matches("/dev/");
    if basename.is_empty() {
        return device.to_string();
    }
    let part_marker = format!("/sys/class/block/{}/partition", basename);
    if !std::path::Path::new(&part_marker).exists() {
        return device.to_string();
    }
    // Resolve sysfs symlink: /sys/class/block/sda1 → ../../devices/.../block/sda/sda1
    let sys_link = format!("/sys/class/block/{}", basename);
    if let Ok(target) = std::fs::read_link(&sys_link) {
        if let Some(parent) = target.parent() {
            if let Some(name) = parent.file_name() {
                let name = name.to_string_lossy();
                if !name.is_empty() && name.as_ref() != "block" {
                    return format!("/dev/{}", name);
                }
            }
        }
    }
    // Fallback: trim digit suffix (and trailing 'p' for nvme/mmc).
    let stripped: String = basename.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
    let stripped = stripped.trim_end_matches('p');
    format!("/dev/{}", stripped)
}

fn disk_friendly_name(disk_basename: &str) -> Option<String> {
    let v = std::fs::read_to_string(format!("/sys/class/block/{}/device/vendor", disk_basename))
        .ok().map(|s| s.trim().to_string()).unwrap_or_default();
    let m = std::fs::read_to_string(format!("/sys/class/block/{}/device/model", disk_basename))
        .ok().map(|s| s.trim().to_string()).unwrap_or_default();
    let combined = format!("{v} {m}").trim().to_string();
    if combined.is_empty() { None } else { Some(combined) }
}

fn disk_size_bytes(disk_basename: &str) -> u64 {
    std::fs::read_to_string(format!("/sys/class/block/{}/size", disk_basename))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|sectors| sectors * 512)
        .unwrap_or(0)
}

fn group_removable_by_disk(drives: Vec<Drive>) -> Vec<Drive> {
    let mut by_disk: HashMap<String, Vec<Drive>> = HashMap::new();
    let mut result: Vec<Drive> = Vec::new();

    for d in drives {
        if d.removable {
            by_disk.entry(d.parent_disk.clone()).or_default().push(d);
        } else {
            result.push(d);
        }
    }

    for (parent_disk, group) in by_disk {
        // Pick representative: prefer mounted, then largest mount.
        let mut sorted = group;
        sorted.sort_by(|a, b| {
            b.mounted.cmp(&a.mounted)
                .then_with(|| b.total_bytes.cmp(&a.total_bytes))
        });
        let mut rep = sorted.into_iter().next().unwrap();

        let basename = parent_disk.trim_start_matches("/dev/");
        if let Some(friendly) = disk_friendly_name(basename) {
            rep.name = friendly;
        }
        // For unmounted disks, show whole-disk size so a 60GB USB displays
        // as 60GB even if it currently has a leftover 1GB partition.
        // (Format always targets parent_disk regardless.)
        if !rep.mounted {
            let whole = disk_size_bytes(basename);
            if whole > 0 { rep.total_bytes = whole; }
        }
        result.push(rep);
    }

    result
}

fn detect_mounted_drives() -> Vec<Drive> {
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

    by_device
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

            let removable = is_block_removable(&device);
            let parent_disk = parent_disk_of(&device);

            Some(Drive {
                name,
                mount_point: PathBuf::from(mount),
                device,
                parent_disk,
                fstype,
                total_bytes: total,
                used_bytes: used,
                free_bytes: free,
                mounted: true,
                removable,
            })
        })
        .collect()
}

/// True if `/sys/class/block/<basename>/removable` (or its parent disk's) is "1".
fn is_block_removable(device: &str) -> bool {
    let Some(name) = device.strip_prefix("/dev/") else { return false; };
    let direct = format!("/sys/class/block/{name}/removable");
    if let Ok(s) = std::fs::read_to_string(&direct) {
        if s.trim() == "1" { return true; }
    }
    // For a partition like sda1, also check the parent disk sda.
    let parent: String = name.trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('p').to_string();
    if !parent.is_empty() && parent != name {
        let p = format!("/sys/class/block/{parent}/removable");
        if let Ok(s) = std::fs::read_to_string(&p) {
            if s.trim() == "1" { return true; }
        }
    }
    false
}

/// Find unmounted removable partitions via `lsblk -J`.
/// Returns Drives with `mounted = false`, total = partition size, free/used = 0.
fn detect_unmounted_removable() -> Vec<Drive> {
    let output = match std::process::Command::new("lsblk")
        .args([
            "-J", "-b",
            "-o", "NAME,LABEL,FSTYPE,SIZE,RM,MOUNTPOINT,PATH,TYPE",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    let Ok(json): Result<LsblkRoot, _> = serde_json::from_slice(&output) else {
        return Vec::new();
    };

    let mut drives = Vec::new();
    for top in json.blockdevices {
        collect_unmounted(&top, false, &mut drives);
    }
    drives
}

fn collect_unmounted(node: &LsblkNode, parent_rm: bool, out: &mut Vec<Drive>) {
    let rm = node.rm.unwrap_or(false) || parent_rm;
    if let Some(children) = &node.children {
        for c in children {
            collect_unmounted(c, rm, out);
        }
        // Disk-level node — children handled, don't emit the disk itself.
        return;
    }
    // Leaf: a partition or a disk with no children.
    if !rm { return; }
    if node.mountpoint.is_some() { return; }
    let Some(fstype) = node.fstype.clone() else { return; };
    if fstype.is_empty() { return; }
    // Skip swap and unknown pseudo-fs that udisks won't handle gracefully.
    if fstype == "swap" || fstype == "linux_raid_member" { return; }

    let device = node.path.clone().unwrap_or_else(|| format!("/dev/{}", node.name));
    let label = node.label.clone().filter(|l| !l.is_empty());
    let name = label.unwrap_or_else(|| node.name.clone());
    let size = node.size.unwrap_or(0);
    let parent_disk = parent_disk_of(&device);

    out.push(Drive {
        name,
        mount_point: PathBuf::new(),
        device,
        parent_disk,
        fstype,
        total_bytes: size,
        used_bytes: 0,
        free_bytes: 0,
        mounted: false,
        removable: true,
    });
}

#[derive(serde::Deserialize)]
struct LsblkRoot {
    blockdevices: Vec<LsblkNode>,
}

#[derive(serde::Deserialize)]
struct LsblkNode {
    name: String,
    label: Option<String>,
    fstype: Option<String>,
    size: Option<u64>,
    rm: Option<bool>,
    mountpoint: Option<String>,
    path: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    r#type: Option<String>,
    children: Option<Vec<LsblkNode>>,
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

/// True if `cmd` resolves to an executable on `$PATH`.
fn has_command(cmd: &str) -> bool {
    std::env::var_os("PATH").map_or(false, |paths| {
        std::env::split_paths(&paths).any(|dir| dir.join(cmd).is_file())
    })
}

/// Install hint tailored to the local package manager — Gentoo PC (`emerge`)
/// vs Arch laptop (`pacman`). Pass the full command for each; falls back to
/// listing both when neither manager is found.
fn install_hint(gentoo: &str, arch: &str) -> String {
    if has_command("emerge") {
        format!("run: {gentoo}")
    } else if has_command("pacman") {
        format!("run: {arch}")
    } else {
        format!("install it \u{2014} {gentoo} (or {arch})")
    }
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
                format!(
                    "jmtpfs is not installed \u{2014} {}",
                    install_hint("sudo emerge sys-fs/jmtpfs", "yay -S jmtpfs")
                )
            } else {
                format!("spawn jmtpfs: {e}")
            }
        })?;
    if !status.success() {
        // Almost always the phone is locked or still in charge-only mode.
        return Err(
            "jmtpfs couldn\u{2019}t open the phone. Unlock it and set USB mode to \
             \u{201C}File transfer\u{201D} (tap the USB notification on the phone), \
             then try again."
                .to_string(),
        );
    }
    // jmtpfs returns once mount is established, but give it a beat.
    for _ in 0..20 {
        if is_path_mounted(&phone.mount_point) { return Ok(()); }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

/// Mount a removable drive via `udisksctl mount -b <device>`. Polkit handles
/// the auth prompt (no sudo). On success returns the new mount point — udisks2
/// mounts to `/run/media/$USER/<LABEL>/`.
pub fn mount_drive(drive: &Drive) -> Result<PathBuf, String> {
    if drive.mounted {
        return Ok(drive.mount_point.clone());
    }
    let output = std::process::Command::new("udisksctl")
        .args(["mount", "-b", &drive.device])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "udisks2 is not installed \u{2014} {}",
                    install_hint("sudo emerge sys-fs/udisks", "sudo pacman -S udisks2")
                )
            } else {
                format!("spawn udisksctl: {e}")
            }
        })?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("udisksctl: {}", msg.trim()));
    }
    // Output looks like: "Mounted /dev/sda1 at /run/media/alva/ARCH_202604."
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(at) = stdout.find(" at ") {
        let tail = &stdout[at + 4..];
        let mount = tail.trim().trim_end_matches('.').trim();
        if !mount.is_empty() {
            return Ok(PathBuf::from(mount));
        }
    }
    // Fallback: re-scan /proc/mounts for the device.
    if let Ok(contents) = std::fs::read_to_string("/proc/mounts") {
        for line in contents.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(drive.device.as_str()) {
                if let Some(mp) = parts.next() {
                    return Ok(PathBuf::from(mp));
                }
            }
        }
    }
    Err("mounted, but couldn't determine mount point".to_string())
}

/// Format a removable drive as ext4 via UDisks2 D-Bus (`busctl call`).
/// Active-seat users are allowed by the `modify-device` polkit rule
/// (`<allow_active>yes</allow_active>`), so no auth prompt is needed.
///
/// For removable drives, this targets the *whole disk* (e.g. `/dev/sda`,
/// not `/dev/sda1`) and writes ext4 directly to the raw block device with
/// no partition table — recovers the full disk capacity even if a leftover
/// partition layout was present.
pub fn format_drive_ext4(drive: &Drive, label: &str) -> Result<(), String> {
    let target = if drive.removable && !drive.parent_disk.is_empty() {
        drive.parent_disk.clone()
    } else {
        drive.device.clone()
    };

    // Unmount everything on the target disk (disk itself + all partitions).
    unmount_all_on_disk(&target);

    let basename = target.trim_start_matches("/dev/");
    if basename.is_empty() || basename.contains('/') {
        return Err(format!("invalid device: {target}"));
    }
    let obj_path = format!("/org/freedesktop/UDisks2/block_devices/{}", basename);

    // Format(in s type, in a{sv} options).
    // `take-ownership` makes UDisks2 chown the new filesystem root to the
    // calling user — without it, ext4 belongs to root and the user can't
    // write to their own USB.
    let mut args: Vec<String> = vec![
        "call".into(), "--system".into(),
        "org.freedesktop.UDisks2".into(),
        obj_path,
        "org.freedesktop.UDisks2.Block".into(),
        "Format".into(),
        "sa{sv}".into(),
        "ext4".into(),
    ];
    let entries = if label.is_empty() { 1 } else { 2 };
    args.push(entries.to_string());
    args.push("take-ownership".into());
    args.push("b".into());
    args.push("true".into());
    if !label.is_empty() {
        args.push("label".into());
        args.push("s".into());
        args.push(label.into());
    }

    let output = std::process::Command::new("busctl")
        .args(&args)
        .output()
        .map_err(|e| format!("spawn busctl: {e}"))?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("format: {}", msg.trim()));
    }
    Ok(())
}

/// Unmount the disk itself and all partitions on it via udisksctl. Errors
/// are ignored — Format will surface a meaningful message if anything is
/// still busy.
fn unmount_all_on_disk(disk_device: &str) {
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else { return; };
    for line in contents.lines() {
        let mut parts = line.split_whitespace();
        let Some(device) = parts.next() else { continue; };
        if !device.starts_with("/dev/") { continue; }
        let parent = parent_disk_of(device);
        if device == disk_device || parent == disk_device {
            let _ = std::process::Command::new("udisksctl")
                .args(["unmount", "-b", device])
                .output();
        }
    }
}

/// Set a drive's filesystem label via UDisks2 D-Bus.
#[allow(dead_code)]
pub fn relabel_drive(drive: &Drive, new_label: &str) -> Result<(), String> {
    let basename = drive.device.trim_start_matches("/dev/");
    if basename.is_empty() || basename.contains('/') {
        return Err(format!("invalid device: {}", drive.device));
    }
    let obj_path = format!("/org/freedesktop/UDisks2/block_devices/{}", basename);

    let output = std::process::Command::new("busctl")
        .args([
            "call", "--system",
            "org.freedesktop.UDisks2",
            &obj_path,
            "org.freedesktop.UDisks2.Filesystem",
            "SetLabel",
            "sa{sv}",
            new_label,
            "0",
        ])
        .output()
        .map_err(|e| format!("spawn busctl: {e}"))?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("relabel: {}", msg.trim()));
    }
    Ok(())
}

/// Unmount a removable drive via `udisksctl unmount -b <device>`.
pub fn unmount_drive(drive: &Drive) -> Result<(), String> {
    let output = std::process::Command::new("udisksctl")
        .args(["unmount", "-b", &drive.device])
        .output()
        .map_err(|e| format!("spawn udisksctl: {e}"))?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("udisksctl: {}", msg.trim()));
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
