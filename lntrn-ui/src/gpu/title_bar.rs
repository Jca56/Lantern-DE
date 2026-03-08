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

    fn control_size(&self) -> f32 {
        28.0_f32.min((self.rect.h - 8.0).max(20.0))
    }

    fn control_pad(&self) -> f32 {
        (self.rect.h - self.control_size()) * 0.5
    }

    fn control_gap(&self) -> f32 {
        6.0
    }

    pub fn minimize_button_rect(&self) -> Rect {
        let size = self.control_size();
        let pad = self.control_pad();
        let gap = self.control_gap();
        let right = self.rect.x + self.rect.w - pad;
        Rect::new(right - size * 3.0 - gap * 2.0, self.rect.y + pad, size, size)
    }

    pub fn maximize_button_rect(&self) -> Rect {
        let size = self.control_size();
        let pad = self.control_pad();
        let gap = self.control_gap();
        let right = self.rect.x + self.rect.w - pad;
        Rect::new(right - size * 2.0 - gap, self.rect.y + pad, size, size)
    }

    /// Returns the hit-rect of the close button (for input handling).
    pub fn close_button_rect(&self) -> Rect {
        let size = self.control_size();
        let pad = self.control_pad();
        Rect::new(
            self.rect.x + self.rect.w - size - pad,
            self.rect.y + pad,
            size,
            size,
        )
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        // Title bar gradient with rounded top corners (10px radius)
        painter.rect_gradient_linear(
            self.rect,
            10.0,
            std::f32::consts::FRAC_PI_2,
            palette.surface_2,
            palette.surface,
        );
        // Fill in bottom corners to keep them square
        let r = 10.0;
        painter.rect_filled(
            Rect::new(self.rect.x, self.rect.y + self.rect.h - r, r, r),
            0.0,
            palette.surface,
        );
        painter.rect_filled(
            Rect::new(self.rect.x + self.rect.w - r, self.rect.y + self.rect.h - r, r, r),
            0.0,
            palette.surface,
        );

        // Subtle bottom border
        painter.rect_filled(
            Rect::new(self.rect.x, self.rect.y + self.rect.h - 1.0, self.rect.w, 1.0),
            0.0,
            palette.bg,
        );

        // Title text (centered vertically, left-padded)
        let s = self.ui_scale;
        let font_size = 22.0 * s;
        let text_x = self.rect.x + 14.0 * s;
        let text_y = self.rect.y + (self.rect.h - font_size) * 0.5;
        text_renderer.queue(
            self.title,
            font_size,
            text_x,
            text_y,
            palette.text,
            (self.rect.w - 144.0).max(40.0),
            screen_w,
            screen_h,
        );

        let minimize_rect = self.minimize_button_rect();
        let maximize_rect = self.maximize_button_rect();
        let close_rect = self.close_button_rect();

        // Minimize: yellow hover
        let min_hover_bg = Color::from_rgb8(190, 155, 15).with_alpha(0.14);
        self.draw_control_button(painter, minimize_rect, self.hover.minimize, min_hover_bg);
        self.draw_minimize_icon(
            painter,
            minimize_rect,
            if self.hover.minimize {
                Color::from_rgb8(253, 224, 71)
            } else {
                palette.muted
            },
        );

        // Maximize: green hover
        let max_hover_bg = Color::from_rgb8(30, 150, 70).with_alpha(0.14);
        self.draw_control_button(painter, maximize_rect, self.hover.maximize, max_hover_bg);
        self.draw_maximize_icon(
            painter,
            maximize_rect,
            if self.hover.maximize {
                Color::from_rgb8(74, 222, 128)
            } else {
                palette.muted
            },
        );

        // Close: red hover
        let close_hover_bg = Color::from_rgb8(180, 50, 50).with_alpha(0.14);
        self.draw_control_button(painter, close_rect, self.hover.close, close_hover_bg);
        self.draw_close_icon(
            painter,
            close_rect,
            if self.hover.close {
                Color::from_rgb8(255, 100, 100)
            } else {
                palette.muted
            },
        );
    }

    fn draw_control_button(&self, painter: &mut Painter, rect: Rect, hovered: bool, hover_fill: Color) {
        let r = 4.0 * self.ui_scale;
        if hovered {
            painter.rect_filled(rect, r, hover_fill);
        }
        painter.rect_stroke(
            rect, r, 1.0,
            if hovered { Color::WHITE.with_alpha(0.15) } else { Color::WHITE.with_alpha(0.06) },
        );
    }

    fn draw_minimize_icon(&self, painter: &mut Painter, rect: Rect, color: Color) {
        let m = rect.w * 0.28;
        let y = rect.y + rect.h * 0.64;
        painter.line(rect.x + m, y, rect.x + rect.w - m, y, 2.0 * self.ui_scale, color);
    }

    fn draw_maximize_icon(&self, painter: &mut Painter, rect: Rect, color: Color) {
        let m = rect.w * 0.28;
        painter.rect_stroke(
            Rect::new(rect.x + m, rect.y + m, rect.w - m * 2.0, rect.h - m * 2.0),
            0.0, 1.5 * self.ui_scale, color,
        );
    }

    fn draw_close_icon(&self, painter: &mut Painter, rect: Rect, color: Color) {
        let cx = rect.center_x();
        let cy = rect.center_y();
        let half = rect.w * 0.2;
        painter.line(cx - half, cy - half, cx + half, cy + half, 1.8 * self.ui_scale, color);
        painter.line(cx + half, cy - half, cx - half, cy + half, 1.8 * self.ui_scale, color);
    }
}
