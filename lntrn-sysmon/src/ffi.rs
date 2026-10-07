//! The few libc calls std has no face for: how big a page is, how full
//! a filesystem is, and sending a process a signal.

use std::ffi::{CString, c_char, c_int, c_long};
use std::sync::OnceLock;

unsafe extern "C" {
    fn sysconf(name: c_int) -> c_long;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> c_int;
}

const SC_PAGESIZE: c_int = 30;
const EPERM: i32 = 1;
const ESRCH: i32 = 3;

/// Asks a process to quit: it may tidy up first, or not listen.
pub const SIGTERM: i32 = 15;
/// Stops a process at once. It gets no say.
pub const SIGKILL: i32 = 9;

/// `struct statvfs` as glibc and musl lay it out on 64-bit Linux, where
/// every one of its counts is eight bytes.
#[repr(C)]
struct StatVfs {
    bsize: u64,
    frsize: u64,
    blocks: u64,
    bfree: u64,
    bavail: u64,
    files: u64,
    ffree: u64,
    favail: u64,
    fsid: u64,
    flag: u64,
    namemax: u64,
    spare: [c_int; 6],
}

/// Bytes in a page of memory.
pub fn page_size() -> u64 {
    static SIZE: OnceLock<u64> = OnceLock::new();
    // SAFETY: sysconf only reads its argument.
    *SIZE.get_or_init(|| u64::try_from(unsafe { sysconf(SC_PAGESIZE) }).ok().filter(|n| *n > 0).unwrap_or(4096))
}

/// How big the filesystem mounted at `path` is and how much of it an
/// ordinary user can still write, in bytes: `(total, available)`.
pub fn space(path: &str) -> Option<(u64, u64)> {
    let c = CString::new(path).ok()?;
    // SAFETY: a valid C string and a buffer of the struct's own size,
    // which statvfs fills in or leaves alone.
    let mut buf: StatVfs = unsafe { std::mem::zeroed() };
    if unsafe { statvfs(c.as_ptr(), &mut buf) } != 0 {
        return None;
    }
    Some((buf.blocks.saturating_mul(buf.frsize), buf.bavail.saturating_mul(buf.frsize)))
}

/// Send `signal` to `pid`. What went wrong, in words, when it couldn't.
pub fn signal(pid: u32, signal: i32) -> Result<(), String> {
    let Ok(pid) = c_int::try_from(pid) else { return Err("not a process id".to_owned()) };
    // A pid of 0 or less means a whole group: never what is asked here.
    if pid <= 0 {
        return Err("not a process id".to_owned());
    }
    // SAFETY: kill takes two integers.
    if unsafe { kill(pid, signal) } == 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    Err(match e.raw_os_error() {
        Some(EPERM) => "it belongs to someone else".to_owned(),
        Some(ESRCH) => "it has already gone".to_owned(),
        _ => e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_answers_and_nonsense_is_refused() {
        assert!(page_size().is_power_of_two());
        let (total, free) = space("/").expect("the root filesystem");
        assert!(total > 0 && free <= total);
        assert!(space("/no/such/place").is_none());
        assert!(signal(0, SIGTERM).is_err(), "never a whole process group");
    }
}
