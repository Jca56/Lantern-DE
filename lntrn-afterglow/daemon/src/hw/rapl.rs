//! Intel's package power limits (RAPL) through powercap: the sustained
//! limit (PL1, "long_term") and the boost limit (PL2, "short_term"), and
//! what the package draws, from its energy counter. Setting a limit and
//! reading the counter both take root.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::sysfs;

/// Which limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Sustained,
    Boost,
}

pub struct Rapl {
    dir: PathBuf,
    sustained: Option<u32>,
    boost: Option<u32>,
    wrap_uj: u64,
    last: Option<(u64, Instant)>,
}

impl Rapl {
    /// The first package's zone, if this machine has one.
    pub fn discover() -> Option<Rapl> {
        let mut zones: Vec<PathBuf> = std::fs::read_dir("/sys/class/powercap").ok()?.flatten().map(|e| e.path()).collect();
        zones.sort();
        // "intel-rapl:0" is a package; "intel-rapl:0:0" one of its parts.
        let dir = zones.into_iter().find(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            name.starts_with("intel-rapl:") && name.matches(':').count() == 1 && sysfs::read(&p.join("name")).is_some_and(|n| n.starts_with("package"))
        })?;
        let (mut sustained, mut boost) = (None, None);
        for i in 0..8 {
            match sysfs::read(&dir.join(format!("constraint_{i}_name"))).as_deref() {
                Some("long_term") => sustained = Some(i),
                Some("short_term") => boost = Some(i),
                Some(_) => {}
                None => break,
            }
        }
        let wrap_uj = sysfs::read_u64(&dir.join("max_energy_range_uj")).unwrap_or(u64::MAX);
        Some(Rapl { dir, sustained, boost, wrap_uj, last: None })
    }

    fn file(&self, limit: Limit, what: &str) -> Option<PathBuf> {
        let i = match limit {
            Limit::Sustained => self.sustained?,
            Limit::Boost => self.boost?,
        };
        Some(self.dir.join(format!("constraint_{i}_{what}")))
    }

    fn watts_in(path: &Path) -> Option<u32> {
        sysfs::read_u64(path).map(|uw| ((uw + 500_000) / 1_000_000) as u32)
    }

    /// The limit in force now, watts.
    pub fn limit(&self, limit: Limit) -> Option<u32> {
        Self::watts_in(&self.file(limit, "power_limit_uw")?)
    }

    /// The most this limit is rated for, as the firmware says it (which
    /// is often nothing, or the part's base power rather than a ceiling).
    pub fn rated(&self, limit: Limit) -> Option<u32> {
        Self::watts_in(&self.file(limit, "max_power_uw")?).filter(|w| *w > 0)
    }

    pub fn set(&self, limit: Limit, watts: u32) -> Result<(), String> {
        let path = self.file(limit, "power_limit_uw").ok_or("this package has no such power limit")?;
        sysfs::write(&path, &(u64::from(watts) * 1_000_000).to_string())
    }

    /// What the package has drawn since the last call, watts: `None` the
    /// first time, and without root.
    pub fn watts(&mut self) -> Option<f32> {
        let now = Instant::now();
        let energy = sysfs::read_u64(&self.dir.join("energy_uj"))?;
        let drawn = self.last.and_then(|(was, at)| {
            let uj = if energy >= was { energy - was } else { self.wrap_uj.saturating_sub(was) + energy };
            let seconds = now.duration_since(at).as_secs_f64();
            (seconds > 0.05).then(|| (uj as f64 / 1e6 / seconds) as f32)
        });
        self.last = Some((energy, now));
        drawn
    }
}
