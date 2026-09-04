//! Files changing under the editor: an inotify watch on the folders of
//! the open files and of the listed project folders, read by a thread
//! that wakes the loop. The app reloads clean files, warns about dirty
//! ones, and re-reads folders that changed.

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use lntrn_app::Waker;

unsafe extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
    fn inotify_rm_watch(fd: c_int, wd: c_int) -> c_int;
}

const IN_CLOEXEC: c_int = 0o2000000;
pub const IN_MODIFY: u32 = 0x2;
pub const IN_ATTRIB: u32 = 0x4;
pub const IN_CLOSE_WRITE: u32 = 0x8;
pub const IN_MOVED_FROM: u32 = 0x40;
pub const IN_MOVED_TO: u32 = 0x80;
pub const IN_CREATE: u32 = 0x100;
pub const IN_DELETE: u32 = 0x200;
pub const IN_DELETE_SELF: u32 = 0x400;
pub const IN_IGNORED: u32 = 0x8000;
const IN_ONLYDIR: u32 = 0x0100_0000;
const WATCH_MASK: u32 = IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | IN_MOVED_FROM | IN_MOVED_TO | IN_CREATE | IN_DELETE | IN_DELETE_SELF;
/// Folders watched at most; a huge tree does not exhaust the kernel's limit.
const MAX_WATCHES: usize = 1024;

/// Something happened in a watched folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub dir: PathBuf,
    /// The entry it concerns; `None` for the folder itself.
    pub name: Option<String>,
    pub mask: u32,
}

impl Change {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn path(&self) -> PathBuf {
        match &self.name {
            Some(n) => self.dir.join(n),
            None => self.dir.clone(),
        }
    }

    /// The entry's contents changed (a write, a rename onto it).
    pub fn is_write(&self) -> bool {
        self.mask & (IN_CLOSE_WRITE | IN_MOVED_TO | IN_MODIFY | IN_ATTRIB) != 0
    }

    /// The entry went away.
    pub fn is_removal(&self) -> bool {
        self.mask & (IN_DELETE | IN_MOVED_FROM | IN_DELETE_SELF) != 0
    }

    /// The folder's listing is out of date.
    pub fn is_listing(&self) -> bool {
        self.mask & (IN_CREATE | IN_DELETE | IN_MOVED_FROM | IN_MOVED_TO | IN_DELETE_SELF) != 0
    }
}

pub struct Watcher {
    /// The inotify descriptor; closing it (on drop) ends the reader.
    file: File,
    /// Watch descriptors by folder, shared with the reader thread.
    dirs: Arc<Mutex<HashMap<c_int, PathBuf>>>,
    by_path: HashMap<PathBuf, c_int>,
    rx: Receiver<Change>,
}

impl Watcher {
    pub fn new(waker: Option<Waker>) -> io::Result<Self> {
        // SAFETY: a plain syscall.
        let fd = unsafe { inotify_init1(IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let dirs: Arc<Mutex<HashMap<c_int, PathBuf>>> = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = channel();
        // SAFETY: the descriptor is ours from here on.
        let file = unsafe { File::from_raw_fd(fd) };
        let mut reader = file.try_clone()?;
        let shared = Arc::clone(&dirs);
        std::thread::Builder::new().name("watch".into()).spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let mut i = 0;
                let mut any = false;
                while i + 16 <= n {
                    let wd = i32::from_ne_bytes(buf[i..i + 4].try_into().expect("4 bytes"));
                    let mask = u32::from_ne_bytes(buf[i + 4..i + 8].try_into().expect("4 bytes"));
                    let len = u32::from_ne_bytes(buf[i + 12..i + 16].try_into().expect("4 bytes")) as usize;
                    let name_bytes = &buf[i + 16..(i + 16 + len).min(n)];
                    let name = name_bytes.split(|&b| b == 0).next().filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned());
                    i += 16 + len;
                    if mask & IN_IGNORED != 0 {
                        continue;
                    }
                    let dir = shared.lock().ok().and_then(|d| d.get(&wd).cloned());
                    if let Some(dir) = dir {
                        let _ = tx.send(Change { dir, name, mask });
                        any = true;
                    }
                }
                if any && let Some(w) = &waker {
                    w.wake();
                }
            }
        })?;
        Ok(Self { file, dirs, by_path: HashMap::new(), rx })
    }

    /// Watch `dir` (a folder), if it is not watched already.
    pub fn watch(&mut self, dir: &Path) {
        if self.by_path.contains_key(dir) || self.by_path.len() >= MAX_WATCHES {
            return;
        }
        let Ok(c) = CString::new(dir.as_os_str().as_encoded_bytes()) else {
            return;
        };
        // SAFETY: a valid C string and our descriptor.
        let wd = unsafe { inotify_add_watch(self.file.as_raw_fd(), c.as_ptr(), WATCH_MASK | IN_ONLYDIR) };
        if wd >= 0 {
            self.by_path.insert(dir.to_path_buf(), wd);
            if let Ok(mut d) = self.dirs.lock() {
                d.insert(wd, dir.to_path_buf());
            }
        }
    }

    /// Stop watching every folder not in `keep`.
    pub fn retain(&mut self, keep: impl Fn(&Path) -> bool) {
        let gone: Vec<PathBuf> = self.by_path.keys().filter(|p| !keep(p)).cloned().collect();
        for p in gone {
            if let Some(wd) = self.by_path.remove(&p) {
                // SAFETY: our descriptor and a watch we added.
                unsafe {
                    inotify_rm_watch(self.file.as_raw_fd(), wd);
                }
                if let Ok(mut d) = self.dirs.lock() {
                    d.remove(&wd);
                }
            }
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn watched(&self) -> usize {
        self.by_path.len()
    }

    /// Changes since the last call, one per path (later masks folded in).
    pub fn poll(&self) -> Vec<Change> {
        let mut out: Vec<Change> = Vec::new();
        while let Ok(c) = self.rx.try_recv() {
            match out.iter_mut().find(|o| o.dir == c.dir && o.name == c.name) {
                Some(o) => o.mask |= c.mask,
                None => out.push(c),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_writes_and_listings() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = Watcher::new(None).unwrap();
        w.watch(&dir);
        w.watch(&dir);
        assert_eq!(w.watched(), 1);
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::rename(dir.join("a.txt"), dir.join("b.txt")).unwrap();
        std::fs::remove_file(dir.join("b.txt")).unwrap();
        let mut changes = Vec::new();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            changes.extend(w.poll());
            if changes.iter().any(|c| c.name.as_deref() == Some("b.txt") && c.is_removal()) {
                break;
            }
        }
        let a = changes.iter().find(|c| c.name.as_deref() == Some("a.txt")).expect("a.txt seen");
        assert!(a.is_write() && a.is_listing(), "{a:?}");
        assert_eq!(a.path(), dir.join("a.txt"));
        let b = changes.iter().find(|c| c.name.as_deref() == Some("b.txt")).expect("b.txt seen");
        assert!(b.is_removal());
        w.retain(|_| false);
        assert_eq!(w.watched(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
