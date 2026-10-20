//! Where recordings go, and how they're spoken of: their length and size.

use std::path::PathBuf;

/// `~/Videos/recording_2026-10-20_14-03-22.mp4`, named like the
/// screenshot tool's pictures.
pub fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Videos").join(format!("recording_{}.mp4", timestamp()))
}

/// The local time, `2026-10-20_14-03-22`.
fn timestamp() -> String {
    // SAFETY: localtime_r fills `tm`, ours, from `t`, ours.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return t.to_string();
        }
        format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

/// `0:07`, `12:34`, `1:02:03`.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// `840 KB`, `12.4 MB`, `1.31 GB`.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..1_000_000 => format!("{:.0} KB", b / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", b / 1e6),
        _ => format!("{:.2} GB", b / 1e9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_sizes_read_as_people_say_them() {
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(754.0), "12:34");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(size(840_000), "840 KB");
        assert_eq!(size(12_400_000), "12.4 MB");
        assert_eq!(size(1_310_000_000), "1.31 GB");
    }
}
