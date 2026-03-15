///! Snapshot manager — high-level operations built on btrfs ioctls

use crate::ioctl;
use std::fs;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// A snapshot record with metadata
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub name: String,
    pub path: PathBuf,
    pub subvol_id: u64,
    pub timestamp: i64, // unix timestamp parsed from name
    pub kind: SnapshotKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    Manual,
    Boot,
    Hourly,
    Daily,
    Weekly,
}

impl SnapshotKind {
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Boot => "boot",
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }

    fn from_name(name: &str) -> Self {
        if name.starts_with("boot-") {
            Self::Boot
        } else if name.starts_with("hourly-") {
            Self::Hourly
        } else if name.starts_with("daily-") {
            Self::Daily
        } else if name.starts_with("weekly-") {
            Self::Weekly
        } else {
            Self::Manual
        }
    }
}

/// Retention policy — how many snapshots of each kind to keep
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub manual: usize,
    pub boot: usize,
    pub hourly: usize,
    pub daily: usize,
    pub weekly: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            manual: 10,
            boot: 3,
            hourly: 5,
            daily: 7,
            weekly: 4,
        }
    }
}

/// Snapshot manager for a single btrfs subvolume
pub struct SnapshotManager {
    /// The subvolume to snapshot (e.g. "/" or "/home")
    pub source: PathBuf,
    /// Where snapshots are stored (e.g. "/.snapshots")
    pub snapshot_dir: PathBuf,
    pub retention: RetentionPolicy,
}

impl SnapshotManager {
    pub fn new(source: PathBuf, snapshot_dir: PathBuf) -> Self {
        Self {
            source,
            snapshot_dir,
            retention: RetentionPolicy::default(),
        }
    }

    /// Apply a retention policy from config
    pub fn retention_mut(&mut self, policy: &RetentionPolicy) {
        self.retention = policy.clone();
    }

    /// Ensure the snapshot directory exists (create as btrfs subvolume)
    pub fn init(&self) -> Result<(), SnapError> {
        if self.snapshot_dir.exists() {
            return Ok(());
        }

        let parent = self.snapshot_dir.parent()
            .ok_or_else(|| SnapError::InvalidPath(self.snapshot_dir.clone()))?;
        let name = self.snapshot_dir.file_name()
            .ok_or_else(|| SnapError::InvalidPath(self.snapshot_dir.clone()))?
            .to_string_lossy();

        let parent_fd = fs::File::open(parent)
            .map_err(|e| SnapError::Io("open snapshot parent dir", e))?;

        unsafe {
            ioctl::subvol_create(parent_fd.as_raw_fd(), &name)
                .map_err(|e| SnapError::Io("create snapshot subvolume", e))?;
        }

        Ok(())
    }

    /// Generate a snapshot name like "manual-2026-03-11_143022"
    fn make_name(kind: SnapshotKind) -> String {
        let now = chrono_timestamp();
        format!("{}-{}", kind.prefix(), now)
    }

    /// Create a new snapshot
    pub fn create(&self, kind: SnapshotKind) -> Result<Snapshot, SnapError> {
        let name = Self::make_name(kind);

        let src_fd = fs::File::open(&self.source)
            .map_err(|e| SnapError::Io("open source subvolume", e))?;
        let dst_fd = fs::File::open(&self.snapshot_dir)
            .map_err(|e| SnapError::Io("open snapshot dir", e))?;

        unsafe {
            ioctl::snap_create(
                dst_fd.as_raw_fd(),
                src_fd.as_raw_fd(),
                &name,
                true, // always read-only
            )
            .map_err(|e| SnapError::Io("create snapshot", e))?;
        }

        let path = self.snapshot_dir.join(&name);
        let timestamp = parse_timestamp_from_name(&name);

        Ok(Snapshot {
            name,
            path,
            subvol_id: 0, // we don't know it yet, list will find it
            timestamp,
            kind,
        })
    }

    /// List all snapshots in the snapshot directory
    pub fn list(&self) -> Result<Vec<Snapshot>, SnapError> {
        if !self.snapshot_dir.exists() {
            return Ok(Vec::new());
        }

        let mut snapshots = Vec::new();

        let entries = fs::read_dir(&self.snapshot_dir)
            .map_err(|e| SnapError::Io("read snapshot dir", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| SnapError::Io("read dir entry", e))?;
            let name = entry.file_name().to_string_lossy().into_owned();

            // Only list our managed snapshots (must have a known prefix)
            if !is_our_snapshot(&name) {
                continue;
            }

            let kind = SnapshotKind::from_name(&name);
            let timestamp = parse_timestamp_from_name(&name);

            snapshots.push(Snapshot {
                name,
                path: entry.path(),
                subvol_id: 0,
                timestamp,
                kind,
            });
        }

        // Sort newest first
        snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(snapshots)
    }

    /// Delete a snapshot by name
    pub fn delete(&self, name: &str) -> Result<(), SnapError> {
        let parent_fd = fs::File::open(&self.snapshot_dir)
            .map_err(|e| SnapError::Io("open snapshot dir", e))?;

        unsafe {
            ioctl::snap_destroy(parent_fd.as_raw_fd(), name)
                .map_err(|e| SnapError::Io("destroy snapshot", e))?;
        }

        Ok(())
    }

