use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    /// A folder, or a symbolic link to one: something that is entered.
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// The entry itself is a symbolic link. `is_dir`, `size` and `modified`
    /// then describe what it points at; every operation on the entry
    /// (trash, delete, move, copy, rename) still acts on the link, by its
    /// path, and never on the target.
    pub is_symlink: bool,
    pub selected: bool,
    /// A folder's custom icon and colour (its `user.lantern.*` attributes),
    /// read once when the listing is built so that drawing a frame costs no
    /// syscall. `None` for files and for folders without one.
    pub folder_icon: Option<String>,
    pub folder_color: Option<String>,
}

impl FileEntry {
    /// File extension (lowercase), or empty string for dirs / no extension.
    pub fn extension(&self) -> String {
        if self.is_dir {
            return String::new();
        }
        self.path
            .extension()
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
        path.strip_prefix(prefix)
            .ok()
            .map(|p| p.components().count())
    };
    matches!(comps("/run/media"), Some(2))
        || matches!(comps("/media"), Some(2))
        || matches!(comps("/mnt"), Some(1))
}

/// List a directory, returning sorted entries (dirs first, then files).
/// Blocking: on a slow mount (`is_slow_path`) call it off the render thread
/// only (app/dir_load.rs).
pub fn list_directory(
    path: &Path,
    show_hidden: bool,
    sort_by: SortBy,
    sort_dir: SortDir,
) -> Vec<FileEntry> {
    let listed = read_directory(path, show_hidden);
    note_listing(path, listed.is_some());
    let mut entries = listed.unwrap_or_default();
    sort_entries(&mut entries, sort_by, sort_dir);
    entries
}

/// Folders whose last listing failed. A folder that cannot be read lists
/// as nothing, and nothing is exactly what an empty folder lists as: the
/// views ask here to tell the two apart and say so.
static UNREADABLE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Record how the listing of `dir` went.
pub fn note_listing(dir: &Path, readable: bool) {
    let mut known = UNREADABLE.lock().unwrap_or_else(|e| e.into_inner());
    let at = known.iter().position(|d| d == dir);
    match (readable, at) {
        (true, Some(i)) => {
            known.swap_remove(i);
        }
        (false, None) => known.push(dir.to_path_buf()),
        _ => {}
    }
}

/// The last listing of `dir` failed (permission denied, a device gone).
pub fn is_unreadable(dir: &Path) -> bool {
    let known = UNREADABLE.lock().unwrap_or_else(|e| e.into_inner());
    known.iter().any(|d| d == dir)
}

/// The entries of a directory in no particular order, or `None` when it
/// cannot be read at all.
pub fn read_directory(path: &Path, show_hidden: bool) -> Option<Vec<FileEntry>> {
    let read_dir = std::fs::read_dir(path).ok()?;

    let hide_lost_found = is_removable_mount_root(path);
    // Mount points of phones and network shares among the entries (an
    // sshfs folder in the home directory, the phones under
    // ~/.lantern/mounts). They are listed as folders without being looked
    // at: this listing may be running on the render thread, and theirs is
    // the one stat in it that can wait on a device.
    let slow_children = slow_mounts_in(path);

    let mut entries = Vec::new();
    let mut links = crate::links::Looker::new();

    // The Trash is one place to the user, however many drives hold a trash
    // of their own: the home trash also lists theirs. Entries keep their
    // real paths, which is how restore finds the right trash again.
    let mut extra: Vec<std::fs::DirEntry> = Vec::new();
    if path == crate::trash::home_trash().files() {
        for dir in crate::trash::other_files_dirs() {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                extra.extend(rd.flatten());
            }
        }
    }

    // Unfinished copies are looked for in ordinary local folders. Not in a
    // trash (one moved there keeps its name, and has been dealt with), and
    // not on a share or a phone: the Fox that is still writing it may run
    // on another machine, where this one cannot see it.
    let spot_leftovers = !is_slow_path(path) && crate::trash::locate(path).is_none();

    for entry in read_dir.flatten().chain(extra) {
        let name = entry.file_name().to_string_lossy().into_owned();

        // What a copy that was cut off left here (by its name alone: no
        // look at the disk). It is hidden, so nobody would ever find it.
        if spot_leftovers && crate::copy_tree::is_stale_staging(&name) {
            crate::copy_tree::report_stale(entry.path());
        }
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        if hide_lost_found && name == "lost+found" {
            continue;
        }

        let path = entry.path();
        if slow_children.contains(&path) {
            entries.push(FileEntry {
                name,
                path,
                is_dir: true,
                size: 0,
                modified: None,
                is_symlink: false,
                selected: false,
                folder_icon: None,
                folder_color: None,
            });
            continue;
        }
        let metadata = entry.metadata().ok();
        entries.push(FileEntry::listed(name, path, metadata.as_ref(), &mut links));
    }
    links.finish(path);
    Some(entries)
}

