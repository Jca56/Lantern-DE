//! NVIDIA's management library, spoken to directly: the proprietary
//! driver keeps a card's load and memory out of sysfs, and this is where
//! it says them. Opened at run time, so a machine without the driver
//! simply has no cards here. Only readings are asked for: nothing in
//! this file can change a card.

use std::ffi::{CStr, c_char, c_uint, c_void};

use lntrn_sys::plugin::Library;

type Status = c_uint;
type Device = *mut c_void;

const SUCCESS: Status = 0;
const TEMPERATURE_GPU: c_uint = 0;
const CLOCK_GRAPHICS: c_uint = 0;
const CLOCK_MEMORY: c_uint = 2;
const NAME_LEN: usize = 96;

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
type FnCount = unsafe extern "C" fn(*mut c_uint) -> Status;
type FnText = unsafe extern "C" fn(*mut c_char, c_uint) -> Status;
type FnHandle = unsafe extern "C" fn(c_uint, *mut Device) -> Status;
type FnDevText = unsafe extern "C" fn(Device, *mut c_char, c_uint) -> Status;
type FnDevNumber = unsafe extern "C" fn(Device, *mut c_uint) -> Status;
type FnDevIndexed = unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> Status;
type FnDevUtilization = unsafe extern "C" fn(Device, *mut Utilization) -> Status;
type FnDevMemory = unsafe extern "C" fn(Device, *mut MemoryInfo) -> Status;

/// What one card reads right now. Whatever it won't say is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub usage: Option<f32>,
    pub memory: Option<(u64, u64)>,
    pub celsius: Option<f32>,
    pub watts: Option<f32>,
    pub watts_limit: Option<f32>,
    /// Each fan's speed, percent of its fastest.
    pub fans: Vec<u32>,
    pub core_mhz: Option<u32>,
    pub memory_mhz: Option<u32>,
}

struct Card {
    device: Device,
    name: String,
    fans: u32,
}

pub struct Nvml {
    cards: Vec<Card>,
    driver: String,
    shutdown: FnVoid,
    utilization: FnDevUtilization,
    memory: FnDevMemory,
    temperature: FnDevIndexed,
    power: Option<FnDevNumber>,
    power_limit: Option<FnDevNumber>,
    fan_speed: Option<FnDevIndexed>,
    clock: Option<FnDevIndexed>,
    // Last: the functions above point into it.
    _lib: Library,
}

// SAFETY: NVML is documented thread-safe, and the device handles are
// tokens it gave out, not memory of ours.
unsafe impl Send for Nvml {}

