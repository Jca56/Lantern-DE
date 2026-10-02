//! Symbolic links in a listing: what one points at.
//!
//! A listing's own stat of an entry (`DirEntry::metadata`) describes the
//! link, not its target, so a link to a folder used to be shown, sorted
//! and opened as a file. `Looker::look` answers "what is at the other
//! end": folder or not, the target's size and date.
//!
//! The one thing it must never do is ask a phone or a network share from
//! the thread that draws (fs.rs, "Slow-mount detection"): a link in the
//! home folder may well point into one. So before anything is followed,
//! the link is resolved step by step on local disks only (`resolve`). When
//! the trail leads onto a slow mount:
//!  - the link is remembered here, and from then on `fs::is_slow_path` is
//!    true for it and everything reached through it, which sends listings,
//!    thumbnails, probes and git down their slow-mount paths;
//!  - what the target is gets asked on a thread of its own. Until the
//!    answer is in the entry shows as a file; then the folder is listed
//!    again (`take_changed`).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

/// What a link points at, as far as a listing cares.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Target {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

/// Where following a path's symlinks ends.
#[derive(Debug, PartialEq)]
pub(crate) enum Resolved {
    /// Everything on the way is on a local disk; this is the real path.
    Local(PathBuf),
    /// The trail enters a slow mount here. Nothing past this point was
    /// looked at.
    Slow(PathBuf),
    /// A dangling link, a loop, or a path that cannot be followed.
    Broken,
}

/// More links in a row than this is a loop (the kernel's own limit).
const MAX_HOPS: usize = 40;

/// Follow the symlinks in `path` one component at a time, stopping before
/// the first step onto a slow mount. Every lstat and readlink made here is
/// of a path whose folder is known to be local.
pub(crate) fn resolve(path: &Path, is_slow: &dyn Fn(&Path) -> bool) -> Resolved {
    if !path.is_absolute() {
        return Resolved::Broken;
    }
    resolve_from(PathBuf::from("/"), path, is_slow)
}

/// `resolve` for `rest` below `at`, a folder already known to be real and
/// local. (A listing resolves its own folder once and each of its links
/// from there, instead of walking down from the root for every one.)
fn resolve_from(at: PathBuf, rest: &Path, is_slow: &dyn Fn(&Path) -> bool) -> Resolved {
    let mut todo: VecDeque<OsString> = VecDeque::new();
    let queue = |todo: &mut VecDeque<OsString>, path: &Path| {
        for part in path.components().rev() {
            match part {
                Component::Normal(name) => todo.push_front(name.to_os_string()),
                Component::ParentDir => todo.push_front("..".into()),
                _ => {}
            }
        }
    };
    queue(&mut todo, rest);
    let mut at = at;
    let mut hops = 0;
    while let Some(part) = todo.pop_front() {
        if part == ".." {
            at.pop();
            continue;
        }
        let next = at.join(&part);
        if is_slow(&next) {
            let mut rest = next;
            rest.extend(todo);
            return Resolved::Slow(rest);
        }
        let Ok(meta) = std::fs::symlink_metadata(&next) else {
            return Resolved::Broken;
        };
        if !meta.file_type().is_symlink() {
            at = next;
            continue;
        }
        hops += 1;
        if hops > MAX_HOPS {
            return Resolved::Broken;
        }
        let Ok(text) = std::fs::read_link(&next) else {
            return Resolved::Broken;
        };
        // The link's text takes the link's place; what came after it in
        // the path still follows.
        if text.is_absolute() {
            at = PathBuf::from("/");
        }
        queue(&mut todo, &text);
    }
    Resolved::Local(at)
}

// ── Links that lead onto a slow mount ───────────────────────────────────────

#[derive(Clone, PartialEq)]
enum Probe {
    /// Nobody has asked the device yet.
    Wanted,
    /// The probe thread is asking.
    Asked,
    /// `None`: there is nothing at the other end.
    Known(Option<Target>),
}

struct SlowLink {
    link: PathBuf,
    /// Where the trail enters the slow mount, plus the rest of the path.
    target: PathBuf,
    probe: Probe,
}

