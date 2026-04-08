use lntrn_render::{Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

/// Corner radius for the input field.
const INPUT_RADIUS: f32 = 8.0;
/// Minimum comfortable height for the input field.
const MIN_HEIGHT: f32 = 44.0;
/// Horizontal text padding inside the field.
const TEXT_PAD_X: f32 = 14.0;
/// Border width at rest.
const BORDER_REST: f32 = 1.5;
/// Border width when focused (accent glow).
const BORDER_FOCUS: f32 = 2.0;
/// Font size for input text (Body = 24px).
const TEXT_FONT_SIZE: f32 = 24.0;
/// Font size for placeholder (same as input text for visual alignment).
const PLACEHOLDER_FONT_SIZE: f32 = 24.0;
/// Cursor blink is not animated here — we draw a static caret when focused.
const CARET_WIDTH: f32 = 2.0;

/// A single-line text input field.
///
/// ```ignore
/// TextInput::new(rect)
///     .text("hello world")
///     .placeholder("Type here...")
///     .focused(true)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct TextInput<'a> {
    rect: Rect,
    text: &'a str,
    placeholder: &'a str,
    focused: bool,
    hovered: bool,
    cursor_pos: Option<usize>,
    /// Selection range in char offsets (start, end). Highlighted when start != end.
    selection: Option<(usize, usize)>,
    scale: f32,
}

impl<'a> TextInput<'a> {
    /// Create a new text input. Height is clamped to at least 44px.
    pub fn new(rect: Rect) -> Self {
        let h = rect.h.max(MIN_HEIGHT);
        Self {
            rect: Rect::new(rect.x, rect.y, rect.w, h),
            text: "",
            placeholder: "",
            focused: false,
            hovered: false,
            cursor_pos: None,
            selection: None,
            scale: 1.0,
        }
    }

    pub fn text(mut self, text: &'a str) -> Self {
        self.text = text;
        self
    }

    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    pub fn cursor_pos(mut self, pos: usize) -> Self {
        self.cursor_pos = Some(pos);
        self
    }

    pub fn selection(mut self, sel: Option<(usize, usize)>) -> Self {
        self.selection = sel;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let s = self.scale;
        let radius = INPUT_RADIUS * s;
        let border_rest = BORDER_REST * s;
        let border_focus = BORDER_FOCUS * s;
        let text_pad_x = TEXT_PAD_X * s;
        let font_size = TEXT_FONT_SIZE * s;
        let ph_font_size = PLACEHOLDER_FONT_SIZE * s;
        let caret_w = CARET_WIDTH * s;

        // -- Background --
        let bg = if self.focused {
            palette.surface_2
        } else if self.hovered {
            palette.surface_2
        } else {
            palette.surface
        };
        painter.rect_filled(self.rect, radius, bg);

        // -- Border --
        if self.focused {
            painter.rect_stroke(self.rect, radius, border_focus, palette.accent);

            let glow = Rect::new(
                self.rect.x - 1.5 * s,
                self.rect.y - 1.5 * s,
                self.rect.w + 3.0 * s,
                self.rect.h + 3.0 * s,
            );
            painter.rect_stroke(
                glow,
                radius + 1.5 * s,
                1.0 * s,
                palette.accent.with_alpha(0.2),
            );
        } else {
            let border_color = if self.hovered {
                palette.muted.with_alpha(0.7)
            } else {
                palette.muted.with_alpha(0.35)
            };
            painter.rect_stroke(self.rect, radius, border_rest, border_color);
        }

        // -- Text or placeholder --
        let text_x_base = self.rect.x + text_pad_x;
        let max_text_w = (self.rect.w - text_pad_x * 2.0).max(10.0);

        // Determine caret byte offset (cursor_pos is in characters, strings need bytes)
        let caret_byte = if self.focused {
            let char_pos = self.cursor_pos.unwrap_or(self.text.chars().count());
            self.text.char_indices()
                .nth(char_pos)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len())
        } else {
            0
        };

        // Calculate scroll offset so cursor is always visible
        let scroll_offset = if self.focused && !self.text.is_empty() {
            let text_before_cursor = &self.text[..caret_byte];
            let cursor_x = text_renderer.measure_width(text_before_cursor, font_size);
            if cursor_x > max_text_w {
                cursor_x - max_text_w + text_pad_x
            } else {
                0.0
            }
        } else {
            0.0
        };

        let text_x = text_x_base - scroll_offset;

        // Clip text to input bounds
        let clip_rect = Rect::new(
            self.rect.x + text_pad_x, self.rect.y,
            max_text_w, self.rect.h,
        );
        painter.push_clip(clip_rect);
        text_renderer.push_clip([clip_rect.x, clip_rect.y, clip_rect.w, clip_rect.h]);

        // Use a large max_width so glyphon never wraps — clipping handles overflow
        let layout_w = 10000.0_f32;

        if self.text.is_empty() {
            let ph_y = self.rect.y + (self.rect.h - ph_font_size) * 0.5;
            text_renderer.queue(
                self.placeholder,
                ph_font_size,
                text_x,
                ph_y,
                palette.muted,
                layout_w,
                screen_w,
                screen_h,
            );
        } else {
            let text_y = self.rect.y + (self.rect.h - font_size) * 0.5;
            text_renderer.queue(
                self.text,
                font_size,
                text_x,
                text_y,
                palette.text,
                layout_w,
                screen_w,
                screen_h,
            );
        }

        // -- Selection highlight --
        if self.focused {
            if let Some((sel_start, sel_end)) = self.selection {
                let s_min = sel_start.min(sel_end);
                let s_max = sel_start.max(sel_end);
                if s_min != s_max {
                    let byte_start = self.text.char_indices().nth(s_min).map(|(i,_)| i).unwrap_or(0);
                    let byte_end = self.text.char_indices().nth(s_max).map(|(i,_)| i).unwrap_or(self.text.len());
                    let x_start = text_x + text_renderer.measure_width(&self.text[..byte_start], font_size);
                    let x_end = text_x + text_renderer.measure_width(&self.text[..byte_end], font_size);
                    let sel_y = self.rect.y + (self.rect.h - font_size) * 0.5 - 1.0 * s;
                    let sel_h = font_size + 2.0 * s;
                    painter.rect_filled(
                        Rect::new(x_start, sel_y, x_end - x_start, sel_h),
                        2.0 * s,
                        palette.accent.with_alpha(0.3),
                    );
                }
            }
        }

        // -- Caret --
        if self.focused {
            let text_before_cursor = &self.text[..caret_byte];
            let caret_offset = text_renderer.measure_width(text_before_cursor, font_size);
            let caret_x = text_x + caret_offset;
            let caret_y = self.rect.y + (self.rect.h - font_size) * 0.5;
            let caret_h = font_size + 2.0 * s;
            painter.rect_filled(
                Rect::new(caret_x, caret_y, caret_w, caret_h),
                0.0,
                palette.text.with_alpha(0.85),
            );
        }

        painter.pop_clip();
        text_renderer.pop_clip();
    }
}
