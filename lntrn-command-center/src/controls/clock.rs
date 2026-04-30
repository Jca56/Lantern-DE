//! Time / Date / Calendar control tile.
//!
//! Inline tile shows current time on top + date underneath. Click-expand
//! shows a mini month calendar grid with today highlighted.
//!
//! No background polling — `chrono::Local::now()` is cheap enough to
//! call once per render frame, and the panel only renders while visible.
//! When the user wants live seconds we can add a 1Hz timer; for now we
//! show HH:MM and rely on the next open to update.

use chrono::{Datelike, Local, NaiveDate, Timelike};
use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

/// White text — user prefers white over the Studio tan everywhere.
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
/// Accent gold #C8860A — used for "today" highlight in the calendar.
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

const SECONDARY_ALPHA: f32 = 0.62;

fn text_color(alpha: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(alpha)
}
fn accent_color(alpha: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
}

pub struct Clock;

impl Clock {
    pub fn new() -> Self {
        Self
    }
}

// ── Inline tile ─────────────────────────────────────────────────────────────

const TIME_FONT: f32 = 38.0;
const DATE_FONT: f32 = 16.0;
const TIME_DATE_GAP: f32 = 1.0;

/// Logical px the clock tile asks for in the row layout. Sized to the
/// actual content width — "12:34" at 38pt is roughly 100pt wide, the
/// date "Wed Apr 30" at 16pt is similar. Anything wider just leaves
/// dead space between the clock and the next tile.
pub const TILE_WIDTH: f32 = 110.0;

pub fn draw_inline(
    _painter: &mut Painter,
    text: &mut TextRenderer,
    _clock: &Clock,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let now = Local::now();
    let time_str = format_time(now.hour(), now.minute());
    let date_str = format_short_date(now.weekday(), now.month(), now.day());

    let time_font = TIME_FONT * scale;
    let date_font = DATE_FONT * scale;
    let gap = TIME_DATE_GAP * scale;
    let stack_h = time_font + gap + date_font;
    let cy = layout.y + (layout.h - stack_h) / 2.0;

    // Time on top, left-aligned to the tile's left edge. Date centered
    // horizontally below it (within the bounds of the time string, not
    // the whole tile slot).
    let time_w = text.measure_width(&time_str, time_font);
    let date_w = text.measure_width(&date_str, date_font);

    text.queue(
        &time_str,
        time_font,
        layout.x,
        cy,
        text_color(alpha),
        layout.w,
        surface_w,
        surface_h,
    );
    text.queue(
        &date_str,
        date_font,
        layout.x + (time_w - date_w) / 2.0,
        cy + time_font + gap,
        text_color(SECONDARY_ALPHA * alpha),
        layout.w,
        surface_w,
        surface_h,
    );
}

// ── Click-expand calendar ───────────────────────────────────────────────────

/// Logical px reserved for the calendar grid when expanded.
#[allow(dead_code)] // not used since the panel mode swap; retained for reference.
pub const EXPANDED_HEIGHT: f32 = 280.0;

const CAL_HEADER_FONT: f32 = 22.0;
const CAL_WEEKDAY_FONT: f32 = 14.0;
const CAL_DAY_FONT: f32 = 18.0;
const CAL_TOP_PAD: f32 = 16.0;
const CAL_GRID_TOP_PAD: f32 = 12.0;
const CAL_CELL_GAP: f32 = 4.0;
/// Days-of-week header (single letters keep the row narrow).
const WEEKDAY_LABELS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

