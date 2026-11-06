//! The helper's running parts.
//!
//! - `state` — the hardware, its limits, stock, what is applied and the
//!   trial, and writing a tune.
//! - `fan` — the fan curve's steps.
//! - `socket` — who may connect, and a thread for each window.
//! - `ffi`, `log` — the libc calls std lacks, and the timestamped log.

pub mod fan;
pub mod ffi;
pub mod log;
pub mod socket;
pub mod state;
