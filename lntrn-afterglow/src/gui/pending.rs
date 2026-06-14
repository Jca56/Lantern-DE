//! Staged ("pending") tuning state for the Tune page.
//!
//! In the Apply-button model nothing reaches the hardware until the user
//! commits: sliders/chips/toggles mutate this struct, then Apply turns it into
//! `set` commands and Save turns it into a boot `Profile`. All the per-knob
//! range/format math lives here so the draw and interaction code agree.

use lntrn_afterglow::ipc::{CpuTuning, Info, Snapshot};
use lntrn_afterglow::profile::Profile;

use super::draw::signed;

// ── Hit-zone IDs (window controls 1-3 live in mod.rs) ───────────────
pub const ZONE_TAB_MONITOR: u32 = 4;
pub const ZONE_TAB_TUNE: u32 = 5;
pub const ZONE_APPLY: u32 = 6;
pub const ZONE_SAVE: u32 = 7;
pub const ZONE_RESET: u32 = 8;
pub const ZONE_TGL_FAN: u32 = 10;
pub const ZONE_TGL_LOCK: u32 = 11;
pub const ZONE_TGL_TURBO: u32 = 12;
pub const ZONE_GOV: u32 = 100;
pub const ZONE_EPP: u32 = 101;

// GPU lock-clock range isn't exposed by NVML; use a sane Ampere span.
const LOCK_MIN: u32 = 200;
const LOCK_MAX: u32 = 2100;
const PL_MIN: u32 = 50;

/// Slider drag snaps to this step so it lands on round numbers. Memory's
/// offset range is enormous (GDDR6X tolerates +1000s), so it steps coarser.
fn slider_step(knob: Knob) -> f32 {
    match knob {
        Knob::MemOffset => 50.0,
        _ => 5.0,
    }
}

/// The sliders, identified so geometry (tune.rs) and value math (here) agree.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Knob {
    CoreOffset,
    MemOffset,
    PowerLimit,
    FanPct,
    LockMhz,
    MaxFreq,
    Pl1,
    Pl2,
}

#[derive(Default, Clone)]
pub struct Pending {
    pub initialized: bool,
    pub core_offset: i32,
    pub mem_offset: i32,
    pub power_limit_w: u32,
    pub fan_manual: bool,
    pub fan_pct: u32,
    pub lock_enabled: bool,
    pub lock_mhz: u32,
    pub pl1_w: u32,
    pub pl2_w: u32,
    pub governor: String,
    pub epp: String,
    pub turbo: bool,
    pub max_freq_pct: u32,
}

impl Pending {
    /// Seed staged values from live hardware. Only meaningful the first time —
    /// callers guard on `initialized` so polling never clobbers user edits.
    pub fn init_from(&mut self, info: &Info, snap: &Snapshot, tuning: Option<&CpuTuning>) {
        self.core_offset = info.core_off;
        self.mem_offset = info.mem_off;
        self.power_limit_w = if snap.power_limit_w > 0.0 {
            snap.power_limit_w.round() as u32
        } else {
            info.power_default_w
        };
        self.fan_manual = false;
        self.fan_pct = if snap.fans_pct.is_empty() {
            50
        } else {
            snap.fans_pct.iter().sum::<u32>() / snap.fans_pct.len() as u32
        };
        self.lock_enabled = false;
        self.lock_mhz = snap.core_mhz.clamp(LOCK_MIN, LOCK_MAX);
        self.pl1_w = info.pl1_w;
        self.pl2_w = info.pl2_w;
        if let Some(t) = tuning {
            self.governor = t.governor.clone();
            self.epp = t.epp.clone();
            self.turbo = t.turbo.unwrap_or(true);
            self.max_freq_pct = t.max_freq_pct;
        } else {
            self.turbo = true;
            self.max_freq_pct = 100;
        }
        self.initialized = true;
    }

    /// Revert staged values to stock (offsets off, default power, auto fans,
    /// no lock, turbo on, full frequency). Governor/EPP have no canonical
    /// stock so they're left as-is.
    pub fn reset_to_stock(&mut self, info: &Info) {
        self.core_offset = 0;
        self.mem_offset = 0;
        self.power_limit_w = info.power_default_w;
        self.fan_manual = false;
        self.lock_enabled = false;
        self.turbo = true;
        self.max_freq_pct = 100;
        self.pl1_w = info.pl1_w;
        self.pl2_w = info.pl2_w;
    }

    pub fn range(knob: Knob, info: &Info) -> (f32, f32) {
        match knob {
            Knob::CoreOffset => (info.core_min as f32, info.core_max as f32),
            Knob::MemOffset => (info.mem_min as f32, info.mem_max as f32),
            Knob::PowerLimit => (info.power_min_w as f32, info.power_max_w.max(1) as f32),
            Knob::FanPct => (0.0, 100.0),
            Knob::LockMhz => (LOCK_MIN as f32, LOCK_MAX as f32),
            Knob::MaxFreq => (10.0, 100.0),
            Knob::Pl1 | Knob::Pl2 => (PL_MIN as f32, pl_ceiling(info) as f32),
        }
    }

