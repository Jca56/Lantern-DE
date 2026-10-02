//! How the settings file is read and written.
//!
//! Every Fox window (and every file picker) is a process of its own, all
//! sharing one file. Three rules keep them from destroying each other's
//! settings, or all of them at once:
//!
//!  - A save changes only the keys this process changed since it last read
//!    or wrote them. The file is read again first and everything else in
//!    it stays as some other window left it. The two lists (favourites,
//!    pinned tabs) are merged entry by entry, so an entry added elsewhere
//!    survives an edit made here.
//!  - The file is replaced in one step (write a temporary file, sync it,
//!    rename it over the old one). A crash, a full disk or a second writer
//!    can never leave half a file.
//!  - A file that cannot be understood is never written over. It is moved
//!    aside under a name that says so, and stays there for a human.
//!
//! Reading is forgiving one key at a time: one value of the wrong type
//! costs that one setting its stored value, not all of them.

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

pub(super) type Map = serde_json::Map<String, Value>;

/// The keys whose value is a list that several windows add to and remove
/// from. They are merged, not replaced.
pub(super) const LIST_KEYS: [&str; 2] = ["favorites", "pinned_tabs"];

/// What is in the settings file right now.
#[derive(Debug, PartialEq)]
pub(super) enum OnDisk {
    /// No such file: a first start.
    Missing,
    Map(Map),
    /// There is a file and it is not a JSON object.
    Broken(String),
    /// The file could not be read at all (permissions, I/O error).
    Unreadable(String),
}

pub(super) fn read(path: &Path) -> OnDisk {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return OnDisk::Missing,
        Err(e) => return OnDisk::Unreadable(e.to_string()),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(map)) => OnDisk::Map(map),
        Ok(_) => OnDisk::Broken("it is not a JSON object".into()),
        Err(e) => OnDisk::Broken(e.to_string()),
    }
}

/// Move a file nobody can read out of the way, keeping it. Returns where
/// it went.
pub(super) fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings".into());
    let mut aside = path.with_file_name(format!("{name}.broken-{stamp}"));
    let mut n = 1;
    while std::fs::symlink_metadata(&aside).is_ok() {
        aside = path.with_file_name(format!("{name}.broken-{stamp}-{n}"));
        n += 1;
    }
    std::fs::rename(path, &aside)?;
    Ok(aside)
}

