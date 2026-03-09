use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowControlHover {
    pub minimize: bool,
    pub maximize: bool,
    pub close: bool,
}

pub struct TitleBar<'a> {
    pub rect: Rect,
    pub title: &'a str,
    pub hover: WindowControlHover,
    pub ui_scale: f32,
}

/// Button width matching lntrn-terminal (46px base).
const BTN_W: f32 = 46.0;
/// Icon size matching lntrn-terminal (14px base).
const ICON_SZ: f32 = 14.0;

impl<'a> TitleBar<'a> {
    pub fn new(rect: Rect, title: &'a str) -> Self {
        Self {
            rect,
            title,
            hover: WindowControlHover::default(),
            ui_scale: 1.0,
        }
    }

    pub fn scale(mut self, s: f32) -> Self {
        self.ui_scale = s;
        self
    }

    pub fn minimize_hovered(mut self, hovered: bool) -> Self {
        self.hover.minimize = hovered;
        self
    }

    pub fn maximize_hovered(mut self, hovered: bool) -> Self {
        self.hover.maximize = hovered;
        self
    }

    pub fn close_hovered(mut self, h: bool) -> Self {
        self.hover.close = h;
        self
    }

    /// Close button: rightmost, full height.
    pub fn close_button_rect(&self) -> Rect {
        let s = self.ui_scale;
        let w = BTN_W * s;
        Rect::new(self.rect.x + self.rect.w - w, self.rect.y, w, self.rect.h)
    }

    /// Maximize button: second from right, full height.
    pub fn maximize_button_rect(&self) -> Rect {
        let s = self.ui_scale;
        let w = BTN_W * s;
        Rect::new(self.rect.x + self.rect.w - w * 2.0, self.rect.y, w, self.rect.h)
    }

    /// Minimize button: third from right, full height.
    pub fn minimize_button_rect(&self) -> Rect {
        let s = self.ui_scale;
        let w = BTN_W * s;
        Rect::new(self.rect.x + self.rect.w - w * 3.0, self.rect.y, w, self.rect.h)
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let s = self.ui_scale;
        let r = 10.0 * s;

        // Title bar background with rounded top corners
        painter.rect_filled(self.rect, r, palette.surface);
        // Square off bottom corners
        painter.rect_filled(
            Rect::new(self.rect.x, self.rect.y + self.rect.h - r, r, r),
            0.0, palette.surface,
        );
        painter.rect_filled(
            Rect::new(self.rect.x + self.rect.w - r, self.rect.y + self.rect.h - r, r, r),
            0.0, palette.surface,
        );

        // Title text
        let font_size = 22.0 * s;
        let text_x = self.rect.x + 14.0 * s;
        let text_y = self.rect.y + (self.rect.h - font_size) * 0.5;
        if !self.title.is_empty() {
            text_renderer.queue(
                self.title, font_size, text_x, text_y, palette.text,
                (self.rect.w - BTN_W * 3.0 * s - 20.0 * s).max(40.0),
                screen_w, screen_h,
            );
        }

        // Icon color at rest: slightly transparent light gray (matching terminal)
        let icon_rest = Color::from_rgba8(236, 236, 236, 200);

        // ── Minimize ────────────────────────────────────────────────────
        let min_rect = self.minimize_button_rect();
        if self.hover.minimize {
            painter.rect_filled(min_rect, 0.0, Color::WHITE.with_alpha(0.04));
        }
        self.draw_minimize_icon(painter, min_rect, icon_rest);

        // ── Maximize ────────────────────────────────────────────────────
        let max_rect = self.maximize_button_rect();
        if self.hover.maximize {
            painter.rect_filled(max_rect, 0.0, Color::WHITE.with_alpha(0.04));
        }
        self.draw_maximize_icon(painter, max_rect, icon_rest);

        // ── Close ───────────────────────────────────────────────────────
        let close_rect = self.close_button_rect();
        if self.hover.close {
            painter.rect_filled(close_rect, 0.0, Color::from_rgb8(232, 50, 50));
        }
        self.draw_close_icon(
            painter, close_rect,
            if self.hover.close { Color::WHITE } else { icon_rest },
        );
    }

    fn draw_minimize_icon(&self, painter: &mut Painter, rect: Rect, color: Color) {
        let s = self.ui_scale;
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        painter.rect_filled(
            Rect::new(cx - sz * 0.5, cy, sz, 1.5 * s),
            0.0, color,
        );
    }

    fn draw_maximize_icon(&self, painter: &mut Painter, rect: Rect, color: Color) {
        let s = self.ui_scale;
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        let r = Rect::new(cx - sz * 0.5, cy - sz * 0.5, sz, sz);
        painter.rect_stroke(r, 0.0, 1.5 * s, color);
    }

    fn draw_close_icon(&self, painter: &mut Painter, rect: Rect, color: Color) {
        let s = self.ui_scale;
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        let half = sz * 0.5;
        painter.line(cx - half, cy - half, cx + half, cy + half, 1.5 * s, color);
        painter.line(cx + half, cy - half, cx - half, cy + half, 1.5 * s, color);
    }
}
