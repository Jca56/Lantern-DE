//! Intel RAPL package power limits (PL1/PL2) and package-power readout via
//! /sys/class/powercap. Limit writes and energy reads need root.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{read_trim, read_u64, write_value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// Sustained ("long_term") limit.
    Pl1,
    /// Burst ("short_term") limit.
    Pl2,
}

#[derive(Debug, Clone, Default)]
pub struct RaplLimits {
    pub pl1_w: Option<f64>,
    pub pl2_w: Option<f64>,
    pub pl1_max_w: Option<f64>,
    pub pl2_max_w: Option<f64>,
}

pub struct Rapl {
    dir: PathBuf,
    pl1_idx: Option<u32>,
    pl2_idx: Option<u32>,
    max_energy_uj: u64,
    last_energy: Option<(u64, Instant)>,
}

impl Rapl {
    /// Finds the first package-level RAPL zone; `None` on non-Intel boxes.
    pub fn discover() -> Option<Rapl> {
        let base = Path::new("/sys/class/powercap");
        let mut zone = None;
        for entry in std::fs::read_dir(base).ok()? {
            let path = entry.ok()?.path();
            let fname = path.file_name()?.to_string_lossy().into_owned();
            // Package zones look like "intel-rapl:0"; subzones have a second
            // colon segment ("intel-rapl:0:0" = core/uncore) — skip those.
            if !fname.starts_with("intel-rapl:") || fname.matches(':').count() != 1 {
                continue;
            }
            if read_trim(&path.join("name")).is_some_and(|n| n.starts_with("package")) {
                zone = Some(path);
                break;
            }
        }
        let dir = zone?;
        let (mut pl1_idx, mut pl2_idx) = (None, None);
        for i in 0..8 {
            match read_trim(&dir.join(format!("constraint_{i}_name"))).as_deref() {
                Some("long_term") => pl1_idx = Some(i),
                Some("short_term") => pl2_idx = Some(i),
                Some(_) => {}
                None => break,
            }
        }
        let max_energy_uj = read_u64(&dir.join("max_energy_range_uj")).unwrap_or(u64::MAX);
        Some(Rapl {
            dir,
            pl1_idx,
            pl2_idx,
            max_energy_uj,
            last_energy: None,
        })
    }

    fn idx(&self, limit: Limit) -> Option<u32> {
        match limit {
            Limit::Pl1 => self.pl1_idx,
            Limit::Pl2 => self.pl2_idx,
        }
    }

    pub fn limits(&self) -> RaplLimits {
        let read_w = |file: String| read_u64(&self.dir.join(file)).map(|uw| uw as f64 / 1e6);
        RaplLimits {
            pl1_w: self
                .pl1_idx
                .and_then(|i| read_w(format!("constraint_{i}_power_limit_uw"))),
            pl2_w: self
                .pl2_idx
                .and_then(|i| read_w(format!("constraint_{i}_power_limit_uw"))),
            pl1_max_w: self
                .pl1_idx
                .and_then(|i| read_w(format!("constraint_{i}_max_power_uw"))),
            pl2_max_w: self
                .pl2_idx
                .and_then(|i| read_w(format!("constraint_{i}_max_power_uw"))),
        }
    }

    pub fn set_limit(&self, limit: Limit, watts: f64) -> Result<(), String> {
        let idx = self
            .idx(limit)
            .ok_or("RAPL constraint not present on this package")?;
        let uw = (watts * 1e6) as u64;
        write_value(
            &self.dir.join(format!("constraint_{idx}_power_limit_uw")),
            &uw.to_string(),
        )
        .map_err(|e| format!("RAPL write failed (root needed): {e}"))
    }

    /// Package power from the energy counter delta. Needs root; returns
    /// `None` otherwise (or on the first call, before a delta exists).
    pub fn package_watts(&mut self) -> Option<f64> {
        let now = Instant::now();
        let energy = read_u64(&self.dir.join("energy_uj"))?;
        let watts = self.last_energy.map(|(prev, at)| {
            let delta_uj = if energy >= prev {
                energy - prev
            } else {
                // counter wrapped
                self.max_energy_uj - prev + energy
            };
            let secs = now.duration_since(at).as_secs_f64();
            if secs > 0.0 {
                delta_uj as f64 / 1e6 / secs
            } else {
                0.0
            }
        });
        self.last_energy = Some((energy, now));
        watts
    }
}
