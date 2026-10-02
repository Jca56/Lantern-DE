// Server time.
//
// Firestore and Storage report times as RFC 3339 strings in UTC
// ("2026-10-02T09:15:04.123456Z"). Sync keeps them as microseconds since the
// Unix epoch so they can be compared and offset. Every stamp sync orders by
// or measures an age against comes from the SERVER's clock (a write stamp, a
// document's update time, the read time of a query): the two machines' own
// clocks never enter into it, so a machine whose clock is wrong cannot hide
// its writes from the other, or the other's from itself.

/// Microseconds since the Unix epoch, on the server's clock.
pub type Micros = u64;

pub const SECOND: Micros = 1_000_000;
pub const MINUTE: Micros = 60 * SECOND;
pub const HOUR: Micros = 60 * MINUTE;
pub const DAY: Micros = 24 * HOUR;

// Civil date <-> day number, after Howard Hinnant's algorithms (proleptic
// Gregorian calendar, day 0 = 1970-01-01).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

/// `width` ASCII digits at `at`, as a number.
fn digits(s: &[u8], at: usize, width: usize) -> Option<i64> {
    let part = s.get(at..at + width)?;
    part.iter().try_fold(0i64, |acc, b| {
        b.is_ascii_digit().then(|| acc * 10 + i64::from(b - b'0'))
    })
}

/// Parse an RFC 3339 time. `None` for anything else, and for a time before
/// 1970: a caller that cannot read a stamp must treat it as unknown, never
/// as "long ago".
pub fn parse(text: &str) -> Option<Micros> {
    let s = text.as_bytes();
    let year = digits(s, 0, 4)?;
    let month = digits(s, 5, 2)? as u32;
    let day = digits(s, 8, 2)? as u32;
    let hour = digits(s, 11, 2)?;
    let minute = digits(s, 14, 2)?;
    let second = digits(s, 17, 2)?;
    if s[4] != b'-' || s[7] != b'-' || !matches!(s[10], b'T' | b't' | b' ') {
        return None;
    }
    if s[13] != b':' || s[16] != b':' {
        return None;
    }
    // 60 is a leap second; the servers smear those, but the format allows it.
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    // Fraction: any number of digits, cut to microseconds.
    let mut at = 19;
    let mut micros = 0i64;
    if s.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while s.get(at).is_some_and(u8::is_ascii_digit) {
            if at - start < 6 {
                micros = micros * 10 + i64::from(s[at] - b'0');
            }
            at += 1;
        }
        if at == start {
            return None;
        }
        for _ in (at - start)..6 {
            micros *= 10;
        }
    }

    // Zone: "Z", or an offset to take off.
    let offset_secs = match s.get(at)? {
        b'Z' | b'z' if at + 1 == s.len() => 0,
        sign @ (b'+' | b'-') if at + 6 == s.len() && s[at + 3] == b':' => {
            let secs = digits(s, at + 1, 2)? * 3600 + digits(s, at + 4, 2)? * 60;
            if *sign == b'+' {
                secs
            } else {
                -secs
            }
        }
        _ => return None,
    };

    let secs = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
        - offset_secs;
    let total = secs.checked_mul(1_000_000)?.checked_add(micros)?;
    u64::try_from(total).ok()
}

/// RFC 3339 in UTC with microseconds: what Firestore takes as a
/// `timestampValue`.
pub fn format(at: Micros) -> String {
    let secs = (at / SECOND) as i64;
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let in_day = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
        at % SECOND
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_the_servers_send() {
        assert_eq!(parse("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse("1970-01-01T00:00:01.5Z"), Some(1_500_000));
        // Firestore: microseconds. Storage: milliseconds.
        assert_eq!(
            parse("2026-10-02T09:15:04.123456Z"),
            Some(1_790_932_504_123_456)
        );
        assert_eq!(parse("2026-10-02T09:15:04.123Z"), Some(1_790_932_504_123_000));
        // Nanoseconds are cut, not rounded.
        assert_eq!(
            parse("2026-10-02T09:15:04.123456999Z"),
            Some(1_790_932_504_123_456)
        );
        // An offset is taken off.
        assert_eq!(
            parse("2026-10-02T11:15:04+02:00"),
            parse("2026-10-02T09:15:04Z")
        );
        assert_eq!(
            parse("2026-10-02T04:15:04.5-05:00"),
            parse("2026-10-02T09:15:04.5Z")
        );
        // Leap day.
        assert_eq!(parse("2024-02-29T00:00:00Z"), Some(1_709_164_800_000_000));
    }

    #[test]
    fn rejects_what_is_not_a_time() {
        for bad in [
            "",
            "2026-10-02",
            "2026-10-02T09:15:04",
            "2026-10-02T09:15:04.Z",
            "2026-13-02T09:15:04Z",
            "2026-02-30T09:15:04Z",
            "2025-02-29T09:15:04Z",
            "2026-10-02T24:00:00Z",
            "2026-10-02T09:15:04Zjunk",
            "2026-10-02T09:15:04+0200",
            "1969-12-31T23:59:59Z",
            "20261002T091504Z",
            "not a time at all!!!",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn format_and_parse_agree() {
        for at in [
            0,
            1,
            999_999,
            1_709_164_800_000_000,
            1_790_932_504_123_456,
            4_102_444_799_999_999, // 2099-12-31T23:59:59.999999Z
        ] {
            assert_eq!(parse(&format(at)), Some(at), "{at}");
        }
        assert_eq!(format(1_790_932_504_123_456), "2026-10-02T09:15:04.123456Z");
        assert_eq!(format(0), "1970-01-01T00:00:00.000000Z");
        // Text order is time order, which the fake cloud in the tests and a
        // human reading the mirror both rely on.
        assert!(format(1_790_932_504_123_456) < format(1_790_932_504_123_457));
    }
}
