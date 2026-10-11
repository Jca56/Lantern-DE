use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::config::Style;

/// What the lock screen shows right now, the same on every output.
pub struct Ui<'a> {
    /// Number of password characters entered (we never store the plaintext here).
    pub pw_len: usize,
    /// Error/status message shown under the field, e.g. "Incorrect password".
    pub error: Option<&'a str>,
    /// True while a PAM check is in flight (shows "Checking…" in an empty field).
    pub checking: bool,
    /// Caps Lock indicator.
    pub caps_lock: bool,
}

/// Smallest text the lock screen draws, in physical pixels.
const MIN_TEXT: f32 = 18.0;

/// Current local time as a libc `tm` struct.
fn now_local() -> libc::tm {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn time_string(tm: &libc::tm, clock_24h: bool) -> String {
    if clock_24h {
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    } else {
        // 12-hour the way the Command Center clock writes it: no AM/PM, and
        // the hour unpadded so it reads as a natural human time.
        let hour = if tm.tm_hour % 12 == 0 { 12 } else { tm.tm_hour % 12 };
        format!("{hour}:{:02}", tm.tm_min)
    }
}

fn date_string(tm: &libc::tm) -> String {
    let wd = WEEKDAYS.get(tm.tm_wday as usize).copied().unwrap_or("");
    let mo = MONTHS.get(tm.tm_mon as usize).copied().unwrap_or("");
    format!("{wd}, {mo} {}", tm.tm_mday)
}

/// Draw the full lock screen UI for one output. Coordinates are physical pixels.
/// The background image is drawn separately (texture pass) before this runs.
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ui: &Ui<'_>,
    style: &Style,
    w: f32,
    h: f32,
    sw: u32,
    sh: u32,
) {
    // Darkening scrim so the clock/field stay legible over any wallpaper.
    painter.rect_filled(
        Rect::new(0.0, 0.0, w, h),
        0.0,
        Color::rgba(0.0, 0.0, 0.0, style.scrim_opacity),
    );

    let white = Color::from_rgb8(245, 245, 245);
    let dim = Color::from_rgba8(230, 230, 230, 180);
    let center_x = w / 2.0;

    // ── Clock ────────────────────────────────────────────────────────────────
    let tm = now_local();
    let clock_size = (h * 0.16).clamp(96.0, 240.0);
    let clock = time_string(&tm, style.clock_24h);
    let cw = text.measure_width(&clock, clock_size);
    let clock_y = h * 0.18;
    text.queue(
        &clock,
        clock_size,
        center_x - cw / 2.0,
        clock_y,
        white,
        w,
        sw,
        sh,
    );

    let date_size = (clock_size * 0.26).clamp(28.0, 56.0);
    let date = date_string(&tm);
    let dw = text.measure_width(&date, date_size);
    let date_y = clock_y + clock_size * 1.05;
    text.queue(
        &date,
        date_size,
        center_x - dw / 2.0,
        date_y,
        dim,
        w,
        sw,
        sh,
    );

    // ── Password field ─────────────────────────────────────────────────────────
    let field_w = (w * 0.32).clamp(360.0, 640.0);
    let field_h = (h * 0.08).clamp(56.0, 96.0);
    let field_x = center_x - field_w / 2.0;
    let field_y = h * 0.62;
    let radius = field_h / 2.0;

    painter.rect_filled(
        Rect::new(field_x, field_y, field_w, field_h),
        radius,
        style.field_color,
    );
    // CSS-style border: outer edge aligns exactly with the fill's edge and
    // grows INWARD, so it sits flush on the pill with no gap. (rect_stroke_sdf
    // centers the stroke on the edge and extends outward, leaving a sliver.)
    if style.border_thickness > 0.0 {
        painter.rect_border(
            Rect::new(field_x, field_y, field_w, field_h),
            radius,
            style.border_thickness,
            style.border_color,
        );
    }

    // Typed characters win over "Checking…": typing stays live during a check,
    // and the dots are the only feedback that the keys are landing.
    let cy = field_y + field_h / 2.0;
    if ui.pw_len == 0 && ui.checking {
        let msg = "Checking…";
        let mw = text.measure_width(msg, date_size * 0.7);
        text.queue(
            msg,
            date_size * 0.7,
            center_x - mw / 2.0,
            cy - date_size * 0.35,
            dim,
            w,
            sw,
            sh,
        );
    } else if ui.pw_len == 0 {
        let placeholder = "Enter Password";
        let psize = field_h * 0.36;
        let pw = text.measure_width(placeholder, psize);
        text.queue(
            placeholder,
            psize,
            center_x - pw / 2.0,
            cy - psize * 0.55,
            Color::from_rgba8(220, 220, 220, 120),
            w,
            sw,
            sh,
        );
    } else {
        // Masked dots, centered.
        let dot_r = (field_h * 0.10).clamp(5.0, 10.0);
        let gap = dot_r * 3.0;
        let count = ui.pw_len.min(32);
        let total = (count as f32 - 1.0).max(0.0) * gap;
        let mut dx = center_x - total / 2.0;
        for _ in 0..count {
            painter.circle_filled(dx, cy, dot_r, style.dot_color);
            dx += gap;
        }
    }

    // ── Status line (error / caps lock) ─────────────────────────────────────────
    let status_y = field_y + field_h + field_h * 0.4;
    if let Some(err) = ui.error {
        // PAM's own messages can run long: shrink to fit rather than run off
        // both edges of the screen.
        let mut esize = (field_h * 0.32).clamp(22.0, 36.0);
        let mut ew = text.measure_width(err, esize);
        if ew > w * 0.92 {
            esize = (esize * w * 0.92 / ew).max(MIN_TEXT);
            ew = text.measure_width(err, esize);
        }
        text.queue(
            err,
            esize,
            center_x - ew / 2.0,
            status_y,
            Color::from_rgb8(240, 100, 100),
            w,
            sw,
            sh,
        );
    } else if ui.caps_lock {
        let msg = "Caps Lock is on";
        let esize = (field_h * 0.30).clamp(20.0, 32.0);
        let mw = text.measure_width(msg, esize);
        text.queue(
            msg,
            esize,
            center_x - mw / 2.0,
            status_y,
            Color::from_rgb8(240, 200, 90),
            w,
            sw,
            sh,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: i32, min: i32) -> libc::tm {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_hour = hour;
        tm.tm_min = min;
        tm
    }

    #[test]
    fn the_clock_in_24_hour() {
        assert_eq!(time_string(&at(0, 5), true), "00:05");
        assert_eq!(time_string(&at(14, 5), true), "14:05");
    }

    #[test]
    fn the_clock_in_12_hour() {
        assert_eq!(time_string(&at(0, 5), false), "12:05");
        assert_eq!(time_string(&at(9, 30), false), "9:30");
        assert_eq!(time_string(&at(12, 0), false), "12:00");
        assert_eq!(time_string(&at(14, 5), false), "2:05");
        assert_eq!(time_string(&at(23, 59), false), "11:59");
    }
}
