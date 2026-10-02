//! The second half of a cross-device move: deleting the source, but only
//! the parts whose copy checks out.

use std::io;
use std::path::Path;

use super::{reason_of, special_reason, EntryFailure};

/// Second half of a cross-device move, once the copy is at `dst`: delete
/// from `src` only what verifiably arrived. A file goes when its copy
/// exists with the same size, a link when its copy is a link, a folder when
/// it has become empty. Anything else stays where it was. Returns true when
/// `src` is completely gone.
pub fn remove_moved_source(src: &Path, dst: &Path, failures: &mut Vec<EntryFailure>) -> bool {
    let Ok(sm) = std::fs::symlink_metadata(src) else {
        // Already gone: nothing left to lose.
        return true;
    };
    let dm = std::fs::symlink_metadata(dst).ok();
    let ft = sm.file_type();
    if ft.is_symlink() {
        if !dm.is_some_and(|m| m.file_type().is_symlink()) {
            return kept(failures, src, "left in place: its copy is missing");
        }
        remove_verified(std::fs::remove_file(src), failures, src)
    } else if ft.is_dir() {
        if !dm.is_some_and(|m| m.is_dir()) {
            return kept(failures, src, "left in place: its copy is missing");
        }
        let Ok(entries) = std::fs::read_dir(src) else {
            return kept(failures, src, "left in place: the folder could not be read");
        };
        let mut all_gone = true;
        for entry in entries {
            let Ok(entry) = entry else {
                all_gone = false;
                continue;
            };
            let name = entry.file_name();
            all_gone &= remove_moved_source(&entry.path(), &dst.join(&name), failures);
        }
        // Whatever stayed behind keeps its parent folders (and has already
        // been reported); `remove_dir` only ever removes an empty folder.
        all_gone && remove_verified(std::fs::remove_dir(src), failures, src)
    } else if ft.is_file() {
        if !dm.is_some_and(|m| m.is_file() && m.len() == sm.len()) {
            return kept(failures, src, "left in place: its copy could not be verified");
        }
        remove_verified(std::fs::remove_file(src), failures, src)
    } else {
        kept(failures, src, special_reason(&ft))
    }
}

/// Note that `src` stays where it is (once: the copy pass may already have
/// reported the same path). Always false, for the caller's "all gone".
fn kept(failures: &mut Vec<EntryFailure>, src: &Path, why: &str) -> bool {
    if !failures.iter().any(|f| f.path == src) {
        failures.push(EntryFailure {
            path: src.to_path_buf(),
            reason: why.to_string(),
        });
    }
    false
}

fn remove_verified(res: io::Result<()>, failures: &mut Vec<EntryFailure>, src: &Path) -> bool {
    match res {
        Ok(()) => true,
        Err(e) => kept(
            failures,
            src,
            &format!("copied, but the original could not be removed: {}", reason_of(&e)),
        ),
    }
}

