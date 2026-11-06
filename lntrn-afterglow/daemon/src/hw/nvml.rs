//! NVIDIA's management library, spoken to directly and opened at run
//! time, so a machine without the driver simply has no card here.
//! Readings work for anyone; every setter needs root, which is why only
//! the helper calls them. Only the first card is tuned.
//!
//! Clock offsets go through the driver's newer `ClockOffsets` calls
//! (555 on), falling back to the older `ClkVfOffset` ones.

use std::ffi::{CStr, c_char, c_int, c_uint, c_ulonglong, c_void};

use lntrn_sys::plugin::Library;

use crate::tune::GpuLimits;

type Status = c_uint;
type Device = *mut c_void;

const SUCCESS: Status = 0;
const TEMPERATURE_GPU: c_uint = 0;
const THRESHOLD_SLOWDOWN: c_uint = 1;
const CLOCK_GRAPHICS: c_uint = 0;
const CLOCK_MEMORY: c_uint = 2;
const PSTATE_0: c_uint = 0;
/// `nvmlPstates_t`'s "unknown".
const PSTATE_UNKNOWN: c_uint = 32;
const NAME_LEN: usize = 96;
/// Room for every clock a card lists: a 3080 Ti has about a hundred.
const CLOCKS_LEN: usize = 512;

/// Why the card is running slower than it could, as the driver's bits
/// say it (`nvmlClocksEventReason*`).
pub mod reason {
    pub const IDLE: u64 = 0x1;
    pub const APP_CLOCKS: u64 = 0x2;
    pub const POWER_CAP: u64 = 0x4;
    pub const HW_SLOWDOWN: u64 = 0x8;
    pub const SYNC_BOOST: u64 = 0x10;
    pub const SW_THERMAL: u64 = 0x20;
    pub const HW_THERMAL: u64 = 0x40;
    pub const POWER_BRAKE: u64 = 0x80;
    pub const DISPLAY_CLOCK: u64 = 0x100;
}

/// `nvmlClockOffset_v1_t`.
#[repr(C)]
struct ClockOffset {
    version: c_uint,
    clock_type: c_uint,
    pstate: c_uint,
    offset_mhz: c_int,
    min_offset_mhz: c_int,
    max_offset_mhz: c_int,
}

impl ClockOffset {
    fn new(clock_type: c_uint, offset_mhz: c_int) -> ClockOffset {
        // NVML's struct versions: the size, and the version in the top byte.
        let version = size_of::<ClockOffset>() as c_uint | (1 << 24);
        ClockOffset { version, clock_type, pstate: PSTATE_0, offset_mhz, min_offset_mhz: 0, max_offset_mhz: 0 }
    }
}

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct MemoryInfo {
    total: u64,
    free: u64,
    used: u64,
}

type FnVoid = unsafe extern "C" fn() -> Status;
type FnErrorString = unsafe extern "C" fn(Status) -> *const c_char;
type FnText = unsafe extern "C" fn(*mut c_char, c_uint) -> Status;
type FnCount = unsafe extern "C" fn(*mut c_uint) -> Status;
type FnHandle = unsafe extern "C" fn(c_uint, *mut Device) -> Status;
type FnDev = unsafe extern "C" fn(Device) -> Status;
type FnDevText = unsafe extern "C" fn(Device, *mut c_char, c_uint) -> Status;
type FnDevU32 = unsafe extern "C" fn(Device, *mut c_uint) -> Status;
type FnDevU64 = unsafe extern "C" fn(Device, *mut c_ulonglong) -> Status;
type FnDevPair = unsafe extern "C" fn(Device, *mut c_uint, *mut c_uint) -> Status;
type FnDevIndexed = unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> Status;
type FnDevSet = unsafe extern "C" fn(Device, c_uint) -> Status;
type FnDevSetTwo = unsafe extern "C" fn(Device, c_uint, c_uint) -> Status;
type FnDevList = unsafe extern "C" fn(Device, *mut c_uint, *mut c_uint) -> Status;
type FnDevIndexedList = unsafe extern "C" fn(Device, c_uint, *mut c_uint, *mut c_uint) -> Status;
type FnDevUtilization = unsafe extern "C" fn(Device, *mut Utilization) -> Status;
type FnDevMemory = unsafe extern "C" fn(Device, *mut MemoryInfo) -> Status;
type FnDevOffset = unsafe extern "C" fn(Device, *mut ClockOffset) -> Status;
type FnDevI32 = unsafe extern "C" fn(Device, *mut c_int) -> Status;
type FnDevSetI32 = unsafe extern "C" fn(Device, c_int) -> Status;
type FnDevI32Pair = unsafe extern "C" fn(Device, *mut c_int, *mut c_int) -> Status;

