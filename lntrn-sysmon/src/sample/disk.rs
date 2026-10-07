//! Storage: how full each mounted filesystem is, and how hard each
//! physical drive is being read and written (`/proc/diskstats`).

use std::fs;
use std::path::Path;

use super::{push, read, read_number};
use crate::ffi;

/// `/proc/diskstats` counts in sectors of this many bytes, whatever the
/// drive's own are.
const SECTOR: u64 = 512;

/// One filesystem, however many places it is mounted at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Volume {
    /// The block device it is on: `/dev/nvme0n1p2`.
    pub device: String,
    /// Where it is mounted; the shortest path when there are several.
    pub mount: String,
    /// The other places it is mounted (a btrfs's subvolumes).
    pub also: Vec<String>,
    /// The kind of filesystem: `btrfs`.
    pub kind: String,
    pub total: u64,
    /// Bytes an ordinary user can still write.
    pub available: u64,
}

impl Volume {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

/// One physical drive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drive {
    /// The kernel's name for it: `nvme0n1`.
    pub name: String,
    /// What it calls itself; empty when it doesn't.
    pub model: String,
    pub size: u64,
    /// Bytes a second over the last interval.
    pub read: f32,
    pub written: f32,
    pub read_history: Vec<f32>,
    pub write_history: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Disks {
    pub volumes: Vec<Volume>,
    pub drives: Vec<Drive>,
}

/// A path as `/proc/mounts` writes it, with its `\040`-style escapes
/// (a space, a tab, a backslash) put back.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = b.get(i + 1..i + 4).filter(|d| b[i] == b'\\' && d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match octal {
            Some(d) => {
                out.push(d.iter().fold(0u32, |n, c| n * 8 + u32::from(c - b'0')) as u8);
                i += 4;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `/proc/mounts`: the filesystems on real block devices, each once,
/// with every place it is mounted. Sizes are not filled in.
pub fn parse_mounts(text: &str) -> Vec<Volume> {
    let mut volumes: Vec<Volume> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let (Some(device), Some(mount), Some(kind)) = (fields.next(), fields.next(), fields.next()) else { continue };
        // Loop devices are images (a snap, an ISO), not storage.
        if !device.starts_with("/dev/") || device.starts_with("/dev/loop") {
            continue;
        }
        let mount = unescape(mount);
        match volumes.iter_mut().find(|v| v.device == device) {
            Some(v) => v.also.push(mount),
            None => volumes.push(Volume { device: device.to_owned(), mount, kind: kind.to_owned(), ..Volume::default() }),
        }
    }
    for v in &mut volumes {
        // The shortest path names it; the rest are listed after.
        v.also.push(std::mem::take(&mut v.mount));
        v.also.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        v.also.dedup();
        v.mount = v.also.remove(0);
    }
    volumes
}

/// `/proc/diskstats`: every block device's name with the sectors it has
/// read and written.
pub fn parse_diskstats(text: &str) -> Vec<(String, u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_ascii_whitespace().collect();
            // major minor name reads merged sectors ms writes merged sectors ...
            (f.len() >= 10).then(|| (f[2].to_owned(), f[5].parse().unwrap_or(0), f[9].parse().unwrap_or(0)))
        })
        .collect()
}

/// Whether the block device `name` is a drive: a whole disk with
/// hardware behind it, not a partition, a loop image or a mapping.
fn is_drive(name: &str) -> bool {
    Path::new(&format!("/sys/block/{name}/device")).exists()
}

struct Seen {
    name: String,
    read: u64,
    written: u64,
    read_history: Vec<f32>,
    write_history: Vec<f32>,
}

#[derive(Default)]
pub struct DiskSampler {
    seen: Vec<Seen>,
}

impl DiskSampler {
    /// Note where every counter stands without calling it a reading, so
    /// the first real look has something to measure from.
    pub fn prime(&mut self) {
        self.sample(1.0);
        for s in &mut self.seen {
            s.read_history.clear();
            s.write_history.clear();
        }
    }