/// Order a listing: folders first, then files, each by `sort_by`. Also used
/// to re-order a listing that was loaded for another pane's sort.
pub fn sort_entries(entries: &mut Vec<FileEntry>, sort_by: SortBy, sort_dir: SortDir) {
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

    // Stable, so folders keep leading and equal keys keep their order.
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| sort_fn(a, b)));
}

// ── Drive / mount detection ─────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
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
        if self.total_bytes == 0 {
            return 0.0;
        }
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
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else if bytes >= 1024 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
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
            if !d.mounted {
                return 3;
            }
            match d.name.as_str() {
                "System" => 0,
                "Boot" => 1,
                _ => 2,
            }
        };
        // Device as the last key: two identical sticks share a name, and
        // without it their order follows HashMap iteration and reshuffles on
        // every refresh.
        ord(a)
            .cmp(&ord(b))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.device.cmp(&b.device))
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
    let stripped: String = basename
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string();
    let stripped = stripped.trim_end_matches('p');
    format!("/dev/{}", stripped)
}

fn disk_friendly_name(disk_basename: &str) -> Option<String> {
    let v = std::fs::read_to_string(format!("/sys/class/block/{}/device/vendor", disk_basename))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let m = std::fs::read_to_string(format!("/sys/class/block/{}/device/model", disk_basename))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let combined = format!("{v} {m}").trim().to_string();
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

/// Whole-disk size of `drive`'s physical disk, 0 if it is gone.
pub fn drive_disk_size(drive: &Drive) -> u64 {
    disk_size_bytes(drive.parent_disk.trim_start_matches("/dev/"))
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
            b.mounted
                .cmp(&a.mounted)
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
            if whole > 0 {
                rep.total_bytes = whole;
            }
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
        if parts.len() < 3 {
            continue;
        }
        let device = parts[0];
        // A mount point with a space reads `\040` here; statvfs needs the
        // real path or the drive silently drops out of the sidebar.
        let mount = unescape_mount(parts[1]);
        let mount = mount.to_string_lossy();
        let mount: &str = mount.as_ref();
        let fstype = parts[2];

        // Only real block devices
        if !device.starts_with("/dev/") {
            continue;
        }
        // Skip snap/loop
        if device.contains("loop") {
            continue;
        }

        let entry = by_device
            .entry(device.to_string())
            .or_insert_with(|| (mount.to_string(), fstype.to_string()));
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
            } else if mount.starts_with("/media/")
                || mount.starts_with("/mnt/")
                || mount.starts_with("/run/media/")
            {
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
    let Some(name) = device.strip_prefix("/dev/") else {
        return false;
    };
    let direct = format!("/sys/class/block/{name}/removable");
    if let Ok(s) = std::fs::read_to_string(&direct) {
        if s.trim() == "1" {
            return true;
        }
    }
    // For a partition like sda1, also check the parent disk sda.
    let parent: String = name
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('p')
        .to_string();
    if !parent.is_empty() && parent != name {
        let p = format!("/sys/class/block/{parent}/removable");
        if let Ok(s) = std::fs::read_to_string(&p) {
            if s.trim() == "1" {
                return true;
            }
        }
    }
    false
}

/// Find unmounted removable partitions via `lsblk -J`.
/// Returns Drives with `mounted = false`, total = partition size, free/used = 0.
fn detect_unmounted_removable() -> Vec<Drive> {
    let output = match std::process::Command::new("lsblk")
        .args([
            "-J",
            "-b",
            "-o",
            "NAME,LABEL,FSTYPE,SIZE,RM,MOUNTPOINT,PATH,TYPE",
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
    if !rm {
        return;
    }
    if node.mountpoint.is_some() {
        return;
    }
    let Some(fstype) = node.fstype.clone() else {
        return;
    };
    if fstype.is_empty() {
        return;
    }
    // Skip swap and unknown pseudo-fs that udisks won't handle gracefully.
    if fstype == "swap" || fstype == "linux_raid_member" {
        return;
    }

    let device = node
        .path
        .clone()
        .unwrap_or_else(|| format!("/dev/{}", node.name));
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

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct Phone {
    pub name: String,
    pub manufacturer: String,
    pub product: String,
    pub vendor_id: String,
    pub product_id: String,
    pub serial: String,
    pub mount_point: PathBuf,
    /// Whether something is mounted at `mount_point`, as of the last device
    /// refresh. The sidebar reads this instead of parsing /proc/mounts for
    /// every phone on every frame.
    pub mounted: bool,
    /// Where the device sits on USB right now (sysfs `busnum`, `devnum`).
    /// It tells jmtpfs which of several devices to open, and it changes on
    /// every re-plug, which is how a mount left over from before the
    /// re-plug is recognised as dead. `None` when sysfs did not say.
    pub usb_address: Option<(u32, u32)>,
}

/// Scan /sys/bus/usb/devices/ for devices that expose an MTP/PTP interface
/// (USB class 6 = "Still Image", which covers both PTP cameras and MTP phones).
/// `None` when sysfs itself could not be read: "no phone is attached" and
/// "could not look" must not be confused by code that cleans up after
/// phones that went away.
pub fn scan_phones() -> Option<Vec<Phone>> {
    let read_dir = std::fs::read_dir("/sys/bus/usb/devices").ok()?;
    let mounts_root = mounts_root();
    let mount_points = mount_points();
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
        let mounted = mount_points.contains(&mount_point);
        let number = |file: &str| read_trim(&dev_path.join(file))?.parse::<u32>().ok();
        let usb_address = number("busnum").zip(number("devnum"));

        phones.push(Phone {
            name: display,
            manufacturer,
            product,
            vendor_id,
            product_id,
            serial,
            mount_point,
            mounted,
            usb_address,
        });
    }

    // Mount point as the last key: two phones of one model share a name.
    phones.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.mount_point.cmp(&b.mount_point))
    });
    Some(phones)
}

