///! Snapshot manager — rsync + hardlink snapshots for ext4/any filesystem

use std::fs;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;

/// Progress update from an rsync operation.
#[derive(Debug, Clone)]
pub struct Progress {
    /// 0.0 – 1.0
    pub fraction: f32,
    /// Human-readable status line, e.g. "45%  128.50MB  12.34MB/s"
    pub label: String,
    /// true when the operation finished (check `result` next)
    pub done: bool,
}

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

/// Default paths to exclude from snapshots
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "/proc",
    "/sys",
    "/dev",
    "/tmp",
    "/run",
    "/mnt",
    "/media",
    "/lost+found",
    "/swapfile",
    "/var/tmp",
    "/var/cache/pacman/pkg",
];

/// Snapshot manager for a single source path
pub struct SnapshotManager {
    /// The path to snapshot (e.g. "/")
    pub source: PathBuf,
    /// Where snapshots are stored (e.g. "/.lantern-snapshots")
    pub snapshot_dir: PathBuf,
    pub retention: RetentionPolicy,
    pub excludes: Vec<String>,
}

impl SnapshotManager {
    pub fn new(source: PathBuf, snapshot_dir: PathBuf) -> Self {
        let mut excludes: Vec<String> = DEFAULT_EXCLUDES
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Always exclude the snapshot dir itself
        excludes.push(snapshot_dir.to_string_lossy().to_string());
        Self {
            source,
            snapshot_dir,
            retention: RetentionPolicy::default(),
            excludes,
        }
    }

    pub fn retention_mut(&mut self, policy: &RetentionPolicy) {
        self.retention = policy.clone();
    }

    /// Ensure the snapshot directory exists
    pub fn init(&self) -> Result<(), SnapError> {
        fs::create_dir_all(&self.snapshot_dir)
            .map_err(|e| SnapError::Io("create snapshot dir", e))
    }

    /// Generate a snapshot name like "manual-2026-03-11_143022"
    fn make_name(kind: SnapshotKind) -> String {
        let now = timestamp_string();
        format!("{}-{}", kind.prefix(), now)
    }

    /// Create a new snapshot using rsync --link-dest (blocking, no progress).
    pub fn create(&self, kind: SnapshotKind) -> Result<Snapshot, SnapError> {
        self.init()?;

        let name = Self::make_name(kind);
        let dest = self.snapshot_dir.join(&name);
        let mut cmd = self.build_create_cmd(&dest, false);

        let output = cmd.output()
            .map_err(|e| SnapError::Io("run rsync", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.code() != Some(24) {
                return Err(SnapError::Rsync(stderr.trim().to_string()));
            }
        }

        let timestamp = parse_timestamp_from_name(&name);
        Ok(Snapshot {
            name,
            path: dest,
            timestamp,
            kind,
        })
    }

    /// Build the rsync command for a create operation.
    fn build_create_cmd(&self, dest: &PathBuf, with_progress: bool) -> Command {
        let mut cmd = Command::new("rsync");
        cmd.args(["-aAX", "--delete", "--numeric-ids"]);

        if with_progress {
            cmd.args(["--info=progress2", "--no-inc-recursive"]);
        }

        for exc in &self.excludes {
            cmd.arg(format!("--exclude={exc}"));
        }

        if let Some(ref prev) = self.latest_snapshot_path() {
            cmd.arg(format!("--link-dest={}", prev.display()));
        }

        let src = format!("{}/", self.source.display());
        cmd.arg(src);
        cmd.arg(dest.as_os_str());
        cmd
    }

    /// Create a snapshot with progress reporting.
    ///
    /// Returns a receiver that yields `Progress` updates. The final message
    /// has `done = true`. After that, call the returned join handle to get
    /// the `Result<Snapshot, SnapError>`.
    pub fn create_with_progress(
        &self,
        kind: SnapshotKind,
    ) -> Result<(mpsc::Receiver<Progress>, std::thread::JoinHandle<Result<Snapshot, SnapError>>), SnapError> {
        self.init()?;

        let name = Self::make_name(kind);
        let dest = self.snapshot_dir.join(&name);
        let mut cmd = self.build_create_cmd(&dest, true);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| SnapError::Io("spawn rsync", e))?;

        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();

        let dest_clone = dest.clone();
        let name_clone = name.clone();

        let handle = std::thread::spawn(move || {
            // Parse rsync --info=progress2 output on this thread.
            // Output looks like: "  1,234,567  45%  12.34MB/s    0:01:23\r"
            let reader = BufReader::new(stdout);
            // rsync uses \r for progress lines, not \n
            let mut buf = Vec::new();
            let mut byte_reader = reader;
            loop {
                buf.clear();
                // Read until \r or \n
                let bytes_read = read_until_cr_or_lf(&mut byte_reader, &mut buf);
                if bytes_read == 0 { break; }

                let line = String::from_utf8_lossy(&buf).trim().to_string();
                if line.is_empty() { continue; }

                if let Some(pct) = parse_progress_line(&line) {
                    let _ = tx.send(Progress {
                        fraction: pct.0,
                        label: pct.1,
                        done: false,
                    });
                }
            }

            let status = child.wait()
                .map_err(|e| SnapError::Io("wait rsync", e))?;

            let success = status.success() || status.code() == Some(24);

            // Send final progress
            let _ = tx.send(Progress {
                fraction: if success { 1.0 } else { 0.0 },
                label: if success { "Done!".into() } else { "Failed".into() },
                done: true,
            });

            if success {
                let timestamp = parse_timestamp_from_name(&name_clone);
                Ok(Snapshot {
                    name: name_clone,
                    path: dest_clone,
                    timestamp,
                    kind,
                })
            } else {
                Err(SnapError::Rsync("rsync failed".to_string()))
            }
        });

        Ok((rx, handle))
    }