static LINKS: Mutex<Vec<SlowLink>> = Mutex::new(Vec::new());
/// `LINKS.len()`, readable without the lock: `leads_to_slow` runs for every
/// entry of every frame and the list is nearly always empty.
static LINK_COUNT: AtomicUsize = AtomicUsize::new(0);
static PROBING: AtomicBool = AtomicBool::new(false);
static CHANGED: AtomicBool = AtomicBool::new(false);

fn links() -> std::sync::MutexGuard<'static, Vec<SlowLink>> {
    LINKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `path` is, or lies behind, a link known to lead onto one of the slow
/// mounts `roots`. Part of `fs::is_slow_path`.
pub(crate) fn leads_to_slow(path: &Path, roots: &[PathBuf]) -> bool {
    if LINK_COUNT.load(Ordering::Relaxed) == 0 {
        return false;
    }
    links()
        .iter()
        .any(|l| path.starts_with(&l.link) && roots.iter().any(|root| l.target.starts_with(root)))
}

/// A probe answered since the last call: the listings that show such a
/// link are out of date.
pub(crate) fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

/// Remember `link` → `target` and say what is known about the target.
/// `None`: not known yet (a probe is started) or nothing there.
fn slow_target(link: &Path, target: PathBuf, is_mount_point: bool) -> Option<Target> {
    let (known, ask) = {
        let mut list = links();
        let at = match list.iter().position(|l| l.link == link) {
            Some(at) => at,
            None => {
                list.push(SlowLink {
                    link: link.to_path_buf(),
                    target: target.clone(),
                    probe: Probe::Wanted,
                });
                LINK_COUNT.store(list.len(), Ordering::Relaxed);
                list.len() - 1
            }
        };
        let slot = &mut list[at];
        if slot.target != target {
            // The link was pointed somewhere else.
            slot.target = target;
            slot.probe = Probe::Wanted;
        }
        if is_mount_point {
            // The mount itself: a folder, without asking.
            slot.probe = Probe::Known(Some(Target {
                is_dir: true,
                size: 0,
                modified: None,
            }));
        }
        match &slot.probe {
            Probe::Known(found) => (found.clone(), false),
            Probe::Asked => (None, false),
            Probe::Wanted => (None, true),
        }
    };
    if ask {
        start_probe();
    }
    known
}

/// Ask the devices about every link still marked `Wanted`, on one thread:
/// a phone that is busy holds up that thread and nothing else.
fn start_probe() {
    if PROBING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("fox-link-probe".into())
        .spawn(|| loop {
            let batch: Vec<(PathBuf, PathBuf)> = links()
                .iter_mut()
                .filter(|l| l.probe == Probe::Wanted)
                .map(|l| {
                    l.probe = Probe::Asked;
                    (l.link.clone(), l.target.clone())
                })
                .collect();
            if batch.is_empty() {
                PROBING.store(false, Ordering::Release);
                // One may have been added between the look and the store.
                let more = links().iter().any(|l| l.probe == Probe::Wanted);
                if more && !PROBING.swap(true, Ordering::AcqRel) {
                    continue;
                }
                return;
            }
            for (link, target) in batch {
                // The one call here that waits on the device.
                let found = std::fs::metadata(&target).ok().map(|m| Target::of(&m));
                let mut list = links();
                if let Some(slot) = list
                    .iter_mut()
                    .find(|l| l.link == link && l.target == target)
                {
                    slot.probe = Probe::Known(found);
                }
            }
            CHANGED.store(true, Ordering::Release);
            crate::bg::wake();
        });
    if spawned.is_err() {
        PROBING.store(false, Ordering::Release);
    }
}

/// Looks at the links of one listing.
pub(crate) struct Looker {
    /// The slow mounts as of the start of the listing.
    slow_roots: Vec<PathBuf>,
    /// The folder of the link last looked at, and what is known of it.
    dir: Option<Dir>,
    /// Links of this listing that lead onto a slow mount.
    seen: Vec<PathBuf>,
}

struct Dir {
    path: PathBuf,
    /// The folder itself is on a slow mount (or behind a link onto one):
    /// its listing runs on a worker, where a link is simply followed.
    slow: bool,
    /// Its real path, when everything up to it is local.
    real: Option<PathBuf>,
}