/// Which clock an offset moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    Core,
    Memory,
}

/// What the card reads right now. Whatever the driver won't say is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub celsius: Option<u32>,
    pub watts: Option<f32>,
    /// The power limit in force, watts.
    pub watts_limit: Option<f32>,
    pub core_mhz: Option<u32>,
    pub memory_mhz: Option<u32>,
    /// How busy its cores are, percent.
    pub usage: Option<u32>,
    /// Video memory used and in all, bytes.
    pub memory: Option<(u64, u64)>,
    /// Each fan's speed, percent of its fastest.
    pub fans: Vec<u32>,
    /// Why it is holding back: the [`reason`] bits.
    pub reasons: Option<u64>,
    /// Its performance state: 0 is flat out, 8 and up asleep.
    pub pstate: Option<u32>,
}

/// The driver's functions, each `None` when this driver lacks it.
struct Calls {
    shutdown: FnVoid,
    error_string: FnErrorString,
    temperature: Option<FnDevIndexed>,
    threshold: Option<FnDevIndexed>,
    power: Option<FnDevU32>,
    power_enforced: Option<FnDevU32>,
    power_limit: Option<FnDevU32>,
    power_default: Option<FnDevU32>,
    power_range: Option<FnDevPair>,
    set_power_limit: Option<FnDevSet>,
    clock: Option<FnDevIndexed>,
    utilization: Option<FnDevUtilization>,
    memory: Option<FnDevMemory>,
    pstate: Option<FnDevU32>,
    reasons: Option<FnDevU64>,
    fan_speed: Option<FnDevIndexed>,
    fan_range: Option<FnDevPair>,
    set_fan: Option<FnDevSetTwo>,
    set_fan_auto: Option<FnDevSet>,
    memory_clocks: Option<FnDevList>,
    graphics_clocks: Option<FnDevIndexedList>,
    lock: Option<FnDevSetTwo>,
    unlock: Option<FnDev>,
    get_offset: Option<FnDevOffset>,
    set_offset: Option<FnDevOffset>,
    old_core_offset: Option<FnDevI32>,
    old_set_core_offset: Option<FnDevSetI32>,
    old_core_range: Option<FnDevI32Pair>,
    old_memory_offset: Option<FnDevI32>,
    old_set_memory_offset: Option<FnDevSetI32>,
    old_memory_range: Option<FnDevI32Pair>,
}

pub struct Nvml {
    device: Device,
    name: String,
    driver: String,
    fans: u32,
    /// The lowest and highest core clocks it lists, MHz.
    clocks: Option<(u32, u32)>,
    calls: Calls,
    // Last: every function above points into it.
    _lib: Library,
}

// SAFETY: NVML is documented thread-safe, and the device handle is a
// token it gave out, not memory of ours.
unsafe impl Send for Nvml {}