fn device_has_image_class(dev_path: &Path, dev_name: &str) -> bool {
    let Ok(read_dir) = std::fs::read_dir(dev_path) else {
        return false;
    };
    let prefix = format!("{dev_name}:");
    for entry in read_dir.flatten() {
        let n = entry.file_name();
        let n = n.to_string_lossy();
        if !n.starts_with(&prefix) {
            continue;
        }
        if let Some(class) = read_trim(&entry.path().join("bInterfaceClass")) {
            if class.eq_ignore_ascii_case("06") {
                return true;
            }
        }
    }
    false
}

fn read_trim(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
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
    // By chars, and alphanumerics only: a byte slice panics on a non-ASCII
    // serial, and this ends up in a directory name.
    let short: String = serial
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(4)
        .collect();
    if short.is_empty() {
        trimmed
    } else {
        format!("{trimmed}-{short}")
    }
}

/// Where phones are mounted: one folder per device under it.
pub(crate) fn mounts_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".lantern/mounts")
}

/// Every mount point in /proc/mounts, unescaped.
fn mount_points() -> std::collections::HashSet<PathBuf> {
    std::fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(unescape_mount)
        .collect()
}

/// Every mount as (mount point, filesystem type), in /proc/mounts order.
pub(crate) fn mounts() -> Vec<(PathBuf, String)> {
    std::fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _device = parts.next()?;
            let mount = parts.next()?;
            let fstype = parts.next()?;
            Some((unescape_mount(mount), fstype.to_string()))
        })
        .collect()
}

