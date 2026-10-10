//! When a shell daemon that died is started again.

use std::time::{Duration, Instant};

/// Each death doubles the wait (1 s, 2 s, 4 s, 8 s). The fifth inside a
/// minute means it cannot stay up, and it is left down until the next login
/// rather than spun forever.
#[derive(Default)]
pub struct Backoff {
    deaths: Vec<Instant>,
}

impl Backoff {
    const WINDOW: Duration = Duration::from_secs(60);
    pub const MAX_DEATHS: usize = 5;

    /// Count a death at `now`: how long to wait before the restart, or
    /// `None` to give up.
    pub fn next(&mut self, now: Instant) -> Option<Duration> {
        self.deaths.retain(|&death| now.duration_since(death) < Self::WINDOW);
        self.deaths.push(now);
        (self.deaths.len() < Self::MAX_DEATHS)
            .then(|| Duration::from_secs(1 << (self.deaths.len() - 1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_then_gives_up() {
        let start = Instant::now();
        let mut backoff = Backoff::default();
        let delays: Vec<_> = (0..5).map(|i| backoff.next(start + Duration::from_secs(i))).collect();
        let secs = |s| Some(Duration::from_secs(s));
        assert_eq!(delays, [secs(1), secs(2), secs(4), secs(8), None]);
    }

    #[test]
    fn backoff_forgets_deaths_older_than_a_minute() {
        let start = Instant::now();
        let mut backoff = Backoff::default();
        for i in 0..4 {
            backoff.next(start + Duration::from_secs(i));
        }
        // An hour of running well, then one more death: back to a short wait.
        let later = start + Duration::from_secs(3600);
        assert_eq!(backoff.next(later), Some(Duration::from_secs(1)));
    }
}
