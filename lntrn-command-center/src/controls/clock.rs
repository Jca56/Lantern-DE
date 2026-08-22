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
use crate::controls::toolbar::{ClockOpts, DatePos};

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

/// Click target inside the calendar view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarHit {
    PrevMonth,
    NextMonth,
    Day(NaiveDate),
}

/// Click target inside the day-detail panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailHit {
    /// Close the detail panel (× button or back arrow).
    Close,
    /// Reveal the inline "add event" text field.
    OpenAddInput,
    /// One of the existing event rows. `idx` matches the day's event order.
    EventRow(usize),
}

pub struct Clock {
    /// Offset from the current month, in months. 0 = current,
    /// -1 = previous, +1 = next, etc. Reset to 0 whenever the panel
    /// re-opens (handled by the caller).
    pub month_offset: i32,
    /// When `Some`, the day-detail panel is shown for this date and
    /// the calendar grid is squeezed to the right half. `None` =
    /// full-width calendar.
    pub selected_day: Option<NaiveDate>,
    /// When `Some`, the inline "Add event" text field is open and
    /// holds the user's in-progress title. Tied to whatever day is
    /// currently `selected_day`.
    pub add_event_input: Option<crate::search::input::Input>,
    /// Right-click menu over an event row. Stored as (date, idx_in_date).
    pub event_menu: Option<EventContextMenu>,
}

#[derive(Debug, Clone)]
pub struct EventContextMenu {
    pub date: NaiveDate,
    pub idx_in_date: usize,
    /// Anchor in physical pixels — top-left of where the menu wants to draw.
    pub anchor_x: f32,
    pub anchor_y: f32,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            month_offset: 0,
            selected_day: None,
            add_event_input: None,
            event_menu: None,
        }
    }

    pub fn next_month(&mut self) {
        self.month_offset += 1;
    }
    pub fn prev_month(&mut self) {
        self.month_offset -= 1;
    }
    pub fn reset_month(&mut self) {
        self.month_offset = 0;
        self.selected_day = None;
        self.add_event_input = None;
        self.event_menu = None;
    }

    /// The (year, month) currently being viewed.
    pub fn viewed_year_month(&self) -> (i32, u32) {
        let now = Local::now();
        let mut year = now.year();
        let mut month = now.month() as i32 + self.month_offset;
        while month <= 0 {
            month += 12;
            year -= 1;
        }
        while month > 12 {
            month -= 12;
            year += 1;
        }
        (year, month as u32)
    }
}

// ── Inline tile ─────────────────────────────────────────────────────────────

const TIME_FONT: f32 = 38.0;
const DATE_FONT: f32 = 16.0;
const TIME_DATE_GAP: f32 = 1.0;

/// Optical-centering rise as a fraction of font size. cosmic-text lays
/// glyphs low within their 1.2× line box, so plain `(h - font)/2`
/// centering renders text bottom-heavy; lifting by this fraction puts the
/// ink on the row's true vertical center.
const V_CENTER_RISE: f32 = 0.12;

/// Default logical width (date below, no seconds). Used as the generic
/// `TileId::logical_width` fallback; the live width comes from
/// [`tile_width`] which accounts for the clock's options.
pub const TILE_WIDTH: f32 = 110.0;

/// Approximate logical widths used for slot sizing (we can't measure text
/// without a `TextRenderer` here, so these are generous estimates).
const TIME_W: f32 = 100.0;
const TIME_SECONDS_W: f32 = 150.0;
const DATE_W: f32 = 100.0;
/// Gap between time and date when the date is beside the time.
const DATE_SIDE_GAP: f32 = 8.0;

