use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_FULL: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

const ZONE_CLOCK: u32 = 0xEE_0000;
const ZONE_PREV_MONTH: u32 = 0xEE_0001;
const ZONE_NEXT_MONTH: u32 = 0xEE_0002;

pub struct Clock {
    hour: i32,
    minute: i32,
    weekday: i32,
    month: i32,
    day: i32,
    year: i32,
    // Calendar popup state
    pub open: bool,
    view_month: i32, // 0-11
    view_year: i32,
}

impl Clock {
    pub fn new() -> Self {
        let mut c = Self {
            hour: 0, minute: 0, weekday: 0, month: 0, day: 0, year: 0,
            open: false, view_month: 0, view_year: 0,
        };
        c.tick();
        c.view_month = c.month;
        c.view_year = c.year;
        c
    }

    pub fn tick(&mut self) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&secs, &mut tm) };
        self.hour = tm.tm_hour;
        self.minute = tm.tm_min;
        self.weekday = tm.tm_wday;
        self.month = tm.tm_mon;
        self.day = tm.tm_mday;
        self.year = tm.tm_year + 1900;
    }

    pub fn time_text_len(&self) -> usize {
        self.time_text().len()
    }

    fn time_text(&self) -> String {
        let h12 = match self.hour {
            0 => 12,
            13..=23 => self.hour - 12,
            _ => self.hour,
        };
        format!("{}:{:02}", h12, self.minute)
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.view_month = self.month;
            self.view_year = self.year;
        }
    }

    pub fn prev_month(&mut self) {
        self.view_month -= 1;
        if self.view_month < 0 {
            self.view_month = 11;
            self.view_year -= 1;
        }
    }

    pub fn next_month(&mut self) {
        self.view_month += 1;
        if self.view_month > 11 {
            self.view_month = 0;
            self.view_year += 1;
        }
    }

    /// Handle a left click. Returns true if it was consumed.
    pub fn handle_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) -> bool {
        if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
            match zone {
                ZONE_CLOCK => { self.toggle(); true }
                ZONE_PREV_MONTH => { self.prev_month(); true }
                ZONE_NEXT_MONTH => { self.next_month(); true }
                _ => false,
            }
        } else {
            false
        }
    }

    /// Draw the clock time right-aligned within the visual bar rect.
    /// Returns the hit-test rect for the clock text area.
    pub fn draw(
        &self,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        font_size: f32,
        color: Color,
        bar_w: f32,
        bar_h: f32,
        bar_x: f32,
        bar_y: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        let display = self.time_text();
        let padding = font_size * 0.6;
        let char_w = font_size * 0.52;
        let text_w = display.len() as f32 * char_w;
        let x = bar_x + (bar_w - text_w - padding).max(0.0);
        let y = bar_y + (bar_h - font_size) / 2.0 - font_size * 0.10;
        text.queue(&display, font_size, x, y, color, bar_w, screen_w, screen_h);

        // Hit-test zone for the clock area
        let zone_rect = Rect::new(x - 4.0, bar_y, text_w + padding + 4.0, bar_h);
        ix.add_zone(ZONE_CLOCK, zone_rect);
    }

    /// Draw the calendar popup above the bar.
    pub fn draw_calendar(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        bar_w: f32,
        bar_y: f32,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        if !self.open { return; }

        let font_size = 22.0 * scale;
        let header_font = 26.0 * scale;
        let cell_size = 50.0 * scale;
        let pad = 18.0 * scale;
        let corner_r = 12.0 * scale;
        let gap = 8.0 * scale;
        let arrow_size = header_font;

        let grid_w = cell_size * 7.0;
        let popup_w = grid_w + pad * 2.0;

        // Calculate grid rows needed
        let first_weekday = day_of_week(self.view_year, self.view_month + 1, 1);
        let days_in = days_in_month(self.view_year, self.view_month + 1);
        let total_cells = first_weekday as i32 + days_in;
        let rows = (total_cells + 6) / 7;

        let header_h = header_font + pad;
        let day_labels_h = font_size + 6.0 * scale;
        let grid_h = rows as f32 * cell_size;
        let popup_h = pad + header_h + day_labels_h + grid_h + pad;

        // Position: right-aligned, above bar
        let clock_font = (bar_y - ((screen_h as f32) - (bar_y + (screen_h as f32 - bar_y)))) * 0.7;
        let _ = clock_font;
        let right_margin = 12.0 * scale;
        let popup_x = (bar_w - popup_w - right_margin).max(0.0);
        let popup_y = (bar_y - popup_h - gap).max(0.0);

        // Shadow
        let shadow_rect = Rect::new(
            popup_x - 2.0 * scale,
            popup_y + 3.0 * scale,
            popup_w + 4.0 * scale,
            popup_h + 3.0 * scale,
        );
        painter.rect_filled(shadow_rect, corner_r + 2.0, Color::BLACK.with_alpha(0.35));

        // Background
        let bg_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
        painter.rect_filled(bg_rect, corner_r, palette.surface_2);
        let border_color = Color::rgba(1.0, 1.0, 1.0, 0.1);
        painter.rect_stroke(bg_rect, corner_r, 1.0 * scale, border_color);

        // ── Header: < March 2026 > ──
        let header_y = popup_y + pad;
        let month_name = MONTHS_FULL[self.view_month as usize % 12];
        let header_text = format!("{} {}", month_name, self.view_year);
        let header_text_w = header_text.len() as f32 * header_font * 0.5;
        let header_text_x = popup_x + (popup_w - header_text_w) / 2.0;
        text.queue(
            &header_text, header_font,
            header_text_x, header_y, palette.text,
            popup_w, screen_w, screen_h,
        );

        // Arrow buttons
        let arrow_pad = 8.0 * scale;
        let prev_x = popup_x + pad;
        let prev_rect = Rect::new(prev_x, header_y - 2.0 * scale, arrow_size + arrow_pad * 2.0, header_font + 4.0 * scale);
        ix.add_zone(ZONE_PREV_MONTH, prev_rect);
        let prev_hover = ix.is_hovered(&prev_rect);
        if prev_hover {
            painter.rect_filled(prev_rect, 6.0 * scale, palette.surface);
        }
        text.queue(
            "<", header_font,
            prev_x + arrow_pad, header_y, palette.muted,
            popup_w, screen_w, screen_h,
        );

        let next_x = popup_x + popup_w - pad - arrow_size - arrow_pad * 2.0;
        let next_rect = Rect::new(next_x, header_y - 2.0 * scale, arrow_size + arrow_pad * 2.0, header_font + 4.0 * scale);
        ix.add_zone(ZONE_NEXT_MONTH, next_rect);
        let next_hover = ix.is_hovered(&next_rect);
        if next_hover {
            painter.rect_filled(next_rect, 6.0 * scale, palette.surface);
        }
        text.queue(
            ">", header_font,
            next_x + arrow_pad, header_y, palette.muted,
            popup_w, screen_w, screen_h,
        );

        // ── Day-of-week labels ──
        let labels_y = header_y + header_h;
        let day_labels = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
        for (i, label) in day_labels.iter().enumerate() {
            let cx = popup_x + pad + i as f32 * cell_size + cell_size / 2.0;
            let lw = label.len() as f32 * font_size * 0.5;
            text.queue(
                label, font_size,
                cx - lw / 2.0, labels_y, palette.muted,
                popup_w, screen_w, screen_h,
            );
        }

        // ── Day grid ──
        let grid_y = labels_y + day_labels_h;
        let is_current_month = self.view_month == self.month && self.view_year == self.year;

        for day in 1..=days_in {
            let dow = (first_weekday as i32 + day - 1) % 7;
            let row = (first_weekday as i32 + day - 1) / 7;
            let cx = popup_x + pad + dow as f32 * cell_size + cell_size / 2.0;
            let cy = grid_y + row as f32 * cell_size + cell_size / 2.0;

            let is_today = is_current_month && day == self.day;

            if is_today {
                // Highlight circle for today
                let r = cell_size * 0.42;
                painter.rect_filled(
                    Rect::new(cx - r, cy - r, r * 2.0, r * 2.0),
                    r, palette.accent,
                );
            }

            let label = format!("{}", day);
            let lw = label.len() as f32 * font_size * 0.5;
            let day_color = if is_today { palette.surface } else { palette.text };
            text.queue(
                &label, font_size,
                cx - lw / 2.0, cy - font_size / 2.0, day_color,
                popup_w, screen_w, screen_h,
            );
        }
    }
}

/// Day of week for a given date (0 = Sunday). Tomohiko Sakamoto's algorithm.
fn day_of_week(year: i32, month: i32, day: i32) -> u32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let dow = (y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day) % 7;
    dow as u32
}

/// Number of days in a given month (1-indexed).
fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 },
        _ => 30,
    }
}
