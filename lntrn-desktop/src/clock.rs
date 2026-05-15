//! Big clock widget rendered top-center on the desktop.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

/// Vertical offset from screen top in logical pixels.
const TOP_OFFSET: f32 = 80.0;
const TIME_FONT_PX: f32 = 170.0;
const DOW_FONT_PX: f32 = 44.0;
const DATE_FONT_PX: f32 = 34.0;

const TEXT: Color = Color {
    r: 0.96,
    g: 0.94,
    b: 0.88,
    a: 1.0,
};
const SHADOW: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.75,
};

/// Format the wall clock as ("h:mm", "Thursday", "May 14, 2026"). 12-hour
/// with no AM/PM, just the numbers, as requested.
pub fn now_strings() -> (String, String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0));
    let local = local_components(now.as_secs());
    let hour12 = match local.hour % 12 {
        0 => 12,
        h => h,
    };
    let time = format!("{}:{:02}", hour12, local.minute);
    let dow = day_of_week_name(local.year, local.month, local.day).to_string();
    let date = format!(
        "{} {}, {}",
        month_name(local.month),
        local.day,
        local.year,
    );
    (time, dow, date)
}

pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    surface_w: u32,
    surface_h: u32,
    scale: f32,
) {
    let (time_s, dow_s, date_s) = now_strings();
    let sw = surface_w as f32;
    let _sh = surface_h;

    // Time — huge, centered.
    let time_w = text.measure_width(&time_s, TIME_FONT_PX * scale);
    let time_x = (sw - time_w) * 0.5;
    let time_y = TOP_OFFSET * scale;
    queue_with_shadow(
        text,
        &time_s,
        TIME_FONT_PX * scale,
        time_x,
        time_y,
        sw,
        surface_w,
        surface_h,
    );

    // Day of week — beneath the time.
    let dow_y = time_y + TIME_FONT_PX * scale * 1.05;
    let dow_w = text.measure_width(&dow_s, DOW_FONT_PX * scale);
    let dow_x = (sw - dow_w) * 0.5;
    queue_with_shadow(
        text,
        &dow_s,
        DOW_FONT_PX * scale,
        dow_x,
        dow_y,
        sw,
        surface_w,
        surface_h,
    );

    // Date — beneath the day of week.
    let date_y = dow_y + DOW_FONT_PX * scale * 1.25;
    let date_w = text.measure_width(&date_s, DATE_FONT_PX * scale);
    let date_x = (sw - date_w) * 0.5;
    queue_with_shadow(
        text,
        &date_s,
        DATE_FONT_PX * scale,
        date_x,
        date_y,
        sw,
        surface_w,
        surface_h,
    );

    // No background plate — text shadow keeps it legible against any wallpaper.
    let _ = painter;
    let _ = Rect::new(0.0, 0.0, 0.0, 0.0);
}

fn queue_with_shadow(
    text: &mut TextRenderer,
    s: &str,
    font_size: f32,
    x: f32,
    y: f32,
    max_w: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let off = (font_size * 0.04).max(1.5);
    // Drop shadow — render first so the main glyph paints on top.
    text.queue(s, font_size, x + off, y + off, SHADOW, max_w, surface_w, surface_h);
    text.queue(s, font_size, x, y, TEXT, max_w, surface_w, surface_h);
}

// ── Calendar math ───────────────────────────────────────────────────────────

struct DateTime {
    year: i32,
    month: u32, // 1..=12
    day: u32,   // 1..=31
    hour: u32,
    minute: u32,
}

/// Convert a unix timestamp (seconds) to local broken-down time using libc.
fn local_components(secs: u64) -> DateTime {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t: libc::time_t = secs as libc::time_t;
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    DateTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon as u32) + 1,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
    }
}

fn day_of_week_name(year: i32, month: u32, day: u32) -> &'static str {
    // Zeller's congruence (0 = Saturday, 1 = Sunday, …, 6 = Friday)
    let (y, m) = if month < 3 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let k = y.rem_euclid(100) as i32;
    let j = y.div_euclid(100) as i32;
    let h = (day as i32 + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    // h: 0=Sat, 1=Sun, 2=Mon, …, 6=Fri
    match h {
        0 => "Saturday",
        1 => "Sunday",
        2 => "Monday",
        3 => "Tuesday",
        4 => "Wednesday",
        5 => "Thursday",
        _ => "Friday",
    }
}

fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "",
    }
}
