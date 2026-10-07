//! A machine that isn't this one, for tests: one of everything a page
//! can show (a graphics card that says it all, swap, a battery, fans, a
//! browser in many processes), the same on every run.

use std::sync::Arc;

use super::cpu::{Cpu, Thread};
use super::disk::{Disks, Drive, Volume};
use super::gpu::Gpu;
use super::mem::Mem;
use super::net::Iface;
use super::procs::Proc;
use super::sensors::{Battery, Chip, Fan, Sensors, Temp};
use super::system::Facts;
use super::{Frame, HISTORY};

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

/// A history that rises and falls between `low` and `high`.
fn wave(low: f32, high: f32) -> Vec<f32> {
    (0..HISTORY).map(|i| low + (high - low) * (0.5 + 0.5 * (i as f32 * 0.21).sin())).collect()
}

/// One process. `app` is the program it is folded under.
pub fn proc(pid: u32, name: &str, app: &str, user: &str, cpu: f32, memory: u64) -> Proc {
    Proc { pid, parent: 1, name: Arc::from(name), app: Arc::from(app), command: Arc::from(format!("/usr/bin/{app} --pid-{pid}")), user: Arc::from(user), kernel: false, threads: 4, cpu, memory, started: u64::from(pid) * 10 }
}

fn kernel(pid: u32, name: &str, app: &str) -> Proc {
    Proc { kernel: true, command: Arc::from(""), threads: 1, parent: 2, ..proc(pid, name, app, "root", 0.0, 0) }
}

pub fn procs() -> Vec<Proc> {
    vec![
        proc(1, "init", "init", "root", 0.0, 2 * MIB),
        kernel(2, "kthreadd", "kthreadd"),
        kernel(31, "kworker/0:1-events", "kworker"),
        kernel(32, "kworker/1:0-mm", "kworker"),
        proc(900, "sshd", "sshd", "root", 0.0, 3 * MIB),
        proc(1201, "lntrn-compositor", "lntrn-compositor", "alva", 3.1, 310 * MIB),
        proc(1300, "zsh", "zsh", "alva", 0.0, 9 * MIB),
        proc(4410, "firefox", "firefox", "alva", 6.0, 900 * MIB),
        proc(4411, "Web Content", "firefox", "alva", 4.0, 600 * MIB),
        proc(4412, "Isolated Web Co", "firefox", "alva", 2.0, 400 * MIB),
        proc(4413, "RDD Process", "firefox", "alva", 0.4, 200 * MIB),
        proc(5001, "rustc", "rustc", "alva", 12.0, 700 * MIB),
        proc(5002, "rustc", "rustc", "alva", 11.5, 650 * MIB),
        proc(6000, "steam", "steam", "alva", 1.0, 890 * MIB),
    ]
}

