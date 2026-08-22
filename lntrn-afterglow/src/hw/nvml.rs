//! Hand-rolled NVML FFI bindings — no external crates, per house rules.
//!
//! libnvidia-ml is dlopen'd at runtime so the same binary runs on the laptop
//! (no NVIDIA driver installed): `Nvml::load()` returns `Ok(None)` there and
//! callers hide the GPU features. Getters work unprivileged; setters need
//! root.
//!
//! Clock offsets use the driver 555+ `nvmlDeviceGet/SetClockOffsets` API with
//! the legacy `*ClkVfOffset` symbols as fallback for older drivers.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::mem;

type NvmlReturn = c_uint;
type Device = *mut c_void;

const SUCCESS: NvmlReturn = 0;
/// nvmlInit result when the kernel module isn't loaded (driverless machine).
const ERROR_DRIVER_NOT_LOADED: NvmlReturn = 9;

const TEMPERATURE_GPU: c_uint = 0;
pub const CLOCK_GRAPHICS: c_uint = 0;
pub const CLOCK_MEM: c_uint = 2;
const PSTATE_0: c_uint = 0;

const NAME_BUF: usize = 96;
const VERSION_BUF: usize = 80;

#[repr(C)]
struct ClockOffsetV1 {
    version: c_uint,
    clock_type: c_uint,
    pstate: c_uint,
    offset_mhz: c_int,
    min_offset_mhz: c_int,
    max_offset_mhz: c_int,
}

/// NVML struct-version convention: `sizeof | (version << 24)`.
fn offset_version() -> c_uint {
    mem::size_of::<ClockOffsetV1>() as c_uint | (1 << 24)
}

#[repr(C)]
struct Utilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
struct MemoryInfo {
    total: u64,
    free: u64,
    used: u64,
}

type FnVoid = unsafe extern "C" fn() -> NvmlReturn;
type FnErrStr = unsafe extern "C" fn(NvmlReturn) -> *const c_char;
type FnStrBuf = unsafe extern "C" fn(*mut c_char, c_uint) -> NvmlReturn;
type FnHandle = unsafe extern "C" fn(c_uint, *mut Device) -> NvmlReturn;
type FnDev = unsafe extern "C" fn(Device) -> NvmlReturn;
type FnDevStrBuf = unsafe extern "C" fn(Device, *mut c_char, c_uint) -> NvmlReturn;
type FnDevU32p = unsafe extern "C" fn(Device, *mut c_uint) -> NvmlReturn;
type FnDevU32 = unsafe extern "C" fn(Device, c_uint) -> NvmlReturn;
type FnDevU32U32 = unsafe extern "C" fn(Device, c_uint, c_uint) -> NvmlReturn;
type FnDevU32U32p = unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> NvmlReturn;
type FnDevU32pU32p = unsafe extern "C" fn(Device, *mut c_uint, *mut c_uint) -> NvmlReturn;
type FnDevUtil = unsafe extern "C" fn(Device, *mut Utilization) -> NvmlReturn;
type FnDevMem = unsafe extern "C" fn(Device, *mut MemoryInfo) -> NvmlReturn;
type FnDevOffset = unsafe extern "C" fn(Device, *mut ClockOffsetV1) -> NvmlReturn;
type FnDevI32p = unsafe extern "C" fn(Device, *mut c_int) -> NvmlReturn;
type FnDevI32 = unsafe extern "C" fn(Device, c_int) -> NvmlReturn;
type FnDevI32pI32p = unsafe extern "C" fn(Device, *mut c_int, *mut c_int) -> NvmlReturn;