impl Looker {
    pub(crate) fn new() -> Self {
        Self::with_roots(crate::fs::slow_roots())
    }

    fn with_roots(slow_roots: Vec<PathBuf>) -> Self {
        Self {
            slow_roots,
            dir: None,
            seen: Vec::new(),
        }
    }

    /// What the symlink `link` points at. `None`: nothing (a dangling
    /// link), or not known yet for a target on a slow mount; the entry
    /// then shows as the link itself.
    pub(crate) fn look(&mut self, link: &Path) -> Option<Target> {
        let follow = |path: &Path| std::fs::metadata(path).ok().map(|m| Target::of(&m));
        // No phone, no share: nothing a stat could wait on.
        if self.slow_roots.is_empty() {
            return follow(link);
        }
        let (Some(parent), Some(name)) = (link.parent(), link.file_name()) else {
            return None;
        };
        let roots = &self.slow_roots;
        let on_slow = |p: &Path| roots.iter().any(|root| p.starts_with(root));
        if self.dir.as_ref().map(|d| d.path.as_path()) != Some(parent) {
            let slow = crate::fs::is_slow_path(parent);
            let real = match resolve(parent, &on_slow) {
                Resolved::Local(real) if !slow => Some(real),
                _ => None,
            };
            self.dir = Some(Dir {
                path: parent.to_path_buf(),
                slow,
                real,
            });
        }
        let dir = self.dir.as_ref()?;
        if dir.slow {
            // The round trip per link is the device's to make, and this
            // thread's to wait for: it is not the one that draws.
            return follow(link);
        }
        let resolved = match &dir.real {
            Some(real) => resolve_from(real.clone(), Path::new(name), &on_slow),
            None => resolve(link, &on_slow),
        };
        match resolved {
            Resolved::Local(real) => follow(&real),
            Resolved::Broken => None,
            Resolved::Slow(target) => {
                let is_mount_point = roots.contains(&target);
                self.seen.push(link.to_path_buf());
                slow_target(link, target, is_mount_point)
            }
        }
    }

    /// The listing of `dir` is complete: forget the links of `dir` that it
    /// did not come across (deleted, or pointing somewhere local now).
    pub(crate) fn finish(self, dir: &Path) {
        if LINK_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let mut list = links();
        list.retain(|l| l.link.parent() != Some(dir) || self.seen.contains(&l.link));
        LINK_COUNT.store(list.len(), Ordering::Relaxed);
    }
}

impl crate::fs::FileEntry {
    /// The entry of a listing. `meta` is the entry's own metadata, as
    /// `DirEntry::metadata` gives it: for a symbolic link that describes
    /// the link, so the target is looked at through `links` (which never
    /// asks a slow mount on this thread).
    pub(crate) fn listed(
        name: String,
        path: PathBuf,
        meta: Option<&std::fs::Metadata>,
        links: &mut Looker,
    ) -> Self {
        let is_symlink = meta.is_some_and(|m| m.file_type().is_symlink());
        let own = || {
            (
                meta.is_some_and(|m| m.is_dir()),
                meta.map_or(0, |m| m.len()),
                meta.and_then(|m| m.modified().ok()),
            )
        };
        let (is_dir, size, modified) = if is_symlink {
            match links.look(&path) {
                Some(target) => (target.is_dir, target.size, target.modified),
                // A dangling link (or one whose target is not known yet)
                // is listed as the link itself: a file.
                None => own(),
            }
        } else {
            own()
        };
        // The listing is the one place the folder attributes are read: on
        // the listing thread for a slow mount, never while drawing. And not
        // through a link that leads onto one: that read would be answered
        // by the device, and this may be the thread that draws.
        let through_slow_link = is_symlink && crate::fs::is_slow_path(&path);
        let (folder_icon, folder_color) = if is_dir && !through_slow_link {
            crate::icons::read_folder_attrs(&path)
        } else {
            (None, None)
        };
        Self {
            name,
            path,
            is_dir,
            size,
            modified,
            is_symlink,
            selected: false,
            folder_icon,
            folder_color,
        }
    }
}

#[cfg(test)]
mod tests;