/// Logical width the clock asks for given its options.
pub fn tile_width(opts: &ClockOpts) -> f32 {
    let time_w = if opts.seconds { TIME_SECONDS_W } else { TIME_W };
    if !opts.show_date {
        return time_w;
    }
    match opts.date_pos {
        DatePos::Below => time_w.max(DATE_W),
        DatePos::Left | DatePos::Right => time_w + DATE_SIDE_GAP + DATE_W,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    _painter: &mut Painter,
    text: &mut TextRenderer,
    _clock: &Clock,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    lit: bool,
    opts: &ClockOpts,
) {
    let now = Local::now();
    let time_str = format_time(
        now.hour(),
        now.minute(),
        now.second(),
        opts.hour24,
        opts.seconds,
    );

    let time_font = TIME_FONT * scale;
    let date_font = DATE_FONT * scale;
    let primary = if lit {
        accent_color(alpha)
    } else {
        text_color(alpha)
    };
    let secondary = if lit {
        accent_color(SECONDARY_ALPHA * alpha)
    } else {
        text_color(SECONDARY_ALPHA * alpha)
    };
    let time_w = text.measure_width(&time_str, time_font);

    // Date hidden → just center the time in the tile.
    if !opts.show_date {
        let tx = layout.x + (layout.w - time_w) / 2.0;
        let ty = layout.y + (layout.h - time_font) / 2.0 - time_font * V_CENTER_RISE;
        text.queue(
            &time_str, time_font, tx, ty, primary, layout.w, surface_w, surface_h,
        );
        return;
    }

    let date_str = format_short_date(now.weekday(), now.month(), now.day());
    let date_w = text.measure_width(&date_str, date_font);

    match opts.date_pos {
        DatePos::Below => {
            let gap = TIME_DATE_GAP * scale;
            let stack_h = time_font + gap + date_font;
            let cy = layout.y + (layout.h - stack_h) / 2.0 - time_font * V_CENTER_RISE;
            let group_w = time_w.max(date_w);
            let gx = layout.x + (layout.w - group_w) / 2.0;
            text.queue(
                &time_str,
                time_font,
                gx + (group_w - time_w) / 2.0,
                cy,
                primary,
                layout.w,
                surface_w,
                surface_h,
            );
            text.queue(
                &date_str,
                date_font,
                gx + (group_w - date_w) / 2.0,
                cy + time_font + gap,
                secondary,
                layout.w,
                surface_w,
                surface_h,
            );
        }
        DatePos::Left | DatePos::Right => {
            let side_gap = DATE_SIDE_GAP * scale;
            let total_w = time_w + side_gap + date_w;
            let gx = layout.x + (layout.w - total_w) / 2.0;
            let ty = layout.y + (layout.h - time_font) / 2.0 - time_font * V_CENTER_RISE;
            let dy = layout.y + (layout.h - date_font) / 2.0 - date_font * V_CENTER_RISE;
            let (time_x, date_x) = if opts.date_pos == DatePos::Left {
                (gx + date_w + side_gap, gx)
            } else {
                (gx, gx + time_w + side_gap)
            };
            text.queue(
                &time_str, time_font, time_x, ty, primary, layout.w, surface_w, surface_h,
            );
            text.queue(
                &date_str, date_font, date_x, dy, secondary, layout.w, surface_w, surface_h,
            );
        }
    }
}

// ── Click-expand calendar ───────────────────────────────────────────────────

/// Logical px reserved for the calendar grid when expanded.
#[allow(dead_code)] // not used since the panel mode swap; retained for reference.
pub const EXPANDED_HEIGHT: f32 = 280.0;

const CAL_HEADER_FONT: f32 = 34.0;
const CAL_WEEKDAY_FONT: f32 = 22.0;
const CAL_DAY_FONT: f32 = 34.0;
/// Font for the event title rendered inside a day cell. Small enough
/// to fit a short title under the day number without truncating too
/// hard at the cell width cap.
const CAL_EVENT_FONT: f32 = 18.0;
/// Padding inside a day cell (logical px) — keeps the day number
/// from kissing the cell border.
const CAL_CELL_PAD: f32 = 6.0;
/// Vertical gap between the day number and the event title beneath it.
const CAL_EVENT_GAP: f32 = 2.0;
const CAL_TOP_PAD: f32 = 16.0;
const CAL_GRID_TOP_PAD: f32 = 12.0;
const CAL_CELL_GAP: f32 = 4.0;
/// Gap (logical px) between the month title and each side-arrow.
const ARROW_GAP: f32 = 32.0;
/// Maximum cell size (logical px). Caps the grid so it doesn't fill
/// the full panel width with comically tall cells. The 7-cell row
/// stays centered when the cap kicks in.
const CAL_MAX_CELL: f32 = 90.0;
/// Days-of-week header (single letters keep the row narrow).
const WEEKDAY_LABELS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

// ── Day-detail panel (side panel that appears when a day is selected) ──────

/// Width fraction of the content area the detail panel claims when open.
const DETAIL_W_FRAC: f32 = 0.40;
/// Padding inside the detail panel (logical px).
const DETAIL_PAD: f32 = 16.0;
/// Header font (the date the panel is for).
const DETAIL_HEADER_FONT: f32 = 26.0;
/// Event row font.
const DETAIL_EVENT_FONT: f32 = 18.0;
/// Height of one event row (logical px).
const DETAIL_EVENT_ROW_H: f32 = 36.0;
/// Vertical gap between header and the event list.
const DETAIL_LIST_TOP_GAP: f32 = 16.0;
/// Vertical gap between rows.
const DETAIL_ROW_GAP: f32 = 4.0;
/// Vertical gap above the "+ Add event" / input row.
const DETAIL_ADD_TOP_GAP: f32 = 12.0;
/// Height of the "+ Add event" button / input row.
const DETAIL_ADD_ROW_H: f32 = 40.0;
/// Close-button (×) hit area (logical px square).
const DETAIL_CLOSE_SIZE: f32 = 28.0;

/// Draw the calendar grid below the controls row when this tile is expanded.
/// `top_y` is the physical-pixel y at which the expansion starts.
/// Returns the bottom y of the rendered area.
pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    clock: &Clock,
    events: &crate::controls::events::Events,
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
    let (view_year, view_month) = clock.viewed_year_month();

    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    // Layout: when a day is selected, slice the content area in two —
    // detail panel on the left, calendar squeezed on the right. When
    // nothing is selected, calendar uses the whole inner width.
    let detail_w = if clock.selected_day.is_some() {
        inner_w * DETAIL_W_FRAC
    } else {
        0.0
    };
    let detail_gap = if clock.selected_day.is_some() {
        16.0 * scale
    } else {
        0.0
    };
    let cal_x = inner_x + detail_w + detail_gap;
    let cal_w = inner_w - detail_w - detail_gap;

    // Render the detail panel first so its background sits underneath
    // anything that overlaps (it shouldn't, but order is cheap).
    if let Some(date) = clock.selected_day {
        draw_detail_panel(
            painter,
            text,
            clock,
            events,
            date,
            inner_x,
            top_y,
            detail_w,
            panel.h - (top_y - panel.y),
            scale,
            alpha,
            surface_w,
            surface_h,
        );
    }

    // Header: "‹ April 2026 ›"
    let header_font = CAL_HEADER_FONT * scale;
    let header = format!("{} {}", month_name(view_month), view_year);
    let header_w = text.measure_width(&header, header_font);
    let mut y = top_y + CAL_TOP_PAD * scale;

    let header_x = cal_x + (cal_w - header_w) / 2.0;
    text.queue(
        &header,
        header_font,
        header_x,
        y,
        text_color(alpha),
        cal_w,
        surface_w,
        surface_h,
    );

    // Arrows flanking the title. We measure the chevrons so they
    // visually balance and reserve a hit area around each.
    let arrow_font = header_font;
    let prev_str = "‹";
    let next_str = "›";
    let prev_w = text.measure_width(prev_str, arrow_font);
    let next_w = text.measure_width(next_str, arrow_font);
    let arrow_gap = ARROW_GAP * scale;
    let prev_x = header_x - arrow_gap - prev_w;
    let next_x = header_x + header_w + arrow_gap;
    text.queue(
        prev_str,
        arrow_font,
        prev_x,
        y,
        text_color(SECONDARY_ALPHA * alpha),
        prev_w,
        surface_w,
        surface_h,
    );
    text.queue(
        next_str,
        arrow_font,
        next_x,
        y,
        text_color(SECONDARY_ALPHA * alpha),
        next_w,
        surface_w,
        surface_h,
    );
    y += header_font + CAL_GRID_TOP_PAD * scale;

    // Weekday letters across the top of the grid.
    let weekday_font = CAL_WEEKDAY_FONT * scale;
    let cell_gap = CAL_CELL_GAP * scale;
    let raw_cell_w = (cal_w - cell_gap * 6.0) / 7.0;
    let max_cell = CAL_MAX_CELL * scale;
    let cell_w = raw_cell_w.min(max_cell);
    let cell_h = cell_w; // square cells
                         // Center the grid horizontally when capped.
    let grid_w = cell_w * 7.0 + cell_gap * 6.0;
    let grid_x = cal_x + (cal_w - grid_w) / 2.0;
    for (i, label) in WEEKDAY_LABELS.iter().enumerate() {
        let cx = grid_x + i as f32 * (cell_w + cell_gap);
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
    let first =
        NaiveDate::from_ymd_opt(view_year, view_month, 1).unwrap_or_else(|| NaiveDate::default());
    let first_col = first.weekday().num_days_from_sunday() as usize;
    let days_in_month = days_in_month(view_year, view_month);

    let day_font = CAL_DAY_FONT * scale;
    let event_font = CAL_EVENT_FONT * scale;
    let cell_pad = CAL_CELL_PAD * scale;
    let event_gap = CAL_EVENT_GAP * scale;

    for d in 1..=days_in_month {
        let idx = first_col + d as usize - 1;
        let row = idx / 7;
        let col = idx % 7;
        let cx = grid_x + col as f32 * (cell_w + cell_gap);
        let cy = y + row as f32 * (cell_h + cell_gap);
        let cell_rect = Rect::new(cx, cy, cell_w, cell_h);

        let is_today = today.day() == d && today.month() == view_month && today.year() == view_year;
        let cell_date = NaiveDate::from_ymd_opt(view_year, view_month, d);
        let is_selected = clock.selected_day == cell_date;

        if is_today {
            painter.rect_filled(cell_rect, cell_w * 0.18, accent_color(0.65 * alpha));
        }
        if is_selected && !is_today {
            // Subtle accent ring for selected (but not today) days so
            // the user can always see what the detail panel is about.
            painter.rect_stroke_sdf(
                cell_rect,
                cell_w * 0.18,
                2.0 * scale,
                accent_color(0.85 * alpha),
            );
        }

        let day_str = d.to_string();
        let day_w = text.measure_width(&day_str, day_font);
        let day_color = if is_today {
            Color::rgba(1.0, 1.0, 1.0, alpha)
        } else {
            text_color(alpha)
        };
        // Day number sits at the top-center of the cell so we have
        // room beneath it for an event title.
        let day_x = cx + (cell_w - day_w) / 2.0;
        let day_y = cy + cell_pad;
        text.queue(
            &day_str, day_font, day_x, day_y, day_color, cell_w, surface_w, surface_h,
        );

        // Event title underneath. Show first event's title; if more
        // exist, show a "+N" suffix on the title line so all info is
        // visible at a glance without cluttering the cell.
        if let Some(date) = cell_date {
            let day_events: Vec<&str> = events.on_date(date).map(|e| e.title.as_str()).collect();
            if !day_events.is_empty() {
                let extra = day_events.len() - 1;
                let label_owned;
                let label = if extra > 0 {
                    label_owned = format!("{} +{}", day_events[0], extra);
                    label_owned.as_str()
                } else {
                    day_events[0]
                };
                let inner_w = cell_w - cell_pad * 2.0;
                let truncated = truncate_to_width(text, label, event_font, inner_w);
                let lw = text.measure_width(&truncated, event_font);
                let event_color = if is_today {
                    Color::rgba(1.0, 1.0, 1.0, alpha)
                } else {
                    text_color(SECONDARY_ALPHA * alpha)
                };
                let ex = cx + (cell_w - lw) / 2.0;
                let ey = day_y + day_font + event_gap;
                text.queue(
                    &truncated,
                    event_font,
                    ex,
                    ey,
                    event_color,
                    inner_w,
                    surface_w,
                    surface_h,
                );
            }
        }
    }

    let total_rows = ((first_col + days_in_month as usize + 6) / 7) as f32;
    let grid_h = total_rows * (cell_h + cell_gap);
    y + grid_h
}

/// Draw the day-detail side panel: header (e.g. "April 30") with an
/// × close button, list of events for that day, and either a "+ Add
/// event" button or an inline text input. All coordinates in physical
/// pixels.
#[allow(clippy::too_many_arguments)]
fn draw_detail_panel(
    painter: &mut Painter,
    text: &mut TextRenderer,
    clock: &Clock,
    events: &crate::controls::events::Events,
    date: NaiveDate,
    x: f32,
    top_y: f32,
    w: f32,
    h: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    use chrono::Datelike;

    let pad = DETAIL_PAD * scale;
    let header_font = DETAIL_HEADER_FONT * scale;
    let event_font = DETAIL_EVENT_FONT * scale;
    let row_h = DETAIL_EVENT_ROW_H * scale;
    let row_gap = DETAIL_ROW_GAP * scale;
    let close_size = DETAIL_CLOSE_SIZE * scale;

    // Soft background plate so the detail panel reads as a separate
    // surface beside the calendar.
    let panel_rect = Rect::new(x, top_y, w, h);
    painter.rect_filled(
        panel_rect,
        14.0 * scale,
        Color::rgba(1.0, 1.0, 1.0, 0.04 * alpha),
    );

    // Header: "Apr 30" (or whichever short form fits)
    let header = format!("{} {}", month_short(date.month()), date.day());
    let mut y = top_y + pad;
    text.queue(
        &header,
        header_font,
        x + pad,
        y,
        text_color(alpha),
        w - pad * 2.0 - close_size,
        surface_w,
        surface_h,
    );

    // × close button — top-right of the panel.
    let close_x = x + w - pad - close_size;
    let close_y = y;
    let close_str = "×";
    let close_w = text.measure_width(close_str, header_font);
    text.queue(
        close_str,
        header_font,
        close_x + (close_size - close_w) / 2.0,
        close_y,
        text_color(SECONDARY_ALPHA * alpha),
        close_size,
        surface_w,
        surface_h,
    );
    y += header_font + DETAIL_LIST_TOP_GAP * scale;

    // Event list.
    let day_events: Vec<&str> = events.on_date(date).map(|e| e.title.as_str()).collect();
    if day_events.is_empty() {
        text.queue(
            "No events",
            event_font,
            x + pad,
            y,
            text_color(SECONDARY_ALPHA * alpha),
            w - pad * 2.0,
            surface_w,
            surface_h,
        );
        y += event_font + row_gap;
    } else {
        for title in &day_events {
            // Row plate (subtle) for visual separation + hit-feedback.
            let row_rect = Rect::new(x + pad * 0.5, y, w - pad, row_h);
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                Color::rgba(1.0, 1.0, 1.0, 0.04 * alpha),
            );
            let truncated = truncate_to_width(text, title, event_font, w - pad * 2.0);
            text.queue(
                &truncated,
                event_font,
                x + pad,
                y + (row_h - event_font) / 2.0,
                text_color(alpha),
                w - pad * 2.0,
                surface_w,
                surface_h,
            );
            y += row_h + row_gap;
        }
    }

    // "+ Add event" button or inline input.
    y += DETAIL_ADD_TOP_GAP * scale;
    let add_h = DETAIL_ADD_ROW_H * scale;
    let add_rect = Rect::new(x + pad * 0.5, y, w - pad, add_h);

    if let Some(input) = clock.add_event_input.as_ref() {
        // Active text field.
        painter.rect_filled(
            add_rect,
            10.0 * scale,
            Color::rgba(1.0, 1.0, 1.0, 0.06 * alpha),
        );
        painter.rect_stroke_sdf(
            add_rect,
            10.0 * scale,
            1.5 * scale,
            accent_color(0.6 * alpha),
        );
        let query = input.query();
        let display = if query.is_empty() {
            "Type a title, Enter to save…".to_string()
        } else {
            query.to_string()
        };
        let display_color = if query.is_empty() {
            text_color(SECONDARY_ALPHA * alpha)
        } else {
            text_color(alpha)
        };
        text.queue(
            &display,
            event_font,
            x + pad,
            y + (add_h - event_font) / 2.0,
            display_color,
            w - pad * 2.0,
            surface_w,
            surface_h,
        );
    } else {
        painter.rect_filled(
            add_rect,
            10.0 * scale,
            Color::rgba(1.0, 1.0, 1.0, 0.04 * alpha),
        );
        let label = "+  Add event";
        let lw = text.measure_width(label, event_font);
        text.queue(
            label,
            event_font,
            x + pad + (w - pad * 2.0 - lw) / 2.0,
            y + (add_h - event_font) / 2.0,
            text_color(SECONDARY_ALPHA * alpha),
            w - pad * 2.0,
            surface_w,
            surface_h,
        );
    }
}

