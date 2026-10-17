//! The processors: how busy each has been since the last look, from the
//! tick counters in `/proc/stat`, how fast each is clocked, and what
//! they are called.

use std::collections::BTreeSet;
use std::fs;

use super::{push, read, read_number, trim};

/// A processor's clock ticks so far: all of them, and those spent
/// working.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ticks {
    pub busy: u64,
    pub total: u64,
}

impl Ticks {
    /// From a `/proc/stat` line's numbers: user, nice, system, idle,
    /// iowait, irq, softirq, steal. The guest columns after them are
    /// already counted in user and nice.
    fn from_fields<'a>(fields: impl Iterator<Item = &'a str>) -> Ticks {
        let n: Vec<u64> = fields.take(8).map(|f| f.parse().unwrap_or(0)).collect();
        let total: u64 = n.iter().sum();
        let idle = n.get(3).copied().unwrap_or(0) + n.get(4).copied().unwrap_or(0);
        Ticks { busy: total - idle, total }
    }

    /// Percent of the time between `earlier` and this spent working.
    pub fn usage_since(self, earlier: Ticks) -> f32 {
        let total = self.total.saturating_sub(earlier.total);
        if total == 0 {
            return 0.0;
        }
        (self.busy.saturating_sub(earlier.busy) as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
    }
}

/// `/proc/stat`: the whole machine's ticks, then each processor's with
/// its number (one that is switched off has no line).
pub fn parse_stat(text: &str) -> (Ticks, Vec<(usize, Ticks)>) {
    let mut all = Ticks::default();
    let mut each = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let Some(name) = fields.next() else { continue };
        let Some(number) = name.strip_prefix("cpu") else {
            // The cpu lines come first and together.
            break;
        };
        if number.is_empty() {
            all = Ticks::from_fields(fields);
        } else if let Ok(n) = number.parse() {
            each.push((n, Ticks::from_fields(fields)));
        }
    }
    (all, each)
}

/// The `cpu MHz` of every processor in `/proc/cpuinfo`, in order: where
/// the clocks are read when the kernel has no cpufreq for them.
pub fn parse_cpuinfo_mhz(text: &str) -> Vec<f32> {
    text.lines().filter(|l| l.starts_with("cpu MHz")).filter_map(|l| l.split_once(':')).filter_map(|(_, v)| v.trim().parse().ok()).collect()
}

/// One logical processor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Thread {
    /// Its number, as the kernel counts them.
    pub index: usize,
    /// Percent of the last interval it spent working.
    pub usage: f32,
    /// What it is clocked at; zero when that can't be read.
    pub mhz: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cpu {
    /// Percent of the last interval the whole machine spent working.
    pub usage: f32,
    pub threads: Vec<Thread>,
    /// The mean of the threads' clocks.
    pub mhz: f32,
    /// What decides the clocks ("powersave"); empty when nothing says.
    pub governor: String,
    /// `usage` over time, oldest first.
    pub history: Vec<f32>,
}

pub struct CpuSampler {
    all: Ticks,
    each: Vec<(usize, Ticks)>,
    history: Vec<f32>,
    /// How many looks the history keeps.
    cap: usize,
}

fn stat() -> (Ticks, Vec<(usize, Ticks)>) {
    parse_stat(&fs::read_to_string("/proc/stat").unwrap_or_default())
}

fn cpufreq(index: usize, file: &str) -> String {
    format!("/sys/devices/system/cpu/cpu{index}/cpufreq/{file}")
}

impl CpuSampler {
    /// Take the first reading: usage is always since the one before. The
    /// history keeps the last `cap` looks.
    pub fn new(cap: usize) -> Self {
        let (all, each) = stat();
        Self { all, each, history: Vec::new(), cap }
    }

    /// Keep `cap` looks from now on.
    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap;
        trim(&mut self.history, cap);
    }

    /// The processors now, and how many ticks the whole machine has
    /// counted since the last look: what a process's own ticks are a
    /// share of.
    pub fn sample(&mut self) -> (Cpu, u64) {
        let (all, each) = stat();
        let usage = all.usage_since(self.all);
        let elapsed = all.total.saturating_sub(self.all.total);
        // Without cpufreq (a virtual machine, some ARM boards) the clocks
        // are in cpuinfo, in the same order.
        let fallback = if fs::exists(cpufreq(each.first().map_or(0, |e| e.0), "scaling_cur_freq")).unwrap_or(false) { Vec::new() } else { parse_cpuinfo_mhz(&fs::read_to_string("/proc/cpuinfo").unwrap_or_default()) };
        let threads: Vec<Thread> = each
            .iter()
            .enumerate()
            .map(|(order, (index, ticks))| {
                let before = self.each.iter().find(|e| e.0 == *index).map_or(*ticks, |e| e.1);
                let khz: Option<f32> = read_number(&cpufreq(*index, "scaling_cur_freq"));
                let mhz = khz.map(|k| k / 1000.0).or(fallback.get(order).copied()).unwrap_or(0.0);
                Thread { index: *index, usage: ticks.usage_since(before), mhz }
            })
            .collect();
        let clocked: Vec<f32> = threads.iter().map(|t| t.mhz).filter(|m| *m > 0.0).collect();
        let mhz = if clocked.is_empty() { 0.0 } else { clocked.iter().sum::<f32>() / clocked.len() as f32 };
        let governor = read(&cpufreq(threads.first().map_or(0, |t| t.index), "scaling_governor")).unwrap_or_default();
        push(&mut self.history, usage, self.cap);
        self.all = all;
        self.each = each;
        (Cpu { usage, threads, mhz, governor, history: self.history.clone() }, elapsed)
    }
}