pub fn frame() -> Frame {
    let temp = |label: &str, celsius: f32, limit: Option<f32>| Temp { label: label.to_owned(), celsius, limit };
    Frame {
        seq: 1,
        interval: 1.0,
        facts: Arc::new(Facts {
            hostname: "testbench".to_owned(),
            os: "Gentoo Linux".to_owned(),
            kernel: "7.2.9-gentoo".to_owned(),
            shell: "zsh".to_owned(),
            cpu: "Intel Core i7-14700K".to_owned(),
            cores: 4,
            threads: 8,
            board: "ASUSTeK ROG STRIX Z790-E GAMING WIFI".to_owned(),
            displays: vec!["3840×2160 (DP-1)".to_owned(), "2560×1440 (HDMI-A-1)".to_owned()],
            packages: Some((1203, "Portage")),
        }),
        uptime: 273_600.0,
        load: [1.20, 0.98, 0.75],
        cpu: Cpu { usage: 37.0, threads: (0..8).map(|i| Thread { index: i, usage: (i * 13 % 100) as f32, mhz: 3600.0 + i as f32 * 100.0 }).collect(), mhz: 3950.0, governor: "powersave".to_owned(), history: wave(10.0, 60.0) },
        mem: Mem { total: 32 * GIB, available: 11 * GIB, free: 3 * GIB, shared: GIB, swap_total: 8 * GIB, swap_free: 6 * GIB, history: wave(55.0, 65.0), swap_history: wave(20.0, 25.0) },
        gpus: vec![Gpu {
            name: "NVIDIA GeForce RTX 3080 Ti".to_owned(),
            driver: "nvidia 615.71.09".to_owned(),
            usage: Some(12.0),
            memory: Some((3 * GIB, 12 * GIB)),
            celsius: Some(48.0),
            watts: Some(64.5),
            watts_limit: Some(350.0),
            fans: vec![30, 32],
            core_mhz: Some(1905),
            memory_mhz: Some(9501),
            usage_history: wave(5.0, 40.0),
            memory_history: wave(24.0, 26.0),
        }],
        disks: Disks {
            volumes: vec![
                Volume { device: "/dev/nvme0n1p2".to_owned(), mount: "/".to_owned(), also: vec!["/home".to_owned(), "/.snapshots".to_owned()], kind: "btrfs".to_owned(), total: 931 * GIB, available: 519 * GIB },
                Volume { device: "/dev/sda1".to_owned(), mount: "/mnt/storage".to_owned(), also: Vec::new(), kind: "btrfs".to_owned(), total: 3726 * GIB, available: 200 * GIB },
            ],
            drives: vec![
                Drive { name: "nvme0n1".to_owned(), model: "Samsung SSD 990 PRO 1TB".to_owned(), size: 931 * GIB, read: 1.2e6, written: 3.0e5, read_history: wave(0.0, 4.0e6), write_history: wave(0.0, 1.0e6) },
                Drive { name: "sda".to_owned(), model: String::new(), size: 3726 * GIB, read: 0.0, written: 0.0, read_history: vec![0.0; HISTORY], write_history: vec![0.0; HISTORY] },
            ],
        },
        net: vec![
            Iface { name: "eno1".to_owned(), up: true, wireless: false, speed: Some(1000), received: 600 * MIB, sent: 23 * MIB, down: 2.2e6, up_rate: 8.0e4, down_history: wave(0.0, 3.0e6), up_history: wave(0.0, 2.0e5) },
            Iface { name: "wlan0".to_owned(), up: false, wireless: true, speed: None, received: 25 * MIB, sent: MIB, down: 0.0, up_rate: 0.0, down_history: vec![0.0; 3], up_history: vec![0.0; 3] },
        ],
        sensors: Sensors {
            chips: vec![
                Chip { name: "Processor".to_owned(), driver: "coretemp".to_owned(), temps: vec![temp("Package id 0", 54.0, Some(100.0)), temp("Core 0", 50.0, Some(100.0)), temp("Core 4", 58.0, Some(100.0)), temp("Core 8", 47.0, Some(100.0))], fans: Vec::new() },
                Chip { name: "NVMe drive".to_owned(), driver: "nvme".to_owned(), temps: vec![temp("Composite", 41.0, Some(84.0)), temp("Sensor 1", 39.0, None)], fans: Vec::new() },
                Chip { name: "Mainboard".to_owned(), driver: "nct6798".to_owned(), temps: Vec::new(), fans: vec![Fan { label: "Fan 1".to_owned(), rpm: 1180 }, Fan { label: "Fan 2".to_owned(), rpm: 0 }] },
            ],
            batteries: vec![Battery { name: "BAT0".to_owned(), percent: 71.0, status: "Discharging".to_owned(), watts: Some(11.5), hours: Some(3.4), health: Some(0.91), cycles: Some(212) }],
        },
        procs: procs(),
    }
}

/// A machine with as little as one can have: no graphics card that
/// talks, no swap, no sensors, no battery, no network, nothing mounted.
pub fn bare() -> Frame {
    Frame { seq: 1, interval: 1.0, cpu: Cpu { usage: 3.0, threads: vec![Thread::default()], history: vec![3.0], ..Cpu::default() }, mem: Mem { total: 4 * GIB, available: 3 * GIB, free: 2 * GIB, history: vec![25.0], ..Mem::default() }, gpus: vec![Gpu { name: "Intel graphics".to_owned(), driver: "i915".to_owned(), ..Gpu::default() }], procs: vec![proc(1, "init", "init", "root", 0.0, MIB)], ..Frame::default() }
}
