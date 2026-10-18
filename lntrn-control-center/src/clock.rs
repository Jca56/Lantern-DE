//! The time and date at the bar's right end. Local time comes from libc
//! (the system's zone, its summer time, a new zone picked up by `tzset`),
//! drawn time over date, or on one line when the bar is too short for
//! two. A thread wakes the loop on each wall-clock minute: it sleeps on
//! the real-time clock to an absolute deadline, so it is on time after a
//! suspend and when the clock is set forward, which the UI's own timers
//! (on the monotonic clock) are not.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lntrn_app::Waker;
use lntrn_core::log_warn;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::Ui;

/// glibc's `struct tm`.
#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const std::ffi::c_char,
}

#[repr(C)]
struct Timespec {
    sec: i64,
    nsec: i64,
}

unsafe extern "C" {
    fn tzset();
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    fn mktime(tm: *mut Tm) -> i64;
    fn clock_nanosleep(clock: i32, flags: i32, request: *const Timespec, remain: *mut Timespec) -> i32;
}
const CLOCK_REALTIME: i32 = 0;
const TIMER_ABSTIME: i32 = 1;
const EINTR: i32 = 4;

/// Logical sizes, to read from across the desk: as big as still stacks
/// time over date on the default 64px bar, line height and all.
const TIME_SIZE: f64 = 28.0;
const DATE_SIZE: f64 = 21.0;
/// From the plate's right end to the text.
const PAD: f64 = 20.0;
/// Between date and time when they share a line.
const SPACING: f64 = 14.0;

pub const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// A moment on the local clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    /// 0 Sunday to 6 Saturday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Local {
    /// Now, in the system's zone as it is now.
    pub fn now() -> Option<Local> {
        let secs = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
        // SAFETY: plain libc calls; `tm` is a plain struct that zeroes
        // validly and is only read after `localtime_r` filled it.
        let tm = unsafe {
            let mut tm: Tm = std::mem::zeroed();
            tzset();
            if localtime_r(&secs, &mut tm).is_null() {
                return None;
            }
            tm
        };
        let u = |v: i32| u32::try_from(v).unwrap_or(0);
        Some(Local { year: tm.year + 1900, month: u(tm.mon + 1), day: u(tm.mday), weekday: u(tm.wday) % 7, hour: u(tm.hour), minute: u(tm.min), second: u(tm.sec) })
    }

    /// Seconds since the epoch at `minute` into the day `year`-`month`-
    /// `day` on the local clock, its summer time worked out for that day
    /// (1440 is the next day's midnight).
    pub fn epoch_at(year: i32, month: u32, day: u32, minute: u32) -> Option<i64> {
        let i = |v: u32| i32::try_from(v).ok();
        // SAFETY: plain libc calls on our own zeroed `tm`; `mktime` only
        // reads it and writes it back normalized.
        let t = unsafe {
            let mut tm: Tm = std::mem::zeroed();
            (tm.year, tm.mon, tm.mday, tm.hour, tm.min, tm.isdst) = (year - 1900, i(month)? - 1, i(day)?, i(minute / 60)?, i(minute % 60)?, -1);
            tzset();
            mktime(&mut tm)
        };
        (t != -1).then_some(t)
    }

    fn weekday_name(&self) -> &'static str {
        WEEKDAYS[self.weekday as usize % 7]
    }

    fn month_name(&self) -> &'static str {
        MONTHS[(self.month as usize).clamp(1, 12) - 1]
    }
}

/// How the date is written: the stored word in `[control_center]
/// clock_date_style`, which System Settings offers by example.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateStyle {
    /// Thu, Oct 16
    #[default]
    Short,
    /// 10/16/2026
    Numeric,
    /// Thursday, October 16
    Long,
    /// Oct 16, 2026
    WithYear,
}

impl DateStyle {
    /// From its stored word; anything else is the default.
    pub fn from_word(word: &str) -> DateStyle {
        match word {
            "numeric" => DateStyle::Numeric,
            "long" => DateStyle::Long,
            "with_year" => DateStyle::WithYear,
            _ => DateStyle::Short,
        }
    }
}

/// What the clock shows: `[control_center]`'s `clock_*` keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockOptions {
    pub hour24: bool,
    pub seconds: bool,
    /// AM or PM after a 12-hour time.
    pub ampm: bool,
    pub date: bool,
    pub style: DateStyle,
}

