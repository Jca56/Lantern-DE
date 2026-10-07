//! The network interfaces: how much each has carried, from
//! `/proc/net/dev`, and so how fast it is carrying now.

use std::fs;
use std::path::Path;

use super::{push, read, read_number};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Iface {
    pub name: String,
    /// The link is up (or the interface is of a kind that doesn't say,
    /// like a tunnel).
    pub up: bool,
    pub wireless: bool,
    /// What the link negotiated, in megabits a second, where it says.
    pub speed: Option<u32>,
    /// Bytes since the machine started.
    pub received: u64,
    pub sent: u64,
    /// Bytes a second over the last interval.
    pub down: f32,
    pub up_rate: f32,
    /// The two rates over time, oldest first.
    pub down_history: Vec<f32>,
    pub up_history: Vec<f32>,
}

/// `/proc/net/dev`: every interface but the loopback, with the bytes it
/// has received and sent.
pub fn parse_net_dev(text: &str) -> Vec<(String, u64, u64)> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(name, rest)| {
            let name = name.trim();
            let n: Vec<u64> = rest.split_ascii_whitespace().map(|f| f.parse().unwrap_or(0)).collect();
            // Sixteen counters: eight received, then eight sent.
            (name != "lo" && n.len() >= 9).then(|| (name.to_owned(), n[0], n[8]))
        })
        .collect()
}

/// What the sampler keeps about an interface between looks.
struct Seen {
    name: String,
    received: u64,
    sent: u64,
    down_history: Vec<f32>,
    up_history: Vec<f32>,
}

#[derive(Default)]
pub struct NetSampler {
    seen: Vec<Seen>,
}

impl NetSampler {
    /// Note where every counter stands without calling it a reading, so
    /// the first real look has something to measure from.
    pub fn prime(&mut self) {
        self.sample(1.0);
        for s in &mut self.seen {
            s.down_history.clear();
            s.up_history.clear();
        }
    }

    /// The interfaces now; `dt` is the seconds since the last look.
    pub fn sample(&mut self, dt: f64) -> Vec<Iface> {
        let now = parse_net_dev(&fs::read_to_string("/proc/net/dev").unwrap_or_default());
        // An interface that has gone takes its history with it.
        self.seen.retain(|s| now.iter().any(|n| n.0 == s.name));
        let mut out: Vec<Iface> = now
            .into_iter()
            .map(|(name, received, sent)| {
                let i = self.seen.iter().position(|s| s.name == name).unwrap_or_else(|| {
                    // Just appeared: nothing to measure a rate against yet.
                    self.seen.push(Seen { name: name.clone(), received, sent, down_history: Vec::new(), up_history: Vec::new() });
                    self.seen.len() - 1
                });
                let seen = &mut self.seen[i];
                let down = (received.saturating_sub(seen.received) as f64 / dt) as f32;
                let up_rate = (sent.saturating_sub(seen.sent) as f64 / dt) as f32;
                (seen.received, seen.sent) = (received, sent);
                push(&mut seen.down_history, down);
                push(&mut seen.up_history, up_rate);
                let sys = format!("/sys/class/net/{name}");
                Iface {
                    up: read(&format!("{sys}/operstate")).is_none_or(|s| s != "down"),
                    wireless: Path::new(&format!("{sys}/wireless")).exists() || Path::new(&format!("{sys}/phy80211")).exists(),
                    // A link that is down, or has no speed, says -1 or nothing.
                    speed: read_number::<i64>(&format!("{sys}/speed")).and_then(|s| u32::try_from(s).ok()).filter(|s| *s > 0),
                    name,
                    received,
                    sent,
                    down,
                    up_rate,
                    down_history: seen.down_history.clone(),
                    up_history: seen.up_history.clone(),
                }
            })
            .collect();
        // The ones carrying something first, then by name.
        out.sort_by(|a, b| b.up.cmp(&a.up).then((b.received + b.sent > 0).cmp(&(a.received + a.sent > 0))).then(a.name.cmp(&b.name)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 146799080  100 0 0 0 0 0 0 146799080 100 0 0 0 0 0 0\n  eno1: 600571968 5000 0 0 0 0 0 9 23614456 4000 0 0 0 0 0 0\n wlan0: 25818188 300 0 0 0 0 0 0 754851 200 0 0 0 0 0 0\n";

    #[test]
    fn the_counters_are_read_and_the_loopback_is_left_out() {
        assert_eq!(parse_net_dev(DEV), [("eno1".to_owned(), 600_571_968, 23_614_456), ("wlan0".to_owned(), 25_818_188, 754_851)]);
        assert!(parse_net_dev("garbage\n eth0: 1 2\n").is_empty());
    }

    #[test]
    fn this_machine_is_read_twice_without_inventing_a_rate() {
        let mut s = NetSampler::default();
        let first = s.sample(1.0);
        // Nothing to compare the first look with: no rate, whatever the
        // interface has carried since the machine started.
        assert!(first.iter().all(|i| i.down == 0.0 && i.up_rate == 0.0));
        let again = s.sample(1.0);
        assert_eq!(again.len(), first.len());
        assert!(again.iter().all(|i| i.down_history.len() == 2 && i.down.is_finite()));
    }
}
