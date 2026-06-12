///! Snapshot manager — btrfs CoW snapshots (instant, near-zero space)

use crate::btrfs;
use std::fs;
use std::path::PathBuf;

/// A snapshot record with metadata
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub name: String,
    pub path: PathBuf,
    pub timestamp: i64,
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

/// Snapshot manager for a single source subvolume
pub struct SnapshotManager {
    /// The subvolume mount point to snapshot (e.g. "/" or "/home")
    pub source: PathBuf,
    /// Where snapshots are stored (e.g. "/.snapshots/root")
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

    pub fn retention_mut(&mut self, policy: &RetentionPolicy) {
        self.retention = policy.clone();
    }

    /// Ensure the source is snapshot-able and the snapshot directory exists.
    pub fn init(&self) -> Result<(), SnapError> {
        if !btrfs::is_btrfs(&self.source) {
            return Err(SnapError::NotBtrfs(self.source.clone()));
        }
        fs::create_dir_all(&self.snapshot_dir)
            .map_err(|e| SnapError::Io("create snapshot dir", e))
    }

    /// Generate a snapshot name like "manual-2026-03-11_143022"
    fn make_name(kind: SnapshotKind) -> String {
        format!("{}-{}", kind.prefix(), timestamp_string())
    }

    /// Create a new read-only CoW snapshot. Instant.
    pub fn create(&self, kind: SnapshotKind) -> Result<Snapshot, SnapError> {
        self.init()?;

        let name = Self::make_name(kind);
        let dest = self.snapshot_dir.join(&name);
        btrfs::snapshot(&self.source, &dest, true)
            .map_err(|e| SnapError::Io("create snapshot", e))?;

        let timestamp = parse_timestamp_from_name(&name);
        Ok(Snapshot { name, path: dest, timestamp, kind })
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

            if !is_our_snapshot(&name) {
                continue;
            }
            if !btrfs::is_subvolume(&entry.path()) {
                continue;
            }

            let kind = SnapshotKind::from_name(&name);
            let timestamp = parse_timestamp_from_name(&name);

            snapshots.push(Snapshot {
                name,
                path: entry.path(),
                timestamp,
                kind,
            });
        }

        // Sort newest first
        snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(snapshots)
    }

    /// Delete a snapshot by name (subvolume destroy)
    pub fn delete(&self, name: &str) -> Result<(), SnapError> {
        if !valid_name(name) {
            return Err(SnapError::NotFound(name.to_string()));
        }
        let path = self.snapshot_dir.join(name);
        if !path.exists() {
            return Err(SnapError::NotFound(name.to_string()));
        }
        // Safety: only delete subvolumes inside our snapshot dir
        if !path.starts_with(&self.snapshot_dir) || !btrfs::is_subvolume(&path) {
            return Err(SnapError::InvalidPath(path));
        }
        btrfs::delete_subvolume(&path)
            .map_err(|e| SnapError::Io("delete snapshot", e))
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
        if !valid_name(old_name) || !valid_name(new_name) {
            return Err(SnapError::NotFound(old_name.to_string()));
        }
        let old_path = self.snapshot_dir.join(old_name);
        if !old_path.exists() {
            return Err(SnapError::NotFound(old_name.to_string()));
        }
        let new_path = self.snapshot_dir.join(new_name);
        if new_path.exists() {
            return Err(SnapError::Io(
                "rename snapshot",
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "name already taken",
                ),
            ));
        }
        fs::rename(&old_path, &new_path)
            .map_err(|e| SnapError::Io("rename snapshot", e))
    }

    /// Rollback: atomically replace the live subvolume with a snapshot.
    ///
    /// 1. Takes an instant read-only "pre-rollback-*" snapshot of the live
    ///    subvolume so nothing is lost.
    /// 2. Mounts the filesystem's top level (subvolid=5), renames the live
    ///    subvolume aside ("@-stale-<ts>"), and snapshots the chosen
    ///    snapshot into its place (read-write).
    /// 3. A reboot then mounts the restored subvolume. The stale subvolume
    ///    is reaped by `cleanup_stale` on a later prune, once unmounted.
    ///
    /// Returns the name of the pre-rollback backup snapshot.
    pub fn rollback(&self, snapshot_name: &str) -> Result<String, SnapError> {
        let snap_path = self.snapshot_dir.join(snapshot_name);
        if !snap_path.exists() {
            return Err(SnapError::NotFound(snapshot_name.to_string()));
        }

        let info = btrfs::mount_info_for(&self.source)
            .ok_or_else(|| SnapError::NotBtrfs(self.source.clone()))?;
        // The top-level subvolume can't be renamed aside — rollback needs
        // the source mounted from a named subvolume (e.g. subvol=@).
        if info.subvol == "/" {
            return Err(SnapError::NoSubvolLayout(self.source.clone()));
        }
        // If the snapshot dir lives inside the subvolume being swapped,
        // the rename-aside would carry every snapshot with it (e.g. a
        // /.snapshots that isn't its own mounted subvolume).
        if btrfs::nearest_mount_point(&self.snapshot_dir)
            == btrfs::nearest_mount_point(&self.source)
        {
            return Err(SnapError::SnapshotsInsideSource(self.snapshot_dir.clone()));
        }

        // 1. Instant safety net
        let ts = timestamp_string();
        let backup_name = format!("pre-rollback-{ts}");
        btrfs::snapshot(&self.source, &self.snapshot_dir.join(&backup_name), true)
            .map_err(|e| SnapError::Io("create pre-rollback snapshot", e))?;

        // 2. Swap at the top level
        let top = btrfs::ToplevelMount::new(&info.device)
            .map_err(|e| SnapError::Io("mount btrfs top level", e))?;
        let live = top.subvol_path(&info.subvol);
        let stale = PathBuf::from(format!("{}-stale-{ts}", live.display()));

        fs::rename(&live, &stale)
            .map_err(|e| SnapError::Io("set live subvolume aside", e))?;

        if let Err(e) = btrfs::snapshot(&snap_path, &live, false) {
            // Put the live subvolume back so the system still boots.
            let _ = fs::rename(&stale, &live);
            return Err(SnapError::Io("restore snapshot into place", e));
        }

        Ok(backup_name)
    }

    /// Delete "*-stale-*" subvolumes left behind by previous rollbacks.
    /// Mounted (pre-reboot) ones fail with EBUSY and are silently skipped.
    pub fn cleanup_stale(&self) -> Vec<String> {
        let mut reaped = Vec::new();
        let Some(info) = btrfs::mount_info_for(&self.source) else {
            return reaped;
        };
        let Ok(top) = btrfs::ToplevelMount::new(&info.device) else {
            return reaped;
        };
        let Ok(entries) = fs::read_dir(&top.path) else {
            return reaped;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.contains("-stale-") || !btrfs::is_subvolume(&entry.path()) {
                continue;
            }
            if btrfs::delete_subvolume(&entry.path()).is_ok() {
                reaped.push(name);
            }
        }
        reaped
    }
}

