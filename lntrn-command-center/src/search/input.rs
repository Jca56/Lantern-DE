//! Search input — text buffer, cursor, evdev-keycode handling, and draw.
//!
//! Just the text widget. The orchestration (running the apps provider
//! when the buffer changes, holding ranked results, etc.) lives in
//! `search/mod.rs::Search`. We keep this one focused on input mechanics.
//!
//! We own our keycode → char mapping rather than pulling in a crate; the
//! shape mirrors `lntrn-bar/src/wifi.rs:512-545` (consulted in spirit; no
//! shared code, per the "Command Center is self-contained" rule).

use lntrn_render::{Color, Painter, Rect, TextRenderer};

// ── Evdev keycodes ──────────────────────────────────────────────────────────
//
// We match against Linux evdev key codes (the same ones the kernel emits and
// `wl_keyboard::Event::Key` carries). These are stable.

pub const KEY_BACKSPACE: u32 = 14;
pub const KEY_DELETE: u32 = 111;
pub const KEY_LEFT: u32 = 105;
pub const KEY_RIGHT: u32 = 106;
pub const KEY_HOME: u32 = 102;
pub const KEY_END: u32 = 107;
// The following keycodes are forwarded as-is by `Input::on_key` (the
// input struct ignores them) — Phase 2.6 will catch them in the render
// loop for navigation and launch.
#[allow(dead_code)]
pub const KEY_ENTER: u32 = 28;
#[allow(dead_code)]
pub const KEY_KP_ENTER: u32 = 96;
#[allow(dead_code)]
pub const KEY_TAB: u32 = 15;
#[allow(dead_code)]
pub const KEY_UP: u32 = 103;
#[allow(dead_code)]
pub const KEY_DOWN: u32 = 108;

// ── Input state ─────────────────────────────────────────────────────────────

/// Text input state: a UTF-8 buffer plus a byte-offset cursor.
pub struct Input {
    buf: String,
    /// Cursor byte offset into `buf`. Always lies on a UTF-8 char boundary.
    cursor: usize,
    /// Wall-clock time of the last edit; drives cursor blink phase.
    last_edit: std::time::Instant,
}

