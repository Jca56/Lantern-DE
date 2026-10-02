//! The Trash, per the freedesktop.org Trash specification 1.0.
//!
//! Where a trashed item goes:
//!  - the *home trash* (`$XDG_DATA_HOME/Trash`) for anything on the same
//!    filesystem as it;
//!  - on any other mount, that mount's own trash: `$top/.Trash/$uid` when an
//!    administrator set up a valid shared `.Trash` (a real directory with
//!    the sticky bit), else `$top/.Trash-$uid`, created on first use. `$top`
//!    is found by walking up while `st_dev` stays the same.
//!
//! A trash directory holds `files/` (the items) and `info/` (one
//! `<name>.trashinfo` per item: original path and deletion date). The home
//! trash records absolute paths; a volume trash records paths relative to
//! `$top`, so a stick still restores correctly when mounted elsewhere.
//!
//! Where no trash can exist (a phone, a network share, a read-only or
//! unwritable volume) [`trash`] returns [`TrashError::NoTrash`] and changes
//! nothing; the caller decides what to offer instead. Nothing in this module
//! ever deletes an item.
//!
//! The one call the rest of Fox needs is [`trash`]: it returns a [`Trashed`],
//! which is everything [`restore`] (undo) takes. Redo is a fresh [`trash`].

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

use crate::copy_tree::{reason_of, rename_noreplace};

mod info;
mod locate;
mod registry;

use info::info_contents;
pub use locate::{is_trashed, locate, shows_trash};
use registry::remember;
#[cfg(test)]
use info::read_trashinfo_path;
pub use info::{restore_item, RestoreError};

/// An item in a trash: where it came from, where it is now, and its
/// `.trashinfo` sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    pub original: PathBuf,
    pub trashed: PathBuf,
    pub info: PathBuf,
}

#[derive(Debug)]
pub enum TrashError {
    /// There is no usable trash for this item. Nothing was changed.
    NoTrash(String),
    /// A trash exists but the item may not be moved (a folder without the
    /// write bit, someone else's file). Nothing was changed.
    Denied(io::Error),
    /// A drive, phone or share is mounted on the item or inside it. It is
    /// not trashed, and it must not be deleted instead (mount_guard.rs).
    Mounted,
    /// Anything else. Nothing was changed.
    Io(io::Error),
}

impl TrashError {
    /// A short phrase that completes "can't be moved to the Trash: ...".
    pub fn reason(&self) -> String {
        match self {
            TrashError::NoTrash(why) => why.clone(),
            TrashError::Denied(e) | TrashError::Io(e) => reason_of(e),
            TrashError::Mounted => crate::mount_guard::REASON.into(),
        }
    }
}

/// One trash directory (`files/` + `info/` live in `dir`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashDir {
    pub dir: PathBuf,
    /// The mount's top directory for a volume trash (its records are
    /// relative to this). `None` for the home trash (absolute records).
    pub top: Option<PathBuf>,
}

impl TrashDir {
    pub fn files(&self) -> PathBuf {
        self.dir.join("files")
    }

    pub fn info(&self) -> PathBuf {
        self.dir.join("info")
    }

    /// Create the directory and its two halves if missing, private to the
    /// user, and make sure they are real directories of ours: in a
    /// world-writable top directory (/tmp) anyone could plant a `.Trash-N`
    /// link and collect what gets trashed.
    fn ensure(&self) -> io::Result<()> {
        if self.top.is_none() {
            if let Some(parent) = self.dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        for dir in [self.dir.clone(), self.files(), self.info()] {
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            if !own_dir_ok(&dir) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "the Trash folder on this drive belongs to someone else",
                ));
            }
        }
        Ok(())
    }
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// A real directory (not a link to one) owned by this user.
fn own_dir_ok(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && m.uid() == uid())
}