/// Hit-test a click against the calendar's interactive elements
/// (month-nav arrows + day cells). Mirrors `draw_view`'s layout math
/// so hit-rects line up with what the user sees.
pub fn hit_test_view(
    panel: Rect,
    top_y: f32,
    scale: f32,
    clock: &Clock,
    text: &mut TextRenderer,
    phys_x: f32,
    phys_y: f32,
) -> Option<CalendarHit> {
    let (view_year, view_month) = clock.viewed_year_month();
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    // Same layout split as draw_view.
    let detail_w = if clock.selected_day.is_some() {
        inner_w * DETAIL_W_FRAC
    } else {
        0.0
    };
    let detail_gap = if clock.selected_day.is_some() {
        16.0 * scale
    } else {
        0.0
    };
    let cal_x = inner_x + detail_w + detail_gap;
    let cal_w = inner_w - detail_w - detail_gap;

    let header_font = CAL_HEADER_FONT * scale;
    let header = format!("{} {}", month_name(view_month), view_year);
    let header_w = text.measure_width(&header, header_font);
    let mut y = top_y + CAL_TOP_PAD * scale;
    let header_x = cal_x + (cal_w - header_w) / 2.0;

    let prev_w = text.measure_width("‹", header_font);
    let next_w = text.measure_width("›", header_font);
    let arrow_gap = ARROW_GAP * scale;
    let prev_x = header_x - arrow_gap - prev_w;
    let next_x = header_x + header_w + arrow_gap;

    // Generous click padding so the arrows are easy targets.
    let pad_h = 12.0 * scale;
    let pad_v = 10.0 * scale;
    let prev_rect = Rect::new(
        prev_x - pad_h,
        y - pad_v,
        prev_w + pad_h * 2.0,
        header_font + pad_v * 2.0,
    );
    let next_rect = Rect::new(
        next_x - pad_h,
        y - pad_v,
        next_w + pad_h * 2.0,
        header_font + pad_v * 2.0,
    );

    if phys_x >= prev_rect.x
        && phys_x <= prev_rect.x + prev_rect.w
        && phys_y >= prev_rect.y
        && phys_y <= prev_rect.y + prev_rect.h
    {
        return Some(CalendarHit::PrevMonth);
    }
    if phys_x >= next_rect.x
        && phys_x <= next_rect.x + next_rect.w
        && phys_y >= next_rect.y
        && phys_y <= next_rect.y + next_rect.h
    {
        return Some(CalendarHit::NextMonth);
    }

    // Day cells.
    y += header_font + CAL_GRID_TOP_PAD * scale;
    let weekday_font = CAL_WEEKDAY_FONT * scale;
    let cell_gap = CAL_CELL_GAP * scale;
    let raw_cell_w = (cal_w - cell_gap * 6.0) / 7.0;
    let max_cell = CAL_MAX_CELL * scale;
    let cell_w = raw_cell_w.min(max_cell);
    let cell_h = cell_w;
    let grid_w = cell_w * 7.0 + cell_gap * 6.0;
    let grid_x = cal_x + (cal_w - grid_w) / 2.0;
    y += weekday_font + cell_gap;

    let first =
        NaiveDate::from_ymd_opt(view_year, view_month, 1).unwrap_or_else(|| NaiveDate::default());
    let first_col = first.weekday().num_days_from_sunday() as usize;
    let dim = days_in_month(view_year, view_month);
    for d in 1..=dim {
        let idx = first_col + d as usize - 1;
        let row = idx / 7;
        let col = idx % 7;
        let cx = grid_x + col as f32 * (cell_w + cell_gap);
        let cy = y + row as f32 * (cell_h + cell_gap);
        if phys_x >= cx && phys_x <= cx + cell_w && phys_y >= cy && phys_y <= cy + cell_h {
            if let Some(date) = NaiveDate::from_ymd_opt(view_year, view_month, d) {
                return Some(CalendarHit::Day(date));
            }
        }
    }

    None
}

