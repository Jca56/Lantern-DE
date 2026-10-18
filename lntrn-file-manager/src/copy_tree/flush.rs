//! Copies onto a drive that can be pulled out (a USB stick, an SD card):
//! "done" means on the drive, not in memory.
//!
//! Linux takes a write into memory and puts it on the drive later, so a copy
//! onto a stick finishes long before the stick has the data. Pulled out in
//! between, the copy is lost, and after a move the original is gone too. On
//! such a drive the copy flushes what it wrote every [`WINDOW`], which also
//! keeps Cancel quick: the kernel never sits on gigabytes that a flush would
//! have to wait for. The worker settles each item before it counts as done.

use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// Written but unflushed data allowed to pile up before the next flush.
const WINDOW: u64 = 32 * 1024 * 1024;

/// What each new file, folder or link counts for against [`WINDOW`]. Flash
/// is slow at many small writes: a folder of ten thousand tiny files is a
/// long flush even though it is few bytes.
const ENTRY_COST: u64 = 256 * 1024;

pub struct Flusher {
    /// A folder on the destination drive: `syncfs` on it flushes the whole
    /// filesystem. `None` when the destination needs no flushing.
    fs: Option<File>,
    pending: u64,
    /// The error of a flush that failed. The drive lost writes, so nothing
    /// more of the item can be trusted.
    failed: Option<i32>,
}

impl Flusher {
    /// No flushing: an internal disk, a phone, a network folder.
    pub fn off() -> Self {
        Self {
            fs: None,
            pending: 0,
            failed: None,
        }
    }

    /// Flushing when `target` goes onto a drive that can be pulled out.
    pub fn for_target(target: &Path) -> Self {
        match target.parent() {
            Some(dir) if on_pull_out_drive(dir) => Self::at(dir),
            _ => Self::off(),
        }
    }

    /// Flushing the filesystem `dir` is on, whatever drive that is.
    pub(super) fn at(dir: &Path) -> Self {
        Self {
            fs: File::open(dir).ok(),
            ..Self::off()
        }
    }

    /// `bytes` more were written.
    pub fn wrote(&mut self, bytes: u64) -> io::Result<()> {
        if self.fs.is_none() {
            return Ok(());
        }
        self.pending += bytes;
        if self.pending >= WINDOW {
            self.settle()
        } else {
            Ok(())
        }
    }

    /// A new file, folder or link is about to be made.
    pub fn new_entry(&mut self) -> io::Result<()> {
        self.wrote(ENTRY_COST)
    }

    /// Flush now and wait until the drive holds everything written so far.
    pub fn settle(&mut self) -> io::Result<()> {
        if let Some(code) = self.failed {
            return Err(io::Error::from_raw_os_error(code));
        }
        let Some(fs) = &self.fs else {
            return Ok(());
        };
        if unsafe { libc::syncfs(fs.as_raw_fd()) } != 0 {
            let err = io::Error::last_os_error();
            self.failed = Some(err.raw_os_error().unwrap_or(libc::EIO));
            return Err(err);
        }
        self.pending = 0;
        Ok(())
    }

    pub fn failed(&self) -> bool {
        self.failed.is_some()
    }
}

/// True when `path` is on a block device that can be pulled out: one the
/// kernel flags removable, or one on a USB or SD/MMC bus (external SSDs and
/// card readers often don't set the flag). Filesystems without a block
/// device number of their own (btrfs, tmpfs, FUSE, network) never are.
fn on_pull_out_drive(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let dev = meta.dev();
    // A link to the sysfs node of the partition, or of the whole disk.
    let link = format!("/sys/dev/block/{}:{}", libc::major(dev), libc::minor(dev));
    let Ok(node) = std::fs::canonicalize(link) else {
        return false;
    };
    let flagged =
        |dir: &Path| std::fs::read_to_string(dir.join("removable")).is_ok_and(|s| s.trim() == "1");
    if flagged(&node) || node.parent().is_some_and(flagged) {
        return true;
    }
    node.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        name.starts_with("usb") || name.starts_with("mmc")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_filesystems_are_not_pull_out_drives() {
        assert!(!on_pull_out_drive(Path::new("/proc")));
        assert!(!on_pull_out_drive(Path::new("/sys")));
        assert!(!on_pull_out_drive(Path::new("/no/such/place")));
    }

    #[test]
    fn an_off_flusher_never_flushes_or_fails() {
        let mut f = Flusher::off();
        for _ in 0..1000 {
            f.new_entry().unwrap();
        }
        f.wrote(10 * WINDOW).unwrap();
        f.settle().unwrap();
        assert!(!f.failed());
    }

    #[test]
    fn a_flusher_settles_once_the_window_fills() {
        let mut f = Flusher::at(&std::env::temp_dir());
        f.wrote(WINDOW - 1).unwrap();
        assert_eq!(f.pending, WINDOW - 1);
        f.wrote(1).unwrap();
        assert_eq!(f.pending, 0);
        assert!(!f.failed());
    }
}