/// The record half of a volume trash is ours as well, or not there yet. A
/// trash on a drive someone else prepared can have `info` point anywhere;
/// everything that lists or empties a trash would walk into that.
fn info_half_ok(td: &TrashDir) -> bool {
    match std::fs::symlink_metadata(td.info()) {
        Ok(m) => m.is_dir() && m.uid() == uid(),
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

/// The spec's test for an administrator-made `$top/.Trash`: a real
/// directory with the sticky bit. Anything else must not be used.
fn shared_trash_is_valid(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && m.mode() & 0o1000 != 0)
}

/// Make sure the home trash exists, so its folder can be opened and
/// watched before anything was ever trashed.
pub fn ensure_home() {
    if let Err(e) = home_trash().ensure() {
        eprintln!("[fox] home Trash unusable: {e}");
    }
}

pub fn home_trash() -> TrashDir {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::app::dirs_home().join(".local/share"));
    TrashDir {
        dir: data.join("Trash"),
        top: None,
    }
}

/// The trash directories a mount may have, in the order the spec tries them.
fn volume_trashes(top: &Path) -> Vec<TrashDir> {
    let uid = uid();
    let mut out = Vec::new();
    let shared = top.join(".Trash");
    if shared_trash_is_valid(&shared) {
        out.push(TrashDir {
            dir: shared.join(uid.to_string()),
            top: Some(top.to_path_buf()),
        });
    }
    out.push(TrashDir {
        dir: top.join(format!(".Trash-{uid}")),
        top: Some(top.to_path_buf()),
    });
    out
}

/// The top directory of the filesystem holding `dir` (canonical): walk up
/// while the device stays the same. A mount point also ends the walk, which
/// catches a bind mount of the same device (rename across it fails too).
fn top_dir(dir: &Path) -> PathBuf {
    let mounts: std::collections::HashSet<PathBuf> =
        crate::fs::mounts().into_iter().map(|(m, _)| m).collect();
    let Ok(dev) = std::fs::metadata(dir).map(|m| m.dev()) else {
        return dir.to_path_buf();
    };
    let mut cur = dir.to_path_buf();
    loop {
        if mounts.contains(&cur) {
            return cur;
        }
        match cur.parent() {
            Some(parent) if std::fs::metadata(parent).is_ok_and(|m| m.dev() == dev) => {
                cur = parent.to_path_buf();
            }
            _ => return cur,
        }
    }
}

/// Every trash that exists right now: the home trash first (always listed),
/// then the volume trashes of the mounted drives. Slow mounts are left out;
/// they never get a trash from us and must not be probed from the UI thread.
pub fn all_trash_dirs() -> Vec<TrashDir> {
    let mut out = vec![home_trash()];
    let mut candidates: Vec<TrashDir> = Vec::new();
    for (mount, fstype) in crate::fs::mounts() {
        if fstype == "autofs" || crate::fs::is_slow_path(&mount) {
            continue;
        }
        candidates.extend(volume_trashes(&mount));
    }
    candidates.extend(registry::remembered());
    for td in candidates {
        // `$top/.Trash/$uid` only counts inside a valid shared `.Trash`.
        let shared_parent_ok = match td.dir.parent() {
            Some(p) if p.file_name() == Some(OsStr::new(".Trash")) => shared_trash_is_valid(p),
            _ => true,
        };
        if !out.contains(&td)
            && shared_parent_ok
            && own_dir_ok(&td.dir)
            && own_dir_ok(&td.files())
            && info_half_ok(&td)
        {
            out.push(td);
        }
    }
    out
}

/// The `files/` directories of every trash but the home one: what the Trash
/// view shows in addition to its own folder.
pub fn other_files_dirs() -> Vec<PathBuf> {
    all_trash_dirs().iter().skip(1).map(|t| t.files()).collect()
}

/// `path` with its folder resolved (symlinks, `..`) and its own name kept:
/// the item itself may be a link, and it is the link that gets trashed.
fn canonical_entry(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "it has no name"))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    Ok(parent.canonicalize()?.join(name))
}