// ── Event right-click context menu ──────────────────────────────────────────

/// Width / row metrics for the event context menu (logical px).
const EVENT_MENU_W: f32 = 160.0;
const EVENT_MENU_ROW_H: f32 = 36.0;
const EVENT_MENU_PAD_V: f32 = 6.0;
const EVENT_MENU_FONT: f32 = 16.0;

/// Compute the menu's physical-pixel rect, clamped inside the panel.
pub fn event_menu_rect(menu: &EventContextMenu, panel: Rect, scale: f32) -> Rect {
    let w = EVENT_MENU_W * scale;
    let h = EVENT_MENU_ROW_H * scale + EVENT_MENU_PAD_V * 2.0 * scale;
    let max_x = panel.x + panel.w - 8.0 * scale - w;
    let max_y = panel.y + panel.h - 8.0 * scale - h;
    let min_x = panel.x + 8.0 * scale;
    let min_y = panel.y + 8.0 * scale;
    let x = menu.anchor_x.min(max_x).max(min_x);
    let y = menu.anchor_y.min(max_y).max(min_y);
    Rect::new(x, y, w, h)
}

/// Returns true if the click landed on the (single) Delete row.
pub fn event_menu_hit_delete(
    menu: &EventContextMenu,
    panel: Rect,
    scale: f32,
    phys_x: f32,
    phys_y: f32,
) -> bool {
    let rect = event_menu_rect(menu, panel, scale);
    let pad_v = EVENT_MENU_PAD_V * scale;
    let row_y = rect.y + pad_v;
    let row_h = EVENT_MENU_ROW_H * scale;
    phys_x >= rect.x && phys_x <= rect.x + rect.w && phys_y >= row_y && phys_y <= row_y + row_h
}

