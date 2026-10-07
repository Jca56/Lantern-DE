//! The sampler's thread: it looks at the machine on a beat and leaves
//! what it saw where the window can pick it up, then wakes the window.
//! Only the newest look is kept, so a window that isn't being drawn (it
//! is minimized, say) leaves nothing piling up behind it.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use lntrn_app::Waker;

use crate::sample::{Frame, Sampler};

/// How soon after starting the first look is taken: long enough for the
/// counters to have moved, short enough that the window isn't empty.
const FIRST_LOOK: Duration = Duration::from_millis(250);

/// What the window asks of the thread.
pub enum Cmd {
    /// Look every this many seconds from now on.
    Interval(f64),
    /// Stop looking, or start again.
    Paused(bool),
    /// Look now, whatever the beat (and even while paused).
    Now,
}

struct Shared {
    frame: Mutex<Option<Arc<Frame>>>,
    waker: OnceLock<Waker>,
}

/// The window's end of the thread.
pub struct Link {
    /// `None` when there is no thread: a frame fixed for a test.
    tx: Option<Sender<Cmd>>,
    shared: Arc<Shared>,
}

impl Link {
    /// Start the thread, looking every `interval` seconds.
    pub fn spawn(interval: f64) -> Link {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared { frame: Mutex::new(None), waker: OnceLock::new() });
        let theirs = shared.clone();
        let spawned = std::thread::Builder::new().name("sampler".into()).spawn(move || run(rx, theirs, interval));
        if let Err(e) = spawned {
            lntrn_core::log_error!("the sampler could not start: {e}");
        }
        Link { tx: Some(tx), shared }
    }

    /// No thread: `frame` is all there ever is to show.
    #[cfg(test)]
    pub fn fixed(frame: Frame) -> Link {
        Link { tx: None, shared: Arc::new(Shared { frame: Mutex::new(Some(Arc::new(frame))), waker: OnceLock::new() }) }
    }

    /// The loop's waker, so a new look shows without waiting for input.
    pub fn set_waker(&self, waker: Waker) {
        let _ = self.shared.waker.set(waker);
    }

    pub fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        }
    }

    /// The newest look, if one has been taken.
    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.shared.frame.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn run(rx: Receiver<Cmd>, shared: Arc<Shared>, mut interval: f64) {
    let mut sampler = Sampler::new();
    let mut paused = false;
    let mut due = Instant::now() + FIRST_LOOK;
    loop {
        // Sleep until the next look is due, or for good while paused;
        // a command ends the sleep either way.
        let cmd = if paused {
            match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        match cmd {
            // A new beat shows at once rather than after the old one.
            Some(Cmd::Interval(seconds)) => interval = seconds,
            Some(Cmd::Paused(true)) => {
                paused = true;
                continue;
            }
            Some(Cmd::Paused(false)) => paused = false,
            Some(Cmd::Now) | None => {}
        }
        // The next is due a beat after this one began, however long the
        // looking takes.
        let began = Instant::now();
        let frame = sampler.sample(interval);
        *shared.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(frame));
        if let Some(waker) = shared.waker.get() {
            waker.wake();
        }
        due = began + Duration::from_secs_f64(interval.clamp(0.1, 60.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thread_looks_soon_after_starting_and_again_when_told() {
        let link = Link::spawn(30.0);
        let wait_for = |seq: u64| {
            let until = Instant::now() + Duration::from_secs(10);
            while link.latest().is_none_or(|f| f.seq < seq) {
                assert!(Instant::now() < until, "no look {seq} within ten seconds");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait_for(1);
        // Thirty seconds to the next beat: only being told brings these.
        link.send(Cmd::Now);
        wait_for(2);
        link.send(Cmd::Paused(true));
        link.send(Cmd::Now);
        wait_for(3);
        link.send(Cmd::Interval(0.5));
        wait_for(4);
        assert_eq!(link.latest().unwrap().interval, 0.5);
    }

    #[test]
    fn a_fixed_link_shows_its_frame_and_ignores_commands() {
        let link = Link::fixed(Frame { seq: 7, ..Frame::default() });
        link.send(Cmd::Now);
        assert_eq!(link.latest().unwrap().seq, 7);
    }
}
