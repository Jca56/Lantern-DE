//! The ops worker thread: runs one operation's items in order and reports
//! back. The guarantees it keeps are listed at the top of `ops.rs`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use super::{OpItem, OpKind, OpOutcome, OpProgress};
use crate::copy_tree::{
    copy_item, reason_of, remove_moved_source, remove_tree, rename_noreplace, temp_sibling,
    CopyError,
};
use crate::trash::{TrashError, Trashed};

/// How the worker moves a replaced item to the Trash. A parameter so the
/// tests can point it at a scratch trash instead of the real one.
pub(super) type Trasher<'a> = &'a dyn Fn(&Path) -> Result<Trashed, TrashError>;

pub(super) fn run(
    kind: OpKind,
    items: Vec<OpItem>,
    tx: Sender<OpProgress>,
    cancel: &AtomicBool,
    trasher: Trasher,
) {
    let total = items.len();
    let mut out = OpOutcome::default();
    for (i, item) in items.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            out.cancelled = true;
            break;
        }
        let shown = match kind {
            OpKind::Delete => &item.src,
            _ => &item.target,
        };
        let name = shown
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| shown.display().to_string());
        let _ = tx.send(OpProgress::StartedItem {
            index: i,
            total,
            name,
        });
        match kind {
            OpKind::Copy => copy_one(item, cancel, trasher, &mut out),
            OpKind::Move => move_one(item, cancel, trasher, &mut out),
            OpKind::Delete => delete_one(&item.src, &mut out),
            // Runs in `history::run`, never through here.
            OpKind::History => {}
        }
        if out.cancelled {
            break;
        }
    }
    // Final tick so the bar reaches 100% before the Done event.
    let _ = tx.send(OpProgress::StartedItem {
        index: total,
        total,
        name: String::new(),
    });
    let _ = tx.send(OpProgress::Done(out));
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Checks before anything is written. `Some(replacing)` to go ahead.
fn preflight(item: &OpItem, out: &mut OpOutcome) -> Option<bool> {
    if !exists(&item.target) {
        return Some(false);
    }
    if !item.replace {
        // The conflict dialog saw a free name; something took it since.
        out.fail(
            &item.target,
            "an item with this name already exists; nothing was overwritten",
        );
        return None;
    }
    // The UI never offers Replace for these. Checked again here because
    // this is the code that would trash the target.
    if crate::app::is_same_entry(&item.src, &item.target)
        || crate::file_ops::target_holds(&item.target, &item.src)
    {
        out.fail(
            &item.target,
            "not replaced: it is, or contains, the item being pasted",
        );
        return None;
    }
    if crate::fs::is_slow_path(&item.target) {
        out.fail(
            &item.target,
            "not replaced: this device has no Trash to keep the existing item in",
        );
        return None;
    }
    Some(true)
}

pub(super) fn copy_one(item: &OpItem, cancel: &AtomicBool, trasher: Trasher, out: &mut OpOutcome) {
    let Some(replacing) = preflight(item, out) else {
        return;
    };
    if copy_and_land(item, replacing, cancel, trasher, out) {
        out.created.push((item.src.clone(), item.target.clone()));
    }
}

/// Copy `item.src` and put the finished copy at `item.target`. True when it
/// is there. On false nothing of it is left and an existing target is
/// untouched.
fn copy_and_land(
    item: &OpItem,
    replacing: bool,
    cancel: &AtomicBool,
    trasher: Trasher,
    out: &mut OpOutcome,
) -> bool {
    // `replacing` is never set on a slow mount (see preflight).
    let in_place = crate::fs::is_slow_path(&item.target);
    let staging = if in_place {
        item.target.clone()
    } else {
        staging_for(item)
    };
    let report = match copy_item(&item.src, &staging, cancel) {
        Ok(report) => report,
        Err(CopyError::Cancelled) => {
            out.cancelled = true;
            return false;
        }
        // Worth a try as root, except on a phone or a network folder:
        // root cannot even see a user's FUSE mount.
        Err(CopyError::Failed(e))
            if e.kind() == std::io::ErrorKind::PermissionDenied
                && !in_place
                && !crate::fs::is_slow_path(&item.src) =>
        {
            out.perm_fails.push(item.clone());
            return false;
        }
        Err(CopyError::Failed(e)) => {
            out.fail(&item.src, &reason_of(&e));
            return false;
        }
    };
    if replacing && !report.failures.is_empty() {
        // The old item only makes way for a complete new one.
        remove_tree(&staging);
        out.failures.extend(report.failures);
        out.fail(
            &item.target,
            "not replaced: the new copy is incomplete, so the existing item was kept",
        );
        return false;
    }
    if !in_place {
        if let Err(why) = land(&staging, &item.target, replacing, trasher, out) {
            remove_tree(&staging);
            out.fail(&item.target, &why);
            return false;
        }
    }
    out.failures.extend(report.failures);
    true
}

/// Where a new item is built until it is complete: a hidden name next to
/// its target, so the last step is a rename inside one folder.
///
/// One exception. Cloud sync skips staging *files* by their name, but it
/// walks into every folder under ~/Cloud: a folder staged there would be
/// uploaded under its staging name and then read as deleted when it is
/// renamed. A folder bound for the synced tree is staged next to the cloud
/// folder instead, when that is the same filesystem (the rename must not
/// turn into a second copy).
fn staging_for(item: &OpItem) -> std::path::PathBuf {
    let cloud = crate::cloud::cloud_root();
    let is_folder = std::fs::symlink_metadata(&item.src).is_ok_and(|m| m.is_dir());
    if is_folder && item.target.starts_with(&cloud) {
        if let (Some(outside), Some(name)) = (cloud.parent(), item.target.file_name()) {
            let candidate = temp_sibling(&outside.join(name));
            if same_filesystem(&candidate, &item.target) {
                return candidate;
            }
        }
    }
    temp_sibling(&item.target)
}

