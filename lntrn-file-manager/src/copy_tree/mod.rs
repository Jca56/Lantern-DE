//! Copying one item (a file, a symlink or a whole folder) without ever
//! overwriting anything, plus the checks a cross-device move makes before it
//! deletes its source.
//!
//! The rules this module keeps:
//!  - The destination is created exclusively. A name that is already taken
//!    is an error, never a merge and never an overwrite.
//!  - A symlink is recreated as a symlink. FIFOs, sockets and device nodes
//!    are not copied and are reported: opening a FIFO blocks until someone
//!    writes to it, and a device node is not data.
//!  - Inside a folder one bad entry fails that entry, not the tree. The
//!    exception is an error every later entry would hit as well (disk full,
//!    read-only, device gone): that fails the whole item.
//!  - When the item fails or is cancelled, everything it created is removed.
//!    Nothing that existed before is ever touched.
//!  - The cancel flag is read between entries and between chunks of a file.

use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, IntoRawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

mod moved;

pub use moved::remove_moved_source;

/// One entry that did not make it, with the reason shown to the user.
#[derive(Clone, Debug)]
pub struct EntryFailure {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug)]
pub enum CopyError {
    /// The cancel flag was raised. Nothing of the item is left behind.
    Cancelled,
    /// The item itself could not be copied. Nothing of it is left behind.
    Failed(io::Error),
}

/// What happened inside an item that did land.
#[derive(Debug, Default)]
pub struct CopyReport {
    /// Entries inside a folder that were skipped or failed. The rest of the
    /// folder is at the destination.
    pub failures: Vec<EntryFailure>,
}

impl CopyReport {
    fn fail(&mut self, path: &Path, err: &io::Error) {
        self.failures.push(EntryFailure {
            path: path.to_path_buf(),
            reason: reason_of(err),
        });
    }
}

/// An I/O error as a sentence for the user: the common cases in plain words,
/// everything else as the system words it, without the "(os error N)" tail.
pub fn reason_of(err: &io::Error) -> String {
    match err.raw_os_error() {
        Some(libc::ENOSPC) | Some(libc::EDQUOT) => "no space left on the destination".into(),
        Some(libc::EFBIG) => "the file is too large for the destination's filesystem".into(),
        Some(libc::EROFS) => "the drive is read-only".into(),
        Some(libc::EACCES) | Some(libc::EPERM) => "permission denied".into(),
        Some(libc::ENAMETOOLONG) => "the name is too long".into(),
        Some(libc::EEXIST) => "an item with this name already exists".into(),
        _ => {
            let text = err.to_string();
            match text.find(" (os error") {
                Some(i) => text[..i].to_string(),
                None => text,
            }
        }
    }
}

/// Errors that will repeat for every later entry of the same item.
fn aborts_item(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(libc::ENOSPC)
            | Some(libc::EDQUOT)
            | Some(libc::EROFS)
            | Some(libc::ENOTCONN)
            | Some(libc::ENODEV)
    )
}

fn special_reason(ft: &std::fs::FileType) -> &'static str {
    if ft.is_fifo() {
        "named pipes are not copied"
    } else if ft.is_socket() {
        "sockets are not copied"
    } else if ft.is_block_device() || ft.is_char_device() {
        "device nodes are not copied"
    } else {
        "this kind of file is not copied"
    }
}

/// Copy `src` to `dst`, which must not exist. `Ok` means `dst` is in place
/// (for a folder, possibly with the listed entries missing). `Err` means
/// `dst` does not exist: whatever was written has been removed again.
pub fn copy_item(src: &Path, dst: &Path, cancel: &AtomicBool) -> Result<CopyReport, CopyError> {
    let meta = std::fs::symlink_metadata(src).map_err(CopyError::Failed)?;
    let ft = meta.file_type();
    let mut report = CopyReport::default();
    if ft.is_symlink() {
        copy_symlink(src, dst).map_err(CopyError::Failed)?;
    } else if ft.is_dir() {
        std::fs::create_dir(dst).map_err(CopyError::Failed)?;
        if let Err(e) = copy_dir_contents(src, dst, cancel, &mut report, true) {
            remove_tree(dst);
            return Err(e);
        }
        finish_dir(&meta, dst);
    } else if ft.is_file() {
        copy_file_new(src, &meta, dst, cancel)?;
    } else {
        return Err(CopyError::Failed(io::Error::new(
            io::ErrorKind::Unsupported,
            special_reason(&ft),
        )));
    }
    Ok(report)
}

