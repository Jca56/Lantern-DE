use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

pub struct Slider {
    pub rect: Rect,
    pub value: f32,
    pub hovered: bool,
    pub active: bool,
    pub fill_start: Color,
    pub fill_end: Color,
    pub thumb_color: Color,
}

impl Slider {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            value: 0.5,
            hovered: false,
            active: false,
            fill_start: Color::from_rgb8(170, 110, 8),
            fill_end: Color::from_rgb8(250, 204, 21),
            thumb_color: Color::WHITE,
        }
    }

    pub fn value(mut self, value: f32) -> Self {
        self.value = value.clamp(0.0, 1.0);
        self
    }

    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn draw(&self, painter: &mut Painter, palette: &FoxPalette) {
        let track_h = 10.0;
        let track_y = self.rect.y + (self.rect.h - track_h) * 0.5;
        let track = Rect::new(self.rect.x, track_y, self.rect.w, track_h);
        let fill_w = (track.w * self.value).clamp(track_h, track.w.max(track_h));
        let fill = Rect::new(track.x, track.y, fill_w, track.h);
        let thumb_x = track.x + track.w * self.value;
        let thumb_r = if self.active { 11.0 } else if self.hovered { 10.0 } else { 9.0 };

        painter.rect_filled(track, track_h * 0.5, palette.surface_2.with_alpha(0.95));
        painter.rect_stroke(track, track_h * 0.5, 1.0, palette.text_secondary.with_alpha(0.16));
        painter.rect_gradient_linear(fill, track_h * 0.5, 0.0, self.fill_start, self.fill_end);

        painter.circle_filled(thumb_x, self.rect.y + self.rect.h * 0.5, thumb_r, self.thumb_color);
        painter.circle_stroke(
            thumb_x,
            self.rect.y + self.rect.h * 0.5,
            thumb_r + 3.0,
            2.0,
            self.fill_end.with_alpha(if self.active { 0.46 } else { 0.22 }),
        );
        painter.circle_stroke(
            thumb_x,
            self.rect.y + self.rect.h * 0.5,
            thumb_r,
            1.0,
            Color::BLACK.with_alpha(0.12),
        );
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonVariant {
    Default,
    Primary,
    Ghost,
}

pub struct Button<'a> {
    pub rect: Rect,
    pub label: &'a str,
    pub variant: ButtonVariant,
    pub hovered: bool,
    pub pressed: bool,
}

impl<'a> Button<'a> {
    pub fn new(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            variant: ButtonVariant::Default,
            hovered: false,
            pressed: false,
        }
    }

    pub fn variant(mut self, v: ButtonVariant) -> Self {
        self.variant = v;
        self
    }

    pub fn hovered(mut self, h: bool) -> Self {
        self.hovered = h;
        self
    }

    pub fn pressed(mut self, p: bool) -> Self {
        self.pressed = p;
        self
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let (bg, text_color) = self.resolve_colors(palette);
        let radius = 6.0;

        painter.rect_filled(self.rect, radius, bg);

        let font_size = 20.0;
        let text_x = self.rect.x + self.rect.w * 0.5 - (self.label.len() as f32 * font_size * 0.3);
        let text_y = self.rect.y + (self.rect.h - font_size) * 0.5;
        text.queue(
            self.label,
            font_size,
            text_x,
            text_y,
            text_color,
            self.rect.w,
            screen_w,
            screen_h,
        );
    }

    fn resolve_colors(&self, p: &FoxPalette) -> (Color, Color) {
        match self.variant {
            ButtonVariant::Default => {
                let bg = if self.pressed {
                    p.bg
                } else if self.hovered {
                    p.surface
                } else {
                    p.bg.with_alpha(0.8)
                };
                let alpha = if self.hovered { 1.0 } else { 0.9 };
                (bg, p.text.with_alpha(alpha))
            }
            ButtonVariant::Primary => {
                let bg = if self.pressed {
                    Color::from_rgb8(170, 110, 8)
                } else if self.hovered {
                    Color::from_rgb8(220, 150, 15)
                } else {
                    p.accent
                };
                (bg, Color::from_rgb8(20, 20, 20))
            }
            ButtonVariant::Ghost => {
                let bg = if self.pressed {
                    p.surface.with_alpha(0.5)
                } else if self.hovered {
                    p.surface.with_alpha(0.35)
                } else {
                    p.surface.with_alpha(0.12)
                };
                (bg, p.text)
            }
        }
    }
}