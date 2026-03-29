///! Simple config file parser — no serde, no toml crate, just vibes

use crate::manager::RetentionPolicy;
use std::fs;
use std::path::PathBuf;

/// Configuration for lntrn-snapshot
#[derive(Debug, Clone)]
pub struct Config {
    /// Paths to snapshot: (source_path, snapshot_dir_path)
    pub targets: Vec<SnapshotTarget>,
    pub retention: RetentionPolicy,
    pub excludes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SnapshotTarget {
    pub source: PathBuf,
    pub snapshot_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            targets: vec![SnapshotTarget {
                source: PathBuf::from("/"),
                snapshot_dir: PathBuf::from("/.lantern-snapshots"),
            }],
            retention: RetentionPolicy::default(),
            excludes: crate::manager::DEFAULT_EXCLUDES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                PathBuf::from(home).join(".config")
            });
        config_dir.join("lntrn-snapshot").join("config")
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

        let content = r#"# lntrn-snapshot configuration
#
# Targets: which paths to snapshot
# Format: target <source_path> <snapshot_dir>

target / /.lantern-snapshots

# Retention policy: how many of each kind to keep
retention.manual  = 10
retention.boot    = 3
retention.hourly  = 5
retention.daily   = 7
retention.weekly  = 4

# Excluded paths (virtual filesystems, caches, temp)
exclude /proc
exclude /sys
exclude /dev
exclude /tmp
exclude /run
exclude /mnt
exclude /media
exclude /lost+found
exclude /swapfile
exclude /var/tmp
exclude /var/cache/pacman/pkg
"#;

        fs::write(&path, content)?;
        Ok(())
    }

    fn parse(content: &str) -> Self {
        let mut targets = Vec::new();
        let mut retention = RetentionPolicy::default();
        let mut excludes = Vec::new();

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
            } else if let Some(path) = line.strip_prefix("exclude ") {
                let path = path.trim();
                if !path.is_empty() {
                    excludes.push(path.to_string());
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
            targets.push(SnapshotTarget {
                source: PathBuf::from("/"),
                snapshot_dir: PathBuf::from("/.lantern-snapshots"),
            });
        }

        // Fall back to defaults if no excludes in config
        if excludes.is_empty() {
            excludes = crate::manager::DEFAULT_EXCLUDES
                .iter()
                .map(|s| s.to_string())
                .collect();
        }

        Self { targets, retention, excludes }
    }
}
