//! Saved tuning profile — flat key=value file in the house config style.
//!
//! The crash seatbelt lives here: `stable` starts false and every settings
//! change resets it. The daemon only re-applies a profile at startup when
//! `stable = 1`, so a session that died mid-crank boots back at stock.

use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub core_offset: i32,
    pub mem_offset: i32,
    /// 0 = stock (leave the driver default alone).
    pub power_limit_w: u32,
    pub fan_manual: bool,
    pub fan_pct: u32,
    /// Core clock lock — the undervolt half of lock+offset.
    pub lock_enabled: bool,
    pub lock_mhz: u32,
    /// 0 = leave firmware value alone.
    pub pl1_w: u32,
    pub pl2_w: u32,
    /// Empty = leave alone.
    pub governor: String,
    pub epp: String,
    pub max_freq_pct: u32,
    pub stable: bool,
}

impl Default for Profile {
    fn default() -> Profile {
        Profile {
            core_offset: 0,
            mem_offset: 0,
            power_limit_w: 0,
            fan_manual: false,
            fan_pct: 50,
            lock_enabled: false,
            lock_mhz: 0,
            pl1_w: 0,
            pl2_w: 0,
            governor: String::new(),
            epp: String::new(),
            max_freq_pct: 100,
            stable: false,
        }
    }
}

pub fn load(path: &Path) -> Option<Profile> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut p = Profile::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "core_offset" => p.core_offset = value.parse().unwrap_or(0),
            "mem_offset" => p.mem_offset = value.parse().unwrap_or(0),
            "power_limit_w" => p.power_limit_w = value.parse().unwrap_or(0),
            "fan_manual" => p.fan_manual = value == "1",
            "fan_pct" => p.fan_pct = value.parse().unwrap_or(50),
            "lock_enabled" => p.lock_enabled = value == "1",
            "lock_mhz" => p.lock_mhz = value.parse().unwrap_or(0),
            "pl1_w" => p.pl1_w = value.parse().unwrap_or(0),
            "pl2_w" => p.pl2_w = value.parse().unwrap_or(0),
            "governor" => p.governor = value.to_string(),
            "epp" => p.epp = value.to_string(),
            "max_freq_pct" => p.max_freq_pct = value.parse().unwrap_or(100),
            "stable" => p.stable = value == "1",
            _ => {}
        }
    }
    Some(p)
}

pub fn save(path: &Path, p: &Profile) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = format!(
        "# lntrn-afterglow tuning profile\n\
         core_offset = {}\n\
         mem_offset = {}\n\
         power_limit_w = {}\n\
         fan_manual = {}\n\
         fan_pct = {}\n\
         lock_enabled = {}\n\
         lock_mhz = {}\n\
         pl1_w = {}\n\
         pl2_w = {}\n\
         governor = {}\n\
         epp = {}\n\
         max_freq_pct = {}\n\
         stable = {}\n",
        p.core_offset,
        p.mem_offset,
        p.power_limit_w,
        p.fan_manual as u8,
        p.fan_pct,
        p.lock_enabled as u8,
        p.lock_mhz,
        p.pl1_w,
        p.pl2_w,
        p.governor,
        p.epp,
        p.max_freq_pct,
        p.stable as u8,
    );
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}
