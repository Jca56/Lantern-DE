//! The helper's log: lines on stderr with the time they were written,
//! which the service sends to its log file.

use std::time::{SystemTime, UNIX_EPOCH};

/// The date and time as UTC `YYYY-MM-DD HH:MM:SS`.
fn now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

pub fn say(message: &str) {
    eprintln!("{} UTC  {message}", now());
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_time_reads_like_a_date() {
        let t = super::now();
        assert_eq!(t.len(), 19, "{t}");
        assert!(t.starts_with("20"), "{t}");
    }
}