    /// Find the most recent snapshot path for --link-dest
    fn latest_snapshot_path(&self) -> Option<PathBuf> {
        let snaps = self.list().ok()?;
        snaps.into_iter().next().map(|s| s.path) // already sorted newest-first
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
            // Must be a directory
            if !entry.path().is_dir() {
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

    /// Delete a snapshot by name (rm -rf)
    pub fn delete(&self, name: &str) -> Result<(), SnapError> {
        let path = self.snapshot_dir.join(name);
        if !path.exists() {
            return Err(SnapError::NotFound(name.to_string()));
        }
        // Safety: only delete inside our snapshot dir
        if !path.starts_with(&self.snapshot_dir) {
            return Err(SnapError::InvalidPath(path));
        }
        fs::remove_dir_all(&path)
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

    /// Rollback: restore system from a snapshot using rsync
    ///
    /// This rsyncs the snapshot back over the source, excluding the
    /// snapshot dir itself and virtual filesystems.
    pub fn rollback(&self, snapshot_name: &str) -> Result<PathBuf, SnapError> {
        let snap_path = self.snapshot_dir.join(snapshot_name);
        if !snap_path.exists() {
            return Err(SnapError::NotFound(snapshot_name.to_string()));
        }

        // First create a pre-rollback backup
        let backup_name = format!("pre-rollback-{}", timestamp_string());
        let backup = self.create(SnapshotKind::Manual)?;
        // Rename to pre-rollback-*
        let backup_new = self.snapshot_dir.join(&backup_name);
        fs::rename(&backup.path, &backup_new)
            .map_err(|e| SnapError::Io("rename pre-rollback backup", e))?;

        // rsync snapshot back to source
        let mut cmd = Command::new("rsync");
        cmd.args([
            "-aAX",
            "--delete",
            "--numeric-ids",
        ]);

        for exc in &self.excludes {
            cmd.arg(format!("--exclude={exc}"));
        }

        let src = format!("{}/", snap_path.display());
        let dst = format!("{}/", self.source.display());
        cmd.arg(&src);
        cmd.arg(&dst);

        let output = cmd.output()
            .map_err(|e| SnapError::Io("run rsync rollback", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.code() != Some(24) {
                return Err(SnapError::Rsync(stderr.trim().to_string()));
            }
        }

        Ok(backup_new)
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
    let date_part = match name.find('-') {
        Some(i) => &name[i + 1..],
        None => return 0,
    };

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
    Rsync(String),
    InvalidPath(PathBuf),
    NotFound(String),
}

impl std::fmt::Display for SnapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(ctx, e) => write!(f, "{}: {}", ctx, e),
            Self::Rsync(msg) => write!(f, "rsync: {}", msg),
            Self::InvalidPath(p) => write!(f, "invalid path: {}", p.display()),
            Self::NotFound(name) => write!(f, "snapshot not found: {}", name),
        }
    }
}

impl std::error::Error for SnapError {}

// ── rsync progress parsing ────────────────────────────────────────

/// Read from a BufReader until \r or \n (rsync uses \r for progress lines).
fn read_until_cr_or_lf<R: std::io::Read>(reader: &mut BufReader<R>, buf: &mut Vec<u8>) -> usize {
    let mut total = 0;
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                total += 1;
                if byte[0] == b'\r' || byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(_) => break,
        }
    }
    total
}

/// Parse an rsync --info=progress2 line.
/// Example: "  1,234,567  45%  12.34MB/s    0:01:23"
/// Returns (fraction 0.0-1.0, display label).
fn parse_progress_line(line: &str) -> Option<(f32, String)> {
    // Find the percentage
    let pct_pos = line.find('%')?;
    // Walk backwards from '%' to find the number
    let before = &line[..pct_pos];
    let num_str: String = before.chars().rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .chars().rev().collect();
    let pct: f32 = num_str.parse().ok()?;
    let fraction = (pct / 100.0).clamp(0.0, 1.0);

    // Build a nice label from the line
    let label = line.split_whitespace().collect::<Vec<_>>().join("  ");
    Some((fraction, label))
}