/// Give a complete staging item its real name. With `replacing` the old
/// item goes to the Trash first and comes back if the rename fails.
fn land(
    staging: &Path,
    target: &Path,
    replacing: bool,
    trasher: Trasher,
    out: &mut OpOutcome,
) -> Result<(), String> {
    let old = if replacing {
        match trasher(target) {
            Ok(t) => Some(t),
            Err(e) => {
                return Err(format!(
                    "not replaced: the existing item can't be moved to the Trash ({})",
                    e.reason()
                ))
            }
        }
    } else {
        None
    };
    match rename_noreplace(staging, target) {
        Ok(()) => {
            out.replaced.extend(old);
            Ok(())
        }
        Err(e) => {
            if let Some(t) = &old {
                if crate::trash::restore(t).is_err() {
                    // Nothing is lost, but it is not where it was: say where.
                    return Err(format!(
                        "could not be put in place ({}); the item it was replacing is in the Trash",
                        reason_of(&e)
                    ));
                }
            }
            Err(reason_of(&e))
        }
    }
}

fn move_one(item: &OpItem, cancel: &AtomicBool, trasher: Trasher, out: &mut OpOutcome) {
    let Some(replacing) = preflight(item, out) else {
        return;
    };
    if same_filesystem(&item.src, &item.target) {
        match rename_into_place(item, replacing, trasher, out) {
            Renamed::Done | Renamed::Failed => return,
            // Same device, but across a bind mount: copy after all.
            Renamed::CrossDevice => {}
        }
    }
    // By copy: that would walk into a drive, phone or share mounted inside
    // the folder, and the clean-up afterwards would empty it.
    if crate::mount_guard::holds_mount(&item.src) {
        out.fail(&item.src, crate::mount_guard::REASON);
        return;
    }
    if copy_and_land(item, replacing, cancel, trasher, out) {
        // The copy is in place. Only now may the source go, and only the
        // parts whose copy checks out.
        let gone = remove_moved_source(&item.src, &item.target, &mut out.failures);
        if !gone && !out.failures.iter().any(|f| f.path.starts_with(&item.src)) {
            out.fail(&item.src, "copied, but the original could not be removed");
        }
        out.created.push((item.src.clone(), item.target.clone()));
    }
}

/// Both paths' folders on one device, so a rename can do the move.
pub(super) fn same_filesystem(src: &Path, target: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let dev = |p: &Path| {
        p.parent()
            .and_then(|d| std::fs::metadata(d).ok())
            .map(|m| m.dev())
    };
    match (dev(src), dev(target)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

enum Renamed {
    Done,
    CrossDevice,
    Failed,
}

fn rename_into_place(
    item: &OpItem,
    replacing: bool,
    trasher: Trasher,
    out: &mut OpOutcome,
) -> Renamed {
    let old = if replacing {
        match trasher(&item.target) {
            Ok(t) => Some(t),
            Err(e) => {
                out.fail(
                    &item.target,
                    &format!(
                        "not replaced: the existing item can't be moved to the Trash ({})",
                        e.reason()
                    ),
                );
                return Renamed::Failed;
            }
        }
    } else {
        None
    };
    match rename_noreplace(&item.src, &item.target) {
        Ok(()) => {
            out.replaced.extend(old);
            out.renamed.push((item.src.clone(), item.target.clone()));
            Renamed::Done
        }
        Err(e) => {
            // The old item comes back before anything else is tried.
            if let Some(t) = &old {
                if crate::trash::restore(t).is_err() {
                    out.fail(
                        &item.target,
                        &format!(
                            "not moved ({}); the item it was replacing is in the Trash",
                            reason_of(&e)
                        ),
                    );
                    return Renamed::Failed;
                }
            }
            if e.raw_os_error() == Some(libc::EXDEV) {
                return Renamed::CrossDevice;
            }
            // Worth a try as root, except on a phone or a network folder:
            // root cannot even see a user's FUSE mount.
            if e.kind() == std::io::ErrorKind::PermissionDenied
                && !crate::fs::is_slow_path(&item.src)
                && !crate::fs::is_slow_path(&item.target)
            {
                out.perm_fails.push(item.clone());
            } else {
                out.fail(&item.src, &reason_of(&e));
            }
            Renamed::Failed
        }
    }
}

fn delete_one(path: &Path, out: &mut OpOutcome) {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        // Already gone is what was asked for.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            out.fail(path, &reason_of(&e));
            return;
        }
    };
    // Deleting through a mount point would empty the mounted drive, phone
    // or share (a trashed folder can still carry one).
    if meta.is_dir() && crate::mount_guard::holds_mount(path) {
        out.fail(path, crate::mount_guard::REASON);
        return;
    }
    // A symlink is unlinked, whatever it points at.
    let res = if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match res {
        // A top-level trash item takes its restore record with it.
        Ok(()) => crate::trash::forget(path),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            out.perm_fails.push(OpItem::delete(path.to_path_buf()));
        }
        Err(e) => out.fail(path, &reason_of(&e)),
    }
}

