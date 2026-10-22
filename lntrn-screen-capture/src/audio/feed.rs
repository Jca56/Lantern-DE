//! One source's sound as it comes in: 48 kHz stereo, interleaved, with
//! the moment on the wall clock its newest frame came. A capture thread
//! pushes; the mixer takes. Once the recording's start is known, every
//! frame has a place on the timeline: frames since the start.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Frames kept while waiting for the start, at most: two seconds.
const WAITING: usize = RATE as usize * 2;
/// A source whose newest frame is older than this has gone quiet: the
/// mix goes on without it.
const QUIET: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Inner {
    samples: VecDeque<f32>,
    /// Frames ever pushed.
    received: u64,
    /// When the newest frame came.
    newest: Option<Duration>,
    /// The recording's start, once known.
    start: Option<Duration>,
    /// The timeline place of the first frame held, once placed.
    front: Option<i64>,
    /// Why it stopped, if it did.
    ended: Option<String>,
}

impl Inner {
    fn held(&self) -> i64 {
        (self.samples.len() / CHANNELS) as i64
    }

    /// Place what's held: the newest frame came at `newest`.
    fn place(&mut self) {
        if let (Some(start), Some(newest), None) = (self.start, self.newest, self.front) {
            self.front = Some(frames_between(start, newest) - self.held());
        }
    }
}

pub struct Feed {
    pub name: &'static str,
    inner: Mutex<Inner>,
}

impl Feed {
    pub fn new(name: &'static str) -> Feed {
        Feed { name, inner: Mutex::new(Inner::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A block of interleaved frames, the last of them come `at`.
    pub fn push(&self, block: &[f32], at: Duration) {
        let mut i = self.lock();
        i.samples.extend(block.iter().copied());
        i.received += (block.len() / CHANNELS) as u64;
        i.newest = Some(at);
        if i.start.is_none() {
            let extra = i.samples.len().saturating_sub(WAITING * CHANNELS);
            i.samples.drain(..extra);
        }
        i.place();
    }

    /// It stopped, and why. The first reason stays.
    pub fn end(&self, why: impl Into<String>) {
        self.lock().ended.get_or_insert_with(|| why.into());
    }

    /// The recording starts at `start` (wall clock): what's held, and all
    /// that comes after, takes its place on the timeline.
    pub fn start(&self, start: Duration) {
        let mut i = self.lock();
        i.start = Some(start);
        i.place();
    }

    /// Whether it is still sending, as of `now`.
    pub fn live(&self, now: Duration) -> bool {
        let i = self.lock();
        i.ended.is_none() && i.newest.is_some_and(|at| now.saturating_sub(at) < QUIET)
    }

    /// The timeline place just past its newest frame, and when that came:
    /// how far it can give, once placed.
    pub fn reach(&self) -> Option<(i64, Duration)> {
        let i = self.lock();
        Some((i.front? + i.held(), i.newest?))
    }

    /// Add its frames for timeline places `from..to` into `out` (which
    /// holds that many frames), leaving silence where it has none. Frames
    /// before `from` are dropped: they came too late.
    pub fn mix_into(&self, from: i64, to: i64, out: &mut [f32]) {
        let mut i = self.lock();
        let Some(mut front) = i.front else { return };
        if front < from {
            let late = (from - front).min(i.held());
            i.samples.drain(..late as usize * CHANNELS);
            front += late;
        }
        let end = (front + i.held()).min(to);
        if end > front {
            let at = (front - from) as usize * CHANNELS;
            let n = (end - front) as usize * CHANNELS;
            for (o, s) in out[at..at + n].iter_mut().zip(i.samples.drain(..n)) {
                *o += s;
            }
            front = end;
        }
        i.front = Some(front);
    }

    /// Frames ever received, and why it stopped if it did.
    pub fn report(&self) -> (u64, Option<String>) {
        let i = self.lock();
        (i.received, i.ended.clone())
    }
}

/// Frames at 48 kHz from `from` to `to` on the wall clock, either way.
pub fn frames_between(from: Duration, to: Duration) -> i64 {
    ((to.as_secs_f64() - from.as_secs_f64()) * RATE as f64).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(1000.0 + s)
    }

    #[test]
    fn frames_before_the_start_are_dropped_and_a_late_source_starts_with_silence() {
        // 0.1 s held up to 0.05 s past the start: the first 0.05 s of it
        // came before the start.
        let early = Feed::new("early");
        early.push(&vec![0.5; 4800 * CHANNELS], secs(0.05));
        early.start(secs(0.0));
        let mut out = vec![0.0; 2400 * CHANNELS];
        early.mix_into(0, 2400, &mut out);
        assert!(out.iter().all(|&s| s == 0.5));
        assert_eq!(early.reach().unwrap().0, 2400);

        // Nothing until 0.1 s in: silence till then.
        let late = Feed::new("late");
        late.start(secs(0.0));
        late.push(&vec![0.25; 2400 * CHANNELS], secs(0.15));
        let mut out = vec![0.0; 7200 * CHANNELS];
        late.mix_into(0, 7200, &mut out);
        assert!(out[..4800 * CHANNELS].iter().all(|&s| s == 0.0));
        assert!(out[4800 * CHANNELS..].iter().all(|&s| s == 0.25));
    }

    #[test]
    fn two_sources_add_up_and_a_quiet_one_drops_out() {
        let (a, b) = (Feed::new("a"), Feed::new("b"));
        a.start(secs(0.0));
        b.start(secs(0.0));
        a.push(&vec![0.25; 960 * CHANNELS], secs(0.02));
        b.push(&vec![0.5; 960 * CHANNELS], secs(0.02));
        let mut out = vec![0.0; 960 * CHANNELS];
        a.mix_into(0, 960, &mut out);
        b.mix_into(0, 960, &mut out);
        assert!(out.iter().all(|&s| s == 0.75));
        assert!(a.live(secs(0.3)) && !a.live(secs(0.6)));
    }
}
