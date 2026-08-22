//! Background file indexer: initial `$HOME` walk + live inotify watch.
//!
//! Runs on its own thread (spawned by [`super::FileIndex::spawn`]). The
//! walk seeds the shared index; the inotify loop then blocks on the
//! watch fd and applies create/delete/move events as they arrive, so the
//! index tracks the filesystem without polling.
//!
//! We add one inotify watch per directory (inotify is not recursive). On
//! a very large tree this can exhaust `max_user_watches`; we degrade
//! gracefully — the affected subtree keeps its startup snapshot, it just
//! stops receiving live updates.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::{should_descend, should_index, FileEntry, MAX_INDEXED};

/// Size of the fixed `struct inotify_event` header (wd, mask, cookie,
/// len — four 32-bit fields). The variable-length name follows.
const EVENT_HDR: usize = 16;

/// inotify watch mask. We care about additions, removals and moves
/// within each watched directory. `IN_EXCL_UNLINK` stops events for
/// already-unlinked files; `IN_ONLYDIR` makes add_watch fail loudly if
/// we ever hand it a non-directory.
fn watch_mask() -> u32 {
    (libc::IN_CREATE
        | libc::IN_DELETE
        | libc::IN_MOVED_FROM
        | libc::IN_MOVED_TO
        | libc::IN_DELETE_SELF
        | libc::IN_MOVE_SELF
        | libc::IN_EXCL_UNLINK
        | libc::IN_ONLYDIR) as u32
}

/// Per-thread watcher state shared between the walk and the event loop.
struct Watcher {
    fd: i32,
    /// watch descriptor → the directory it watches. Needed to rebuild a
    /// full path from an event (which only carries the wd + leaf name).
    wd_to_dir: HashMap<i32, PathBuf>,
    /// Latched once we hit the kernel watch limit, so we only warn once.
    warned_enospc: bool,
}

/// Thread entry point. Never returns until the watch fd errors out
/// (which it shouldn't under normal operation).
pub fn run(entries: Arc<Mutex<Vec<FileEntry>>>, ready: Arc<AtomicBool>, roots: Vec<PathBuf>) {
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        tracing::warn!(
            err = %std::io::Error::last_os_error(),
            "inotify_init1 failed — file search will be a static snapshot",
        );
        // Still build a one-shot snapshot so search isn't dead.
        let mut w = Watcher {
            fd: -1,
            wd_to_dir: HashMap::new(),
            warned_enospc: false,
        };
        for root in roots {
            walk(&mut w, &entries, root);
        }
        ready.store(true, Ordering::Relaxed);
        return;
    }

    let mut w = Watcher {
        fd,
        wd_to_dir: HashMap::new(),
        warned_enospc: false,
    };

    let t0 = std::time::Instant::now();
    for root in roots {
        walk(&mut w, &entries, root);
    }
    ready.store(true, Ordering::Relaxed);
    tracing::info!(
        count = entries.lock().map(|g| g.len()).unwrap_or(0),
        watches = w.wd_to_dir.len(),
        ms = t0.elapsed().as_millis(),
        "file index built",
    );

    watch_loop(&mut w, &entries);
    tracing::warn!("inotify watch loop ended — file index frozen");
    unsafe { libc::close(fd) };
}

// ── Initial walk ──────────────────────────────────────────────────────────────

/// Iterative depth-first walk. Watches each directory before listing it
/// (so events during the scan aren't lost) and appends children to the
/// shared index one directory at a time, giving incremental visibility.
fn walk(w: &mut Watcher, entries: &Arc<Mutex<Vec<FileEntry>>>, root: PathBuf) {
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        if w.fd >= 0 {
            add_watch(w, &dir);
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue, // unreadable (perms / vanished) — skip
        };

        let mut batch: Vec<FileEntry> = Vec::new();
        for ent in rd.flatten() {
            let path = ent.path();
            let Some(name) = path.file_name().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            // `file_type` does NOT follow symlinks, so a symlinked
            // directory reports `is_dir() == false` and we never recurse
            // into it — that's our loop guard.
            let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && should_descend(&name) {
                stack.push(path.clone());
            }
            // Hidden files / hidden dirs stay out of results entirely.
            if should_index(&name) {
                batch.push(FileEntry { path, name, is_dir });
            }
        }

        if let Ok(mut g) = entries.lock() {
            let room = MAX_INDEXED.saturating_sub(g.len());
            if room == 0 {
                tracing::warn!(cap = MAX_INDEXED, "file index hit cap — stopping walk");
                return;
            }
            if batch.len() > room {
                batch.truncate(room);
            }
            g.extend(batch);
        }
    }
}

// ── Live event loop ─────────────────────────────────────────────────────────

