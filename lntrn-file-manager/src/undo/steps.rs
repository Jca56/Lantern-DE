//! The single steps of an undo or redo: one item, one direction. Each
//! looks at what is on disk now before it touches anything; `exec` has the
//! rules they keep and strings them together.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::exec::{Env, Step};
use super::words::name_of;
use super::{Note, Stamp};
use crate::copy_tree::{reason_of, rename_noreplace};
use crate::ops::OpFailure;
use crate::trash::{TrashError, Trashed};

/// How the thing at a path relates to the item an action recorded.
enum Seen {
    Gone,
    /// The very item.
    Same,
    /// Something is there and no stamp was taken to compare it with.
    Unknown,
    /// Another file has the name now.
    Other,
}

fn look(path: &Path, stamp: Option<Stamp>) -> Seen {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Seen::Gone;
    };
    match stamp {
        None => Seen::Unknown,
        Some(then) if then.same_item(&Stamp::from_meta(&meta)) => Seen::Same,
        Some(_) => Seen::Other,
    }
}

fn already_exists(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::EEXIST)
}

/// A rename that only changes letter case, on a drive that ignores case:
/// there the new name "exists" because it is the item itself.
fn is_case_change(from: &Path, to: &Path) -> bool {
    let lower = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    from.parent() == to.parent() && lower(from) == lower(to) && crate::app::is_same_entry(from, to)
}

fn move_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    match rename_noreplace(from, to) {
        Err(e) if already_exists(&e) && is_case_change(from, to) => std::fs::rename(from, to),
        other => other,
    }
}

/// Move the item at `here` to `there` (one half of a rename or a move, in
/// either direction) without replacing anything.
pub(super) fn shift<T>(here: &Path, there: &Path, note: &Note, item: T) -> Step<T> {
    match look(here, note.stamp) {
        Seen::Gone => return Step::Failed(here.to_path_buf(), "it is no longer there".into()),
        Seen::Other => {
            return Step::Failed(
                here.to_path_buf(),
                "another item has this name now; it was left alone".into(),
            )
        }
        Seen::Same | Seen::Unknown => {}
    }
    let moved = |stamp: Option<Stamp>| Note {
        stamp,
        parked: None,
    };
    match move_noreplace(here, there) {
        // The inode travels with a rename, so the stamp stays good.
        Ok(()) => Step::Done(item, moved(note.stamp.or_else(|| Stamp::of(there)))),
        Err(e) if already_exists(&e) => Step::Failed(
            there.to_path_buf(),
            format!(
                "an item with this name exists now; nothing was overwritten and \u{201C}{}\u{201D} stays where it is",
                name_of(here)
            ),
        ),
        // Root cannot see into a user's phone or network mount.
        Err(e)
            if e.kind() == io::ErrorKind::PermissionDenied
                && !crate::fs::is_slow_path(here)
                && !crate::fs::is_slow_path(there) =>
        {
            Step::Root(here.to_path_buf(), there.to_path_buf(), item, moved(note.stamp))
        }
        Err(e) => Step::Failed(here.to_path_buf(), reason_of(&e)),
    }
}

type TrashItem = (PathBuf, PathBuf, PathBuf);

/// Undo of a trash: back to where it was, unless something else is there.
pub(super) fn untrash(item: &TrashItem, note: &Note) -> Step<TrashItem> {
    let (original, trashed, info) = item;
    match look(trashed, note.stamp) {
        Seen::Gone => {
            return Step::Failed(original.clone(), "it is no longer in the Trash".into())
        }
        Seen::Other => {
            return Step::Failed(
                original.clone(),
                "the Trash holds a different item under its name now".into(),
            )
        }
        Seen::Same | Seen::Unknown => {}
    }
    let record = Trashed {
        original: original.clone(),
        trashed: trashed.clone(),
        info: info.clone(),
    };
    match crate::trash::restore(&record) {
        Ok(()) => Step::Done(
            item.clone(),
            Note {
                stamp: note.stamp.or_else(|| Stamp::of(original)),
                parked: None,
            },
        ),
        Err(e) if already_exists(&e) => Step::Failed(
            original.clone(),
            "an item with this name exists now; nothing was overwritten and yours is still in the Trash"
                .into(),
        ),
        Err(e) => Step::Failed(original.clone(), reason_of(&e)),
    }
}