/// The name an item gets inside `files/`: its own, unless that is so long
/// that `<name>.trashinfo` would not fit in a file name.
fn slot_base(name: &OsStr) -> OsString {
    if name.len() <= 200 {
        return name.to_os_string();
    }
    let lossy = name.to_string_lossy();
    let mut cut = String::new();
    for ch in lossy.chars() {
        if cut.len() + ch.len_utf8() > 200 {
            break;
        }
        cut.push(ch);
    }
    OsString::from(cut)
}

/// Claim a free name in `td` by creating its `.trashinfo` exclusively (the
/// spec's way of making two programs trashing at once pick different
/// names). The item itself is not moved yet.
fn reserve_in(td: &TrashDir, original: &Path, canonical: &Path) -> io::Result<Trashed> {
    let name = canonical
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "it has no name"))?;
    let base = slot_base(name);
    let as_path = Path::new(&base);
    let stem = as_path.file_stem().unwrap_or(&base).to_os_string();
    let ext = as_path.extension().map(|e| e.to_os_string());
    let contents = info_contents(td, canonical);
    for n in 0u32..100_000 {
        let mut slot = if n == 0 {
            base.clone()
        } else {
            let mut s = stem.clone();
            s.push(format!(".{n}"));
            if let Some(ext) = &ext {
                s.push(".");
                s.push(ext);
            }
            s
        };
        let trashed = td.files().join(&slot);
        if std::fs::symlink_metadata(&trashed).is_ok() {
            continue;
        }
        slot.push(".trashinfo");
        let info = td.info().join(&slot);
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        if let Err(e) = file.write_all(contents.as_bytes()) {
            drop(file);
            let _ = std::fs::remove_file(&info);
            return Err(e);
        }
        return Ok(Trashed {
            original: original.to_path_buf(),
            trashed,
            info,
        });
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "the Trash has no free name for it",
    ))
}

/// Give back a reservation that was not used: the sidecar goes, but only
/// while there is no item behind it.
pub fn release(t: &Trashed) {
    if std::fs::symlink_metadata(&t.trashed).is_err() {
        let _ = std::fs::remove_file(&t.info);
    }
}

enum Attempt {
    /// This trash will not do; the next candidate might.
    Next(String),
    Stop(TrashError),
}

fn try_in(td: &TrashDir, path: &Path, canonical: &Path) -> Result<Trashed, Attempt> {
    td.ensure().map_err(|e| {
        Attempt::Next(format!(
            "this drive has no Trash and one can't be created ({})",
            reason_of(&e)
        ))
    })?;
    let slot = reserve_in(td, path, canonical)
        .map_err(|e| Attempt::Next(format!("the Trash can't take it ({})", reason_of(&e))))?;
    match rename_noreplace(canonical, &slot.trashed) {
        Ok(()) => {
            remember(td);
            Ok(slot)
        }
        Err(e) => {
            release(&slot);
            Err(match e.raw_os_error() {
                Some(libc::EXDEV) => {
                    Attempt::Next("the Trash is on another filesystem".to_string())
                }
                Some(libc::EROFS) => {
                    Attempt::Stop(TrashError::NoTrash("the drive is read-only".into()))
                }
                _ if e.kind() == io::ErrorKind::PermissionDenied => {
                    Attempt::Stop(TrashError::Denied(e))
                }
                _ => Attempt::Stop(TrashError::Io(e)),
            })
        }
    }
}

/// Trash `path` into one given trash directory: lets tests (here and in
/// `ops`) work against a scratch trash instead of the user's real one.
#[cfg(test)]
pub(crate) fn trash_in(td: &TrashDir, path: &Path) -> Result<Trashed, TrashError> {
    let canonical = canonical_entry(path).map_err(TrashError::Io)?;
    try_in(td, path, &canonical).map_err(|attempt| match attempt {
        Attempt::Next(why) => TrashError::NoTrash(why),
        Attempt::Stop(e) => e,
    })
}

