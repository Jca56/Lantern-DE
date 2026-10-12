//! Resume-where-you-left-off positions, keyed by file, in
//! `~/.local/share/lntrn-media-player/positions.json`.
//!
//! Every change re-reads the file, edits one entry and renames a fresh copy
//! into place. Nothing is cached between calls: a second player window
//! saving its own position can't overwrite this one's with a stale map, and
//! a crash mid-write leaves the old file intact instead of an empty one.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    positions: HashMap<String, i64>,
}

fn data_path() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("lntrn-media-player");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("positions.json"))
}

fn load(path: &PathBuf) -> Stored {
    fs::read_to_string(path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// The remembered position for `key`, in nanoseconds.
pub fn get(key: &str) -> Option<u64> {
    get_in(&data_path()?, key)
}

/// Remember `position_ns` for `key`, or forget it with `None`.
pub fn set(key: &str, position_ns: Option<u64>) {
    if let Some(path) = data_path() {
        set_in(&path, key, position_ns);
    }
}

fn get_in(path: &PathBuf, key: &str) -> Option<u64> {
    load(path).positions.get(key).map(|&p| p.max(0) as u64)
}

fn set_in(path: &PathBuf, key: &str, position_ns: Option<u64>) {
    let mut stored = load(path);
    let changed = match position_ns {
        Some(p) => stored.positions.insert(key.to_string(), p as i64) != Some(p as i64),
        None => stored.positions.remove(key).is_some(),
    };
    if !changed {
        return;
    }
    let Ok(data) = serde_json::to_string_pretty(&stored) else {
        return;
    };
    // Unique per process so two windows never share a temp file.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if fs::write(&tmp, data).is_ok() && fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lntrn-media-player-positions-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("positions.json")
    }

    #[test]
    fn remembers_forgets_and_keeps_other_entries() {
        let path = scratch("basic");
        assert_eq!(get_in(&path, "file:///a.mp3"), None);
        set_in(&path, "file:///a.mp3", Some(42_000_000_000));
        // A second window saving its own file must not lose the first's.
        set_in(&path, "file:///b.mkv", Some(7_000_000_000));
        assert_eq!(get_in(&path, "file:///a.mp3"), Some(42_000_000_000));
        assert_eq!(get_in(&path, "file:///b.mkv"), Some(7_000_000_000));
        set_in(&path, "file:///a.mp3", None);
        assert_eq!(get_in(&path, "file:///a.mp3"), None);
        assert_eq!(get_in(&path, "file:///b.mkv"), Some(7_000_000_000));
        // No temp files left behind.
        let leftovers = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn reads_the_format_older_builds_wrote() {
        let path = scratch("compat");
        fs::write(&path, r#"{ "positions": { "file:///old.mp4": 123456789000 } }"#).unwrap();
        assert_eq!(get_in(&path, "file:///old.mp4"), Some(123_456_789_000));
        // A damaged file reads as empty instead of failing.
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(get_in(&path, "file:///old.mp4"), None);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