/// A processor's name as `/proc/cpuinfo` has it, without the trademark
/// signs and the clock speed it is sold at.
pub fn cpu_model(cpuinfo: &str) -> String {
    let raw = cpuinfo.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim()).unwrap_or("");
    let name = raw.replace("(R)", "").replace("(TM)", "").replace("(tm)", "").replace(" CPU", "").replace(" Processor", "");
    let name = name.split(" @ ").next().unwrap_or("");
    name.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// How many cores and threads `/proc/cpuinfo` lists. Cores are told
/// apart by which package and which core in it a thread sits on; a
/// kernel that doesn't say gets zero cores.
pub fn cpu_counts(cpuinfo: &str) -> (usize, usize) {
    let mut cores = BTreeSet::new();
    let (mut threads, mut package) = (0, "");
    for line in cpuinfo.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        match key.trim() {
            "processor" => threads += 1,
            "physical id" => package = value.trim(),
            "core id" => {
                cores.insert((package, value.trim()));
            }
            _ => {}
        }
    }
    (cores.len(), threads)
}

/// This machine's processor's name, from `/proc/cpuinfo`.
pub fn this_cpu() -> String {
    cpu_model(&fs::read_to_string("/proc/cpuinfo").unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  1201073 2882 184789 22741242 170766 45831 13440 0 0 0\ncpu0 36676 103 5055 807257 9377 1222 1740 0 0 0\ncpu2 31339 126 3410 824276 4098 323 576 0 0 0\nintr 123 4 5\nctxt 99\n";

    #[test]
    fn stat_gives_the_machine_and_each_processor_by_number() {
        let (all, each) = parse_stat(STAT);
        assert_eq!(all.total, 1_201_073 + 2882 + 184_789 + 22_741_242 + 170_766 + 45_831 + 13_440);
        assert_eq!(all.busy, all.total - 22_741_242 - 170_766, "waiting on a disk is not working");
        // cpu1 is switched off: the numbers are the kernel's, not a count.
        assert_eq!(each.iter().map(|e| e.0).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(parse_stat("").1.len(), 0);
    }

    #[test]
    fn usage_is_the_share_of_the_ticks_between_two_looks() {
        let before = Ticks { busy: 100, total: 1000 };
        assert_eq!(Ticks { busy: 150, total: 1200 }.usage_since(before), 25.0);
        assert_eq!(before.usage_since(before), 0.0, "no time passed");
        // A counter that went backwards (a processor switched off and on).
        assert_eq!(Ticks { busy: 10, total: 2000 }.usage_since(before), 0.0);
    }

    #[test]
    fn clocks_are_read_from_cpuinfo_when_that_is_all_there_is() {
        let text = "processor\t: 0\nmodel name\t: Some CPU\ncpu MHz\t\t: 3633.868\n\nprocessor\t: 1\ncpu MHz\t\t: 800.000\n";
        assert_eq!(parse_cpuinfo_mhz(text), [3633.868, 800.0]);
    }

    #[test]
    fn a_processor_is_named_plainly_and_its_cores_are_counted_once() {
        assert_eq!(cpu_model("model name\t: Intel(R) Core(TM) i7-14700K\n"), "Intel Core i7-14700K");
        assert_eq!(cpu_model("model name\t: Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz\n"), "Intel Core i5-8250U");
        assert_eq!(cpu_model("model name\t: AMD Ryzen 7 5800X 8-Core Processor\n"), "AMD Ryzen 7 5800X 8-Core");
        assert_eq!(cpu_model(""), "");
        // Two threads on core 0, one on core 1, and core 0 of a second package.
        let info = "processor: 0\nphysical id: 0\ncore id: 0\n\nprocessor: 1\nphysical id: 0\ncore id: 0\n\nprocessor: 2\nphysical id: 0\ncore id: 1\n\nprocessor: 3\nphysical id: 1\ncore id: 0\n";
        assert_eq!(cpu_counts(info), (3, 4));
        assert_eq!(cpu_counts("processor: 0\nprocessor: 1\n"), (0, 2));
    }

    #[test]
    fn this_machine_is_read_and_its_history_kept_to_its_cap() {
        let mut s = CpuSampler::new(2);
        let (cpu, _) = s.sample();
        assert!(!cpu.threads.is_empty());
        assert!((0.0..=100.0).contains(&cpu.usage));
        assert_eq!(cpu.history.len(), 1);
        s.sample();
        assert_eq!(s.sample().0.history.len(), 2);
        s.set_cap(1);
        assert_eq!(s.history.len(), 1);
        assert!(!this_cpu().is_empty());
    }
}