impl Default for ClockOptions {
    fn default() -> Self {
        ClockOptions { hour24: false, seconds: false, ampm: true, date: true, style: DateStyle::Short }
    }
}

/// `6:52 PM`, `18:52`, with `:07` before the AM/PM when asked, and
/// `6:52` alone when the AM/PM isn't.
pub fn time_text(t: &Local, opts: ClockOptions) -> String {
    let sec = if opts.seconds { format!(":{:02}", t.second) } else { String::new() };
    if opts.hour24 {
        return format!("{:02}:{:02}{sec}", t.hour, t.minute);
    }
    let h = match t.hour % 12 {
        0 => 12,
        h => h,
    };
    if !opts.ampm {
        return format!("{h}:{:02}{sec}", t.minute);
    }
    format!("{h}:{:02}{sec} {}", t.minute, if t.hour < 12 { "AM" } else { "PM" })
}

pub fn date_text(t: &Local, style: DateStyle) -> String {
    match style {
        DateStyle::Short => format!("{}, {} {}", &t.weekday_name()[..3], &t.month_name()[..3], t.day),
        DateStyle::Numeric => format!("{}/{}/{}", t.month, t.day, t.year),
        DateStyle::Long => format!("{}, {} {}", t.weekday_name(), t.month_name(), t.day),
        DateStyle::WithYear => format!("{} {}, {}", &t.month_name()[..3], t.day, t.year),
    }
}

/// Seconds from `now` (since the epoch) to just past the next wall-clock
/// second or minute: when the shown time next changes. Zones are offset
/// by whole minutes, so a UTC minute starts a local one.
pub fn until_next(now: Duration, seconds: bool) -> f64 {
    let step = if seconds { 1.0 } else { 60.0 };
    let t = now.as_secs_f64();
    step - t % step + 0.005
}

/// When the UI should next rebuild for the clock: on the next second
/// while seconds show; else a second past the next minute, a fallback the
/// thread's own wake beats unless the clock was set back under it.
pub fn redraw_after(opts: ClockOptions) -> Option<f64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let soon = until_next(now, opts.seconds);
    Some(if opts.seconds { soon } else { soon + 1.0 })
}

/// The clock's lines placed at the right end of a plate, not yet drawn:
/// what goes under them (the calendar's lit face) is painted first.
pub struct Layout {
    /// The room it takes, so what is laid out beside it knows where to
    /// stop: the plate's height, from the widest line's left to the text's
    /// right end. The time's share is the most it can ever take, so it
    /// stays put as the seconds tick, at ten o'clock and at noon.
    pub rect: Rect,
    lines: Vec<Line>,
}

/// One of the clock's lines, right-aligned in its rect; the time's with
/// its digits [`tabular`].
struct Line {
    text: String,
    style: TextStyle,
    rect: Rect,
    color: Color,
    tabular: bool,
}

impl Layout {
    pub fn paint(&self, ui: &mut Ui) {
        for l in &self.lines {
            if l.tabular {
                let w = tabular_w(ui, &l.text, &l.style);
                let top = (l.rect.center().y - f64::from(l.style.line_height()) * 0.5).round();
                tabular(ui, &l.text, &l.style, Vec2::new(l.rect.max.x - w, top), l.color);
            } else {
                ui.text_right(&l.text, &l.style, l.rect, l.color);
            }
        }
    }
}

/// Place the clock at the right end of `plate` (window pixels).
pub fn layout(ui: &mut Ui, plate: Rect, opts: ClockOptions, now: &Local) -> Layout {
    let m = ui.m;
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let time = time_text(now, opts);
    let time_style = TextStyle::new(m.px(TIME_SIZE) as f32).bold();
    let date_style = TextStyle::new(m.px(DATE_SIZE) as f32);
    let right = plate.max.x - m.px(PAD);
    let time_w = time_room(ui, opts, &time_style);
    let time_h = f64::from(time_style.line_height());
    let line = |left: f64, top: f64, h: f64| Rect::new(Vec2::new(left, top), Vec2::new(right, top + h));
    let time_line = |rect: Rect| Line { text: time.clone(), style: time_style.clone(), rect, color: ink, tabular: true };
    let Some(date) = opts.date.then(|| date_text(now, opts.style)) else {
        let rect = line(right - time_w, plate.min.y, plate.height());
        return Layout { rect, lines: vec![time_line(rect)] };
    };
    let date_w = ui.measure(&date, &date_style);
    let date_h = f64::from(date_style.line_height());
    let date_line = |rect: Rect| Line { text: date.clone(), style: date_style.clone(), rect, color: dim, tabular: false };
    // Time over date when both lines fit the plate, else side by side.
    if time_h + date_h <= plate.height() - m.px(4.0) {
        let left = right - time_w.max(date_w);
        let top = (plate.center().y - (time_h + date_h) * 0.5).round();
        let lines = vec![time_line(line(left, top, time_h)), date_line(line(left, top + time_h, date_h))];
        return Layout { rect: line(left, plate.min.y, plate.height()), lines };
    }
    let time_left = right - time_w;
    let date_right = time_left - m.px(SPACING);
    let left = date_right - date_w;
    let date_at = Rect::new(Vec2::new(left, plate.min.y), Vec2::new(date_right, plate.max.y));
    let lines = vec![time_line(line(time_left, plate.min.y, plate.height())), date_line(date_at)];
    Layout { rect: line(left, plate.min.y, plate.height()), lines }
}

