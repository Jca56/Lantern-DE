//! Waiting for Wayland events with a deadline. `blocking_dispatch` can only
//! wait forever, and sleeping between non-blocking reads makes input lag by
//! the sleep; poll() on the connection fd wakes the moment an event lands
//! or when the next timer (GIF frame, slideshow, key repeat) is due.

use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_short, c_ulong};
use std::time::Duration;

use anyhow::Result;
use wayland_client::EventQueue;

use crate::wayland::State;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

const POLLIN: c_short = 0x001;

extern "C" {
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
}

/// Flush requests, then dispatch whatever arrives within `timeout`
/// (`None` waits as long as it takes; `Some(ZERO)` only takes what's there).
pub fn pump(queue: &mut EventQueue<State>, state: &mut State, timeout: Option<Duration>) -> Result<()> {
    match queue.flush() {
        Ok(()) => {}
        // A full socket isn't a dead one: the rest goes out on a later flush.
        Err(wayland_client::backend::WaylandError::Io(e))
            if e.kind() == std::io::ErrorKind::WouldBlock => {}
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
    let timeout_ms: c_int = match timeout {
        None => -1,
        // Rounded up: waking a fraction of a millisecond early finds nothing
        // due and spins once more.
        Some(t) => t.as_micros().div_ceil(1000).min(c_int::MAX as u128) as c_int,
    };
    let mut fds = [PollFd {
        fd: guard.connection_fd().as_raw_fd(),
        events: POLLIN,
        revents: 0,
    }];
    let ret = unsafe { poll(fds.as_mut_ptr(), fds.len() as c_ulong, timeout_ms) };
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
            Err(wayland_client::backend::WaylandError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
    }
    queue.dispatch_pending(state)?;
    Ok(())
}
