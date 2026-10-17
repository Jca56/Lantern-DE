//! Days on the calendar: a date's place in its month and week, counted in
//! days from the epoch to step and compare (Howard Hinnant's civil-day
//! arithmetic, exact over any year a calendar sees), and how dates and
//! times of day are said, and typed.

use crate::clock::{self, ClockOptions, Local, MONTHS, WEEKDAYS};

/// A day on the calendar, valid by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Date> {
        ((1..=12).contains(&month) && (1..=days_in(year, month)).contains(&day)).then_some(Date { year, month, day })
    }

    /// The day the local clock is on.
    pub fn of(t: &Local) -> Date {
        Date::new(t.year, t.month, t.day).unwrap_or(Date { year: 1970, month: 1, day: 1 })
    }

    /// Days since 1970-01-01.
    pub fn days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(z: i64) -> Date {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Date { year, month, day }
    }

    /// 0 Sunday to 6 Saturday.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday.
        (self.days() + 4).rem_euclid(7) as u32
    }

    pub fn plus(self, days: i64) -> Date {
        Date::from_days(self.days() + days)
    }

    /// The first of its month.
    pub fn first(self) -> Date {
        Date { day: 1, ..self }
    }

    /// The same day `n` months on, or its month's last day where that
    /// month is shorter.
    pub fn plus_months(self, n: i32) -> Date {
        let index = self.year * 12 + self.month as i32 - 1 + n;
        let (year, month) = (index.div_euclid(12), index.rem_euclid(12) as u32 + 1);
        Date { year, month, day: self.day.min(days_in(year, month)) }
    }

    /// From `2026-10-17`.
    pub fn parse(text: &str) -> Option<Date> {
        let mut parts = text.trim().splitn(3, '-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.get(..2)?.parse().ok()?;
        Date::new(year, month, day)
    }

    /// As it is kept: `2026-10-17`.
    pub fn word(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn weekday_name(self) -> &'static str {
        WEEKDAYS[self.weekday() as usize]
    }

    pub fn month_name(self) -> &'static str {
        MONTHS[self.month as usize - 1]
    }

    /// `Saturday, October 17, 2026`.
    pub fn long(self) -> String {
        format!("{}, {} {}, {}", self.weekday_name(), self.month_name(), self.day, self.year)
    }

    /// `Sat, Oct 17, 2026`.
    pub fn short(self) -> String {
        format!("{}, {} {}, {}", &self.weekday_name()[..3], &self.month_name()[..3], self.day, self.year)
    }

    /// `October 2026`.
    pub fn month_title(self) -> String {
        format!("{} {}", self.month_name(), self.year)
    }

    /// How a day is spoken of from `today`: `Today`, `Tomorrow`,
    /// `Yesterday`, else `Saturday, October 24` (with the year when it
    /// isn't this one).
    pub fn spoken(self, today: Date) -> String {
        match self.days() - today.days() {
            0 => "Today".to_owned(),
            1 => "Tomorrow".to_owned(),
            -1 => "Yesterday".to_owned(),
            _ if self.year == today.year => format!("{}, {} {}", self.weekday_name(), self.month_name(), self.day),
            _ => self.long(),
        }
    }
}

pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Which day a week starts on: `[control_center] calendar_week_start`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeekStart {
    #[default]
    Sunday,
    Monday,
}

impl WeekStart {
    /// From its stored word; anything else is the default.
    pub fn from_word(word: &str) -> WeekStart {
        if word == "monday" { WeekStart::Monday } else { WeekStart::Sunday }
    }

    /// The weekday a row of the month starts on, 0 Sunday.
    pub fn weekday(self) -> u32 {
        match self {
            WeekStart::Sunday => 0,
            WeekStart::Monday => 1,
        }
    }
}

/// The six weeks a month is shown in: the first row holds its first day,
/// days of the months either side fill the rest.
pub fn grid(month: Date, start: WeekStart) -> [Date; 42] {
    let first = month.first();
    let lead = (first.weekday() + 7 - start.weekday()) % 7;
    let top = first.plus(-i64::from(lead));
    std::array::from_fn(|i| top.plus(i as i64))
}

/// Minutes into a day as the clock says them: `3:00 PM`, `15:00`; the
/// day's end (1440) is the midnight after it.
pub fn time_text(minute: u32, opts: ClockOptions) -> String {
    if minute >= 1440 && opts.hour24 {
        return "24:00".to_owned();
    }
    let m = minute % 1440;
    let t = Local { year: 1970, month: 1, day: 1, weekday: 4, hour: m / 60, minute: m % 60, second: 0 };
    // AM/PM always: a time beside nothing else is ambiguous without it.
    clock::time_text(&t, ClockOptions { seconds: false, ampm: true, ..opts })
}

