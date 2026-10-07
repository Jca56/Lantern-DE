//! Memory and swap, from `/proc/meminfo`.

use std::fs;

use super::push;

/// Everything in bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mem {
    pub total: u64,
    /// What a program could still be given without swapping: what is
    /// free, and what only holds things that can be dropped.
    pub available: u64,
    /// What holds nothing at all.
    pub free: u64,
    /// Shared between programs and held in memory-backed files.
    pub shared: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    /// Percent in use over time, oldest first.
    pub history: Vec<f32>,
    /// Percent of swap in use over time; empty when there is no swap.
    pub swap_history: Vec<f32>,
}

impl Mem {
    /// What programs hold and would not give back.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// What the kernel keeps handy (files read lately) and would give
    /// back the moment a program wanted it.
    pub fn cache(&self) -> u64 {
        self.available.saturating_sub(self.free)
    }

    pub fn used_percent(&self) -> f32 {
        share(self.used(), self.total)
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }

    pub fn swap_percent(&self) -> f32 {
        share(self.swap_used(), self.swap_total)
    }
}

/// `part` of `whole` as a percentage; zero of nothing.
pub fn share(part: u64, whole: u64) -> f32 {
    if whole == 0 { 0.0 } else { (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0) as f32 }
}

/// `/proc/meminfo`, which counts in kibibytes, as bytes. An old kernel
/// without `MemAvailable` gets the free memory in its place.
pub fn parse_meminfo(text: &str) -> Mem {
    let mut mem = Mem::default();
    let mut available = None;
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else { continue };
        let bytes = rest.split_ascii_whitespace().next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0).saturating_mul(1024);
        match key {
            "MemTotal" => mem.total = bytes,
            "MemAvailable" => available = Some(bytes),
            "MemFree" => mem.free = bytes,
            "Shmem" => mem.shared = bytes,
            "SwapTotal" => mem.swap_total = bytes,
            "SwapFree" => mem.swap_free = bytes,
            _ => {}
        }
    }
    mem.available = available.unwrap_or(mem.free).min(mem.total);
    mem
}

#[derive(Default)]
pub struct MemSampler {
    history: Vec<f32>,
    swap_history: Vec<f32>,
}

impl MemSampler {
    pub fn sample(&mut self) -> Mem {
        let mut mem = parse_meminfo(&fs::read_to_string("/proc/meminfo").unwrap_or_default());
        push(&mut self.history, mem.used_percent());
        if mem.swap_total > 0 {
            push(&mut self.swap_history, mem.swap_percent());
        } else {
            self.swap_history.clear();
        }
        mem.history = self.history.clone();
        mem.swap_history = self.swap_history.clone();
        mem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:       32547948 kB\nMemFree:         2937056 kB\nMemAvailable:   29528792 kB\nBuffers:            1796 kB\nCached:         25289816 kB\nSwapTotal:       8388604 kB\nSwapFree:        6291452 kB\nShmem:            109336 kB\nHugePages_Total:       0\n";

    #[test]
    fn meminfo_is_read_in_bytes_and_split_into_used_cache_and_free() {
        let m = parse_meminfo(MEMINFO);
        assert_eq!(m.total, 32_547_948 * 1024);
        assert_eq!(m.used(), (32_547_948 - 29_528_792) * 1024);
        assert_eq!(m.cache(), (29_528_792 - 2_937_056) * 1024);
        // The three parts are the whole.
        assert_eq!(m.used() + m.cache() + m.free, m.total);
        assert!((m.used_percent() - 9.276).abs() < 0.01, "{}", m.used_percent());
        assert_eq!(m.swap_used(), (8_388_604 - 6_291_452) * 1024);
        assert!((m.swap_percent() - 25.0).abs() < 0.01);
    }

    #[test]
    fn nothing_and_nonsense_are_survived() {
        let m = parse_meminfo("");
        assert_eq!((m.used(), m.used_percent(), m.swap_percent()), (0, 0.0, 0.0));
        // No MemAvailable: what is free stands in. More available than
        // there is: held to the total.
        assert_eq!(parse_meminfo("MemTotal: 100 kB\nMemFree: 40 kB\n").available, 40 * 1024);
        assert_eq!(parse_meminfo("MemTotal: 100 kB\nMemAvailable: 900 kB\n").used(), 0);
    }
}
