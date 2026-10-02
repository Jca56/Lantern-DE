//! The record half of the Trash: `.trashinfo` sidecars (what they say and
//! how they are read back) and restoring an item from its record.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};

use super::{locate, TrashDir};
use crate::copy_tree::{reason_of, rename_noreplace};

/// Local time, as the spec asks: `YYYY-MM-DDThh:mm:ss`.
fn deletion_date_now() -> String {
    match crate::datetime::local(std::time::SystemTime::now()) {
        Some(t) => format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        ),
        None => "1970-01-01T00:00:00".into(),
    }
}

/// The sidecar text for an item whose canonical original path is `canonical`.
pub(super) fn info_contents(td: &TrashDir, canonical: &Path) -> String {
    let recorded = match &td.top {
        Some(top) => canonical.strip_prefix(top).unwrap_or(canonical),
        None => canonical,
    };
    crate::file_ops::trashinfo_contents(recorded, &deletion_date_now())
}

#[derive(Debug)]
pub enum RestoreError {
    /// Its original place is on another filesystem than the trash it is in
    /// (an item trashed as root, or by cloud sync, from another drive): a
    /// rename cannot take it back. Nothing was changed. The caller moves
    /// `trashed` to `dest` by copying; the record goes with it
    /// (`trash::forget`) once the item has left the trash.
    OtherDrive { trashed: PathBuf, dest: PathBuf },
    /// Nothing was changed; the reason completes "could not be restored".
    Failed(String),
}

impl From<&str> for RestoreError {
    fn from(reason: &str) -> Self {
        RestoreError::Failed(reason.to_string())
    }
}

/// Restore the trashed item that `path` is (or is inside of) to where its
/// record says it came from. A taken name gets a " (restored N)" suffix.
/// Returns where it landed.
pub fn restore_item(path: &Path) -> Result<PathBuf, RestoreError> {
    let loc = locate(path).ok_or("it is not in the Trash")?;
    let Some(Component::Normal(top_name)) = loc.rel.components().next() else {
        return Err("it is not an item in the Trash".into());
    };
    let trashed = loc.trash.files().join(top_name);
    let mut info_name = top_name.to_os_string();
    info_name.push(".trashinfo");
    let info = loc.trash.info().join(info_name);
    let recorded = read_trashinfo_path(&info, &top_name.to_string_lossy())
        .ok_or("its restore record is missing")?;
    let original = original_place(&loc.trash, recorded)
        .ok_or("its restore record is not usable")?;
    // A drive's trash sends things back onto that drive and nowhere else.
    // The record was checked as text; a link on the drive (`Path=lnk/x`,
    // `lnk` pointing into the home folder) would still lead off it.
    if let Some(top) = &loc.trash.top {
        if !stays_inside(top, &original) {
            return Err("its restore record points outside its drive".into());
        }
    }
    let dest = pick_restore_dest(&original);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            RestoreError::Failed(format!(
                "its folder can't be recreated ({})",
                reason_of(&e)
            ))
        })?;
    }
    match rename_noreplace(&trashed, &dest) {
        Ok(()) => {
            let _ = std::fs::remove_file(&info);
            Ok(dest)
        }
        // Only for the home trash, whose records the user's own programs
        // wrote. Within a drive's trash "another filesystem" means the
        // record leads off the drive: refused, never copied.
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) && loc.trash.top.is_none() => {
            Err(RestoreError::OtherDrive { trashed, dest })
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => Err(RestoreError::Failed(
            "its original place is on another drive".to_string(),
        )),
        Err(e) => Err(RestoreError::Failed(reason_of(&e))),
    }
}

/// Where a record says its item came from, if that is a place this trash
/// may send things back to. The home trash keeps absolute paths. A drive's
/// trash keeps paths relative to the drive's top folder, and whatever its
/// records say, nothing leaves that drive: a record is a file anyone who
/// had the drive could have written.
fn original_place(trash: &TrashDir, recorded: PathBuf) -> Option<PathBuf> {
    let plain = |p: &Path| {
        p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::RootDir))
    };
    match &trash.top {
        None => (recorded.is_absolute() && plain(&recorded)).then_some(recorded),
        Some(top) if recorded.is_absolute() => {
            (plain(&recorded) && recorded.starts_with(top) && recorded != *top).then_some(recorded)
        }
        Some(top) => (plain(&recorded) && !recorded.as_os_str().is_empty())
            .then(|| top.join(recorded)),
    }
}

/// `place` really lies under `top`: the deepest folder of its path that
/// exists, with every link resolved, is `top` or inside it. (The folders
/// below that one do not exist yet and will be made as plain folders.)
fn stays_inside(top: &Path, place: &Path) -> bool {
    let Ok(top) = top.canonicalize() else {
        return false;
    };
    let mut at = place.parent();
    while let Some(dir) = at {
        match dir.canonicalize() {
            Ok(real) => return real.starts_with(&top),
            Err(_) => at = dir.parent(),
        }
    }
    false
}

/// Parse the Path= line out of a `.trashinfo` file. Values are
/// percent-encoded per spec; the result is absolute for the home trash and
/// relative to the drive's top folder for a volume trash.
///
/// `trashed_name` is the item's name inside `files/`. Builds before 2026-10
/// wrote `Path=` raw, so a name like `25%fat.txt` sits there unencoded and
/// "decodes" to garbage bytes. Such an entry is recognised by its raw file
/// name matching the trashed name, and is taken as written.
pub(crate) fn read_trashinfo_path(info_path: &Path, trashed_name: &str) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(info_path).ok()?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Path=") {
            let decoded = percent_decode(rest);
            let written_raw = std::str::from_utf8(&decoded).is_err()
                && Path::new(rest)
                    .file_name()
                    .is_some_and(|n| n == trashed_name);
            if written_raw {
                return Some(PathBuf::from(rest));
            }
            return Some(PathBuf::from(OsString::from_vec(decoded)));
        }
    }
    None
}

fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// If the original path is taken, append " (restored N)" before the
/// extension rather than clobber whatever is there now.
fn pick_restore_dest(original: &Path) -> PathBuf {
    if std::fs::symlink_metadata(original).is_err() {
        return original.to_path_buf();
    }
    let parent = original.parent().unwrap_or(Path::new("/"));
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = original
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    for n in 1u32..1000 {
        let candidate = parent.join(format!("{stem} (restored {n}){ext}"));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    parent.join(format!("{stem} (restored){ext}"))
}

