use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;
use super::text::FontSize;

const PADDING_H: f32 = 12.0;
const PADDING_V: f32 = 4.0;
const FONT_SIZE: FontSize = FontSize::Label;

#[derive(Clone, Copy, PartialEq)]
pub enum BadgeVariant {
    Default,
    Success,
    Warning,
    Danger,
    Info,
}

/// A small colored tag/label.
///
/// ```ignore
/// Badge::new("NEW", x, y)
///     .variant(BadgeVariant::Success)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct Badge<'a> {
    text: &'a str,
    x: f32,
    y: f32,
    variant: BadgeVariant,
}

impl<'a> Badge<'a> {
    pub fn new(text: &'a str, x: f32, y: f32) -> Self {
        Self {
            text,
            x,
            y,
            variant: BadgeVariant::Default,
        }
    }

    pub fn variant(mut self, v: BadgeVariant) -> Self {
        self.variant = v;
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
        let font_px = FONT_SIZE.px();
        let text_w = self.text.len() as f32 * font_px * 0.55;
        let w = text_w + PADDING_H * 2.0;
        let h = font_px + PADDING_V * 2.0;
        let radius = h * 0.5; // pill shape

        let color = self.resolve_color(palette);

        // Low-alpha background
        let bg = Rect::new(self.x, self.y, w, h);
        painter.rect_filled(bg, radius, color.with_alpha(0.15));

        // Text in full color
        let text_x = self.x + PADDING_H;
        let text_y = self.y + PADDING_V;
        text_renderer.queue(
            self.text,
            font_px,
            text_x,
            text_y,
            color,
            w,
            screen_w,
            screen_h,
        );
    }

    fn resolve_color(&self, p: &FoxPalette) -> Color {
        match self.variant {
            BadgeVariant::Default => p.text_secondary,
            BadgeVariant::Success => p.success,
            BadgeVariant::Warning => p.warning,
            BadgeVariant::Danger => p.danger,
            BadgeVariant::Info => p.info,
        }
    }
}