/// Move `path` to the trash of its filesystem. On success the item is at
/// `Trashed::trashed` with its sidecar written; on any error nothing has
/// changed. A symlink is trashed as the link, never its target.
pub fn trash(path: &Path) -> Result<Trashed, TrashError> {
    // Into the Trash it would take the mounted device along. Asked first:
    // the mount point of a phone is on a slow path and is not "an item
    // without a Trash" that may be deleted instead.
    if crate::mount_guard::holds_mount(path) {
        return Err(TrashError::Mounted);
    }
    if crate::fs::is_slow_path(path) {
        return Err(TrashError::NoTrash(
            "phones and network folders have no Trash".into(),
        ));
    }
    let canonical = canonical_entry(path).map_err(TrashError::Io)?;
    std::fs::symlink_metadata(&canonical).map_err(TrashError::Io)?;
    if locate(&canonical).is_some() {
        return Err(TrashError::NoTrash("it is already in the Trash".into()));
    }
    let parent = canonical.parent().unwrap_or(Path::new("/")).to_path_buf();
    let dev = std::fs::metadata(&parent)
        .map(|m| m.dev())
        .map_err(TrashError::Io)?;

    // The home trash first when the item shares its filesystem; the volume
    // trash otherwise, or when the home trash turns out to sit across a bind
    // mount. Volume trashes are only created when they are needed, so no
    // stray `.Trash-N` appears next to a working home trash.
    let mut why: Option<String> = None;
    let home = home_trash();
    let mut candidates = Vec::new();
    match home.ensure() {
        Ok(()) => {
            if std::fs::metadata(home.files()).is_ok_and(|m| m.dev() == dev) {
                candidates.push(home);
            }
        }
        Err(e) => eprintln!("[fox] home Trash unusable: {e}"),
    }
    for round in 0..2 {
        for td in &candidates {
            match try_in(td, path, &canonical) {
                Ok(t) => return Ok(t),
                Err(Attempt::Stop(e)) => return Err(e),
                Err(Attempt::Next(w)) => {
                    why.get_or_insert(w);
                }
            }
        }
        if round == 0 {
            candidates = volume_trashes(&top_dir(&parent));
        }
    }
    Err(TrashError::NoTrash(
        why.unwrap_or_else(|| "this drive has no Trash".into()),
    ))
}

/// A slot in the home trash for an item that a privileged `mv` will put
/// there (root can cross filesystems; the record is absolute). The caller
/// moves the item to `Trashed::trashed`, or calls [`release`].
pub fn reserve_in_home(path: &Path) -> Result<Trashed, TrashError> {
    let canonical = canonical_entry(path).map_err(TrashError::Io)?;
    let home = home_trash();
    home.ensure()
        .map_err(|e| TrashError::NoTrash(format!("the Trash can't be used ({})", reason_of(&e))))?;
    reserve_in(&home, path, &canonical).map_err(TrashError::Io)
}

/// Undo of a trash: put the item back at its original path. Refuses when
/// something else has that name now; the item then stays in the trash.
pub fn restore(t: &Trashed) -> io::Result<()> {
    if let Some(parent) = t.original.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rename_noreplace(&t.trashed, &t.original)?;
    let _ = std::fs::remove_file(&t.info);
    Ok(())
}

/// After an item at the top level of a trash was deleted for good: its
/// restore record goes too. Anything else is left alone.
pub fn forget(path: &Path) {
    let Some(loc) = locate(path) else { return };
    let mut comps = loc.rel.components();
    if let (Some(Component::Normal(name)), None) = (comps.next(), comps.next()) {
        let mut info = name.to_os_string();
        info.push(".trashinfo");
        let _ = std::fs::remove_file(loc.trash.info().join(info));
    }
}

#[cfg(test)]
mod tests;
