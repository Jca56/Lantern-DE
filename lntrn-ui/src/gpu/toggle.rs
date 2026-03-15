use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

const TRACK_W: f32 = 52.0;
const TRACK_H: f32 = 28.0;
const TRACK_RADIUS: f32 = 14.0;
const THUMB_RADIUS: f32 = 10.0;
const THUMB_MARGIN: f32 = 4.0;
const LABEL_GAP: f32 = 12.0;
const LABEL_FONT_SIZE: f32 = 20.0;

/// An iOS-style toggle switch.
///
/// ```ignore
/// Toggle::new(rect, is_on)
///     .label("Dark mode")
///     .hovered(true)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct Toggle<'a> {
    rect: Rect,
    on: bool,
    label: Option<&'a str>,
    hovered: bool,
    disabled: bool,
    /// Animation progress 0.0 (off) → 1.0 (on). If `None`, snaps to current state.
    transition: Option<f32>,
    scale: f32,
}

impl<'a> Toggle<'a> {
    pub fn new(rect: Rect, on: bool) -> Self {
        Self {
            rect,
            on,
            label: None,
            hovered: false,
            disabled: false,
            transition: None,
            scale: 1.0,
        }
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Set the animation progress (0.0 = fully off, 1.0 = fully on).
    /// When not set, the toggle snaps to its current state.
    pub fn transition(mut self, t: f32) -> Self {
        self.transition = Some(t.clamp(0.0, 1.0));
        self
    }

    /// Returns the rect of the track (for external hit-testing).
    pub fn track_rect(&self) -> Rect {
        let s = self.scale;
        let tw = TRACK_W * s;
        let th = TRACK_H * s;
        let track_y = self.rect.y + (self.rect.h - th) * 0.5;
        Rect::new(self.rect.x, track_y, tw, th)
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
        let opacity = if self.disabled { 0.4 } else { 1.0 };
        let t = self.transition.unwrap_or(if self.on { 1.0 } else { 0.0 });
        let track = self.track_rect();
        let tw = TRACK_W * s;
        let th = TRACK_H * s;
        let tr = TRACK_RADIUS * s;
        let thumb_r_base = THUMB_RADIUS * s;
        let thumb_margin = THUMB_MARGIN * s;
        let label_gap = LABEL_GAP * s;
        let label_font = LABEL_FONT_SIZE * s;

        // -- Track background: interpolate between muted and accent --
        let track_color = lerp_color(palette.surface_2, palette.accent, t).with_alpha(opacity);
        painter.rect_filled(track, tr, track_color);

        // Track border
        let border_color = if self.on {
            palette.accent.with_alpha(0.3 * opacity)
        } else {
            palette.muted.with_alpha(0.4 * opacity)
        };
        painter.rect_stroke_sdf(track, tr, 1.0 * s, border_color);

        // -- Thumb --
        let thumb_off_x = track.x + thumb_margin + thumb_r_base;
        let thumb_on_x = track.x + tw - thumb_margin - thumb_r_base;
        let thumb_x = thumb_off_x + (thumb_on_x - thumb_off_x) * t;
        let thumb_y = track.y + th * 0.5;
        let thumb_r = if self.hovered && !self.disabled {
            thumb_r_base + 1.0 * s
        } else {
            thumb_r_base
        };

        painter.circle_filled(thumb_x, thumb_y, thumb_r, Color::WHITE.with_alpha(opacity));
        painter.circle_stroke(
            thumb_x,
            thumb_y,
            thumb_r,
            1.0 * s,
            Color::BLACK.with_alpha(0.12 * opacity),
        );

        // Hover glow
        if self.hovered && !self.disabled {
            painter.circle_stroke(
                thumb_x,
                thumb_y,
                thumb_r + 3.0 * s,
                2.0 * s,
                palette.accent.with_alpha(0.2),
            );
        }

        // -- Label --
        if let Some(label) = self.label {
            let text_x = track.x + tw + label_gap;
            let text_y = self.rect.y + (self.rect.h - label_font) * 0.5;
            let text_color = if self.disabled {
                palette.muted
            } else {
                palette.text
            };
            let max_w = (self.rect.w - tw - label_gap).max(20.0);
            text_renderer.queue(
                label,
                label_font,
                text_x,
                text_y,
                text_color.with_alpha(opacity),
                max_w,
                screen_w,
                screen_h,
            );
        }
    }
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}
