//! How fast the sound card's clock runs against the wall's: a straight
//! line fitted through (seconds into the recording, frames a feed had
//! reached) over the whole recording. Each block's moment is a few
//! milliseconds off, never the same way twice; fitted over minutes, the
//! slope comes out within a few parts in a million.

use super::feed::RATE;

/// Points from this far in, once the start has settled.
const FROM: f64 = 2.0;
/// Trusted once the points span this long.
const TRUSTED: f64 = 30.0;
/// The most a sound card's crystal is ever off: 500 parts in a million.
const MOST: f64 = 0.0005;

#[derive(Default)]
pub struct Clock {
    /// The feed measured: another one starts the fit over.
    source: Option<usize>,
    /// The first point; the rest are counted from it.
    origin: Option<(f64, f64)>,
    last: f64,
    /// Welford's running means and co-moments, steady over hours.
    n: f64,
    mean_x: f64,
    mean_y: f64,
    cxx: f64,
    cxy: f64,
    rate: Option<f64>,
}

impl Clock {
    /// Feed number `source` had reached timeline frame `frames` when its
    /// newest block came, `secs` into the recording.
    pub fn measure(&mut self, source: usize, secs: f64, frames: f64) {
        if self.source != Some(source) {
            *self = Clock { source: Some(source), rate: self.rate, ..Clock::default() };
        }
        if secs < FROM {
            return;
        }
        let (x0, y0) = *self.origin.get_or_insert((secs, frames));
        let (x, y) = (secs - x0, frames - y0);
        // The same block asked about again is no new point.
        if self.n > 0.0 && x <= self.last {
            return;
        }
        self.last = x;
        self.n += 1.0;
        let dx = x - self.mean_x;
        self.mean_x += dx / self.n;
        self.mean_y += (y - self.mean_y) / self.n;
        self.cxx += dx * (x - self.mean_x);
        self.cxy += dx * (y - self.mean_y);
        if x >= TRUSTED && self.cxx > 0.0 {
            let slope = self.cxy / self.cxx;
            self.rate = Some((slope / RATE as f64).clamp(1.0 - MOST, 1.0 + MOST));
        }
    }

    /// The sound card's frames a wall-clock second, over 48000: 1 until
    /// measured.
    pub fn rate(&self) -> f64 {
        self.rate.unwrap_or(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card 200 parts in a million fast, each block's moment up to 10 ms
    /// off: measured within 20.
    #[test]
    fn a_fast_card_is_measured_through_the_jitter() {
        let true_rate = 1.0002;
        let mut clock = Clock::default();
        let mut seed = 12345u64;
        for i in 0..(90 * 50) {
            let secs = i as f64 / 50.0;
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let jitter = ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.02;
            clock.measure(0, secs + jitter.abs(), secs * RATE as f64 * true_rate);
        }
        assert!((clock.rate() - true_rate).abs() < 20e-6, "{}", clock.rate());
    }

    #[test]
    fn untrusted_until_long_enough_and_never_past_the_most() {
        let mut clock = Clock::default();
        for i in 0..(20 * 50) {
            let secs = i as f64 / 50.0;
            clock.measure(0, secs, secs * RATE as f64 * 1.01);
        }
        assert_eq!(clock.rate(), 1.0);
        for i in (20 * 50)..(60 * 50) {
            let secs = i as f64 / 50.0;
            clock.measure(0, secs, secs * RATE as f64 * 1.01);
        }
        assert_eq!(clock.rate(), 1.0 + MOST);
    }
}
