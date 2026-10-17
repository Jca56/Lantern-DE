//! How hard the machine is working: the processors (and how hot they
//! run), the memory, the graphics cards and the processes, read from
//! `/proc`, sysfs and NVIDIA's own library. Two kinds of reader use it:
//!
//! - the samplers (`cpu`, `mem`, `gpu`, `procs`), which look each time
//!   they are asked and keep a history of what they saw. The System
//!   Monitor drives them from a thread of its own, with more besides.
//! - [`Load`] (`live`): a thread looking on a beat for a bar's mini
//!   monitors, waking the UI after each look.
//!
//! The parsing is plain functions over text, tested without a machine in
//! any particular state. A graphics card that is asleep (a laptop's,
//! powered down while nothing uses it) is left asleep: asking it anything
//! would wake it.

pub mod cpu;
pub mod gpu;
mod live;
pub mod mem;
mod nvml;
pub mod procs;
pub mod temp;

pub use live::{Load, LoadOptions, LoadState, Top};

use std::fs;
use std::str::FromStr;

/// Add `v` to the end of a history, dropping its oldest past `cap`.
pub fn push(history: &mut Vec<f32>, v: f32, cap: usize) {
    let cap = cap.max(1);
    if history.len() >= cap {
        history.drain(..=history.len() - cap);
    }
    history.push(if v.is_finite() { v } else { 0.0 });
}

/// Keep no more than the newest `cap` of a history.
pub fn trim(history: &mut Vec<f32>, cap: usize) {
    let cap = cap.max(1);
    if history.len() > cap {
        history.drain(..history.len() - cap);
    }
}

/// A small file's text without the space around it. `None` when it
/// can't be read.
pub fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// A file that holds one number.
pub fn read_number<T: FromStr>(path: &str) -> Option<T> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The first three numbers of `/proc/loadavg`.
pub fn parse_load(text: &str) -> [f32; 3] {
    let mut n = text.split_ascii_whitespace().map(|f| f.parse().unwrap_or(0.0));
    [n.next().unwrap_or(0.0), n.next().unwrap_or(0.0), n.next().unwrap_or(0.0)]
}

/// The load averages now: how many programs wanted a processor, over the
/// last one, five and fifteen minutes.
pub fn load_average() -> [f32; 3] {
    parse_load(&fs::read_to_string("/proc/loadavg").unwrap_or_default())
}

/// Seconds since the machine started: the first number of `/proc/uptime`.
pub fn uptime() -> f64 {
    read("/proc/uptime").and_then(|s| s.split_ascii_whitespace().next()?.parse().ok()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_keeps_its_newest_and_never_holds_nonsense() {
        let mut h = Vec::new();
        for i in 0..15 {
            push(&mut h, i as f32, 10);
        }
        assert_eq!(h.len(), 10);
        assert_eq!((h[0], h[9]), (5.0, 14.0));
        push(&mut h, f32::NAN, 10);
        assert_eq!(h[9], 0.0);
        // A shorter cap than it was kept to drops the oldest at once.
        push(&mut h, 1.0, 4);
        assert_eq!(h, [13.0, 14.0, 0.0, 1.0]);
        trim(&mut h, 2);
        assert_eq!(h, [0.0, 1.0]);
        trim(&mut h, 99);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn the_load_averages_are_read_and_nothing_is_survived() {
        assert_eq!(parse_load("1.20 0.98 0.75 2/1833 205268\n"), [1.20, 0.98, 0.75]);
        assert_eq!(parse_load(""), [0.0; 3]);
        assert!(uptime() > 0.0);
    }
}