/// Returns true if the click is anywhere on the menu surface.
#[allow(dead_code)] // helper for future outside-click dismissal
pub fn event_menu_contains(
    menu: &EventContextMenu,
    panel: Rect,
    scale: f32,
    phys_x: f32,
    phys_y: f32,
) -> bool {
    let r = event_menu_rect(menu, panel, scale);
    phys_x >= r.x && phys_x <= r.x + r.w && phys_y >= r.y && phys_y <= r.y + r.h
}

pub fn draw_event_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    menu: &EventContextMenu,
    panel: Rect,
    scale: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let rect = event_menu_rect(menu, panel, scale);
    let radius = 10.0 * scale;
    painter.shadow(
        rect,
        radius,
        16.0 * scale,
        Color::BLACK.with_alpha(0.45),
        0.0,
        4.0 * scale,
    );
    painter.rect_filled(rect, radius, Color::rgba(0.125, 0.125, 0.125, 0.98));
    painter.rect_stroke_sdf(rect, radius, 1.0 * scale, Color::rgba(1.0, 1.0, 1.0, 0.18));

    let pad_v = EVENT_MENU_PAD_V * scale;
    let pad_h = 16.0 * scale;
    let row_h = EVENT_MENU_ROW_H * scale;
    let font = EVENT_MENU_FONT * scale;
    let row_y = rect.y + pad_v;
    text.queue(
        "Delete",
        font,
        rect.x + pad_h,
        row_y + (row_h - font) / 2.0,
        Color::rgba(1.0, 0.55, 0.55, 0.95),
        rect.w - pad_h * 2.0,
        surface_w,
        surface_h,
    );
}