/// The most room the time can take on `opts`' clock: a two-digit hour,
/// and the wider of AM and PM.
fn time_room(ui: &mut Ui, opts: ClockOptions, style: &TextStyle) -> f64 {
    let at = |hour: u32| Local { year: 2026, month: 1, day: 1, weekday: 4, hour, minute: 0, second: 0 };
    tabular_w(ui, &time_text(&at(10), opts), style).max(tabular_w(ui, &time_text(&at(22), opts), style))
}

const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// The pieces `text` is drawn in: each digit alone (`true`), what is
/// between them whole.
fn pieces(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut run = 0;
    for (i, c) in text.char_indices() {
        if c.is_ascii_digit() {
            if run < i {
                out.push((&text[run..i], false));
            }
            out.push((&text[i..i + 1], true));
            run = i + 1;
        }
    }
    if run < text.len() {
        out.push((&text[run..], false));
    }
    out
}

/// A digit's cell in `style`: as wide as the widest digit.
fn cell(ui: &mut Ui, style: &TextStyle) -> f64 {
    DIGITS.iter().map(|d| ui.measure(d, style)).fold(0.0, f64::max)
}

/// How wide `text` is drawn [`tabular`].
pub fn tabular_w(ui: &mut Ui, text: &str, style: &TextStyle) -> f64 {
    let cell = cell(ui, style);
    pieces(text).into_iter().map(|(piece, digit)| if digit { cell } else { ui.measure(piece, style) }).sum()
}

/// Draw `text` with its line box's top left at `at`, each digit centred in
/// a cell as wide as the widest one: the font's own digits are each their
/// own width, so a ticking time would grow and shrink. What is between the
/// digits keeps its own width.
pub fn tabular(ui: &mut Ui, text: &str, style: &TextStyle, at: Vec2, color: Color) {
    let cell = cell(ui, style);
    let mut x = at.x;
    for (piece, digit) in pieces(text) {
        let w = ui.measure(piece, style);
        let left = if digit { x + (cell - w) * 0.5 } else { x };
        ui.text_at(piece, style, Vec2::new(left.round(), at.y), 1.0e6, color);
        x += if digit { cell } else { w };
    }
}

/// Wake the loop on each wall-clock minute from a thread of its own.
pub fn watch(waker: Waker) {
    if let Err(e) = each_minute("clock", move || waker.wake()) {
        log_warn!("clock: no thread: {e}; the minute turns on the UI's timer alone");
    }
}