fn text(buf: &[u8]) -> String {
    CStr::from_bytes_until_nul(buf).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Nvml {
    /// The library, started, with its first card. An error says why
    /// there is none: no driver, or a driver that won't talk.
    pub fn open() -> Result<Nvml, String> {
        let lib = Library::open("libnvidia-ml.so.1").map_err(|_| "no NVIDIA driver".to_owned())?;
        // SAFETY: each symbol is given the signature NVML's header
        // declares for it, and none outlives `lib`, which the struct keeps
        // for as long as it has them. Every out-pointer is to a local of
        // the type the call fills in.
        unsafe {
            let need = |name: &str| format!("the NVIDIA driver has no {name}");
            let init: FnVoid = lib.symbol("nvmlInit_v2").ok_or_else(|| need("nvmlInit_v2"))?;
            let count: FnCount = lib.symbol("nvmlDeviceGetCount_v2").ok_or_else(|| need("nvmlDeviceGetCount_v2"))?;
            let handle: FnHandle = lib.symbol("nvmlDeviceGetHandleByIndex_v2").ok_or_else(|| need("nvmlDeviceGetHandleByIndex_v2"))?;
            let calls = Calls {
                shutdown: lib.symbol("nvmlShutdown").ok_or_else(|| need("nvmlShutdown"))?,
                error_string: lib.symbol("nvmlErrorString").ok_or_else(|| need("nvmlErrorString"))?,
                temperature: lib.symbol("nvmlDeviceGetTemperature"),
                threshold: lib.symbol("nvmlDeviceGetTemperatureThreshold"),
                power: lib.symbol("nvmlDeviceGetPowerUsage"),
                power_enforced: lib.symbol("nvmlDeviceGetEnforcedPowerLimit"),
                power_limit: lib.symbol("nvmlDeviceGetPowerManagementLimit"),
                power_default: lib.symbol("nvmlDeviceGetPowerManagementDefaultLimit"),
                power_range: lib.symbol("nvmlDeviceGetPowerManagementLimitConstraints"),
                set_power_limit: lib.symbol("nvmlDeviceSetPowerManagementLimit"),
                clock: lib.symbol("nvmlDeviceGetClockInfo"),
                utilization: lib.symbol("nvmlDeviceGetUtilizationRates"),
                memory: lib.symbol("nvmlDeviceGetMemoryInfo"),
                pstate: lib.symbol("nvmlDeviceGetPerformanceState"),
                reasons: lib.symbol("nvmlDeviceGetCurrentClocksEventReasons").or_else(|| lib.symbol("nvmlDeviceGetCurrentClocksThrottleReasons")),
                fan_speed: lib.symbol("nvmlDeviceGetFanSpeed_v2"),
                fan_range: lib.symbol("nvmlDeviceGetMinMaxFanSpeed"),
                set_fan: lib.symbol("nvmlDeviceSetFanSpeed_v2"),
                set_fan_auto: lib.symbol("nvmlDeviceSetDefaultFanSpeed_v2"),
                memory_clocks: lib.symbol("nvmlDeviceGetSupportedMemoryClocks"),
                graphics_clocks: lib.symbol("nvmlDeviceGetSupportedGraphicsClocks"),
                lock: lib.symbol("nvmlDeviceSetGpuLockedClocks"),
                unlock: lib.symbol("nvmlDeviceResetGpuLockedClocks"),
                get_offset: lib.symbol("nvmlDeviceGetClockOffsets"),
                set_offset: lib.symbol("nvmlDeviceSetClockOffsets"),
                old_core_offset: lib.symbol("nvmlDeviceGetGpcClkVfOffset"),
                old_set_core_offset: lib.symbol("nvmlDeviceSetGpcClkVfOffset"),
                old_core_range: lib.symbol("nvmlDeviceGetGpcClkMinMaxVfOffset"),
                old_memory_offset: lib.symbol("nvmlDeviceGetMemClkVfOffset"),
                old_set_memory_offset: lib.symbol("nvmlDeviceSetMemClkVfOffset"),
                old_memory_range: lib.symbol("nvmlDeviceGetMemClkMinMaxVfOffset"),
            };
            let started = init();
            if started != SUCCESS {
                return Err(format!("the NVIDIA driver won't start: {}", said(calls.error_string, started)));
            }
            let mut n: c_uint = 0;
            let mut device: Device = std::ptr::null_mut();
            if count(&mut n) != SUCCESS || n == 0 || handle(0, &mut device) != SUCCESS {
                (calls.shutdown)();
                return Err("the NVIDIA driver lists no card".to_owned());
            }
            let mut buf = [0u8; NAME_LEN];
            let name_of: Option<FnDevText> = lib.symbol("nvmlDeviceGetName");
            let name = match name_of {
                Some(f) if f(device, buf.as_mut_ptr().cast(), NAME_LEN as c_uint) == SUCCESS => text(&buf),
                _ => "NVIDIA graphics".to_owned(),
            };
            let mut buf = [0u8; NAME_LEN];
            let version: Option<FnText> = lib.symbol("nvmlSystemGetDriverVersion");
            let driver = match version {
                Some(f) if f(buf.as_mut_ptr().cast(), NAME_LEN as c_uint) == SUCCESS => text(&buf),
                _ => String::new(),
            };
            let mut fans: c_uint = 0;
            let fan_count: Option<FnDevU32> = lib.symbol("nvmlDeviceGetNumFans");
            if fan_count.is_none_or(|f| f(device, &mut fans) != SUCCESS) {
                fans = 0;
            }
            let mut nvml = Nvml { device, name, driver, fans: fans.min(8), clocks: None, calls, _lib: lib };
            nvml.clocks = nvml.listed_clocks();
            Ok(nvml)
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, status: Status, what: &str) -> Result<(), String> {
        if status == SUCCESS { Ok(()) } else { Err(format!("{what}: {}", said(self.calls.error_string, status))) }
    }

    /// One number from a call that fills in one.
    fn number(&self, f: Option<FnDevU32>) -> Option<u32> {
        let mut v: c_uint = 0;
        // SAFETY: the handle is NVML's and `v` is what the call fills in.
        f.filter(|f| unsafe { f(self.device, &mut v) } == SUCCESS).map(|_| v)
    }

    fn indexed(&self, f: Option<FnDevIndexed>, which: c_uint) -> Option<u32> {
        let mut v: c_uint = 0;
        // SAFETY: as in `number`.
        f.filter(|f| unsafe { f(self.device, which, &mut v) } == SUCCESS).map(|_| v)
    }

    fn two(&self, f: Option<FnDevPair>) -> Option<(u32, u32)> {
        let (mut a, mut b): (c_uint, c_uint) = (0, 0);
        // SAFETY: as in `number`.
        f.filter(|f| unsafe { f(self.device, &mut a, &mut b) } == SUCCESS).map(|_| (a, b))
    }

    /// The core clocks the card lists at its fastest memory clock: the
    /// range it can be held to.
    fn listed_clocks(&self) -> Option<(u32, u32)> {
        let (memory, graphics) = (self.calls.memory_clocks?, self.calls.graphics_clocks?);
        let mut buf = [0 as c_uint; CLOCKS_LEN];
        let mut n = CLOCKS_LEN as c_uint;
        // SAFETY: `buf` holds `n` entries, as the call is told.
        if unsafe { memory(self.device, &mut n, buf.as_mut_ptr()) } != SUCCESS {
            return None;
        }
        let top = *buf[..(n as usize).min(CLOCKS_LEN)].iter().max()?;
        let mut n = CLOCKS_LEN as c_uint;
        // SAFETY: as above.
        if unsafe { graphics(self.device, top, &mut n, buf.as_mut_ptr()) } != SUCCESS {
            return None;
        }
        let listed = &buf[..(n as usize).min(CLOCKS_LEN)];
        Some((*listed.iter().filter(|c| **c > 0).min()?, *listed.iter().max()?))
    }

    pub fn celsius(&self) -> Option<u32> {
        self.indexed(self.calls.temperature, TEMPERATURE_GPU)
    }

    /// What the card reads now.
    pub fn read(&self) -> Reading {
        let c = &self.calls;
        let mut load = Utilization::default();
        let mut mem = MemoryInfo::default();
        let mut reasons: c_ulonglong = 0;
        // SAFETY: the handle is NVML's and each out-pointer is to a local
        // of the type its call fills in.
        unsafe {
            Reading {
                celsius: self.celsius(),
                watts: self.number(c.power).map(|mw| mw as f32 / 1000.0),
                watts_limit: self.number(c.power_enforced).or_else(|| self.number(c.power_limit)).map(|mw| mw as f32 / 1000.0).filter(|w| *w > 0.0),
                core_mhz: self.indexed(c.clock, CLOCK_GRAPHICS),
                memory_mhz: self.indexed(c.clock, CLOCK_MEMORY),
                usage: c.utilization.filter(|f| f(self.device, &mut load) == SUCCESS).map(|_| load.gpu.min(100)),
                memory: c.memory.filter(|f| f(self.device, &mut mem) == SUCCESS && mem.total > 0).map(|_| (mem.used, mem.total)),
                fans: (0..self.fans).filter_map(|fan| self.indexed(c.fan_speed, fan)).collect(),
                reasons: c.reasons.filter(|f| f(self.device, &mut reasons) == SUCCESS).map(|_| reasons),
                pstate: self.number(c.pstate).filter(|p| *p < PSTATE_UNKNOWN),
            }
        }
    }

    /// The current offset on `clock`, and the range it may be set in.
    fn offset(&self, clock: Clock) -> Option<(i32, (i32, i32))> {
        let c = &self.calls;
        if let Some(get) = c.get_offset {
            let mut o = ClockOffset::new(if clock == Clock::Core { CLOCK_GRAPHICS } else { CLOCK_MEMORY }, 0);
            // SAFETY: a versioned struct of the size its version says.
            if unsafe { get(self.device, &mut o) } == SUCCESS {
                return Some((o.offset_mhz, (o.min_offset_mhz, o.max_offset_mhz)));
            }
        }
        let (get, range) = match clock {
            Clock::Core => (c.old_core_offset?, c.old_core_range?),
            Clock::Memory => (c.old_memory_offset?, c.old_memory_range?),
        };
        let (mut now, mut lo, mut hi): (c_int, c_int, c_int) = (0, 0, 0);
        // SAFETY: the handle is NVML's; the pointers are to locals.
        unsafe { (get(self.device, &mut now) == SUCCESS && range(self.device, &mut lo, &mut hi) == SUCCESS).then_some((now, (lo, hi))) }
    }

    /// The offsets in force now, core and memory, MHz.
    pub fn offsets(&self) -> (i32, i32) {
        (self.offset(Clock::Core).map_or(0, |o| o.0), self.offset(Clock::Memory).map_or(0, |o| o.0))
    }

    /// The power limit in force now, watts.
    pub fn power_limit(&self) -> Option<u32> {
        self.number(self.calls.power_limit).map(|mw| (mw + 500) / 1000)
    }

    /// How far every knob of this card goes.
    pub fn limits(&self) -> GpuLimits {
        let c = &self.calls;
        let watts = |mw: u32| (mw + 500) / 1000;
        let (lo, hi) = self.two(c.power_range).unwrap_or((0, 0));
        let power_default = self.number(c.power_default).map_or(0, watts);
        GpuLimits {
            name: self.name.clone(),
            driver: self.driver.clone(),
            core_offset: self.offset(Clock::Core).map_or((0, 0), |o| o.1),
            memory_offset: self.offset(Clock::Memory).map_or((0, 0), |o| o.1),
            power: if hi > 0 { (watts(lo), watts(hi)) } else { (power_default, power_default) },
            power_default,
            lock: self.clocks.filter(|_| c.lock.is_some() && c.unlock.is_some()),
            fans: self.fans,
            fan_range: (self.fans > 0 && c.set_fan.is_some() && c.set_fan_auto.is_some()).then(|| self.two(c.fan_range).filter(|(lo, hi)| lo < hi && *hi <= 100).unwrap_or((30, 100))),
            slowdown: self.indexed(c.threshold, THRESHOLD_SLOWDOWN).filter(|t| (60..=120).contains(t)).unwrap_or(90),
        }
    }

    pub fn set_offset(&self, clock: Clock, mhz: i32) -> Result<(), String> {
        let c = &self.calls;
        if let Some(set) = c.set_offset {
            let mut o = ClockOffset::new(if clock == Clock::Core { CLOCK_GRAPHICS } else { CLOCK_MEMORY }, mhz);
            // SAFETY: a versioned struct of the size its version says.
            return self.check(unsafe { set(self.device, &mut o) }, "setting a clock offset");
        }
        let set = match clock {
            Clock::Core => c.old_set_core_offset,
            Clock::Memory => c.old_set_memory_offset,
        };
        let set = set.ok_or("this driver can't offset clocks")?;
        // SAFETY: the handle is NVML's.
        self.check(unsafe { set(self.device, mhz) }, "setting a clock offset")
    }

    pub fn set_power_limit(&self, watts: u32) -> Result<(), String> {
        let set = self.calls.set_power_limit.ok_or("this driver can't set a power limit")?;
        // SAFETY: the handle is NVML's.
        self.check(unsafe { set(self.device, watts * 1000) }, "setting the power limit")
    }

    /// Every fan at `percent`, or back on the card's own curve (`None`).
    pub fn set_fans(&self, percent: Option<u32>) -> Result<(), String> {
        let c = &self.calls;
        for fan in 0..self.fans {
            // SAFETY: the handle is NVML's and `fan` one of its fans.
            let status = unsafe {
                match percent {
                    Some(p) => c.set_fan.ok_or("this card's fans can't be set")?(self.device, fan, p.min(100)),
                    None => c.set_fan_auto.ok_or("this card's fans can't be set")?(self.device, fan),
                }
            };
            self.check(status, "setting the fans")?;
        }
        Ok(())
    }

    /// Hold the core at or below `max` MHz (from the lowest clock it
    /// lists, so it still rests when idle), or let it boost freely.
    pub fn set_lock(&self, max: Option<u32>) -> Result<(), String> {
        let c = &self.calls;
        // SAFETY: the handle is NVML's.
        let status = unsafe {
            match max {
                Some(mhz) => c.lock.ok_or("this driver can't hold the core clock")?(self.device, self.clocks.map_or(0, |r| r.0).min(mhz), mhz),
                None => c.unlock.ok_or("this driver can't hold the core clock")?(self.device),
            }
        };
        self.check(status, "holding the core clock")
    }
}

/// What NVML calls `status`.
fn said(error_string: FnErrorString, status: Status) -> String {
    // SAFETY: NVML returns a static string, or null.
    let p = unsafe { error_string(status) };
    if p.is_null() {
        return format!("error {status}");
    }
    // SAFETY: a nul-terminated static string from the driver.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: started in `open`; the library is still loaded.
        unsafe { (self.calls.shutdown)() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_ends_at_its_nul_and_whatever_card_is_here_reads_sanely() {
        assert_eq!(text(b"GeForce RTX\0junk"), "GeForce RTX");
        assert_eq!(text(b"no end"), "");
        // On a machine without the driver this has nothing to say, and
        // says so rather than failing.
        let Ok(nvml) = Nvml::open() else { return };
        let l = nvml.limits();
        assert!(!l.name.is_empty());
        assert!(l.power.0 <= l.power.1, "{:?}", l.power);
        assert!(l.core_offset.0 <= 0 && l.core_offset.1 >= 0, "{:?}", l.core_offset);
        if let Some((lo, hi)) = l.lock {
            assert!(lo < hi, "{lo} {hi}");
        }
        let r = nvml.read();
        assert!(r.usage.is_none_or(|u| u <= 100));
        assert!(r.celsius.is_none_or(|c| c < 130));
    }
}