/// Returns true when something is mounted at `path` (per /proc/mounts).
pub fn is_path_mounted(path: &Path) -> bool {
    let Ok(target) = path.canonicalize() else {
        return false;
    };
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else {
        return false;
    };
    for line in contents.lines() {
        let mut parts = line.split_whitespace();
        let _device = parts.next();
        if let Some(mp) = parts.next() {
            if unescape_mount(mp) == target {
                return true;
            }
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
pub(crate) fn install_hint(gentoo: &str, arch: &str) -> String {
    if has_command("emerge") {
        format!("run: {gentoo}")
    } else if has_command("pacman") {
        format!("run: {arch}")
    } else {
        format!("install it \u{2014} {gentoo} (or {arch})")
    }
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
    if ret != 0 {
        return None;
    }
    let buf = unsafe { buf.assume_init() };
    Some(StatVfs {
        block_size: buf.f_frsize,
        blocks: buf.f_blocks,
        blocks_free: buf.f_bavail,
    })
}

// ── Slow-mount detection ────────────────────────────────────────────────────
//
// On a FUSE/network mount every syscall is a device or network round-trip,
// and jmtpfs is worse: the first read() of a file downloads the WHOLE file
// over MTP while holding a global device lock, so one 3 GB video costs
// minutes and every other operation on the phone waits behind it. Anything
// touching such a path must stay off the render thread, and speculative
// work (thumbnails, ffprobe, mtime polls, git) is throttled or skipped.

struct MountTable {
    refreshed: Instant,
    slow_roots: Vec<PathBuf>,
}

static MOUNT_TABLE: std::sync::Mutex<Option<MountTable>> = std::sync::Mutex::new(None);

/// How long a parsed /proc/mounts snapshot is trusted before re-reading.
const MOUNT_TABLE_TTL: Duration = Duration::from_secs(2);

fn is_slow_fstype(fstype: &str) -> bool {
    // `fuseblk` is ntfs-3g & friends on a local block device — fast enough.
    fstype == "fuse"
        || fstype.starts_with("fuse.")
        || matches!(
            fstype,
            "sshfs" | "nfs" | "nfs4" | "cifs" | "smb3" | "davfs" | "afpfs" | "9p"
        )
}

/// /proc/mounts escapes whitespace and backslashes in mount points as octal
/// (`\040` for a space) so the line stays whitespace-separated.
pub(crate) fn unescape_mount(field: &str) -> PathBuf {
    let mut out = String::with_capacity(field.len());
    let bytes = field.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b))
        {
            let code = &field[i + 1..i + 4];
            if let Ok(v) = u8::from_str_radix(code, 8) {
                out.push(v as char);
                i += 4;
                continue;
            }
        }
        // Mount points are UTF-8 on every system we run on; a lossy char
        // walk keeps this simple.
        let ch = field[i..].chars().next().unwrap_or('?');
        out.push(ch);
        i += ch.len_utf8();
    }
    PathBuf::from(out)
}

fn slow_mount_roots() -> Vec<PathBuf> {
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _device = parts.next()?;
            let mount = parts.next()?;
            let fstype = parts.next()?;
            is_slow_fstype(fstype).then(|| unescape_mount(mount))
        })
        .collect()
}