    /// Storage now; `dt` is the seconds since the last look.
    pub fn sample(&mut self, dt: f64) -> Disks {
        let mut volumes = parse_mounts(&fs::read_to_string("/proc/mounts").unwrap_or_default());
        volumes.retain_mut(|v| match ffi::space(&v.mount) {
            Some((total, available)) if total > 0 => {
                (v.total, v.available) = (total, available);
                true
            }
            _ => false,
        });

        let stats = parse_diskstats(&fs::read_to_string("/proc/diskstats").unwrap_or_default());
        let stats: Vec<(String, u64, u64)> = stats.into_iter().filter(|s| is_drive(&s.0)).collect();
        self.seen.retain(|s| stats.iter().any(|n| n.0 == s.name));
        let drives = stats
            .into_iter()
            .map(|(name, read_sectors, written_sectors)| {
                let (now_read, now_written) = (read_sectors * SECTOR, written_sectors * SECTOR);
                let i = self.seen.iter().position(|s| s.name == name).unwrap_or_else(|| {
                    self.seen.push(Seen { name: name.clone(), read: now_read, written: now_written, read_history: Vec::new(), write_history: Vec::new() });
                    self.seen.len() - 1
                });
                let seen = &mut self.seen[i];
                let read_rate = (now_read.saturating_sub(seen.read) as f64 / dt) as f32;
                let write_rate = (now_written.saturating_sub(seen.written) as f64 / dt) as f32;
                (seen.read, seen.written) = (now_read, now_written);
                push(&mut seen.read_history, read_rate);
                push(&mut seen.write_history, write_rate);
                Drive {
                    model: read(&format!("/sys/block/{name}/device/model")).unwrap_or_default(),
                    size: read_number::<u64>(&format!("/sys/block/{name}/size")).unwrap_or(0) * SECTOR,
                    name,
                    read: read_rate,
                    written: write_rate,
                    read_history: seen.read_history.clone(),
                    write_history: seen.write_history.clone(),
                }
            })
            .collect();
        Disks { volumes, drives }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTS: &str = "proc /proc proc rw 0 0\n/dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0\n/dev/nvme0n1p1 /efi vfat rw 0 0\n/dev/nvme0n1p2 /home btrfs rw 0 0\n/dev/nvme0n1p2 /.snapshots btrfs rw 0 0\ntmpfs /tmp tmpfs rw 0 0\n/dev/loop3 /snap/core squashfs ro 0 0\n/dev/sda1 /mnt/My\\040Stuff ext4 rw 0 0\n";

    #[test]
    fn a_filesystem_is_listed_once_under_its_shortest_mount() {
        let v = parse_mounts(MOUNTS);
        assert_eq!(v.iter().map(|v| v.device.as_str()).collect::<Vec<_>>(), ["/dev/nvme0n1p2", "/dev/nvme0n1p1", "/dev/sda1"]);
        assert_eq!((v[0].mount.as_str(), v[0].kind.as_str()), ("/", "btrfs"));
        assert_eq!(v[0].also, ["/home", "/.snapshots"]);
        assert!(v[1].also.is_empty());
        // A space in a path comes out of its escape.
        assert_eq!(v[2].mount, "/mnt/My Stuff");
        assert_eq!(unescape("a\\134b\\04"), "a\\b\\04", "a short escape is left as it is");
    }

    #[test]
    fn diskstats_gives_the_sectors_read_and_written() {
        let text = " 259       0 nvme0n1 136773 16522 12619787 40607 164263 1260 7188393 321107 0 50400 368979\n 259       1 nvme0n1p1 10 0 80 1 0 0 0 0 0 1 1\n   8       0 sda 64134 18724 35774640 2068788 32771 27935 25307920 786672 0 368544 2876854\nshort line\n";
        let s = parse_diskstats(text);
        assert_eq!(s[0], ("nvme0n1".to_owned(), 12_619_787, 7_188_393));
        assert_eq!(s[2], ("sda".to_owned(), 35_774_640, 25_307_920));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn this_machine_is_read() {
        let mut s = DiskSampler::default();
        let disks = s.sample(1.0);
        assert!(disks.volumes.iter().all(|v| v.total > 0 && v.available <= v.total && v.mount.starts_with('/')));
        // Partitions are not drives, and a first look has no rate.
        assert!(disks.drives.iter().all(|d| is_drive(&d.name) && d.read == 0.0 && d.written == 0.0));
    }
}
