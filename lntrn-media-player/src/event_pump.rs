//! Sleeping until there is something to do. Work reaches the main loop from
//! three directions — the Wayland socket, timers, and other threads
//! (GStreamer's streaming threads with a new video frame or spectrum, the
//! bus with EOS/errors, the MPRIS server with a transport command). `pump`
//! waits on the socket and a [`Waker`] together, up to a deadline, so the
//! loop neither spins while idle nor misses a frame.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use wayland_client::backend::WaylandError;
use wayland_client::EventQueue;

use crate::wayland::State;

/// Lets another thread wake the main loop out of its poll(). Cheap to clone;
/// any number of wakes before the loop runs collapse into one.
#[derive(Clone)]
pub struct Waker {
    fd: Arc<OwnedFd>,
}

impl Waker {
    pub fn new() -> std::io::Result<Self> {
        // SAFETY: plain syscall; a valid descriptor is ours to own.
        let raw = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `raw` is a fresh descriptor nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self { fd: Arc::new(fd) })
    }

    pub fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: writes 8 bytes from a live u64. A full counter (EAGAIN)
        // already means "wake up", so the result doesn't matter.
        unsafe {
            libc::write(self.raw(), (&one as *const u64).cast(), 8);
        }
    }

    fn drain(&self) {
        let mut count: u64 = 0;
        // SAFETY: reads 8 bytes into a live u64; EAGAIN when already empty.
        unsafe {
            libc::read(self.raw(), (&mut count as *mut u64).cast(), 8);
        }
    }

    fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Flush requests, then dispatch whatever arrives within `timeout` — or
/// return as soon as `waker` is poked. A zero timeout only takes what's
/// already there.
pub fn pump(
    queue: &mut EventQueue<State>,
    state: &mut State,
    waker: &Waker,
    timeout: Duration,
) -> Result<()> {
    match queue.flush() {
        Ok(()) => {}
        // A full socket isn't a dead one: the rest goes out on a later flush.
        Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) => return Err(e.into()),
    }
    if queue.dispatch_pending(state)? > 0 {
        return Ok(());
    }
    // Stake the read before polling so nothing arriving in between is lost.
    let Some(guard) = queue.prepare_read() else {
        queue.dispatch_pending(state)?;
        return Ok(());
    };

    // Rounded up: waking a fraction of a millisecond early finds nothing due
    // and spins once more.
    let timeout_ms = timeout.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
    let pollin = |fd| libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let mut fds = [pollin(guard.connection_fd().as_raw_fd()), pollin(waker.raw())];
    // SAFETY: `fds` is a live array of `fds.len()` pollfd structs.
    let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
    if ret < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err.into());
        }
        return Ok(());
    }

    // Any revents (POLLHUP/POLLERR on compositor death too) → read, so a dead
    // connection surfaces as an error instead of a busy spin.
    if ret > 0 && fds[0].revents != 0 {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
    } else {
        drop(guard);
    }
    if ret > 0 && fds[1].revents != 0 {
        waker.drain();
    }
    queue.dispatch_pending(state)?;
    Ok(())
}