fn text(buf: &[u8]) -> String {
    CStr::from_bytes_until_nul(buf).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Nvml {
    /// The library, started, with every card it lists. `None` when the
    /// machine has no NVIDIA driver, or the driver won't talk.
    pub fn open() -> Option<Nvml> {
        let lib = Library::open("libnvidia-ml.so.1").ok()?;
        // SAFETY: each symbol is given the signature NVML's header
        // declares for it, and none outlives `lib`, which the struct
        // keeps for as long as it has them.
        unsafe {
            let init: FnVoid = lib.symbol("nvmlInit_v2")?;
            let shutdown: FnVoid = lib.symbol("nvmlShutdown")?;
            let count: FnCount = lib.symbol("nvmlDeviceGetCount_v2")?;
            let handle: FnHandle = lib.symbol("nvmlDeviceGetHandleByIndex_v2")?;
            let name: FnDevText = lib.symbol("nvmlDeviceGetName")?;
            let utilization: FnDevUtilization = lib.symbol("nvmlDeviceGetUtilizationRates")?;
            let memory: FnDevMemory = lib.symbol("nvmlDeviceGetMemoryInfo")?;
            let temperature: FnDevIndexed = lib.symbol("nvmlDeviceGetTemperature")?;
            let driver_version: Option<FnText> = lib.symbol("nvmlSystemGetDriverVersion");
            let fan_count: Option<FnDevNumber> = lib.symbol("nvmlDeviceGetNumFans");
            // The kernel module isn't loaded, or there is no card.
            if init() != SUCCESS {
                return None;
            }
            let mut n: c_uint = 0;
            if count(&mut n) != SUCCESS {
                n = 0;
            }
            let mut cards = Vec::new();
            for i in 0..n {
                let mut device: Device = std::ptr::null_mut();
                if handle(i, &mut device) != SUCCESS {
                    continue;
                }
                let mut buf = [0u8; NAME_LEN];
                let named = name(device, buf.as_mut_ptr().cast(), NAME_LEN as c_uint) == SUCCESS;
                let mut fans: c_uint = 0;
                if fan_count.is_none_or(|f| f(device, &mut fans) != SUCCESS) {
                    fans = 0;
                }
                cards.push(Card { device, name: if named { text(&buf) } else { "NVIDIA graphics".to_owned() }, fans: fans.min(8) });
            }
            if cards.is_empty() {
                shutdown();
                return None;
            }
            let mut buf = [0u8; NAME_LEN];
            let driver = driver_version.filter(|f| f(buf.as_mut_ptr().cast(), NAME_LEN as c_uint) == SUCCESS).map(|_| text(&buf)).unwrap_or_default();
            Some(Nvml {
                cards,
                driver,
                shutdown,
                utilization,
                memory,
                temperature,
                power: lib.symbol("nvmlDeviceGetPowerUsage"),
                power_limit: lib.symbol("nvmlDeviceGetPowerManagementLimit"),
                fan_speed: lib.symbol("nvmlDeviceGetFanSpeed_v2"),
                clock: lib.symbol("nvmlDeviceGetClockInfo"),
                _lib: lib,
            })
        }
    }

    /// The driver's version: "615.71.09".
    pub fn driver(&self) -> &str {
        &self.driver
    }

    /// Each card's name, in the order [`Self::read`] counts them.
    pub fn names(&self) -> Vec<String> {
        self.cards.iter().map(|c| c.name.clone()).collect()
    }

    /// What card `index` reads now.
    pub fn read(&self, index: usize) -> Reading {
        let Some(card) = self.cards.get(index) else { return Reading::default() };
        let d = card.device;
        // SAFETY: the handle is one NVML gave out and every out-pointer
        // is to a local of the type the call fills in.
        unsafe {
            let number = |f: Option<FnDevNumber>| {
                let mut v: c_uint = 0;
                f.filter(|f| f(d, &mut v) == SUCCESS).map(|_| v)
            };
            let indexed = |f: Option<FnDevIndexed>, which: c_uint| {
                let mut v: c_uint = 0;
                f.filter(|f| f(d, which, &mut v) == SUCCESS).map(|_| v)
            };
            let mut load = Utilization::default();
            let mut mem = MemoryInfo::default();
            Reading {
                usage: ((self.utilization)(d, &mut load) == SUCCESS).then_some(load.gpu.min(100) as f32),
                memory: ((self.memory)(d, &mut mem) == SUCCESS && mem.total > 0).then_some((mem.used, mem.total)),
                celsius: indexed(Some(self.temperature), TEMPERATURE_GPU).map(|c| c as f32),
                // Milliwatts.
                watts: number(self.power).map(|mw| mw as f32 / 1000.0),
                watts_limit: number(self.power_limit).map(|mw| mw as f32 / 1000.0).filter(|w| *w > 0.0),
                fans: (0..card.fans).filter_map(|fan| indexed(self.fan_speed, fan)).collect(),
                core_mhz: indexed(self.clock, CLOCK_GRAPHICS).filter(|m| *m > 0),
                memory_mhz: indexed(self.clock, CLOCK_MEMORY).filter(|m| *m > 0),
            }
        }
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: started in `open`; the library is still loaded.
        unsafe { (self.shutdown)() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_ends_at_its_nul_and_a_missing_card_reads_as_nothing() {
        assert_eq!(text(b"GeForce RTX\0junk"), "GeForce RTX");
        assert_eq!(text(b"no end"), "");
        // Whatever this machine has: opening never panics, and a card
        // that isn't there has nothing to say.
        if let Some(nvml) = Nvml::open() {
            assert!(!nvml.names().is_empty());
            assert_eq!(nvml.read(99), Reading::default());
            let r = nvml.read(0);
            assert!(r.usage.is_none_or(|u| (0.0..=100.0).contains(&u)));
        }
    }
}