    fn value(&self, knob: Knob) -> f32 {
        match knob {
            Knob::CoreOffset => self.core_offset as f32,
            Knob::MemOffset => self.mem_offset as f32,
            Knob::PowerLimit => self.power_limit_w as f32,
            Knob::FanPct => self.fan_pct as f32,
            Knob::LockMhz => self.lock_mhz as f32,
            Knob::MaxFreq => self.max_freq_pct as f32,
            Knob::Pl1 => self.pl1_w as f32,
            Knob::Pl2 => self.pl2_w as f32,
        }
    }

    /// 0..1 thumb position for drawing.
    pub fn slider_frac(&self, knob: Knob, info: &Info) -> f32 {
        let (lo, hi) = Self::range(knob, info);
        if hi <= lo {
            0.0
        } else {
            ((self.value(knob) - lo) / (hi - lo)).clamp(0.0, 1.0)
        }
    }

    /// Store a slider value from a 0..1 drag fraction, snapped to the knob's
    /// step (memory's range is huge, so it steps coarser).
    pub fn set_slider(&mut self, knob: Knob, frac: f32, info: &Info) {
        let (lo, hi) = Self::range(knob, info);
        let raw = lo + frac.clamp(0.0, 1.0) * (hi - lo);
        let step = slider_step(knob);
        let v = ((raw / step).round() * step).clamp(lo, hi);
        match knob {
            Knob::CoreOffset => self.core_offset = v as i32,
            Knob::MemOffset => self.mem_offset = v as i32,
            Knob::PowerLimit => self.power_limit_w = v as u32,
            Knob::FanPct => self.fan_pct = v as u32,
            Knob::LockMhz => self.lock_mhz = v as u32,
            Knob::MaxFreq => self.max_freq_pct = v as u32,
            Knob::Pl1 => self.pl1_w = v as u32,
            Knob::Pl2 => self.pl2_w = v as u32,
        }
    }

    pub fn knob_title(knob: Knob) -> &'static str {
        match knob {
            Knob::CoreOffset => "Core Offset",
            Knob::MemOffset => "Memory Offset",
            Knob::PowerLimit => "Power Limit",
            Knob::FanPct => "Fan Speed",
            Knob::LockMhz => "Lock Clock",
            Knob::MaxFreq => "Max Frequency",
            Knob::Pl1 => "RAPL PL1",
            Knob::Pl2 => "RAPL PL2",
        }
    }

    pub fn knob_value_str(&self, knob: Knob) -> String {
        match knob {
            Knob::CoreOffset => format!("{} MHz", signed(self.core_offset)),
            Knob::MemOffset => format!("{} MHz", signed(self.mem_offset)),
            Knob::PowerLimit => format!("{} W", self.power_limit_w),
            Knob::FanPct => format!("{}%", self.fan_pct),
            Knob::LockMhz => format!("{} MHz", self.lock_mhz),
            Knob::MaxFreq => format!("{}%", self.max_freq_pct),
            Knob::Pl1 => format!("{} W", self.pl1_w),
            Knob::Pl2 => format!("{} W", self.pl2_w),
        }
    }

    /// The `set` command lines an Apply sends to the daemon, in order.
    pub fn set_commands(&self, info: &Info) -> Vec<String> {
        let mut cmds = Vec::new();
        if info.gpu {
            cmds.push(format!("set core_offset {}", self.core_offset));
            cmds.push(format!("set mem_offset {}", self.mem_offset));
            cmds.push(format!("set power_limit {}", self.power_limit_w));
            if self.fan_manual {
                cmds.push(format!("set fan {}", self.fan_pct));
            } else {
                cmds.push("set fan auto".to_string());
            }
            if self.lock_enabled {
                cmds.push(format!("set lock {}", self.lock_mhz));
            } else {
                cmds.push("set unlock".to_string());
            }
        }
        if info.rapl {
            cmds.push(format!("set pl1 {}", self.pl1_w));
            cmds.push(format!("set pl2 {}", self.pl2_w));
        }
        if !self.governor.is_empty() {
            cmds.push(format!("set governor {}", self.governor));
        }
        if !self.epp.is_empty() {
            cmds.push(format!("set epp {}", self.epp));
        }
        cmds.push(format!("set turbo {}", self.turbo as u8));
        cmds.push(format!("set max_freq_pct {}", self.max_freq_pct));
        cmds
    }

    pub fn to_profile(&self, stable: bool) -> Profile {
        Profile {
            core_offset: self.core_offset,
            mem_offset: self.mem_offset,
            power_limit_w: self.power_limit_w,
            fan_manual: self.fan_manual,
            fan_pct: self.fan_pct,
            lock_enabled: self.lock_enabled,
            lock_mhz: self.lock_mhz,
            pl1_w: self.pl1_w,
            pl2_w: self.pl2_w,
            governor: self.governor.clone(),
            epp: self.epp.clone(),
            turbo: self.turbo as i8,
            max_freq_pct: self.max_freq_pct,
            stable,
        }
    }
}

fn pl_ceiling(info: &Info) -> u32 {
    info.pl1_w.max(info.pl2_w).max(125) + 60
}

/// Fan and lock sliders are only live when their toggle is on.
pub fn slider_editable(knob: Knob, p: &Pending) -> bool {
    match knob {
        Knob::FanPct => p.fan_manual,
        Knob::LockMhz => p.lock_enabled,
        _ => true,
    }
}