/// Hit-test the day-detail side panel. Returns what was clicked or
/// None if the cursor is not over an interactive widget.
pub fn hit_test_detail(
    panel: Rect,
    top_y: f32,
    scale: f32,
    clock: &Clock,
    events: &crate::controls::events::Events,
    text: &mut TextRenderer,
    phys_x: f32,
    phys_y: f32,
) -> Option<DetailHit> {
    let date = clock.selected_day?;
    let pad_outer = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad_outer;
    let inner_w = panel.w - pad_outer * 2.0;
    let detail_w = inner_w * DETAIL_W_FRAC;
    let x = inner_x;

    let pad = DETAIL_PAD * scale;
    let header_font = DETAIL_HEADER_FONT * scale;
    let event_font = DETAIL_EVENT_FONT * scale;
    let row_h = DETAIL_EVENT_ROW_H * scale;
    let row_gap = DETAIL_ROW_GAP * scale;
    let close_size = DETAIL_CLOSE_SIZE * scale;

    // Close button (× top-right).
    let close_x = x + detail_w - pad - close_size;
    let close_y = top_y + pad;
    if phys_x >= close_x
        && phys_x <= close_x + close_size
        && phys_y >= close_y
        && phys_y <= close_y + close_size
    {
        return Some(DetailHit::Close);
    }

    let mut y = top_y + pad + header_font + DETAIL_LIST_TOP_GAP * scale;

    // Event rows.
    let count = events.on_date(date).count();
    if count > 0 {
        for i in 0..count {
            let row_rect = Rect::new(x + pad * 0.5, y, detail_w - pad, row_h);
            if phys_x >= row_rect.x
                && phys_x <= row_rect.x + row_rect.w
                && phys_y >= row_rect.y
                && phys_y <= row_rect.y + row_rect.h
            {
                return Some(DetailHit::EventRow(i));
            }
            y += row_h + row_gap;
        }
    } else {
        y += event_font + row_gap;
    }

    // Add-event row.
    y += DETAIL_ADD_TOP_GAP * scale;
    let add_h = DETAIL_ADD_ROW_H * scale;
    let add_rect = Rect::new(x + pad * 0.5, y, detail_w - pad, add_h);
    if phys_x >= add_rect.x
        && phys_x <= add_rect.x + add_rect.w
        && phys_y >= add_rect.y
        && phys_y <= add_rect.y + add_rect.h
    {
        return Some(DetailHit::OpenAddInput);
    }

    // Suppress unused-var warning when builds enable strict lints.
    let _ = text;
    None
}

