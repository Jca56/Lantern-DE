//! The libc calls std doesn't wrap, declared here as the rest of Lantern
//! declares them (no libc crate): waiting on several descriptors at once,
//! a bell another thread rings to wake a waiting one, and the kernel's
//! netlink broadcasts (device events, network links coming and going).

use std::ffi::{CString, c_char, c_int, c_ulong, c_void};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Duration;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct SockAddrNl {
    family: u16,
    pad: u16,
    pid: u32,
    groups: u32,
}

const POLLIN: i16 = 0x1;
const O_NONBLOCK: c_int = 0o4000;
const O_CLOEXEC: c_int = 0o2000000;
const AF_NETLINK: c_int = 16;
const SOCK_DGRAM: c_int = 2;
const SOCK_NONBLOCK: c_int = O_NONBLOCK;
const SOCK_CLOEXEC: c_int = O_CLOEXEC;
const W_OK: c_int = 2;
/// The kernel's device events: what `udevadm monitor --kernel` reads.
const NETLINK_KOBJECT_UEVENT: c_int = 15;
const NETLINK_ROUTE: c_int = 0;
/// rtnetlink's group for links: a carrier coming or going.
const RTMGRP_LINK: u32 = 1;

unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
    fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn socket(domain: c_int, kind: c_int, protocol: c_int) -> c_int;
    fn bind(fd: c_int, addr: *const SockAddrNl, len: u32) -> c_int;
    fn access(path: *const c_char, mode: c_int) -> c_int;
    fn prctl(option: c_int, arg: c_ulong, ...) -> c_int;
}

const PR_SET_PDEATHSIG: c_int = 1;
const SIGTERM: c_ulong = 15;

/// Have the child `command` starts end when the thread that starts it
/// does: with the app (a crash too), or when it starts over in place.
pub fn die_with_parent(command: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::unix::process::CommandExt;
    // SAFETY: prctl is async-signal-safe, as a pre_exec hook must be.
    unsafe {
        command.pre_exec(|| {
            prctl(PR_SET_PDEATHSIG, SIGTERM);
            Ok(())
        })
    }
}

/// Wait until any of `fds` can be read, or `timeout` passes (`None`:
/// forever). Returns which can, in order; none on a timeout or a signal.
pub fn wait(fds: &[RawFd], timeout: Option<Duration>) -> Vec<bool> {
    let mut polls: Vec<PollFd> = fds.iter().map(|&fd| PollFd { fd, events: POLLIN, revents: 0 }).collect();
    let ms = timeout.map_or(-1, |t| c_int::try_from(t.as_millis()).unwrap_or(c_int::MAX));
    // SAFETY: `polls` is ours and as long as we say.
    let n = unsafe { poll(polls.as_mut_ptr(), polls.len() as c_ulong, ms) };
    if n <= 0 {
        return vec![false; fds.len()];
    }
    // A hang-up or an error counts as readable: the read says which.
    polls.iter().map(|p| p.revents != 0).collect()
}

/// One read into `buf` from a non-blocking descriptor: `None` when there
/// is nothing more to read (or it failed).
fn read_some(fd: RawFd, buf: &mut [u8]) -> Option<usize> {
    // SAFETY: reading into our own buffer, no more than its length.
    let n = unsafe { read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    usize::try_from(n).ok().filter(|&n| n > 0)
}

/// A pipe another thread rings to wake one waiting on [`wait`].
pub struct Bell {
    read: OwnedFd,
    write: OwnedFd,
}

impl Bell {
    pub fn new() -> io::Result<Bell> {
        let mut ends = [0 as c_int; 2];
        // SAFETY: `ends` holds the two descriptors pipe2 writes.
        if unsafe { pipe2(ends.as_mut_ptr(), O_CLOEXEC | O_NONBLOCK) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: pipe2 just opened both, and nothing else owns them.
        Ok(unsafe { Bell { read: OwnedFd::from_raw_fd(ends[0]), write: OwnedFd::from_raw_fd(ends[1]) } })
    }

    /// Wake the waiting thread. A pipe already full has woken it anyway.
    pub fn ring(&self) {
        // SAFETY: one byte from a local to our own pipe.
        unsafe { write(self.write.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
    }

    /// Hear every ring so far, so the next wait waits again.
    pub fn quiet(&self) {
        let mut buf = [0u8; 64];
        while read_some(self.read.as_raw_fd(), &mut buf).is_some() {}
    }

    pub fn fd(&self) -> RawFd {
        self.read.as_raw_fd()
    }
}

/// A netlink socket listening to one of the kernel's broadcasts.
pub struct Netlink {
    fd: OwnedFd,
}

impl Netlink {
    /// The kernel's device events (a power supply changed, say).
    pub fn uevents() -> io::Result<Netlink> {
        Netlink::open(NETLINK_KOBJECT_UEVENT, 1)
    }

    /// Network links changing: a cable plugged in or pulled out.
    pub fn links() -> io::Result<Netlink> {
        Netlink::open(NETLINK_ROUTE, RTMGRP_LINK)
    }

    fn open(protocol: c_int, groups: u32) -> io::Result<Netlink> {
        // SAFETY: plain calls; the descriptor is owned as soon as it is made.
        let fd = unsafe { socket(AF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, protocol) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: socket just made it.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let addr = SockAddrNl { family: AF_NETLINK as u16, pad: 0, pid: 0, groups };
        // SAFETY: `addr` is a sockaddr_nl of the size given.
        if unsafe { bind(fd.as_raw_fd(), &addr, std::mem::size_of::<SockAddrNl>() as u32) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Netlink { fd })
    }

    /// Every message waiting, each as it came.
    pub fn take(&self) -> Vec<Vec<u8>> {
        let mut buf = vec![0u8; 16 * 1024];
        let mut out = Vec::new();
        while let Some(n) = read_some(self.fd.as_raw_fd(), &mut buf) {
            out.push(buf[..n].to_vec());
        }
        out
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Show `args` on Lantern's on-screen display (`lntrn-osd volume 65`), as
/// the volume and brightness keys do. Reaped on a thread of its own, so
/// it leaves no zombie.
pub fn osd(args: &[&str]) {
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("lntrn-osd").args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

/// Whether this user may write the file at `path`.
pub fn writable(path: &Path) -> bool {
    let Ok(c) = CString::new(path.as_os_str().as_encoded_bytes()) else { return false };
    // SAFETY: a nul-terminated path of our own.
    unsafe { access(c.as_ptr(), W_OK) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rung_bell_wakes_a_wait_until_it_is_heard() {
        let bell = Bell::new().unwrap();
        assert_eq!(wait(&[bell.fd()], Some(Duration::ZERO)), [false]);
        bell.ring();
        bell.ring();
        assert_eq!(wait(&[bell.fd()], Some(Duration::from_secs(1))), [true]);
        bell.quiet();
        assert_eq!(wait(&[bell.fd()], Some(Duration::ZERO)), [false]);
    }

    #[test]
    fn the_kernels_broadcasts_open_for_anyone() {
        // Reading them needs no privilege; nothing may be waiting yet.
        let uevents = Netlink::uevents().unwrap();
        let links = Netlink::links().unwrap();
        let _ = (uevents.take(), links.take());
    }

    #[test]
    fn writable_tells_ours_from_the_systems() {
        assert!(!writable(Path::new("/proc/version")));
        assert!(writable(&std::env::temp_dir()));
    }
}