/// Drop the cached mount table so the next `is_slow_path` re-reads
/// /proc/mounts. Called after every mount and unmount we perform.
pub fn invalidate_mount_table() {
    *MOUNT_TABLE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// True when `path` lives on a slow (FUSE / network / MTP) mount, or is
/// reached through a symbolic link a listing found to lead onto one
/// (links.rs). Cheap enough to call per entry per frame: the mount table
/// is re-read at most every two seconds and the check is a prefix match
/// against a few roots.
pub fn is_slow_path(path: &Path) -> bool {
    with_slow_roots(|roots| {
        roots.iter().any(|root| path.starts_with(root)) || crate::links::leads_to_slow(path, roots)
    })
}

/// The mount points of the slow mounts, as of now.
pub(crate) fn slow_roots() -> Vec<PathBuf> {
    with_slow_roots(|roots| roots.to_vec())
}

/// The slow mounts whose mount point is an entry of `dir` itself. To a
/// listing of `dir` they look like ordinary subfolders, but a stat of one
/// is answered by its filesystem: the phone, or the server that may be
/// gone.
fn slow_mounts_in(dir: &Path) -> Vec<PathBuf> {
    with_slow_roots(|roots| {
        roots
            .iter()
            .filter(|root| root.parent() == Some(dir))
            .cloned()
            .collect()
    })
}

fn with_slow_roots<T>(read: impl FnOnce(&[PathBuf]) -> T) -> T {
    let mut guard = MOUNT_TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let stale = guard
        .as_ref()
        .map_or(true, |t| t.refreshed.elapsed() >= MOUNT_TABLE_TTL);
    if stale {
        *guard = Some(MountTable {
            refreshed: Instant::now(),
            slow_roots: slow_mount_roots(),
        });
    }
    read(guard.as_ref().map_or(&[], |t| t.slow_roots.as_slice()))
}

#[cfg(test)]
mod slow_mount_tests {
    use super::*;

    #[test]
    fn fstype_classification() {
        assert!(is_slow_fstype("fuse.jmtpfs"));
        assert!(is_slow_fstype("fuse.sshfs"));
        assert!(is_slow_fstype("fuse"));
        assert!(is_slow_fstype("nfs4"));
        assert!(is_slow_fstype("cifs"));
        // Local block devices, even via FUSE (ntfs-3g), stay fast.
        assert!(!is_slow_fstype("fuseblk"));
        assert!(!is_slow_fstype("btrfs"));
        assert!(!is_slow_fstype("ext4"));
        assert!(!is_slow_fstype("vfat"));
    }

    #[test]
    fn sorting_keeps_folders_first_in_every_order() {
        let entry = |name: &str, is_dir: bool, size: u64| FileEntry {
            name: name.into(),
            path: PathBuf::from("/x").join(name),
            is_dir,
            size,
            modified: None,
            is_symlink: false,
            selected: false,
            folder_icon: None,
            folder_color: None,
        };
        let mut list = vec![
            entry("b.txt", false, 5),
            entry("Zeta", true, 0),
            entry("a.txt", false, 9),
            entry("alpha", true, 0),
        ];
        let names = |l: &[FileEntry]| l.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        sort_entries(&mut list, SortBy::Name, SortDir::Asc);
        assert_eq!(names(&list), ["alpha", "Zeta", "a.txt", "b.txt"]);
        sort_entries(&mut list, SortBy::Size, SortDir::Desc);
        assert_eq!(names(&list), ["alpha", "Zeta", "a.txt", "b.txt"]);
        sort_entries(&mut list, SortBy::Name, SortDir::Desc);
        assert_eq!(names(&list), ["Zeta", "alpha", "b.txt", "a.txt"]);
    }

    #[test]
    fn mount_unescape() {
        assert_eq!(
            unescape_mount("/run/media/alva/My\\040Stick"),
            PathBuf::from("/run/media/alva/My Stick")
        );
        assert_eq!(unescape_mount("/plain"), PathBuf::from("/plain"));
        // Trailing backslash without a full octal triple is kept literally.
        assert_eq!(unescape_mount("/odd\\4"), PathBuf::from("/odd\\4"));
    }
}
