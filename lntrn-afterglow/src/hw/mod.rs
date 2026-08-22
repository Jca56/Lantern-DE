//! Hardware access layer shared by the GUI and the root daemon.
//!
//! Getters generally work unprivileged; setters (and RAPL energy reads) need
//! root, which is why they live behind `lntrn-afterglowd`.

pub mod cpu;
pub mod nvml;
pub mod rapl;

use std::io;
use std::path::Path;

pub(crate) fn read_trim(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

pub(crate) fn read_u64(path: &Path) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

pub(crate) fn write_value(path: &Path, value: &str) -> io::Result<()> {
    std::fs::write(path, value)
}