/// Draw the calendar grid below the controls row when this tile is expanded.
/// `top_y` is the physical-pixel y at which the expansion starts.
/// Returns the bottom y of the rendered area.
pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    _clock: &Clock,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let now = Local::now();
    let today = NaiveDate::from_ymd_opt(now.year(), now.month(), now.day())
        .unwrap_or_else(|| NaiveDate::default());

    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    // Header: "April 2026"
    let header_font = CAL_HEADER_FONT * scale;
    let header = format!("{} {}", month_name(now.month()), now.year());
    let header_w = text.measure_width(&header, header_font);
    let mut y = top_y + CAL_TOP_PAD * scale;
    text.queue(
        &header,
        header_font,
        inner_x + (inner_w - header_w) / 2.0,
        y,
        text_color(alpha),
        inner_w,
        surface_w,
        surface_h,
    );
    y += header_font + CAL_GRID_TOP_PAD * scale;

    // Weekday letters across the top of the grid.
    let weekday_font = CAL_WEEKDAY_FONT * scale;
    let cell_gap = CAL_CELL_GAP * scale;
    let cell_w = (inner_w - cell_gap * 6.0) / 7.0;
    let cell_h = cell_w; // square cells
    for (i, label) in WEEKDAY_LABELS.iter().enumerate() {
        let cx = inner_x + i as f32 * (cell_w + cell_gap);
        let lw = text.measure_width(label, weekday_font);
        text.queue(
            label,
            weekday_font,
            cx + (cell_w - lw) / 2.0,
            y,
            text_color(SECONDARY_ALPHA * alpha),
            cell_w,
            surface_w,
            surface_h,
        );
    }
    y += weekday_font + cell_gap;

    // Day grid. Sunday = column 0 (matches the WEEKDAY_LABELS order).
    let first = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap_or_else(|| NaiveDate::default());
    let first_col = first.weekday().num_days_from_sunday() as usize;
    let days_in_month = days_in_month(now.year(), now.month());

    let day_font = CAL_DAY_FONT * scale;

    for d in 1..=days_in_month {
        let idx = first_col + d as usize - 1;
        let row = idx / 7;
        let col = idx % 7;
        let cx = inner_x + col as f32 * (cell_w + cell_gap);
        let cy = y + row as f32 * (cell_h + cell_gap);
        let cell_rect = Rect::new(cx, cy, cell_w, cell_h);

        let is_today = today.day() == d
            && today.month() == now.month()
            && today.year() == now.year();

        if is_today {
            // Round-rect filled with accent gold + white text.
            painter.rect_filled(
                cell_rect,
                cell_w * 0.4,
                accent_color(0.65 * alpha),
            );
        }

        let day_str = d.to_string();
        let day_w = text.measure_width(&day_str, day_font);
        let day_color = if is_today {
            // Strong white-ish for contrast inside the gold cell.
            Color::rgba(1.0, 1.0, 1.0, alpha)
        } else {
            text_color(alpha)
        };
        let day_x = cx + (cell_w - day_w) / 2.0;
        let day_y = cy + (cell_h - day_font) / 2.0;
        text.queue(
            &day_str,
            day_font,
            day_x,
            day_y,
            day_color,
            cell_w,
            surface_w,
            surface_h,
        );
    }

    let total_rows = ((first_col + days_in_month as usize + 6) / 7) as f32;
    let grid_h = total_rows * (cell_h + cell_gap);
    y + grid_h
}

// ── Formatting helpers ──────────────────────────────────────────────────────

fn format_time(hour: u32, minute: u32) -> String {
    // 12-hour clock without AM/PM (user preference). 12am → "12:00",
    // 1pm → "1:00", etc. Hour is *not* zero-padded so it reads as a
    // natural human time.
    let h12 = if hour == 0 {
        12
    } else if hour <= 12 {
        hour
    } else {
        hour - 12
    };
    format!("{}:{:02}", h12, minute)
}

fn format_short_date(weekday: chrono::Weekday, month: u32, day: u32) -> String {
    format!("{} {} {}", weekday_short(weekday), month_short(month), day)
}

fn weekday_short(w: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    match w {
        Mon => "Mon",
        Tue => "Tue",
        Wed => "Wed",
        Thu => "Thu",
        Fri => "Fri",
        Sat => "Sat",
        Sun => "Sun",
    }
}

fn month_short(m: u32) -> &'static str {
    match m {
        1 => "Jan", 2 => "Feb", 3 => "Mar", 4 => "Apr",
        5 => "May", 6 => "Jun", 7 => "Jul", 8 => "Aug",
        9 => "Sep", 10 => "Oct", 11 => "Nov", 12 => "Dec",
        _ => "?",
    }
}

fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "?",
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    let first = NaiveDate::from_ymd_opt(year, month, 1);
    match (first, next_month) {
        (Some(f), Some(n)) => (n - f).num_days() as u32,
        _ => 30,
    }
}
