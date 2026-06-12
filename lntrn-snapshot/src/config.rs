///! Simple config file parser — no serde, no toml crate, just vibes

use crate::btrfs;
use crate::manager::RetentionPolicy;
use std::fs;
use std::path::{Path, PathBuf};

/// Configuration for lntrn-snapshot
#[derive(Debug, Clone)]
pub struct Config {
    /// Subvolumes to snapshot: (source mount point, snapshot dir)
    pub targets: Vec<SnapshotTarget>,
    pub retention: RetentionPolicy,
}

#[derive(Debug, Clone)]
pub struct SnapshotTarget {
    pub source: PathBuf,
    pub snapshot_dir: PathBuf,
}

/// Probe the live mounts for sensible default targets: always the root
/// subvolume, plus /home when it's a separate btrfs mount. Nothing is
/// hardcoded to one machine's layout — the laptop picks up its own.
fn default_targets() -> Vec<SnapshotTarget> {
    let mut targets = vec![SnapshotTarget {
        source: PathBuf::from("/"),
        snapshot_dir: PathBuf::from("/.snapshots/root"),
    }];
    if btrfs::mount_info_for(Path::new("/home")).is_some() {
        targets.push(SnapshotTarget {
            source: PathBuf::from("/home"),
            snapshot_dir: PathBuf::from("/.snapshots/home"),
        });
    }
    targets
}

impl Default for Config {
    fn default() -> Self {
        Self {
            targets: default_targets(),
            retention: RetentionPolicy::default(),
        }
    }
}

/// The invoking user's home, resolved through sudo. Root-run invocations
/// (which is all of them) must find the real user's ~/.lantern, not /root's.
pub fn user_home() -> PathBuf {
    if unsafe { libc::geteuid() } == 0 {
        if let Ok(user) = std::env::var("SUDO_USER") {
            if user != "root" {
                let home = PathBuf::from("/home").join(&user);
                if home.exists() {
                    return home;
                }
            }
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()))
}

impl Config {
    pub fn config_path() -> PathBuf {
        user_home().join(".lantern/config/snapshot.conf")
    }

    /// Load config from file, or return default if file doesn't exist
    pub fn load() -> Self {
        let path = Self::config_path();
        if !path.exists() {
            return Self::default();
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };

        Self::parse(&content)
    }

    /// Write default config to disk
    pub fn write_default() -> Result<(), std::io::Error> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut content = String::from(
            "# lntrn-snapshot configuration\n\
             #\n\
             # Targets: which btrfs subvolume mounts to snapshot\n\
             # Format: target <source_mount> <snapshot_dir>\n\n",
        );
        for t in default_targets() {
            content.push_str(&format!(
                "target {} {}\n",
                t.source.display(),
                t.snapshot_dir.display()
            ));
        }
        content.push_str(
            "\n# Retention policy: how many of each kind to keep\n\
             retention.manual  = 10\n\
             retention.boot    = 3\n\
             retention.hourly  = 5\n\
             retention.daily   = 7\n\
             retention.weekly  = 4\n",
        );

        fs::write(&path, content)?;
        Ok(())
    }

    fn parse(content: &str) -> Self {
        let mut targets = Vec::new();
        let mut retention = RetentionPolicy::default();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line.starts_with("target ") {
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                if parts.len() == 3 {
                    targets.push(SnapshotTarget {
                        source: PathBuf::from(parts[1]),
                        snapshot_dir: PathBuf::from(parts[2]),
                    });
                }
            } else if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                if let Ok(n) = value.parse::<usize>() {
                    match key {
                        "retention.manual" => retention.manual = n,
                        "retention.boot" => retention.boot = n,
                        "retention.hourly" => retention.hourly = n,
                        "retention.daily" => retention.daily = n,
                        "retention.weekly" => retention.weekly = n,
                        _ => {}
                    }
                }
            }
        }

        if targets.is_empty() {
            targets = default_targets();
        }

        Self { targets, retention }
    }
}