fn copy_dir_contents(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    report: &mut CopyReport,
    top: bool,
) -> Result<(), CopyError> {
    let entries = match std::fs::read_dir(src) {
        Ok(rd) => rd,
        // The folder that was asked for cannot be listed: the item failed.
        Err(e) if top => return Err(CopyError::Failed(e)),
        // A subfolder that cannot be listed is one bad entry.
        Err(e) => {
            report.fail(src, &e);
            return Ok(());
        }
    };
    for entry in entries {
        if cancel.load(Ordering::SeqCst) {
            return Err(CopyError::Cancelled);
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                report.fail(src, &e);
                continue;
            }
        };
        let from = entry.path();
        let to = dst.join(entry.file_name());
        match copy_entry(&from, &to, cancel, report) {
            Ok(()) => {}
            Err(CopyError::Cancelled) => return Err(CopyError::Cancelled),
            Err(CopyError::Failed(e)) if aborts_item(&e) => return Err(CopyError::Failed(e)),
            Err(CopyError::Failed(e)) => report.fail(&from, &e),
        }
    }
    Ok(())
}

fn copy_entry(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    report: &mut CopyReport,
) -> Result<(), CopyError> {
    // lstat, never stat: a link to a folder is a link, not a folder to
    // descend into (and a dangling one is still worth keeping).
    let meta = std::fs::symlink_metadata(from).map_err(CopyError::Failed)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        copy_symlink(from, to).map_err(CopyError::Failed)
    } else if ft.is_dir() {
        std::fs::create_dir(to).map_err(CopyError::Failed)?;
        copy_dir_contents(from, to, cancel, report, false)?;
        finish_dir(&meta, to);
        Ok(())
    } else if ft.is_file() {
        copy_file_new(from, &meta, to, cancel)
    } else {
        Err(CopyError::Failed(io::Error::new(
            io::ErrorKind::Unsupported,
            special_reason(&ft),
        )))
    }
}

fn copy_symlink(src: &Path, dst: &Path) -> io::Result<()> {
    let target = std::fs::read_link(src)?;
    std::os::unix::fs::symlink(&target, dst).map_err(|e| match e.raw_os_error() {
        // FAT, exFAT and MTP have no symlinks. Following the link instead
        // would silently turn it into a full copy (or fail on a folder).
        Some(libc::EPERM) | Some(libc::EOPNOTSUPP) | Some(libc::ENOSYS) => io::Error::new(
            io::ErrorKind::Unsupported,
            "the destination cannot hold symbolic links",
        ),
        _ => e,
    })
}

