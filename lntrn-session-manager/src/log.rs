//! The session log, `~/.lantern/log/session.log`. Every line is stamped in
//! UTC the way the other Lantern logs are, so they can be read side by side.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Past this size the log is set aside as `session.log.1` at the next login.
const ROTATE_BYTES: u64 = 1024 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Start logging to `session.log` in `dir`. Until this is called (and in
/// tests, which never call it) lines only go to stdout.
pub fn init(dir: &Path) {
    let path = dir.join("session.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_BYTES) {
        let _ = std::fs::rename(&path, dir.join("session.log.1"));
    }
    let _ = PATH.set(path);
}

/// Write a line to the log, and to the terminal the session was started from.
///
/// Neither write may take the session down. At reboot the TTY is hung up and
/// stdout answers EIO; `println!` panics on that, and it used to end the
/// shutdown before anything had been cleaned up.
pub fn log(msg: &str) {
    if let Some(path) = PATH.get() {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{} {msg}", timestamp(SystemTime::now()));
        }
    }
    let _ = writeln!(std::io::stdout(), "{msg}");
}

/// `2026-10-10T03:04:25.328Z`
fn timestamp(time: SystemTime) -> String {
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since_epoch.as_secs();
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let in_day = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
        since_epoch.subsec_millis()
    )
}

/// Days since 1970-01-01 as a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    // Months counted from March, which puts the leap day last.
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 { month_from_march + 3 } else { month_from_march - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64, millis: u64) -> String {
        timestamp(UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_millis(millis))
    }

    #[test]
    fn timestamps_match_the_calendar() {
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(at(951_868_800, 0), "2000-03-01T00:00:00.000Z");
        assert_eq!(at(1_709_208_000, 7), "2024-02-29T12:00:00.007Z");
        assert_eq!(at(1_791_601_465, 328), "2026-10-10T03:04:25.328Z");
        assert_eq!(at(1_798_761_599, 999), "2026-12-31T23:59:59.999Z");
    }
}
