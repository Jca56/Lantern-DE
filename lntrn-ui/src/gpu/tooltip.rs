use lntrn_render::{Painter, Rect, TextRenderer};

use super::palette::FoxPalette;
use super::text::FontSize;

const PADDING_H: f32 = 14.0;
const PADDING_V: f32 = 8.0;
const CORNER_RADIUS: f32 = 6.0;
const FONT_SIZE: FontSize = FontSize::Caption;

/// A floating label that appears near a target position.
///
/// ```ignore
/// Tooltip::new("Save file", x, y)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct Tooltip<'a> {
    text: &'a str,
    x: f32,
    y: f32,
}

impl<'a> Tooltip<'a> {
    pub fn new(text: &'a str, x: f32, y: f32) -> Self {
        Self { text, x, y }
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let font_px = FONT_SIZE.px();
        let text_w = self.text.len() as f32 * font_px * 0.55;
        let w = text_w + PADDING_H * 2.0;
        let h = font_px + PADDING_V * 2.0;

        let bg_rect = Rect::new(self.x, self.y, w, h);

        painter.rect_filled(bg_rect, CORNER_RADIUS, palette.surface.with_alpha(0.95));
        painter.rect_stroke(bg_rect, CORNER_RADIUS, 1.0, palette.muted.with_alpha(0.3));

        let text_x = self.x + PADDING_H;
        let text_y = self.y + PADDING_V;
        text_renderer.queue(
            self.text,
            font_px,
            text_x,
            text_y,
            palette.text,
            w - PADDING_H * 2.0,
            screen_w,
            screen_h,
        );
    }
}
