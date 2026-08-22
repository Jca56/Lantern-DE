//! GPU stats reader. Tries cheap sysfs (amdgpu) first, then falls back
//! to spawning `nvidia-smi`. Returns `None` when no readable GPU exists
//! so the tile hides itself on machines without a discrete GPU we can
//! query (e.g. an Intel-only laptop).
//!
//! No external crates — sysfs is plain file reads; the NVIDIA path shells
//! out to the driver-provided `nvidia-smi` (there's no stable sysfs
//! utilization counter for the proprietary driver).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct GpuStats {
    /// GPU core utilization, 0..100.
    pub util_pct: f32,
    /// GPU temperature in °C.
    pub temp_c: f32,
    pub vram_used_mb: u64,
    pub vram_total_mb: u64,
}

/// One-time probe: is there any GPU we know how to read? Used both to
/// reserve the tile slot and to decide whether the worker should poll.
pub fn gpu_available() -> bool {
    amd_card_path().is_some() || nvidia_smi_path().is_some()
}

/// Read a fresh GPU sample, or `None` if nothing is readable right now.
pub fn read_gpu() -> Option<GpuStats> {
    read_amd().or_else(read_nvidia)
}

// ─── AMD (amdgpu sysfs) ──────────────────────────────────────────────────────

/// First `/sys/class/drm/cardN/device` that exposes `gpu_busy_percent`.
fn amd_card_path() -> Option<PathBuf> {
    let rd = fs::read_dir("/sys/class/drm").ok()?;
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        // "cardN" only — skip the "cardN-CONNECTOR" symlinks.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let dev = e.path().join("device");
        if dev.join("gpu_busy_percent").is_file() {
            return Some(dev);
        }
    }
    None
}

fn read_amd() -> Option<GpuStats> {
    let dev = amd_card_path()?;
    let util = read_f32(dev.join("gpu_busy_percent"))?;
    let bytes_to_mb = |b: u64| b / (1024 * 1024);
    let vram_used_mb = read_u64(dev.join("mem_info_vram_used"))
        .map(bytes_to_mb)
        .unwrap_or(0);
    let vram_total_mb = read_u64(dev.join("mem_info_vram_total"))
        .map(bytes_to_mb)
        .unwrap_or(0);
    let temp_c = read_amd_temp(&dev).unwrap_or(0.0);
    Some(GpuStats {
        util_pct: util,
        temp_c,
        vram_used_mb,
        vram_total_mb,
    })
}

/// amdgpu exposes temperature under a hwmon subdir as millidegrees.
fn read_amd_temp(dev: &PathBuf) -> Option<f32> {
    let hwmon_root = dev.join("hwmon");
    let rd = fs::read_dir(&hwmon_root).ok()?;
    for e in rd.flatten() {
        let t = e.path().join("temp1_input");
        if let Some(millic) = read_u64(t) {
            return Some(millic as f32 / 1000.0);
        }
    }
    None
}

// ─── NVIDIA (nvidia-smi) ─────────────────────────────────────────────────────

fn read_nvidia() -> Option<GpuStats> {
    let bin = nvidia_smi_path()?;
    let out = Command::new(bin)
        .args([
            "--query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next()?;
    let mut f = line.split(',').map(|x| x.trim());
    let util = f.next()?.parse::<f32>().ok()?;
    let temp = f.next()?.parse::<f32>().ok()?;
    let used = f.next()?.parse::<u64>().ok()?; // already MiB (nounits)
    let total = f.next()?.parse::<u64>().ok()?;
    Some(GpuStats {
        util_pct: util,
        temp_c: temp,
        vram_used_mb: used,
        vram_total_mb: total,
    })
}

/// Locate `nvidia-smi` on `$PATH` without spawning a shell.
fn nvidia_smi_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join("nvidia-smi");
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

// ─── small parse helpers ─────────────────────────────────────────────────────

fn read_f32<P: AsRef<std::path::Path>>(p: P) -> Option<f32> {
    fs::read_to_string(p).ok()?.trim().parse::<f32>().ok()
}
fn read_u64<P: AsRef<std::path::Path>>(p: P) -> Option<u64> {
    fs::read_to_string(p).ok()?.trim().parse::<u64>().ok()
}
