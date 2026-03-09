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

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        // -- Background --
        let bg = if self.focused {
            // Slightly brighter when focused for depth
            palette.surface_2
        } else if self.hovered {
            palette.surface_2
        } else {
            palette.surface
        };
        painter.rect_filled(self.rect, INPUT_RADIUS, bg);

        // -- Border --
        if self.focused {
            // Accent glow border
            painter.rect_stroke(self.rect, INPUT_RADIUS, BORDER_FOCUS, palette.accent);

            // Subtle outer glow ring
            let glow = Rect::new(
                self.rect.x - 1.5,
                self.rect.y - 1.5,
                self.rect.w + 3.0,
                self.rect.h + 3.0,
            );
            painter.rect_stroke(
                glow,
                INPUT_RADIUS + 1.5,
                1.0,
                palette.accent.with_alpha(0.2),
            );
        } else {
            let border_color = if self.hovered {
                palette.muted.with_alpha(0.7)
            } else {
                palette.muted.with_alpha(0.35)
            };
            painter.rect_stroke(self.rect, INPUT_RADIUS, BORDER_REST, border_color);
        }

        // -- Text or placeholder --
        let text_x = self.rect.x + TEXT_PAD_X;
        let max_text_w = (self.rect.w - TEXT_PAD_X * 2.0).max(10.0);

        if self.text.is_empty() {
            // Placeholder
            let ph_y = self.rect.y + (self.rect.h - PLACEHOLDER_FONT_SIZE) * 0.5;
            text_renderer.queue(
                self.placeholder,
                PLACEHOLDER_FONT_SIZE,
                text_x,
                ph_y,
                palette.muted,
                max_text_w,
                screen_w,
                screen_h,
            );
        } else {
            // Actual text content
            let text_y = self.rect.y + (self.rect.h - TEXT_FONT_SIZE) * 0.5;
            text_renderer.queue(
                self.text,
                TEXT_FONT_SIZE,
                text_x,
                text_y,
                palette.text,
                max_text_w,
                screen_w,
                screen_h,
            );

            // -- Caret (drawn after text when focused) --
            if self.focused {
                // Approximate caret position at end of text
                let caret_x = text_x + self.text.len() as f32 * TEXT_FONT_SIZE * 0.52;
                let caret_x = caret_x.min(self.rect.x + self.rect.w - TEXT_PAD_X);
                let caret_y = self.rect.y + (self.rect.h - TEXT_FONT_SIZE) * 0.5;
                let caret_h = TEXT_FONT_SIZE + 2.0;
                painter.rect_filled(
                    Rect::new(caret_x, caret_y, CARET_WIDTH, caret_h),
                    0.0,
                    palette.text.with_alpha(0.85),
                );
            }
        }

        // -- Caret for empty + focused --
        if self.text.is_empty() && self.focused {
            let caret_y = self.rect.y + (self.rect.h - TEXT_FONT_SIZE) * 0.5;
            let caret_h = TEXT_FONT_SIZE + 2.0;
            painter.rect_filled(
                Rect::new(text_x, caret_y, CARET_WIDTH, caret_h),
                0.0,
                palette.text.with_alpha(0.85),
            );
        }
    }
}