    /// Apply retention policy — delete oldest snapshots beyond the limit
    pub fn prune(&self) -> Result<Vec<String>, SnapError> {
        let all = self.list()?;
        let mut deleted = Vec::new();

        for kind in &[
            SnapshotKind::Manual,
            SnapshotKind::Boot,
            SnapshotKind::Hourly,
            SnapshotKind::Daily,
            SnapshotKind::Weekly,
        ] {
            let limit = match kind {
                SnapshotKind::Manual => self.retention.manual,
                SnapshotKind::Boot => self.retention.boot,
                SnapshotKind::Hourly => self.retention.hourly,
                SnapshotKind::Daily => self.retention.daily,
                SnapshotKind::Weekly => self.retention.weekly,
            };

            let of_kind: Vec<&Snapshot> =
                all.iter().filter(|s| s.kind == *kind).collect();
            // Already sorted newest-first from list()
            if of_kind.len() > limit {
                for snap in &of_kind[limit..] {
                    self.delete(&snap.name)?;
                    deleted.push(snap.name.clone());
                }
            }
        }

        Ok(deleted)
    }

    /// Rename a snapshot
    pub fn rename(&self, old_name: &str, new_name: &str) -> Result<(), SnapError> {
        let old_path = self.snapshot_dir.join(old_name);
        if !old_path.exists() {
            return Err(SnapError::NotFound(old_name.to_string()));
        }
        let new_path = self.snapshot_dir.join(new_name);
        if new_path.exists() {
            return Err(SnapError::Io(
                "rename snapshot",
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, "name already taken"),
            ));
        }
        fs::rename(&old_path, &new_path)
            .map_err(|e| SnapError::Io("rename snapshot", e))
    }

    /// Rollback: replace source subvolume with a snapshot
    ///
    /// Strategy:
    ///   1. Rename current source → source.old (btrfs mv)
    ///   2. Create writable snapshot of target → source
    ///   3. On next boot, delete source.old
    ///
    /// Returns the backup path so the user knows where the old state is.
    pub fn rollback(&self, snapshot_name: &str) -> Result<PathBuf, SnapError> {
        let snap_path = self.snapshot_dir.join(snapshot_name);
        if !snap_path.exists() {
            return Err(SnapError::NotFound(snapshot_name.to_string()));
        }

        // We can't atomically replace a mounted root — this needs to be
        // done from a rescue/initramfs context for the root subvolume.
        // For non-root subvolumes we can do it live.
        //
        // For root ("/"), we generate a script and tell the user to reboot
        // into rescue mode or use it with btrfs-specific boot params.
        //
        // For now: create a writable snapshot alongside so the user can
        // boot into it by changing the default subvolume.
        let rollback_name = format!("rollback-{}", snapshot_name);
        let parent_fd = fs::File::open(self.source.parent().unwrap_or(Path::new("/")))
            .map_err(|e| SnapError::Io("open parent for rollback", e))?;
        let snap_fd = fs::File::open(&snap_path)
            .map_err(|e| SnapError::Io("open snapshot for rollback", e))?;

        // Create a writable snapshot from the read-only one
        unsafe {
            ioctl::snap_create(
                parent_fd.as_raw_fd(),
                snap_fd.as_raw_fd(),
                &rollback_name,
                false, // writable!
            )
            .map_err(|e| SnapError::Io("create rollback snapshot", e))?;
        }

        // Get the subvol ID so we can set it as default
        let rollback_path = self.source.parent()
            .unwrap_or(Path::new("/"))
            .join(&rollback_name);

        Ok(rollback_path)
    }
}

// ── Helpers ────────────────────────────────────────────────────────

const KNOWN_PREFIXES: &[&str] = &["manual-", "boot-", "hourly-", "daily-", "weekly-", "rollback-"];

fn is_our_snapshot(name: &str) -> bool {
    KNOWN_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Generate a timestamp string like "2026-03-11_143022"
fn chrono_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Convert to broken-down time using libc
    let secs_i64 = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs_i64, &mut tm) };

    format!(
        "{:04}-{:02}-{:02}_{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Parse a unix timestamp from a snapshot name like "manual-2026-03-11_143022"
fn parse_timestamp_from_name(name: &str) -> i64 {
    // Find the date part after the first '-'
    let date_part = match name.find('-') {
        Some(i) => &name[i + 1..],
        None => return 0,
    };

    // Parse "2026-03-11_143022"
    if date_part.len() < 17 {
        return 0;
    }

    let year: i32 = date_part[0..4].parse().unwrap_or(0);
    let month: i32 = date_part[5..7].parse().unwrap_or(0);
    let day: i32 = date_part[8..10].parse().unwrap_or(0);
    let hour: i32 = date_part[11..13].parse().unwrap_or(0);
    let min: i32 = date_part[13..15].parse().unwrap_or(0);
    let sec: i32 = date_part[15..17].parse().unwrap_or(0);

    // Convert to unix timestamp via libc::mktime
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month - 1;
    tm.tm_mday = day;
    tm.tm_hour = hour;
    tm.tm_min = min;
    tm.tm_sec = sec;
    tm.tm_isdst = -1;

    unsafe { libc::mktime(&mut tm) }
}

// ── Errors ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SnapError {
    Io(&'static str, std::io::Error),
    InvalidPath(PathBuf),
    NotFound(String),
}

impl std::fmt::Display for SnapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(ctx, e) => write!(f, "{}: {}", ctx, e),
            Self::InvalidPath(p) => write!(f, "invalid path: {}", p.display()),
            Self::NotFound(name) => write!(f, "snapshot not found: {}", name),
        }
    }
}

impl std::error::Error for SnapError {}