/// Copy a regular file to a name that must be free. On any failure the
/// partly written file is removed (it is ours: `create_new` made it).
fn copy_file_new(
    src: &Path,
    src_meta: &std::fs::Metadata,
    dst: &Path,
    cancel: &AtomicBool,
) -> Result<(), CopyError> {
    // O_NONBLOCK: if the entry was swapped for a FIFO after the lstat, the
    // open returns instead of waiting for a writer. O_NOFOLLOW: same for a
    // symlink. Neither changes how a regular file is read.
    let input = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(src)
        .map_err(CopyError::Failed)?;
    if !input.metadata().map_err(CopyError::Failed)?.is_file() {
        return Err(CopyError::Failed(io::Error::new(
            io::ErrorKind::Unsupported,
            "not a regular file",
        )));
    }
    // Setuid/setgid are not carried to a copy.
    let mode = src_meta.permissions().mode() & 0o777;
    let out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(dst)
        .map_err(CopyError::Failed)?;

    let written = pump(&input, &out, cancel).and_then(|n| {
        // The umask narrowed the mode at create time.
        let _ = out.set_permissions(std::fs::Permissions::from_mode(mode));
        // A FUSE destination commits on close and reports its failure there;
        // dropping the File would swallow it.
        close_checked(out).map_err(CopyError::Failed)?;
        Ok(n)
    });
    let written = match written {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_file(dst);
            return Err(e);
        }
    };
    // Did what we wrote actually arrive?
    match std::fs::symlink_metadata(dst) {
        Ok(m) if m.len() == written => {}
        _ => {
            let _ = std::fs::remove_file(dst);
            return Err(CopyError::Failed(io::Error::other(
                "the copy did not arrive complete",
            )));
        }
    }
    keep_times(src_meta, dst);
    Ok(())
}

fn close_checked(file: std::fs::File) -> io::Result<()> {
    let fd = file.into_raw_fd();
    if unsafe { libc::close(fd) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Move the bytes, in chunks so a cancel does not wait for a 20 GB file.
/// `copy_file_range` first (reflinks on btrfs, stays in the kernel); when
/// the pair of filesystems cannot do it, plain read/write. Both use the
/// files' own offsets, so switching over mid-file continues where it was.
fn pump(input: &std::fs::File, out: &std::fs::File, cancel: &AtomicBool) -> Result<u64, CopyError> {
    const RANGE_CHUNK: usize = 32 * 1024 * 1024;
    const BUF_LEN: usize = 1024 * 1024;
    let mut total: u64 = 0;
    let mut in_kernel = true;
    let mut buf: Vec<u8> = Vec::new();
    let mut reader = input;
    let mut writer = out;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(CopyError::Cancelled);
        }
        if in_kernel {
            let n = unsafe {
                libc::syscall(
                    libc::SYS_copy_file_range,
                    input.as_raw_fd(),
                    std::ptr::null_mut::<libc::loff_t>(),
                    out.as_raw_fd(),
                    std::ptr::null_mut::<libc::loff_t>(),
                    RANGE_CHUNK,
                    0 as libc::c_uint,
                )
            };
            if n > 0 {
                total += n as u64;
                continue;
            }
            if n < 0 {
                let err = io::Error::last_os_error();
                match err.raw_os_error() {
                    Some(libc::EINTR) => continue,
                    Some(libc::ENOSYS)
                    | Some(libc::EXDEV)
                    | Some(libc::EINVAL)
                    | Some(libc::EPERM)
                    | Some(libc::EOPNOTSUPP)
                    | Some(libc::EBADF)
                    | Some(libc::ETXTBSY) => {}
                    _ => return Err(CopyError::Failed(err)),
                }
            }
            // 0 is the end of the file, or a filesystem that cannot serve
            // this call. The read below tells the two apart.
            in_kernel = false;
            continue;
        }
        if buf.is_empty() {
            buf = vec![0u8; BUF_LEN];
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // O_NONBLOCK is set on the source; a regular file never says
            // this, anything that does is not something to copy.
            Err(e) => return Err(CopyError::Failed(e)),
        };
        writer.write_all(&buf[..n]).map_err(CopyError::Failed)?;
        total += n as u64;
    }
}

/// Mode and timestamps of a finished folder. Last, so a read-only source
/// folder does not block its own children and writing them does not bump
/// the mtime just restored.
fn finish_dir(src_meta: &std::fs::Metadata, dst: &Path) {
    // The source's mode, but always owner rwx: a faithful 0555 copy could
    // not be trashed, renamed or undone by the user who made it.
    let mode = (src_meta.permissions().mode() & 0o7777) | 0o700;
    let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(mode));
    keep_times(src_meta, dst);
}