/// Call `f` from a thread called `name` just as each wall-clock minute
/// starts. It sleeps on the real-time clock to an absolute deadline, so it
/// is on time after a suspend and when the clock is set.
pub fn each_minute(name: &str, mut f: impl FnMut() + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new().name(name.into()).spawn(move || {
        loop {
            let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            };
            let next = Timespec { sec: i64::try_from(now.as_secs() / 60 * 60 + 60).unwrap_or(i64::MAX), nsec: 0 };
            // SAFETY: a plain libc call on our own timespec.
            while unsafe { clock_nanosleep(CLOCK_REALTIME, TIMER_ABSTIME, &next, std::ptr::null_mut()) } == EINTR {}
            f();
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, minute: u32, second: u32) -> Local {
        // Thursday, October 16, 2026.
        Local { year: 2026, month: 10, day: 16, weekday: 4, hour, minute, second }
    }

    fn opts(hour24: bool, seconds: bool, ampm: bool) -> ClockOptions {
        ClockOptions { hour24, seconds, ampm, ..ClockOptions::default() }
    }

    #[test]
    fn the_time_reads_as_a_clock_does() {
        assert_eq!(time_text(&at(18, 52, 7), opts(false, false, true)), "6:52 PM");
        assert_eq!(time_text(&at(18, 52, 7), opts(false, true, true)), "6:52:07 PM");
        assert_eq!(time_text(&at(18, 52, 7), opts(true, false, true)), "18:52");
        assert_eq!(time_text(&at(6, 5, 9), opts(true, true, true)), "06:05:09");
        // Midnight and noon are twelve.
        assert_eq!(time_text(&at(0, 0, 0), opts(false, false, true)), "12:00 AM");
        assert_eq!(time_text(&at(12, 30, 0), opts(false, false, true)), "12:30 PM");
        // Without the AM/PM; a 24-hour clock has none to leave out.
        assert_eq!(time_text(&at(18, 52, 7), opts(false, false, false)), "6:52");
        assert_eq!(time_text(&at(0, 5, 7), opts(false, true, false)), "12:05:07");
        assert_eq!(time_text(&at(18, 52, 7), opts(true, false, false)), "18:52");
    }

    #[test]
    fn each_date_style_reads_as_its_example() {
        let t = at(18, 52, 0);
        assert_eq!(date_text(&t, DateStyle::Short), "Thu, Oct 16");
        assert_eq!(date_text(&t, DateStyle::Numeric), "10/16/2026");
        assert_eq!(date_text(&t, DateStyle::Long), "Thursday, October 16");
        assert_eq!(date_text(&t, DateStyle::WithYear), "Oct 16, 2026");
        assert_eq!(DateStyle::from_word("long"), DateStyle::Long);
        assert_eq!(DateStyle::from_word("nonsense"), DateStyle::Short);
    }

    #[test]
    fn the_next_wake_is_just_past_the_next_change() {
        let now = Duration::from_millis(125_250);
        assert!((until_next(now, false) - 54.755).abs() < 1e-9, "to 3:00 on the minute");
        assert!((until_next(now, true) - 0.755).abs() < 1e-9);
    }

    #[test]
    fn time_over_date_fits_the_default_bar() {
        // Text's line is 1.2 of its size; draw() wants 4px to spare.
        let bar = f64::from(crate::bar::Shape::default().height);
        assert!((TIME_SIZE + DATE_SIZE) * 1.2 <= bar - 4.0);
    }

    #[test]
    fn a_time_is_cut_into_its_digits_and_what_is_between() {
        assert_eq!(pieces("12:05 PM"), [("1", true), ("2", true), (":", false), ("0", true), ("5", true), (" PM", false)]);
        assert_eq!(pieces("PM"), [("PM", false)]);
        assert!(pieces("").is_empty());
    }

    #[test]
    fn the_clock_takes_the_same_room_whatever_the_time() {
        use lntrn_kit::desktop::Desktop;
        use lntrn_ui::testing::Harness;
        let mut h = Harness::new(1600.0, 200.0);
        h.theme = crate::bar::theme(&Desktop::default());
        let plate = Rect::new(Vec2::new(0.0, 100.0), Vec2::new(1600.0, 164.0));
        let mine = ClockOptions { seconds: true, ampm: false, ..ClockOptions::default() };
        for opts in [mine, ClockOptions { seconds: true, ..ClockOptions::default() }, ClockOptions { hour24: true, seconds: true, ..ClockOptions::default() }, ClockOptions { date: false, seconds: true, ..ClockOptions::default() }] {
            let mut rooms = Vec::new();
            h.frame(|ui| {
                for (hour, minute, second) in [(1, 11, 11), (8, 8, 8), (9, 59, 59), (10, 0, 0), (12, 47, 3), (23, 58, 40)] {
                    rooms.push(layout(ui, plate, opts, &at(hour, minute, second)).rect);
                }
                // Each digit in a cell of its own: a ticking time is as wide
                // whichever digits it shows.
                let style = TextStyle::new(28.0).bold();
                assert_eq!(tabular_w(ui, "1:11:11", &style), tabular_w(ui, "8:08:08", &style));
            });
            assert!(rooms.windows(2).all(|r| r[0] == r[1]), "{opts:?}: {rooms:?}");
        }
    }

    #[test]
    fn the_system_clock_reads() {
        let now = Local::now().expect("libc's local time");
        assert!((1..=12).contains(&now.month) && (1..=31).contains(&now.day) && now.hour < 24 && now.minute < 60);
    }
}