/// Redo of a trash: into the Trash again, under a fresh record.
pub(super) fn retrash(original: &Path, note: &Note, env: &Env) -> Step<TrashItem> {
    match look(original, note.stamp) {
        Seen::Gone => {
            return Step::Failed(original.to_path_buf(), "it is no longer there".into())
        }
        Seen::Other => {
            return Step::Failed(
                original.to_path_buf(),
                "another item has this name now; it was left alone".into(),
            )
        }
        Seen::Same | Seen::Unknown => {}
    }
    match (env.trash)(original) {
        Ok(t) => {
            let stamp = note.stamp.or_else(|| Stamp::of(&t.trashed));
            Step::Done(
                (original.to_path_buf(), t.trashed, t.info),
                Note {
                    stamp,
                    parked: None,
                },
            )
        }
        Err(e) => Step::Failed(
            original.to_path_buf(),
            format!("it can\u{2019}t be moved to the Trash ({})", e.reason()),
        ),
    }
}

fn no_trash_reason(path: &Path) -> String {
    if crate::fs::is_slow_path(path) {
        "this device has no Trash to move it to; it was left in place".into()
    } else {
        "it can\u{2019}t be told apart from an item that replaced it; it was left alone".into()
    }
}

/// Undo of New File / New Folder: the item goes to the Trash, whatever has
/// been put into it since.
pub(super) fn uncreate(
    path: &Path,
    is_dir: bool,
    note: &Note,
    env: &Env,
) -> Step<(PathBuf, bool)> {
    let item = (path.to_path_buf(), is_dir);
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Step::Gone;
    };
    let now = Stamp::from_meta(&meta);
    match note.stamp {
        Some(then) if then.same_item(&now) => match (env.trash)(path) {
            Ok(t) => {
                return Step::Done(
                    item,
                    Note {
                        stamp: note.stamp,
                        parked: Some(t),
                    },
                )
            }
            // No Trash on this drive: a folder may still go if it is empty.
            Err(TrashError::NoTrash(_)) if meta.is_dir() => {}
            Err(e) => {
                return Step::Failed(
                    item.0,
                    format!(
                        "it can\u{2019}t be moved to the Trash ({}); it was left in place",
                        e.reason()
                    ),
                )
            }
        },
        Some(_) => {
            return Step::Failed(
                item.0,
                "it is not the item that was created any more; it was left alone".into(),
            )
        }
        // No stamp (a phone, a network folder): only a folder that was
        // created as one and is one can be tried, by rmdir.
        None if is_dir && meta.is_dir() => {}
        None => return Step::Failed(item.0, no_trash_reason(path)),
    }
    // rmdir removes nothing that holds anything: the system refuses a
    // folder that is not empty.
    match std::fs::remove_dir(path) {
        Ok(()) => Step::Done(item, Note::default()),
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTEMPTY) | Some(libc::EEXIST)) => {
            Step::Failed(
                item.0,
                "it is not empty and there is no Trash here; it was left in place".into(),
            )
        }
        Err(e) => Step::Failed(item.0, reason_of(&e)),
    }
}

enum Unparked {
    /// Back at its place, out of the Trash.
    Restored,
    /// Not in the Trash any more (emptied since).
    Missing,
    Failed(String),
}