/// Truncate `s` so its rendered width at `font` does not exceed
/// `max_w` logical px. If truncation is needed an ellipsis is
/// appended. Cheap binary-ish search via repeated measurement —
/// strings here are short (event titles) so allocation cost is fine.
fn truncate_to_width(text: &mut TextRenderer, s: &str, font: f32, max_w: f32) -> String {
    if text.measure_width(s, font) <= max_w {
        return s.to_string();
    }
    let ell = "…";
    let ell_w = text.measure_width(ell, font);
    if ell_w >= max_w {
        return ell.to_string();
    }
    // Walk char boundaries from the end until the string + ellipsis fits.
    let mut end = s.len();
    while end > 0 {
        let candidate = &s[..end];
        if !candidate.is_char_boundary(end) {
            end -= 1;
            continue;
        }
        let trial = format!("{candidate}{ell}");
        if text.measure_width(&trial, font) <= max_w {
            return trial;
        }
        end -= 1;
    }
    ell.to_string()
}

// ── Formatting helpers ──────────────────────────────────────────────────────

fn format_time(hour: u32, minute: u32, second: u32, hour24: bool, seconds: bool) -> String {
    if hour24 {
        // 24-hour: zero-pad the hour ("09:05").
        if seconds {
            format!("{:02}:{:02}:{:02}", hour, minute, second)
        } else {
            format!("{:02}:{:02}", hour, minute)
        }
    } else {
        // 12-hour without AM/PM (user preference). Hour not zero-padded so
        // it reads as a natural human time.
        let h12 = if hour % 12 == 0 { 12 } else { hour % 12 };
        if seconds {
            format!("{}:{:02}:{:02}", h12, minute, second)
        } else {
            format!("{}:{:02}", h12, minute)
        }
    }
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
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "?",
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