/// Carry the source's timestamps over. Best effort: FAT and exFAT refuse
/// some or all of this. By path (`utimensat`), never by opening `dst`: on an
/// MTP phone, opening the file just uploaded would make jmtpfs download all
/// of it again. Slow mounts are skipped outright for the same reason.
fn keep_times(src_meta: &std::fs::Metadata, dst: &Path) {
    if crate::fs::is_slow_path(dst) {
        return;
    }
    let Ok(c_dst) = std::ffi::CString::new(dst.as_os_str().as_bytes()) else {
        return;
    };
    let times = [
        libc::timespec {
            tv_sec: src_meta.atime() as libc::time_t,
            tv_nsec: src_meta.atime_nsec() as libc::c_long,
        },
        libc::timespec {
            tv_sec: src_meta.mtime() as libc::time_t,
            tv_nsec: src_meta.mtime_nsec() as libc::c_long,
        },
    ];
    unsafe {
        libc::utimensat(libc::AT_FDCWD, c_dst.as_ptr(), times.as_ptr(), 0);
    }
}

/// Remove something this process created (a staging item, a partial copy).
/// A symlink is unlinked, never followed.
pub fn remove_tree(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// A hidden name next to `target` to build a new item under until it is
/// complete. Same folder, so the final step is a rename on one filesystem.
/// The `.fox-tmp` ending is the one cloud sync already leaves alone, so a
/// file being copied into ~/Cloud is not uploaded half-written.
pub fn temp_sibling(target: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = target.parent().unwrap_or(Path::new("."));
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A hint of what it is, cut short: the whole name must stay under the
    // 255-byte limit of a file name.
    let mut hint = String::new();
    for ch in name.chars() {
        if hint.len() + ch.len_utf8() > 60 {
            break;
        }
        hint.push(ch);
    }
    loop {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            "{STAGING_PREFIX}{}-{n}-{hint}{STAGING_SUFFIX}",
            std::process::id()
        ));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
}

const STAGING_PREFIX: &str = ".fox-part-";
const STAGING_SUFFIX: &str = ".fox-tmp";

/// The process id in a staging name (`temp_sibling`), if `name` is one.
fn staging_pid(name: &str) -> Option<u32> {
    let rest = name
        .strip_prefix(STAGING_PREFIX)?
        .strip_suffix(STAGING_SUFFIX)?;
    let (pid, rest) = rest.split_once('-')?;
    let (serial, _hint) = rest.split_once('-')?;
    serial.parse::<u64>().ok()?;
    pid.parse().ok()
}

/// A Fox process with this id is running. (The id alone means nothing: it
/// may come from another machine by way of a USB stick, or have been
/// reused by another program since.)
fn fox_runs_as(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let comm = |pid: &str| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok();
    match (comm(&pid.to_string()), comm("self")) {
        (Some(theirs), Some(ours)) => theirs == ours,
        _ => false,
    }
}

/// A staging item whose copy no running Fox is working on: what a copy
/// leaves behind when the process is killed, the machine loses power or
/// the drive is pulled. Nothing removes it by itself.
pub fn is_stale_staging(name: &str) -> bool {
    staging_pid(name).is_some_and(|pid| !fox_runs_as(pid))
}

/// Stale staging items seen by directory listings, waiting to be asked
/// about (`App::ask_about_leftovers`).
static STALE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

pub fn report_stale(path: PathBuf) {
    let mut seen = STALE.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.contains(&path) {
        seen.push(path);
    }
}

pub fn take_stale() -> Vec<PathBuf> {
    std::mem::take(&mut *STALE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `rename` that refuses to replace: an existing `to` is `AlreadyExists`.
/// Atomic where the filesystem supports RENAME_NOREPLACE; elsewhere (NFS,
/// some FUSE) a check followed by a plain rename.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let c_from = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let c_to = std::ffi::CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let r = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            c_from.as_ptr(),
            libc::AT_FDCWD,
            c_to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if r == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP) => {
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(io::Error::from_raw_os_error(libc::EEXIST));
            }
            std::fs::rename(from, to)
        }
        _ => Err(err),
    }
}

#[cfg(test)]
mod tests;