unsafe fn sym<T>(lib: *mut c_void, name: &[u8]) -> Option<T> {
    debug_assert!(name.ends_with(&[0]));
    let p = libc::dlsym(lib, name.as_ptr().cast());
    if p.is_null() {
        None
    } else {
        Some(mem::transmute_copy(&p))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OffsetRange {
    pub current: i32,
    pub min: i32,
    pub max: i32,
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub driver: String,
    pub power_min_w: u32,
    pub power_max_w: u32,
    pub power_default_w: u32,
    pub core_offset: OffsetRange,
    pub mem_offset: OffsetRange,
    pub fan_count: u32,
}

#[derive(Debug, Clone)]
pub struct GpuSnapshot {
    pub temp_c: u32,
    pub power_w: f32,
    pub core_mhz: u32,
    pub mem_mhz: u32,
    pub util_pct: u32,
    pub vram_used_mb: u64,
    pub vram_total_mb: u64,
    pub fans_pct: Vec<u32>,
    pub power_limit_w: f32,
}

pub struct Nvml {
    dev: Device,
    err_str: FnErrStr,
    get_temperature: FnDevU32U32p,
    get_power_usage: FnDevU32p,
    get_clock_info: FnDevU32U32p,
    get_utilization: FnDevUtil,
    get_memory_info: FnDevMem,
    get_power_limit: FnDevU32p,
    get_power_default: FnDevU32p,
    get_power_constraints: FnDevU32pU32p,
    set_power_limit_fn: FnDevU32,
    get_fan_speed: FnDevU32U32p,
    set_fan_speed: Option<FnDevU32U32>,
    set_fan_default: Option<FnDevU32>,
    lock_gpu_clocks: Option<FnDevU32U32>,
    reset_gpu_clocks: Option<FnDev>,
    get_offsets: Option<FnDevOffset>,
    set_offsets: Option<FnDevOffset>,
    legacy_get_gpc: Option<FnDevI32p>,
    legacy_set_gpc: Option<FnDevI32>,
    legacy_gpc_range: Option<FnDevI32pI32p>,
    legacy_get_mem: Option<FnDevI32p>,
    legacy_set_mem: Option<FnDevI32>,
    legacy_mem_range: Option<FnDevI32pI32p>,
    name: String,
    driver: String,
    fan_count: u32,
}

// NVML is documented thread-safe; `dev` is an opaque handle, not owned memory.
unsafe impl Send for Nvml {}

impl Nvml {
    /// Returns `Ok(None)` on machines without an NVIDIA driver.
    pub fn load() -> Result<Option<Nvml>, String> {
        let lib = unsafe { libc::dlopen(b"libnvidia-ml.so.1\0".as_ptr().cast(), libc::RTLD_NOW) };
        if lib.is_null() {
            return Ok(None);
        }
        unsafe { Self::from_lib(lib) }
    }

    unsafe fn from_lib(lib: *mut c_void) -> Result<Option<Nvml>, String> {
        macro_rules! req {
            ($name:literal) => {
                match sym(lib, concat!($name, "\0").as_bytes()) {
                    Some(f) => f,
                    None => return Err(concat!("NVML symbol missing: ", $name).to_string()),
                }
            };
        }
        macro_rules! opt {
            ($name:literal) => {
                sym(lib, concat!($name, "\0").as_bytes())
            };
        }

        let init: FnVoid = req!("nvmlInit_v2");
        let err_str: FnErrStr = req!("nvmlErrorString");
        let ret = init();
        if ret == ERROR_DRIVER_NOT_LOADED {
            return Ok(None);
        }
        check(err_str, ret, "nvmlInit")?;

        let get_handle: FnHandle = req!("nvmlDeviceGetHandleByIndex_v2");
        let mut dev: Device = std::ptr::null_mut();
        check(err_str, get_handle(0, &mut dev), "GetHandleByIndex")?;

        let get_name: FnDevStrBuf = req!("nvmlDeviceGetName");
        let mut buf = [0u8; NAME_BUF];
        check(
            err_str,
            get_name(dev, buf.as_mut_ptr().cast(), NAME_BUF as c_uint),
            "GetName",
        )?;
        let name = cstr_buf(&buf);

        let get_driver: FnStrBuf = req!("nvmlSystemGetDriverVersion");
        let mut vbuf = [0u8; VERSION_BUF];
        check(
            err_str,
            get_driver(vbuf.as_mut_ptr().cast(), VERSION_BUF as c_uint),
            "GetDriverVersion",
        )?;
        let driver = cstr_buf(&vbuf);

        let num_fans: Option<FnDevU32p> = opt!("nvmlDeviceGetNumFans");
        let mut fan_count: c_uint = 1;
        if let Some(f) = num_fans {
            if f(dev, &mut fan_count) != SUCCESS {
                fan_count = 1;
            }
        }

        Ok(Some(Nvml {
            dev,
            err_str,
            get_temperature: req!("nvmlDeviceGetTemperature"),
            get_power_usage: req!("nvmlDeviceGetPowerUsage"),
            get_clock_info: req!("nvmlDeviceGetClockInfo"),
            get_utilization: req!("nvmlDeviceGetUtilizationRates"),
            get_memory_info: req!("nvmlDeviceGetMemoryInfo"),
            get_power_limit: req!("nvmlDeviceGetPowerManagementLimit"),
            get_power_default: req!("nvmlDeviceGetPowerManagementDefaultLimit"),
            get_power_constraints: req!("nvmlDeviceGetPowerManagementLimitConstraints"),
            set_power_limit_fn: req!("nvmlDeviceSetPowerManagementLimit"),
            get_fan_speed: req!("nvmlDeviceGetFanSpeed_v2"),
            set_fan_speed: opt!("nvmlDeviceSetFanSpeed_v2"),
            set_fan_default: opt!("nvmlDeviceSetDefaultFanSpeed_v2"),
            lock_gpu_clocks: opt!("nvmlDeviceSetGpuLockedClocks"),
            reset_gpu_clocks: opt!("nvmlDeviceResetGpuLockedClocks"),
            get_offsets: opt!("nvmlDeviceGetClockOffsets"),
            set_offsets: opt!("nvmlDeviceSetClockOffsets"),
            legacy_get_gpc: opt!("nvmlDeviceGetGpcClkVfOffset"),
            legacy_set_gpc: opt!("nvmlDeviceSetGpcClkVfOffset"),
            legacy_gpc_range: opt!("nvmlDeviceGetGpcClkMinMaxVfOffset"),
            legacy_get_mem: opt!("nvmlDeviceGetMemClkVfOffset"),
            legacy_set_mem: opt!("nvmlDeviceSetMemClkVfOffset"),
            legacy_mem_range: opt!("nvmlDeviceGetMemClkMinMaxVfOffset"),
            name,
            driver,
            fan_count,
        }))
    }

    fn check(&self, ret: NvmlReturn, what: &str) -> Result<(), String> {
        check(self.err_str, ret, what)
    }

    pub fn info(&self) -> Result<GpuInfo, String> {
        let (mut min_mw, mut max_mw, mut def_mw) = (0u32, 0u32, 0u32);
        self.check(
            unsafe { (self.get_power_constraints)(self.dev, &mut min_mw, &mut max_mw) },
            "GetPowerConstraints",
        )?;
        self.check(
            unsafe { (self.get_power_default)(self.dev, &mut def_mw) },
            "GetPowerDefaultLimit",
        )?;
        Ok(GpuInfo {
            name: self.name.clone(),
            driver: self.driver.clone(),
            power_min_w: min_mw / 1000,
            power_max_w: max_mw / 1000,
            power_default_w: def_mw / 1000,
            core_offset: self.clock_offset(CLOCK_GRAPHICS)?,
            mem_offset: self.clock_offset(CLOCK_MEM)?,
            fan_count: self.fan_count,
        })
    }

    pub fn snapshot(&self) -> Result<GpuSnapshot, String> {
        let mut temp: c_uint = 0;
        self.check(
            unsafe { (self.get_temperature)(self.dev, TEMPERATURE_GPU, &mut temp) },
            "GetTemperature",
        )?;
        let mut power_mw: c_uint = 0;
        self.check(
            unsafe { (self.get_power_usage)(self.dev, &mut power_mw) },
            "GetPowerUsage",
        )?;
        let mut core: c_uint = 0;
        let mut memc: c_uint = 0;
        self.check(
            unsafe { (self.get_clock_info)(self.dev, CLOCK_GRAPHICS, &mut core) },
            "GetClockInfo(core)",
        )?;
        self.check(
            unsafe { (self.get_clock_info)(self.dev, CLOCK_MEM, &mut memc) },
            "GetClockInfo(mem)",
        )?;
        let mut util = Utilization { gpu: 0, memory: 0 };
        self.check(
            unsafe { (self.get_utilization)(self.dev, &mut util) },
            "GetUtilization",
        )?;
        let mut mem = MemoryInfo {
            total: 0,
            free: 0,
            used: 0,
        };
        self.check(
            unsafe { (self.get_memory_info)(self.dev, &mut mem) },
            "GetMemoryInfo",
        )?;
        let mut limit_mw: c_uint = 0;
        self.check(
            unsafe { (self.get_power_limit)(self.dev, &mut limit_mw) },
            "GetPowerLimit",
        )?;
        let mut fans = Vec::with_capacity(self.fan_count as usize);
        for fan in 0..self.fan_count {
            let mut pct: c_uint = 0;
            if unsafe { (self.get_fan_speed)(self.dev, fan, &mut pct) } == SUCCESS {
                fans.push(pct);
            }
        }
        Ok(GpuSnapshot {
            temp_c: temp,
            power_w: power_mw as f32 / 1000.0,
            core_mhz: core,
            mem_mhz: memc,
            util_pct: util.gpu,
            vram_used_mb: mem.used / (1024 * 1024),
            vram_total_mb: mem.total / (1024 * 1024),
            fans_pct: fans,
            power_limit_w: limit_mw as f32 / 1000.0,
        })
    }

    pub fn clock_offset(&self, clock_type: c_uint) -> Result<OffsetRange, String> {
        if let Some(get) = self.get_offsets {
            let mut o = ClockOffsetV1 {
                version: offset_version(),
                clock_type,
                pstate: PSTATE_0,
                offset_mhz: 0,
                min_offset_mhz: 0,
                max_offset_mhz: 0,
            };
            self.check(unsafe { get(self.dev, &mut o) }, "GetClockOffsets")?;
            return Ok(OffsetRange {
                current: o.offset_mhz,
                min: o.min_offset_mhz,
                max: o.max_offset_mhz,
            });
        }
        let (get, range) = if clock_type == CLOCK_GRAPHICS {
            (self.legacy_get_gpc, self.legacy_gpc_range)
        } else {
            (self.legacy_get_mem, self.legacy_mem_range)
        };
        let get = get.ok_or("clock offset API not available in this driver")?;
        let range = range.ok_or("clock offset range API not available in this driver")?;
        let mut current: c_int = 0;
        self.check(unsafe { get(self.dev, &mut current) }, "GetClkVfOffset")?;
        let (mut min, mut max) = (0 as c_int, 0 as c_int);
        self.check(
            unsafe { range(self.dev, &mut min, &mut max) },
            "GetClkMinMaxVfOffset",
        )?;
        Ok(OffsetRange { current, min, max })
    }

    pub fn set_clock_offset(&self, clock_type: c_uint, offset_mhz: i32) -> Result<(), String> {
        if let Some(set) = self.set_offsets {
            let mut o = ClockOffsetV1 {
                version: offset_version(),
                clock_type,
                pstate: PSTATE_0,
                offset_mhz,
                min_offset_mhz: 0,
                max_offset_mhz: 0,
            };
            return self.check(unsafe { set(self.dev, &mut o) }, "SetClockOffsets");
        }
        let set = if clock_type == CLOCK_GRAPHICS {
            self.legacy_set_gpc
        } else {
            self.legacy_set_mem
        };
        let set = set.ok_or("clock offset API not available in this driver")?;
        self.check(unsafe { set(self.dev, offset_mhz) }, "SetClkVfOffset")
    }

    pub fn set_power_limit_w(&self, watts: u32) -> Result<(), String> {
        self.check(
            unsafe { (self.set_power_limit_fn)(self.dev, watts * 1000) },
            "SetPowerLimit",
        )
    }

    pub fn set_fan_pct(&self, pct: u32) -> Result<(), String> {
        let set = self.set_fan_speed.ok_or("fan control not supported")?;
        for fan in 0..self.fan_count {
            self.check(unsafe { set(self.dev, fan, pct.min(100)) }, "SetFanSpeed")?;
        }
        Ok(())
    }

    pub fn set_fans_auto(&self) -> Result<(), String> {
        let set = self.set_fan_default.ok_or("fan control not supported")?;
        for fan in 0..self.fan_count {
            self.check(unsafe { set(self.dev, fan) }, "SetDefaultFanSpeed")?;
        }
        Ok(())
    }

    /// Pin the core clock into `[min, max]` MHz — combined with a positive
    /// core offset this is the Linux undervolt: same clock, lower point on
    /// the V/F curve.
    pub fn lock_core_clocks(&self, min_mhz: u32, max_mhz: u32) -> Result<(), String> {
        let f = self.lock_gpu_clocks.ok_or("locked clocks not supported")?;
        self.check(
            unsafe { f(self.dev, min_mhz, max_mhz) },
            "SetGpuLockedClocks",
        )
    }

    pub fn unlock_core_clocks(&self) -> Result<(), String> {
        let f = self.reset_gpu_clocks.ok_or("locked clocks not supported")?;
        self.check(unsafe { f(self.dev) }, "ResetGpuLockedClocks")
    }

    /// Back to stock everything: offsets, power limit, fans, clock locks.
    pub fn reset_all(&self) -> Result<(), String> {
        let mut errs = Vec::new();
        let mut run = |r: Result<(), String>| {
            if let Err(e) = r {
                errs.push(e);
            }
        };
        run(self.set_clock_offset(CLOCK_GRAPHICS, 0));
        run(self.set_clock_offset(CLOCK_MEM, 0));
        let mut def_mw: c_uint = 0;
        if unsafe { (self.get_power_default)(self.dev, &mut def_mw) } == SUCCESS {
            run(self.set_power_limit_w(def_mw / 1000));
        }
        if self.set_fan_default.is_some() {
            run(self.set_fans_auto());
        }
        if self.reset_gpu_clocks.is_some() {
            run(self.unlock_core_clocks());
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("; "))
        }
    }
}

fn check(err_str: FnErrStr, ret: NvmlReturn, what: &str) -> Result<(), String> {
    if ret == SUCCESS {
        return Ok(());
    }
    let msg = unsafe {
        let p = err_str(ret);
        if p.is_null() {
            format!("error {ret}")
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };
    Err(format!("{what}: {msg}"))
}

fn cstr_buf(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}