/// Three-way merge of a list: `base` is what this process last saw in the
/// file, `mine` what it has now, `disk` what the file holds now.
///
/// This process's order wins. What it removed stays removed, what another
/// process removed is dropped here too, and what another process added is
/// appended.
pub(super) fn merge_list(base: &[String], mine: &[String], disk: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in mine {
        let removed_elsewhere = base.contains(item) && !disk.contains(item);
        if !removed_elsewhere && !out.contains(item) {
            out.push(item.clone());
        }
    }
    for item in disk {
        let added_elsewhere = !base.contains(item);
        if added_elsewhere && !out.contains(item) {
            out.push(item.clone());
        }
    }
    out
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    value?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// What a save has to write: `disk` with the keys this process changed
/// (those where `mine` differs from `base`) put in. `None` when it changed
/// nothing, and the file is left alone.
pub(super) fn merged(base: &Map, mine: &Map, disk: &Map) -> Option<Map> {
    let mut out = disk.clone();
    let mut changed = false;
    for (key, value) in mine {
        if base.get(key) == Some(value) {
            continue;
        }
        changed = true;
        let list = LIST_KEYS.contains(&key.as_str());
        let merged_list = match (
            list,
            strings(base.get(key)),
            strings(Some(value)),
            strings(disk.get(key)),
        ) {
            (true, Some(base), Some(mine), Some(disk)) => Some(merge_list(&base, &mine, &disk)),
            _ => None,
        };
        let value = match merged_list {
            Some(items) => Value::Array(items.into_iter().map(Value::String).collect()),
            None => value.clone(),
        };
        out.insert(key.clone(), value);
    }
    changed.then_some(out)
}

/// Replace `path` with `map`, in one step.
pub(super) fn write_atomic(path: &Path, map: &Map) -> std::io::Result<()> {
    let mut json = serde_json::to_vec_pretty(map)?;
    json.push(b'\n');
    // A config that is a symlink (into a dotfiles folder, say) stays one:
    // the file it points at is the one replaced.
    let target = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let dir = target.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings".into());
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&json)?;
        // The old file's permissions, if someone tightened them.
        if let Ok(meta) = std::fs::metadata(&target) {
            let mode = meta.permissions().mode() & 0o777;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        // On disk before it takes the old file's place: a crash right
        // after the rename must not find an empty file under the name.
        file.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Held for the length of one read-change-write, so that two windows
/// saving at the same moment do not each write over the other's change.
pub(super) struct Lock {
    _file: std::fs::File,
}

/// A save happens on the thread that draws. Another window holds the lock
/// for a millisecond or two; one that hangs must not hang this one.
const LOCK_WAIT: Duration = Duration::from_millis(250);

impl Lock {
    /// `None` when the lock could not be had in time. The save then goes
    /// ahead without it: still atomic, only not serialised.
    pub(super) fn acquire(settings_path: &Path) -> Option<Self> {
        let name = settings_path.file_name()?.to_string_lossy().into_owned();
        let path = settings_path.with_file_name(format!(".{name}.lock"));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .ok()?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                // Released when the file is closed, i.e. when this drops.
                return Some(Self { _file: file });
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn map(json: &str) -> Map {
        match serde_json::from_str(json).unwrap() {
            Value::Object(map) => map,
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn a_list_keeps_what_other_windows_added_and_drops_what_they_removed() {
        // Nothing changed anywhere.
        assert_eq!(
            merge_list(&list(&["a"]), &list(&["a"]), &list(&["a"])),
            list(&["a"])
        );
        // Added here, while another window added "b".
        assert_eq!(
            merge_list(&list(&["a"]), &list(&["a", "c"]), &list(&["a", "b"])),
            list(&["a", "c", "b"])
        );
        // Removed here: stays removed although the file still has it.
        assert_eq!(
            merge_list(&list(&["a", "b"]), &list(&["b"]), &list(&["a", "b"])),
            list(&["b"])
        );
        // Removed by another window: not brought back from here.
        assert_eq!(
            merge_list(&list(&["a", "b"]), &list(&["a", "b", "c"]), &list(&["b"])),
            list(&["b", "c"])
        );
        // Reordered here: this order, plus the other window's new entry.
        assert_eq!(
            merge_list(
                &list(&["a", "b"]),
                &list(&["b", "a"]),
                &list(&["a", "b", "x"])
            ),
            list(&["b", "a", "x"])
        );
        // Both added the same one: once.
        assert_eq!(
            merge_list(&list(&[]), &list(&["n"]), &list(&["n"])),
            list(&["n"])
        );
    }

    #[test]
    fn a_save_writes_only_the_keys_this_window_changed() {
        let base = map(r#"{"sort_by":"name","show_hidden":false,"favorites":["/a"]}"#);
        let mine = map(r#"{"sort_by":"name","show_hidden":true,"favorites":["/a"]}"#);
        // Meanwhile another window changed the sort and added a favourite,
        // and a newer Fox wrote a key this one has never heard of.
        let disk =
            map(r#"{"sort_by":"date","show_hidden":false,"favorites":["/a","/b"],"future_key":7}"#);
        let out = merged(&base, &mine, &disk).expect("something changed");
        assert_eq!(
            out,
            map(r#"{"sort_by":"date","show_hidden":true,"favorites":["/a","/b"],"future_key":7}"#)
        );
        // Nothing changed here: the file is not touched.
        assert_eq!(merged(&base, &base, &disk), None);
    }

    #[test]
    fn a_changed_list_is_merged_and_a_broken_one_is_replaced() {
        let base = map(r#"{"favorites":["/a"]}"#);
        let mine = map(r#"{"favorites":["/a","/mine"]}"#);
        let disk = map(r#"{"favorites":["/a","/theirs"]}"#);
        assert_eq!(
            merged(&base, &mine, &disk).unwrap(),
            map(r#"{"favorites":["/a","/mine","/theirs"]}"#)
        );
        // What the file holds under the key is no list of strings: there is
        // nothing to merge with, this window's list goes in.
        let disk = map(r#"{"favorites":"oops"}"#);
        assert_eq!(merged(&base, &mine, &disk).unwrap(), mine);
        // A key the file does not have yet is simply added.
        let disk = map(r#"{"other":1}"#);
        assert_eq!(
            merged(&base, &mine, &disk).unwrap(),
            map(r#"{"other":1,"favorites":["/a","/mine"]}"#)
        );
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fox-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reading_tells_missing_broken_and_good_apart() {
        let dir = scratch("read");
        let path = dir.join("s.json");
        assert_eq!(read(&path), OnDisk::Missing);
        std::fs::write(&path, r#"{"a":1}"#).unwrap();
        assert_eq!(read(&path), OnDisk::Map(map(r#"{"a":1}"#)));
        // Cut off mid-write by an older Fox, or emptied by a full disk.
        for broken in ["", r#"{"a":1,"fav"#, "[1,2]", "null"] {
            std::fs::write(&path, broken).unwrap();
            assert!(matches!(read(&path), OnDisk::Broken(_)), "{broken:?}");
        }
        // A folder where the file should be: cannot be read at all.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(read(&path), OnDisk::Unreadable(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_is_kept_under_another_name() {
        let dir = scratch("aside");
        let path = dir.join("s.json");
        std::fs::write(&path, "{ half a fi").unwrap();
        let first = set_aside(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "{ half a fi");
        // A second one in the same second does not replace the first.
        std::fs::write(&path, "another").unwrap();
        let second = set_aside(&path).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "{ half a fi");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "another");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_replaces_the_file_whole_and_leaves_no_temporary_behind() {
        let dir = scratch("write");
        let path = dir.join("sub/s.json");
        write_atomic(&path, &map(r#"{"a":1}"#)).unwrap();
        assert_eq!(read(&path), OnDisk::Map(map(r#"{"a":1}"#)));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&path, &map(r#"{"a":2,"b":[]}"#)).unwrap();
        assert_eq!(read(&path), OnDisk::Map(map(r#"{"a":2,"b":[]}"#)));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "permissions are kept");
        let left: Vec<_> = std::fs::read_dir(dir.join("sub"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["s.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlinked_config_stays_a_symlink() {
        let dir = scratch("link");
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        let real = dir.join("dotfiles/s.json");
        std::fs::write(&real, r#"{"a":1}"#).unwrap();
        let link = dir.join("s.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, &map(r#"{"a":2}"#)).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(read(&real), OnDisk::Map(map(r#"{"a":2}"#)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_lock_is_exclusive_and_gives_up_instead_of_hanging() {
        let dir = scratch("lock");
        let path = dir.join("s.json");
        let held = Lock::acquire(&path).expect("free");
        let started = Instant::now();
        assert!(Lock::acquire(&path).is_none(), "held by someone else");
        assert!(started.elapsed() >= LOCK_WAIT);
        assert!(started.elapsed() < LOCK_WAIT * 20);
        drop(held);
        assert!(Lock::acquire(&path).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
