use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;
use super::text::FontSize;

const TRACK_HEIGHT: f32 = 6.0;
const TRACK_RADIUS: f32 = 3.0;
const LABEL_FONT: FontSize = FontSize::Label;

/// A horizontal progress bar.
///
/// ```ignore
/// ProgressBar::new(rect)
///     .value(0.65)
///     .label(true)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct ProgressBar {
    rect: Rect,
    value: f32,
    show_label: bool,
}

impl ProgressBar {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            value: 0.0,
            show_label: false,
        }
    }

    pub fn value(mut self, v: f32) -> Self {
        self.value = v.clamp(0.0, 1.0);
        self
    }

    pub fn label(mut self, show: bool) -> Self {
        self.show_label = show;
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
        let track_y = self.rect.y + (self.rect.h - TRACK_HEIGHT) * 0.5;
        let track = Rect::new(self.rect.x, track_y, self.rect.w, TRACK_HEIGHT);

        // Track background
        painter.rect_filled(track, TRACK_RADIUS, palette.surface_2);

        // Fill bar — gradient from darker to lighter accent
        if self.value > 0.0 {
            let fill_w = (track.w * self.value).max(TRACK_HEIGHT);
            let fill = Rect::new(track.x, track.y, fill_w, TRACK_HEIGHT);
            let dark_accent = darken(palette.accent, 0.6);
            painter.rect_gradient_linear(fill, TRACK_RADIUS, 0.0, dark_accent, palette.accent);
        }

        // Optional percentage label
        if self.show_label {
            let pct = format!("{}%", (self.value * 100.0).round() as u32);
            let font_px = LABEL_FONT.px();
            let text_w = pct.len() as f32 * font_px * 0.55;
            let text_x = self.rect.x + (self.rect.w - text_w) * 0.5;
            let text_y = self.rect.y + (self.rect.h - font_px) * 0.5;
            text_renderer.queue(
                &pct,
                font_px,
                text_x,
                text_y,
                palette.text,
                self.rect.w,
                screen_w,
                screen_h,
            );
        }
    }
}

/// Darken a color by multiplying RGB channels by `factor` (0.0 = black, 1.0 = unchanged).
fn darken(c: Color, factor: f32) -> Color {
    Color::rgba(c.r * factor, c.g * factor, c.b * factor, c.a)
}
