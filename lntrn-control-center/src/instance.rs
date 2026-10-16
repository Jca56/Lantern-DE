//! One bar at a time: a second start finds the lock taken and leaves. The
//! lock is an `flock` on a file in the runtime directory, so the kernel
//! lets go of it however the process ends, and an exec lets go of it too
//! (the file is opened close-on-exec).

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

use lntrn_core::log_warn;

unsafe extern "C" {
    fn flock(fd: i32, op: i32) -> i32;
}
const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const EWOULDBLOCK: i32 = 11;

/// Held for as long as this is the bar.
pub struct Lock {
    _file: Option<File>,
}

/// Take the lock for `app_id`. `None` when another process holds it. A
/// lock that can't be had for any other reason is no cause to go without
/// a bar: then this runs unlocked, and the log says so.
pub fn claim(app_id: &str) -> Option<Lock> {
    let path = lntrn_sys::dirs::runtime().unwrap_or_else(std::env::temp_dir).join(format!("{app_id}.lock"));
    let file = match OpenOptions::new().create(true).truncate(false).write(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            log_warn!("instance: can't open {}: {e}; running unlocked", path.display());
            return Some(Lock { _file: None });
        }
    };
    // SAFETY: a plain syscall on a descriptor we own.
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
        return Some(Lock { _file: Some(file) });
    }
    let e = std::io::Error::last_os_error();
    if e.raw_os_error() == Some(EWOULDBLOCK) {
        return None;
    }
    log_warn!("instance: can't lock {}: {e}; running unlocked", path.display());
    Some(Lock { _file: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_claim_waits_for_the_first_to_let_go() {
        let id = format!("lntrn-control-center-test-{}", std::process::id());
        let first = claim(&id);
        assert!(first.as_ref().is_some_and(|l| l._file.is_some()));
        assert!(claim(&id).is_none(), "taken");
        drop(first);
        assert!(claim(&id).is_some(), "free again");
        let _ = std::fs::remove_file(lntrn_sys::dirs::runtime().unwrap_or_else(std::env::temp_dir).join(format!("{id}.lock")));
    }
}
