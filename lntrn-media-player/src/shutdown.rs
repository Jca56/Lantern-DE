//! Process-exit guarantees.
//!
//! Closing the window used to unwind everything in declaration order — the
//! GStreamer Null transition, then wgpu/NVIDIA, then the Wayland connection —
//! and each of those can block forever once the audio daemon stops serving
//! us. It happened: the window vanished but the process sat invisible for
//! nineteen minutes with its stream still attached, and system audio stayed
//! dead until it was force-killed. Two guards make that impossible now:
//!
//! * [`Watchdog`] — the main loop beats it every iteration; if the loop stops
//!   beating for longer than any legitimate operation takes, the process ends
//!   itself (and the audio stream goes back to the DE).
//! * [`exit_after`] — armed the moment we decide to close; the process is
//!   gone after this delay even if teardown wedges on the way out.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// How often the watchdog looks for a heartbeat.
const CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// Consecutive silent checks before the loop counts as wedged (≈15 s).
/// Accurate seeks on big 4K files take a second or two; nothing legitimate
/// takes this long.
const MISSES_BEFORE_STALL: u32 = 3;
/// Hard cap on the close path.
pub const EXIT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Watchdog {
    beats: Arc<AtomicU64>,
}

impl Watchdog {
    /// Start watching the main loop. A stall ends the process with `_exit`
    /// rather than `exit`: a wedged main thread may hold the very locks the
    /// atexit handlers would want.
    pub fn start() -> Self {
        Self::with(CHECK_INTERVAL, MISSES_BEFORE_STALL, || {
            let secs = (CHECK_INTERVAL * MISSES_BEFORE_STALL).as_secs();
            eprintln!(
                "[media-player] watchdog: main loop stalled for {secs}s — exiting so the audio stream is released"
            );
            // SAFETY: plain libc call; we are abandoning the process on purpose.
            unsafe { libc::_exit(2) }
        })
    }

    fn with(
        interval: Duration,
        misses_before_stall: u32,
        on_stall: impl FnOnce() + Send + 'static,
    ) -> Self {
        let beats = Arc::new(AtomicU64::new(0));
        let probe = beats.clone();
        let mut on_stall = Some(on_stall);
        let _ = thread::Builder::new()
            .name("watchdog".into())
            .spawn(move || {
                let mut last_seen = probe.load(Ordering::Relaxed);
                let mut misses = 0u32;
                loop {
                    let slept_at = Instant::now();
                    thread::sleep(interval);
                    // If *we* were held up (SIGSTOP, a debugger), the main loop
                    // wasn't necessarily wedged — start the count over.
                    if slept_at.elapsed() > interval * 2 {
                        misses = 0;
                        last_seen = probe.load(Ordering::Relaxed);
                        continue;
                    }
                    let now = probe.load(Ordering::Relaxed);
                    if now != last_seen {
                        misses = 0;
                        last_seen = now;
                        continue;
                    }
                    misses += 1;
                    if misses >= misses_before_stall {
                        if let Some(f) = on_stall.take() {
                            f();
                        }
                        return;
                    }
                }
            });
        Self { beats }
    }

    /// Call once per main-loop iteration.
    pub fn beat(&self) {
        self.beats.fetch_add(1, Ordering::Relaxed);
    }
}

/// Backstop for the close path: end the process after `delay` no matter what
/// teardown is doing. Harmless when close finishes first, which it should.
pub fn exit_after(delay: Duration) {
    let _ = thread::Builder::new()
        .name("exit-backstop".into())
        .spawn(move || {
            thread::sleep(delay);
            eprintln!("[media-player] close path still running after {delay:?}; forcing exit");
            // SAFETY: plain libc call; we are abandoning the process on purpose.
            unsafe { libc::_exit(0) }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn stall_fires_without_beats() {
        let (tx, rx) = mpsc::channel();
        let _wd = Watchdog::with(Duration::from_millis(10), 3, move || {
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn beats_keep_it_quiet() {
        let (tx, rx) = mpsc::channel();
        let wd = Watchdog::with(Duration::from_millis(10), 3, move || {
            let _ = tx.send(());
        });
        let until = Instant::now() + Duration::from_millis(200);
        while Instant::now() < until {
            wd.beat();
            thread::sleep(Duration::from_millis(2));
        }
        assert!(rx.try_recv().is_err());
    }
}
