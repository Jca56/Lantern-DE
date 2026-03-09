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

    /// Set the animation progress (0.0 = fully off, 1.0 = fully on).
    /// When not set, the toggle snaps to its current state.
    pub fn transition(mut self, t: f32) -> Self {
        self.transition = Some(t.clamp(0.0, 1.0));
        self
    }

    /// Returns the rect of the track (for external hit-testing).
    pub fn track_rect(&self) -> Rect {
        let track_y = self.rect.y + (self.rect.h - TRACK_H) * 0.5;
        Rect::new(self.rect.x, track_y, TRACK_W, TRACK_H)
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let opacity = if self.disabled { 0.4 } else { 1.0 };
        let t = self.transition.unwrap_or(if self.on { 1.0 } else { 0.0 });
        let track = self.track_rect();

        // -- Track background: interpolate between muted and accent --
        let track_color = lerp_color(palette.surface_2, palette.accent, t).with_alpha(opacity);
        painter.rect_filled(track, TRACK_RADIUS, track_color);

        // Track border
        let border_color = if self.on {
            palette.accent.with_alpha(0.3 * opacity)
        } else {
            palette.muted.with_alpha(0.4 * opacity)
        };
        painter.rect_stroke(track, TRACK_RADIUS, 1.0, border_color);

        // -- Thumb --
        let thumb_off_x = track.x + THUMB_MARGIN + THUMB_RADIUS;
        let thumb_on_x = track.x + TRACK_W - THUMB_MARGIN - THUMB_RADIUS;
        let thumb_x = thumb_off_x + (thumb_on_x - thumb_off_x) * t;
        let thumb_y = track.y + TRACK_H * 0.5;
        let thumb_r = if self.hovered && !self.disabled {
            THUMB_RADIUS + 1.0
        } else {
            THUMB_RADIUS
        };

        painter.circle_filled(thumb_x, thumb_y, thumb_r, Color::WHITE.with_alpha(opacity));
        painter.circle_stroke(
            thumb_x,
            thumb_y,
            thumb_r,
            1.0,
            Color::BLACK.with_alpha(0.12 * opacity),
        );

        // Hover glow
        if self.hovered && !self.disabled {
            painter.circle_stroke(
                thumb_x,
                thumb_y,
                thumb_r + 3.0,
                2.0,
                palette.accent.with_alpha(0.2),
            );
        }

        // -- Label --
        if let Some(label) = self.label {
            let text_x = track.x + TRACK_W + LABEL_GAP;
            let text_y = self.rect.y + (self.rect.h - LABEL_FONT_SIZE) * 0.5;
            let text_color = if self.disabled {
                palette.muted
            } else {
                palette.text
            };
            let max_w = (self.rect.w - TRACK_W - LABEL_GAP).max(20.0);
            text_renderer.queue(
                label,
                LABEL_FONT_SIZE,
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