/// Take an item that undo moved to the Trash back out, to `t.original`.
fn unpark(t: &Trashed, stamp: Option<Stamp>) -> Unparked {
    if !matches!(look(&t.trashed, stamp), Seen::Same | Seen::Unknown) {
        return Unparked::Missing;
    }
    match crate::trash::restore(t) {
        Ok(()) => Unparked::Restored,
        Err(e) if already_exists(&e) => Unparked::Failed(
            "an item with this name exists now; nothing was overwritten and the one that was undone is still in the Trash"
                .into(),
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Unparked::Missing,
        Err(e) => Unparked::Failed(reason_of(&e)),
    }
}

/// Redo of New File / New Folder: the item that undo trashed comes back; if
/// the Trash was emptied since, a new empty one is made.
pub(super) fn recreate(path: &Path, is_dir: bool, note: &Note) -> Step<(PathBuf, bool)> {
    let item = (path.to_path_buf(), is_dir);
    if let Some(t) = &note.parked {
        match unpark(t, note.stamp) {
            Unparked::Restored => {
                return Step::Done(
                    item,
                    Note {
                        stamp: note.stamp,
                        parked: None,
                    },
                )
            }
            Unparked::Failed(reason) => return Step::Failed(item.0, reason),
            Unparked::Missing => {}
        }
    }
    // Exclusive: never onto, or into, something that has the name now.
    let made = if is_dir {
        std::fs::create_dir(path)
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(drop)
    };
    match made {
        Ok(()) => Step::Done(
            item,
            Note {
                stamp: Stamp::of(path),
                parked: None,
            },
        ),
        Err(e) if already_exists(&e) => Step::Failed(
            item.0,
            "an item with this name exists now; nothing was overwritten".into(),
        ),
        Err(e) => Step::Failed(item.0, reason_of(&e)),
    }
}

type CopyItem = (PathBuf, PathBuf);

/// Undo of a copy: the copy goes to the Trash, if it still is the copy.
pub(super) fn uncopy(item: &CopyItem, note: &Note, env: &Env) -> Step<CopyItem> {
    let created = &item.1;
    match look(created, note.stamp) {
        Seen::Gone => Step::Gone,
        Seen::Other => Step::Failed(
            created.clone(),
            "it is not the item that was copied here any more; it was left alone".into(),
        ),
        Seen::Unknown => Step::Failed(created.clone(), no_trash_reason(created)),
        Seen::Same => match (env.trash)(created) {
            Ok(t) => Step::Done(
                item.clone(),
                Note {
                    stamp: note.stamp,
                    parked: Some(t),
                },
            ),
            Err(e) => Step::Failed(
                created.clone(),
                format!(
                    "it can\u{2019}t be moved to the Trash ({}); it was left in place",
                    e.reason()
                ),
            ),
        },
    }
}

/// Redo of a copy: the copy that undo trashed comes back. Only when the
/// Trash was emptied since is the source copied again.
pub(super) fn recopy(
    item: &CopyItem,
    note: &Note,
    env: &Env,
    failures: &mut Vec<OpFailure>,
) -> Step<CopyItem> {
    let (src, created) = item;
    if let Some(t) = &note.parked {
        match unpark(t, note.stamp) {
            Unparked::Restored => {
                return Step::Done(
                    item.clone(),
                    Note {
                        stamp: note.stamp,
                        parked: None,
                    },
                )
            }
            Unparked::Failed(reason) => return Step::Failed(created.clone(), reason),
            Unparked::Missing => {}
        }
    }
    if std::fs::symlink_metadata(created).is_ok() {
        return Step::Failed(
            created.clone(),
            "an item with this name exists now; nothing was overwritten".into(),
        );
    }
    if std::fs::symlink_metadata(src).is_err() {
        return Step::Failed(
            src.clone(),
            "it is gone, and its copy is no longer in the Trash".into(),
        );
    }
    let (landed, mut missed) = (env.copy)(src, created);
    if !landed && env.cancel.load(Ordering::SeqCst) {
        return Step::Stopped;
    }
    if !landed {
        // Nothing of the copy is left; the copier's first reason speaks
        // for the item, any others are listed as they are.
        if missed.is_empty() {
            return Step::Failed(created.clone(), "it could not be copied again".into());
        }
        let first = missed.remove(0);
        failures.append(&mut missed);
        return Step::Failed(first.path, first.reason);
    }
    // In place, possibly with entries of a folder missing: those are told.
    failures.append(&mut missed);
    Step::Done(
        item.clone(),
        Note {
            stamp: Stamp::of(created),
            parked: None,
        },
    )
}