/// A time of day as typed: `3pm`, `3:30 PM`, `15:30`, `1530`, `noon`,
/// `midnight`. Without AM or PM on a 12-hour clock, 1 to 7 are the
/// afternoon's (nobody books a dentist at 3 in the night) and 8 to 12 the
/// morning's and noon; on a 24-hour clock it reads as written.
pub fn parse_time(text: &str, hour24: bool) -> Option<u32> {
    let t: String = text.trim().to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    match t.as_str() {
        "noon" => return Some(720),
        "midnight" => return Some(0),
        _ => {}
    }
    let (digits, meridiem) = if let Some(d) = t.strip_suffix("am").or_else(|| t.strip_suffix('a')) {
        (d, Some(false))
    } else if let Some(d) = t.strip_suffix("pm").or_else(|| t.strip_suffix('p')) {
        (d, Some(true))
    } else {
        (t.as_str(), None)
    };
    let (h, m): (u32, u32) = match digits.split_once([':', '.']) {
        Some((h, m)) if !m.is_empty() && m.len() <= 2 => (h.parse().ok()?, m.parse().ok()?),
        Some(_) => return None,
        None if !digits.is_empty() && digits.len() <= 2 => (digits.parse().ok()?, 0),
        None if digits.len() <= 4 && digits.len() >= 3 => {
            let split = digits.len() - 2;
            (digits[..split].parse().ok()?, digits[split..].parse().ok()?)
        }
        None => return None,
    };
    if m >= 60 || !digits.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.') {
        return None;
    }
    let h = match meridiem {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if !hour24 && (1..=7).contains(&h) => h + 12,
        None if h < 24 => h,
        None => return None,
    };
    Some(h * 60 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i32, month: u32, day: u32) -> Date {
        Date::new(year, month, day).unwrap()
    }

    #[test]
    fn days_count_from_the_epoch_both_ways() {
        assert_eq!(d(1970, 1, 1).days(), 0);
        assert_eq!(d(2026, 10, 17).days(), 20_743);
        for z in [-800_000, -1, 0, 59, 60, 11_016, 20_743, 400_000] {
            assert_eq!(Date::from_days(z).days(), z);
        }
        assert_eq!(Date::from_days(20_743), d(2026, 10, 17));
        assert_eq!(d(2024, 2, 28).plus(1), d(2024, 2, 29));
        assert_eq!(d(2026, 12, 31).plus(1), d(2027, 1, 1));
    }

    #[test]
    fn weekdays_fall_where_the_calendar_has_them() {
        assert_eq!(d(2026, 10, 17).weekday(), 6, "a Saturday");
        assert_eq!(d(2026, 10, 16).weekday_name(), "Friday");
        assert_eq!(d(2000, 1, 1).weekday(), 6);
        assert_eq!(d(1969, 12, 31).weekday(), 3);
    }

    #[test]
    fn months_are_as_long_as_they_are() {
        assert_eq!((days_in(2026, 2), days_in(2024, 2), days_in(1900, 2), days_in(2000, 2)), (28, 29, 28, 29));
        assert_eq!(Date::new(2026, 2, 29), None);
        assert_eq!(Date::new(2026, 13, 1), None);
        assert_eq!(d(2026, 1, 31).plus_months(1), d(2026, 2, 28), "held to the month's last day");
        assert_eq!(d(2026, 11, 5).plus_months(2), d(2027, 1, 5));
        assert_eq!(d(2026, 1, 5).plus_months(-1), d(2025, 12, 5));
    }

    #[test]
    fn a_month_shows_in_six_weeks_from_the_week_start() {
        // October 2026 starts on a Thursday.
        let sunday = grid(d(2026, 10, 9), WeekStart::Sunday);
        assert_eq!((sunday[0], sunday[4], sunday[41]), (d(2026, 9, 27), d(2026, 10, 1), d(2026, 11, 7)));
        let monday = grid(d(2026, 10, 9), WeekStart::Monday);
        assert_eq!((monday[0], monday[3]), (d(2026, 9, 28), d(2026, 10, 1)));
        // A month starting on the week's first day starts the grid.
        assert_eq!(grid(d(2026, 2, 1), WeekStart::Sunday)[0], d(2026, 2, 1));
    }

    #[test]
    fn dates_read_and_write_as_kept_and_said() {
        assert_eq!(Date::parse("2026-10-17"), Some(d(2026, 10, 17)));
        assert_eq!(Date::parse(" 2026-02-30 "), None);
        assert_eq!(Date::parse("2026-1"), None);
        assert_eq!(d(2026, 3, 4).word(), "2026-03-04");
        assert_eq!(d(2026, 10, 17).long(), "Saturday, October 17, 2026");
        assert_eq!(d(2026, 10, 17).short(), "Sat, Oct 17, 2026");
        let today = d(2026, 10, 17);
        assert_eq!((today.spoken(today), d(2026, 10, 18).spoken(today), d(2026, 10, 16).spoken(today)), ("Today".into(), "Tomorrow".into(), "Yesterday".into()));
        assert_eq!(d(2026, 10, 24).spoken(today), "Saturday, October 24");
        assert_eq!(d(2027, 1, 2).spoken(today), "Saturday, January 2, 2027");
    }

    #[test]
    fn times_read_as_typed() {
        for (typed, minute) in [("3pm", 900), ("3:30 PM", 930), ("3.30p", 930), ("12am", 0), ("12 pm", 720), ("15:30", 930), ("1530", 930), ("930", 570), ("noon", 720), ("Midnight", 0)] {
            assert_eq!(parse_time(typed, false), Some(minute), "{typed}");
        }
        // Bare hours on a 12-hour clock: the afternoon's up to 7.
        assert_eq!((parse_time("3", false), parse_time("9", false), parse_time("12", false)), (Some(900), Some(540), Some(720)));
        assert_eq!(parse_time("3", true), Some(180), "a 24-hour clock reads as written");
        for bad in ["", "25:00", "3:75", "13pm", "abc", "3:", "12345", "3x"] {
            assert_eq!(parse_time(bad, false), None, "{bad}");
        }
    }

    #[test]
    fn times_are_said_as_the_clock_says_them() {
        let twelve = ClockOptions::default();
        let tf = ClockOptions { hour24: true, ..twelve };
        assert_eq!((time_text(900, twelve), time_text(540, tf)), ("3:00 PM".into(), "09:00".into()));
        assert_eq!((time_text(1440, twelve), time_text(1440, tf)), ("12:00 AM".into(), "24:00".into()));
        assert_eq!(time_text(0, ClockOptions { ampm: false, seconds: true, ..twelve }), "12:00 AM", "AM/PM always, no seconds");
    }
}
