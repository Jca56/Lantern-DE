//! The few libc calls std has no face for: who is on the other end of
//! the socket, the file mode the socket is born with, and waiting for a
//! signal on a thread of its own.

use std::ffi::{c_int, c_void};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

const SOL_SOCKET: c_int = 1;
const SO_PEERCRED: c_int = 17;
const SIG_BLOCK: c_int = 0;
pub const SIGHUP: c_int = 1;
pub const SIGINT: c_int = 2;
pub const SIGTERM: c_int = 15;

/// `struct ucred`.
#[repr(C)]
#[derive(Default)]
struct Ucred {
    pid: i32,
    uid: u32,
    gid: u32,
}

/// `sigset_t`: glibc's is 1024 bits.
#[repr(C)]
pub struct SigSet([u64; 16]);

unsafe extern "C" {
    fn getsockopt(fd: c_int, level: c_int, name: c_int, value: *mut c_void, len: *mut u32) -> c_int;
    fn geteuid() -> u32;
    fn umask(mask: u32) -> u32;
    fn sigemptyset(set: *mut SigSet) -> c_int;
    fn sigaddset(set: *mut SigSet, signal: c_int) -> c_int;
    fn pthread_sigmask(how: c_int, set: *const SigSet, old: *mut SigSet) -> c_int;
    fn sigwait(set: *const SigSet, signal: *mut c_int) -> c_int;
}

/// The user on the other end of `stream`, as the kernel vouches.
pub fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred = Ucred::default();
    let mut len = size_of::<Ucred>() as u32;
    // SAFETY: `cred` is a `struct ucred` and `len` says its size.
    let r = unsafe { getsockopt(stream.as_raw_fd(), SOL_SOCKET, SO_PEERCRED, (&mut cred as *mut Ucred).cast(), &mut len) };
    (r == 0 && len as usize == size_of::<Ucred>()).then_some(cred.uid)
}

pub fn euid() -> u32 {
    // SAFETY: always succeeds.
    unsafe { geteuid() }
}

/// Files made from now on get `mask`'s bits taken away.
pub fn set_umask(mask: u32) {
    // SAFETY: always succeeds.
    unsafe { umask(mask) };
}

/// Block hang-up, interrupt and terminate on this thread, and so on every
/// thread it starts after, so only [`wait`] ever sees them.
pub fn block_signals() -> SigSet {
    let mut set = SigSet([0; 16]);
    // SAFETY: a set of the size glibc's calls expect.
    unsafe {
        sigemptyset(&mut set);
        for s in [SIGHUP, SIGINT, SIGTERM] {
            sigaddset(&mut set, s);
        }
        pthread_sigmask(SIG_BLOCK, &set, std::ptr::null_mut());
    }
    set
}

/// Sleep until one of `set`'s signals comes, and say which.
pub fn wait(set: &SigSet) -> c_int {
    let mut signal: c_int = 0;
    // SAFETY: `set` was filled in by `block_signals`.
    unsafe { sigwait(set, &mut signal) };
    signal
}