impl Input {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            cursor: 0,
            last_edit: std::time::Instant::now(),
        }
    }

    pub fn query(&self) -> &str {
        &self.buf
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.touch();
    }

    /// Handle a key press. Returns whether content changed, the cursor
    /// moved, or nothing happened — caller uses this to decide whether to
    /// re-rank results, reset the blink, or do nothing.
    pub fn on_key(&mut self, key: u32, shift: bool) -> KeyEffect {
        match key {
            KEY_BACKSPACE => self.delete_back(),
            KEY_DELETE => self.delete_forward(),
            KEY_LEFT => self.move_left(),
            KEY_RIGHT => self.move_right(),
            KEY_HOME => self.move_home(),
            KEY_END => self.move_end(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    self.insert_char(ch);
                    return KeyEffect::ContentChanged;
                }
                KeyEffect::None
            }
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.buf.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.touch();
    }

    fn delete_back(&mut self) -> KeyEffect {
        if self.cursor == 0 {
            return KeyEffect::None;
        }
        let prev = self.buf[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.buf.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        self.touch();
        KeyEffect::ContentChanged
    }

    fn delete_forward(&mut self) -> KeyEffect {
        if self.cursor >= self.buf.len() {
            return KeyEffect::None;
        }
        let next = self.buf[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buf.len());
        self.buf.replace_range(self.cursor..next, "");
        self.touch();
        KeyEffect::ContentChanged
    }

    fn move_left(&mut self) -> KeyEffect {
        if self.cursor == 0 {
            return KeyEffect::None;
        }
        self.cursor = self.buf[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_right(&mut self) -> KeyEffect {
        if self.cursor >= self.buf.len() {
            return KeyEffect::None;
        }
        self.cursor = self.buf[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buf.len());
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_home(&mut self) -> KeyEffect {
        if self.cursor == 0 {
            return KeyEffect::None;
        }
        self.cursor = 0;
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_end(&mut self) -> KeyEffect {
        if self.cursor == self.buf.len() {
            return KeyEffect::None;
        }
        self.cursor = self.buf.len();
        self.touch();
        KeyEffect::CursorMoved
    }

    fn touch(&mut self) {
        self.last_edit = std::time::Instant::now();
    }

    /// True when the blinking cursor should currently be drawn.
    /// 1Hz blink: 500ms on, 500ms off. Resets to "on" briefly after edits.
    pub fn cursor_visible(&self) -> bool {
        let since_edit = self.last_edit.elapsed().as_millis();
        if since_edit < 250 {
            return true;
        }
        ((since_edit / 500) % 2) == 0
    }

    /// Cursor byte offset into the buffer. Always lies on a UTF-8 char
    /// boundary. Exposed for external draw routines (e.g. the password
    /// modal) that need to compute cursor x.
    pub fn cursor_byte(&self) -> usize {
        self.cursor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEffect {
    /// Key did nothing relevant (e.g., a modifier or unhandled keycode).
    None,
    /// Cursor moved but the buffer didn't change.
    CursorMoved,
    /// The buffer changed — caller should re-rank search results.
    ContentChanged,
}

// ── Evdev keycode → char ────────────────────────────────────────────────────

pub fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    let ch: u8 = match key {
        // Top number row.
        2..=11 => {
            let base = b"1234567890"[(key - 2) as usize];
            if shift { b"!@#$%^&*()"[(key - 2) as usize] } else { base }
        }
        12 => if shift { b'_' } else { b'-' },
        13 => if shift { b'+' } else { b'=' },
        // qwerty row.
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        26 => if shift { b'{' } else { b'[' },
        27 => if shift { b'}' } else { b']' },
        // asdf row.
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        39 => if shift { b':' } else { b';' },
        40 => if shift { b'"' } else { b'\'' },
        41 => if shift { b'~' } else { b'`' },
        43 => if shift { b'|' } else { b'\\' },
        // zxcv row.
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        51 => if shift { b'<' } else { b',' },
        52 => if shift { b'>' } else { b'.' },
        53 => if shift { b'?' } else { b'/' },
        57 => b' ',
        _ => return None,
    };
    Some(ch as char)
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Layout constants (logical pixels).
pub const SEARCH_FONT_SIZE: f32 = 22.0;
pub const SEARCH_ROW_HEIGHT: f32 = 48.0;
pub const SEARCH_HORIZONTAL_PAD: f32 = 24.0;

/// Waffle (all-apps) icon geometry — 9 dots in 3 cols × 3 rows on the
/// right end of the search bar.
const WAFFLE_BTN_SIZE: f32 = 56.0;
const WAFFLE_DOT_RADIUS: f32 = 5.0;
const WAFFLE_DOT_PITCH: f32 = 15.0;
/// Underline thickness (logical px) at the bottom of the search row.
/// Text + waffle are centered in the content area *above* the underline.
const SEARCH_UNDERLINE_THICKNESS: f32 = 2.0;

/// Compute the waffle hit/draw rect in physical pixels. Lives at the
/// far right of the search row, vertically centered in the *visual*
/// search section — i.e. the band from the bottom of the controls
/// underline down to the top of the search underline (this includes the
/// `pad*0.5` gap above the row, which made things look bottom-heavy when
/// we only centered inside the row itself).
pub fn waffle_rect(panel: Rect, scale: f32) -> Rect {
    let pad = SEARCH_HORIZONTAL_PAD * scale;
    let row_h = SEARCH_ROW_HEIGHT * scale;
    let row_y = panel.y + crate::controls::total_logical_height() * scale + pad * 0.5;
    let section_top = panel.y + crate::controls::total_logical_height() * scale;
    let section_bottom = row_y + row_h - SEARCH_UNDERLINE_THICKNESS * scale;
    let section_h = section_bottom - section_top;
    let btn = WAFFLE_BTN_SIZE * scale;
    let x = panel.x + panel.w - pad - btn;
    let y = section_top + (section_h - btn) / 2.0;
    Rect::new(x, y, btn, btn)
}

const PLACEHOLDER: &str = "Search apps, files, web…";
/// Studio tan text — #e8dcc8. The wgpu surface is sRGB so we go through
/// `Color::from_rgb8` (with `with_alpha` later) to get the gamma right.
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const PLACEHOLDER_ALPHA: f32 = 0.45;
const UNDERLINE_ALPHA: f32 = 0.18;
/// Accent gold — #C8860A.
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

/// Draw the search input inside the panel rect.
///
/// `panel` is the (already animated) panel rect in physical pixels.
/// `scale` is the fractional scale factor.
/// `alpha` is the panel's animation alpha (0.0 → 1.0).
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &Input,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    waffle_active: bool,
) {
    let pad = SEARCH_HORIZONTAL_PAD * scale;
    let row_h = SEARCH_ROW_HEIGHT * scale;
    let font_size = SEARCH_FONT_SIZE * scale;

    let row_x = panel.x + pad;
    // Search row sits beneath the controls row + its underline.
    let row_y = panel.y + crate::controls::total_logical_height() * scale + pad * 0.5;
    let row_w = panel.w - pad * 2.0;
    // Text shouldn't run under the waffle button.
    let waffle = waffle_rect(panel, scale);
    let text_w = (waffle.x - row_x - 8.0 * scale).max(0.0);

    let underline_y = row_y + row_h - 2.0 * scale;
    painter.rect_filled(
        Rect::new(row_x, underline_y, row_w, 2.0 * scale),
        1.0 * scale,
        Color::rgba(1.0, 1.0, 1.0, UNDERLINE_ALPHA * alpha),
    );

    // Waffle "all apps" button — 6 dots in 2 cols × 3 rows.
    let dot_r = WAFFLE_DOT_RADIUS * scale;
    let pitch = WAFFLE_DOT_PITCH * scale;
    let span = pitch * 2.0;
    let dots_x0 = waffle.x + (waffle.w - span) / 2.0 - dot_r;
    let dots_y0 = waffle.y + (waffle.h - span) / 2.0 - dot_r;
    let dot_color = if waffle_active {
        Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
    } else {
        Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(0.55 * alpha)
    };
    for col in 0..3 {
        for row in 0..3 {
            let cx = dots_x0 + col as f32 * pitch;
            let cy = dots_y0 + row as f32 * pitch;
            painter.rect_filled(
                Rect::new(cx, cy, dot_r * 2.0, dot_r * 2.0),
                dot_r,
                dot_color,
            );
        }
    }

    let section_top = panel.y + crate::controls::total_logical_height() * scale;
    let section_bottom = row_y + row_h - SEARCH_UNDERLINE_THICKNESS * scale;
    let section_h = section_bottom - section_top;
    let text_y = section_top + (section_h - font_size) / 2.0;

    let (display, is_placeholder) = if input.is_empty() {
        (PLACEHOLDER, true)
    } else {
        (input.query(), false)
    };

    let text_alpha = if is_placeholder {
        PLACEHOLDER_ALPHA * alpha
    } else {
        alpha
    };
    let color = Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(text_alpha);

    text.queue(
        display,
        font_size,
        row_x,
        text_y,
        color,
        text_w,
        surface_w,
        surface_h,
    );

    if !is_placeholder && input.cursor_visible() {
        let prefix = &input.buf[..input.cursor];
        let prefix_w = text.measure_width(prefix, font_size);
        let cursor_x = row_x + prefix_w;
        let cursor_y = text_y - 2.0 * scale;
        let cursor_h = font_size + 4.0 * scale;
        painter.rect_filled(
            Rect::new(cursor_x, cursor_y, 2.0 * scale, cursor_h),
            1.0 * scale,
            Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha),
        );
    }
}