// ── Helpers ────────────────────────────────────────────────────────

const KNOWN_PREFIXES: &[&str] = &[
    "manual-", "boot-", "hourly-", "daily-", "weekly-",
    "rollback-", "pre-rollback-",
];

fn is_our_snapshot(name: &str) -> bool {
    KNOWN_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Snapshot names must be a single path component — no traversal.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

/// Generate a timestamp string like "2026-03-28_143022"
fn timestamp_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

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
    // The date is everything after the last alphabetic prefix segment —
    // "pre-rollback-2026-..." has two prefix words, so find the first digit.
    let date_start = match name.find(|c: char| c.is_ascii_digit()) {
        Some(i) => i,
        None => return 0,
    };
    let date_part = &name[date_start..];

    if date_part.len() < 17 {
        return 0;
    }

    let year: i32 = date_part[0..4].parse().unwrap_or(0);
    let month: i32 = date_part[5..7].parse().unwrap_or(0);
    let day: i32 = date_part[8..10].parse().unwrap_or(0);
    let hour: i32 = date_part[11..13].parse().unwrap_or(0);
    let min: i32 = date_part[13..15].parse().unwrap_or(0);
    let sec: i32 = date_part[15..17].parse().unwrap_or(0);

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
    NotBtrfs(PathBuf),
    NoSubvolLayout(PathBuf),
    SnapshotsInsideSource(PathBuf),
}

impl std::fmt::Display for SnapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(ctx, e) => write!(f, "{}: {}", ctx, e),
            Self::InvalidPath(p) => write!(f, "invalid path: {}", p.display()),
            Self::NotFound(name) => write!(f, "snapshot not found: {}", name),
            Self::NotBtrfs(p) => write!(
                f,
                "{} is not on btrfs — lntrn-snapshot needs a btrfs filesystem",
                p.display()
            ),
            Self::NoSubvolLayout(p) => write!(
                f,
                "{} is mounted from the top-level subvolume — rollback needs \
                 a named subvolume layout (e.g. subvol=@)",
                p.display()
            ),
            Self::SnapshotsInsideSource(p) => write!(
                f,
                "snapshot dir {} lives inside the subvolume being rolled back \
                 — mount it as its own subvolume (e.g. @snapshots) first",
                p.display()
            ),
        }
    }
}

impl std::error::Error for SnapError {}