fn watch_loop(w: &mut Watcher, entries: &Arc<Mutex<Vec<FileEntry>>>) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = unsafe { libc::read(w.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            tracing::warn!(%err, "inotify read failed");
            return;
        }
        if n == 0 {
            return;
        }

        let n = n as usize;
        let mut off = 0usize;
        while off + EVENT_HDR <= n {
            let wd = i32::from_ne_bytes(buf[off..off + 4].try_into().unwrap());
            let mask = u32::from_ne_bytes(buf[off + 4..off + 8].try_into().unwrap());
            let len = u32::from_ne_bytes(buf[off + 12..off + 16].try_into().unwrap()) as usize;
            let name_start = off + EVENT_HDR;
            let name_end = name_start + len;
            if name_end > n {
                break; // truncated — shouldn't happen, but don't over-read
            }
            let name = parse_name(&buf[name_start..name_end]);
            handle_event(w, entries, wd, mask, name);
            off = name_end;
        }
    }
}

/// Extract the leaf name from an inotify event's NUL-padded name field.
fn parse_name(bytes: &[u8]) -> Option<PathBuf> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&bytes[..end])))
}

fn handle_event(
    w: &mut Watcher,
    entries: &Arc<Mutex<Vec<FileEntry>>>,
    wd: i32,
    mask: u32,
    name: Option<PathBuf>,
) {
    // The kernel dropped our watch (dir deleted/unmounted): forget it.
    if mask & libc::IN_IGNORED as u32 != 0 {
        w.wd_to_dir.remove(&wd);
        return;
    }
    // Queue overflow (wd == -1): we may have missed events. Best-effort —
    // we don't re-walk; stale entries get corrected on the next change.
    if wd < 0 {
        tracing::warn!("inotify queue overflow — index may briefly drift");
        return;
    }
    let Some(name) = name else { return };
    let Some(dir) = w.wd_to_dir.get(&wd).cloned() else {
        return;
    };
    let full = dir.join(&name);
    let is_dir = mask & libc::IN_ISDIR as u32 != 0;

    let name_str = name.to_string_lossy();
    if mask & (libc::IN_CREATE | libc::IN_MOVED_TO) as u32 != 0 {
        if should_index(&name_str) {
            add_entry(entries, &full, is_dir);
        }
        // A moved-in directory may already contain a subtree; a freshly
        // created one is empty (cheap walk). Either way, start watching.
        if is_dir && should_descend(&name_str) {
            walk(w, entries, full);
        }
    } else if mask & (libc::IN_DELETE | libc::IN_MOVED_FROM) as u32 != 0 {
        remove_path(entries, &full, is_dir);
        if is_dir {
            remove_watches_under(w, &full);
        }
    }
}

// ── Index mutation helpers ──────────────────────────────────────────────────

fn add_entry(entries: &Arc<Mutex<Vec<FileEntry>>>, full: &Path, is_dir: bool) {
    let Some(name) = full.file_name().map(|s| s.to_string_lossy().into_owned()) else {
        return;
    };
    if let Ok(mut g) = entries.lock() {
        if g.len() >= MAX_INDEXED {
            return;
        }
        g.push(FileEntry {
            path: full.to_path_buf(),
            name,
            is_dir,
        });
    }
}

/// Remove `full` from the index. If it's a directory, also drop every
/// indexed descendant (the kernel only tells us about the dir itself).
fn remove_path(entries: &Arc<Mutex<Vec<FileEntry>>>, full: &Path, is_dir: bool) {
    if let Ok(mut g) = entries.lock() {
        if is_dir {
            g.retain(|e| e.path != full && !e.path.starts_with(full));
        } else {
            g.retain(|e| e.path != full);
        }
    }
}

// ── inotify watch helpers ────────────────────────────────────────────────────

fn add_watch(w: &mut Watcher, dir: &Path) {
    let Ok(cpath) = CString::new(dir.as_os_str().as_bytes()) else {
        return; // path contains an interior NUL — can't happen on real fs
    };
    let wd = unsafe { libc::inotify_add_watch(w.fd, cpath.as_ptr(), watch_mask()) };
    if wd < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOSPC) && !w.warned_enospc {
            w.warned_enospc = true;
            tracing::warn!(
                "inotify watch limit reached (max_user_watches) — some \
                 subtrees won't get live updates. Consider raising \
                 fs.inotify.max_user_watches.",
            );
        }
        return;
    }
    w.wd_to_dir.insert(wd, dir.to_path_buf());
}

/// Drop the watch on `dir` and every watch below it (after the directory
/// was deleted or moved away). Keeps `wd_to_dir` from leaking stale wds.
fn remove_watches_under(w: &mut Watcher, dir: &Path) {
    let stale: Vec<i32> = w
        .wd_to_dir
        .iter()
        .filter(|(_, p)| p.as_path() == dir || p.starts_with(dir))
        .map(|(wd, _)| *wd)
        .collect();
    for wd in stale {
        unsafe { libc::inotify_rm_watch(w.fd, wd) };
        w.wd_to_dir.remove(&wd);
    }
}
