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
