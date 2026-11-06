//! The hardware Afterglow tunes. Each part is found at start and is
//! simply absent on a machine without it: no NVIDIA driver, no Intel
//! power limits.
//!
//! - `nvml` — NVIDIA's management library: readings for anyone, clocks,
//!   power and fans for root.
//! - `cpu` — the frequency driver: each thread's clock and load, the
//!   governor, energy preference, turbo and top speed.
//! - `rapl` — Intel's package power limits, and the package's draw.
//! - `sysfs` — reading and writing the small files all of it goes
//!   through.

pub mod cpu;
pub mod nvml;
pub mod rapl;
pub mod sysfs;
