//! Local wall-clock time for display.
//!
//! Every date shown in the UI goes through here. Dividing the Unix time by
//! 86400 by hand gives UTC, so an evening file showed tomorrow's date, and
//! the year loops that came with it walked one iteration per year — a file
//! with a far-future mtime hung the frame.

use std::time::{SystemTime, UNIX_EPOCH};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub struct LocalTime {
    pub year: i64,
    /// 1..=12
    pub month: u32,
    pub day: u32,
    /// 0..=23
    pub hour: u32,
    pub minute: u32,
}

impl LocalTime {
    pub fn month_name(&self) -> &'static str {
        MONTHS[(self.month as usize).clamp(1, 12) - 1]
    }

    /// (hour on a 12-hour clock, "AM" / "PM").
    pub fn hour12(&self) -> (u32, &'static str) {
        let h = match self.hour % 12 {
            0 => 12,
            h => h,
        };
        (h, if self.hour < 12 { "AM" } else { "PM" })
    }
}

/// `t` in the system's time zone. `None` when the stamp can't be shown as a
/// sane calendar date (out of range for the C library, or a year we would
/// not print), so callers fall back to their "unknown" text.
pub fn local(t: SystemTime) -> Option<LocalTime> {
    let secs: i64 = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).ok()?,
        Err(e) => -i64::try_from(e.duration().as_secs()).ok()?,
    };
    let secs: libc::time_t = secs;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return None;
    }
    let year = tm.tm_year as i64 + 1900;
    if !(1..=9999).contains(&year) {
        return None;
    }
    Some(LocalTime {
        year,
        month: tm.tm_mon as u32 + 1,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn far_future_and_absurd_stamps_return_quickly() {
        // Year ~292 billion: the old per-year loop never came back from this.
        let far = UNIX_EPOCH + Duration::from_secs(i64::MAX as u64);
        assert!(local(far).is_none());
        // Year 5138 is fine to show.
        let ok = UNIX_EPOCH + Duration::from_secs(100_000_000_000);
        assert!(local(ok).is_some_and(|t| t.year > 5000 && (1..=12).contains(&t.month)));
    }

    #[test]
    fn twelve_hour_clock() {
        let at = |hour| LocalTime {
            year: 2026,
            month: 10,
            day: 2,
            hour,
            minute: 0,
        };
        assert_eq!(at(0).hour12(), (12, "AM"));
        assert_eq!(at(11).hour12(), (11, "AM"));
        assert_eq!(at(12).hour12(), (12, "PM"));
        assert_eq!(at(23).hour12(), (11, "PM"));
        assert_eq!(at(0).month_name(), "Oct");
    }
}
