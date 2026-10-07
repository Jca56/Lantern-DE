//! Looking at the machine. A [`Sampler`] reads `/proc`, sysfs and the
//! graphics driver each time it is asked and hands back a [`Frame`]:
//! everything the window shows, as of that moment, with the history
//! that led to it. Each part has a module of its own; the parsing in
//! them is plain functions over text, so it is tested without a machine
//! in any particular state.

pub mod cpu;
pub mod disk;
#[cfg(test)]
pub mod fixture;
pub mod gpu;
pub mod mem;
pub mod net;
mod nvml;
pub mod procs;
pub mod sensors;
pub mod system;

use std::fs;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use self::cpu::{Cpu, CpuSampler};
use self::disk::{DiskSampler, Disks};
use self::gpu::{Gpu, GpuSampler};
use self::mem::{Mem, MemSampler};
use self::net::{Iface, NetSampler};
use self::procs::{Proc, ProcSampler};
use self::sensors::{SensorSampler, Sensors};
use self::system::Facts;

/// How many looks a history keeps: what a graph shows across its width.
pub const HISTORY: usize = 120;

/// Add `v` to the end of a history, dropping its oldest once it is full.
pub fn push(history: &mut Vec<f32>, v: f32) {
    if history.len() >= HISTORY {
        history.remove(0);
    }
    history.push(if v.is_finite() { v } else { 0.0 });
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

/// The machine at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Counts the looks: a new frame has a new number.
    pub seq: u64,
    /// Seconds between looks, as asked for: what one step of a history
    /// spans.
    pub interval: f64,
    pub facts: Arc<Facts>,
    /// Seconds since the machine started.
    pub uptime: f64,
    /// The load averages over one, five and fifteen minutes.
    pub load: [f32; 3],
    pub cpu: Cpu,
    pub mem: Mem,
    pub gpus: Vec<Gpu>,
    pub disks: Disks,
    pub net: Vec<Iface>,
    pub sensors: Sensors,
    pub procs: Vec<Proc>,
}

impl Frame {
    /// How long a full history spans, in seconds.
    pub fn span(&self) -> f64 {
        self.interval * HISTORY as f64
    }

    /// Programs running, not counting the kernel's own threads.
    pub fn programs(&self) -> usize {
        self.procs.iter().filter(|p| !p.kernel).count()
    }

    pub fn threads(&self) -> u64 {
        self.procs.iter().map(|p| u64::from(p.threads)).sum()
    }
}

/// The first number of `/proc/uptime`.
fn uptime() -> f64 {
    read("/proc/uptime").and_then(|s| s.split_ascii_whitespace().next()?.parse().ok()).unwrap_or(0.0)
}

/// The first three numbers of `/proc/loadavg`.
pub fn parse_load(text: &str) -> [f32; 3] {
    let mut n = text.split_ascii_whitespace().map(|f| f.parse().unwrap_or(0.0));
    [n.next().unwrap_or(0.0), n.next().unwrap_or(0.0), n.next().unwrap_or(0.0)]
}

pub struct Sampler {
    facts: Arc<Facts>,
    seq: u64,
    last: Instant,
    cpu: CpuSampler,
    mem: MemSampler,
    gpu: GpuSampler,
    disk: DiskSampler,
    net: NetSampler,
    sensors: SensorSampler,
    procs: ProcSampler,
}

impl Sampler {
    /// Learn what the machine is and take the first readings of
    /// everything that is measured as a change since the last one.
    pub fn new() -> Self {
        let mut s = Self { facts: Arc::new(Facts::gather()), seq: 0, last: Instant::now(), cpu: CpuSampler::new(), mem: MemSampler::default(), gpu: GpuSampler::new(), disk: DiskSampler::default(), net: NetSampler::default(), sensors: SensorSampler::default(), procs: ProcSampler::new() };
        // Counters only: these histories start at the first real look.
        s.procs.sample(0);
        s.disk.prime();
        s.net.prime();
        s
    }

    /// Look at everything. `interval` is how often that is being asked.
    pub fn sample(&mut self, interval: f64) -> Frame {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64().max(0.001);
        self.last = now;
        self.seq += 1;
        let (cpu, machine_ticks) = self.cpu.sample();
        Frame {
            seq: self.seq,
            interval,
            facts: self.facts.clone(),
            uptime: uptime(),
            load: parse_load(&fs::read_to_string("/proc/loadavg").unwrap_or_default()),
            cpu,
            mem: self.mem.sample(),
            gpus: self.gpu.sample(),
            disks: self.disk.sample(dt),
            net: self.net.sample(dt),
            sensors: self.sensors.sample(),
            procs: self.procs.sample(machine_ticks),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_keeps_its_newest_and_never_holds_nonsense() {
        let mut h = Vec::new();
        for i in 0..HISTORY + 5 {
            push(&mut h, i as f32);
        }
        assert_eq!(h.len(), HISTORY);
        assert_eq!((h[0], h[HISTORY - 1]), (5.0, (HISTORY + 4) as f32));
        push(&mut h, f32::NAN);
        assert_eq!(h[HISTORY - 1], 0.0);
        assert_eq!(parse_load("1.20 0.98 0.75 2/1833 205268\n"), [1.20, 0.98, 0.75]);
        assert_eq!(parse_load(""), [0.0; 3]);
    }

    #[test]
    fn this_machine_is_looked_at_twice() {
        let mut s = Sampler::new();
        let first = s.sample(1.0);
        let again = s.sample(1.0);
        assert_eq!((first.seq, again.seq), (1, 2));
        assert!(again.uptime > 0.0 && again.mem.total > 0 && again.programs() > 0 && again.threads() > 0);
        assert_eq!(again.cpu.history.len(), 2);
        assert_eq!(again.span(), HISTORY as f64);
        // Primed when it was made: nothing carried since the machine
        // started is taken for a burst in the first interval.
        assert!(first.net.iter().all(|i| i.down_history.len() == 1));
        assert!(Arc::ptr_eq(&first.facts, &again.facts));
    }
}
