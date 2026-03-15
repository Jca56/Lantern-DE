use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_theme::FONT_BODY;

use super::palette::FoxPalette;

/// Size of the checkbox box in pixels.
const BOX_SIZE: f32 = 28.0;
/// Corner radius for the checkbox box.
const BOX_RADIUS: f32 = 6.0;
/// Gap between the checkbox box and the label text.
const LABEL_GAP: f32 = 12.0;
/// Border width at rest.
const BORDER_WIDTH: f32 = 2.0;
/// Checkmark line thickness.
const CHECK_THICKNESS: f32 = 2.5;

/// A toggle/checkbox widget with optional label.
///
/// ```ignore
/// Checkbox::new(hit_rect, is_checked)
///     .label("Enable feature")
///     .hovered(true)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct Checkbox<'a> {
    rect: Rect,
    checked: bool,
    label: Option<&'a str>,
    hovered: bool,
    disabled: bool,
    scale: f32,
}

impl<'a> Checkbox<'a> {
    /// Create a new checkbox. `rect` is the overall hit area.
    pub fn new(rect: Rect, checked: bool) -> Self {
        Self {
            rect,
            checked,
            label: None,
            hovered: false,
            disabled: false,
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

    /// Returns the rect of the clickable box (for external hit-testing).
    pub fn box_rect(&self) -> Rect {
        let s = self.scale;
        let sz = BOX_SIZE * s;
        let box_y = self.rect.y + (self.rect.h - sz) * 0.5;
        Rect::new(self.rect.x, box_y, sz, sz)
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
        let sz = BOX_SIZE * s;
        let br = BOX_RADIUS * s;
        let bw = BORDER_WIDTH * s;
        let ct = CHECK_THICKNESS * s;
        let gap = LABEL_GAP * s;
        let font = FONT_BODY * s;
        let opacity = if self.disabled { 0.4 } else { 1.0 };
        let box_rect = self.box_rect();

        // -- Box background --
        if self.checked {
            // Filled with accent when checked
            let bg = palette.accent.with_alpha(opacity);
            painter.rect_filled(box_rect, br, bg);

            // Subtle darker border on checked state
            painter.rect_stroke_sdf(
                box_rect,
                br,
                1.0 * s,
                Color::BLACK.with_alpha(0.15 * opacity),
            );

            // -- Checkmark (two lines forming a tick) --
            let cx = box_rect.x + sz * 0.5;
            let cy = box_rect.y + sz * 0.5;
            let check_color = Color::from_rgb8(20, 20, 20).with_alpha(opacity);
            let x1 = cx - 6.0 * s;
            let y1 = cy;
            let x2 = cx - 1.5 * s;
            let y2 = cy + 5.0 * s;
            let x3 = cx + 7.0 * s;
            let y3 = cy - 5.5 * s;
            painter.line(x1, y1, x2, y2, ct, check_color);
            painter.line(x2, y2, x3, y3, ct, check_color);
        } else {
            // Empty box — surface background with border
            let bg = if self.hovered && !self.disabled {
                palette.surface_2
            } else {
                palette.surface
            };
            painter.rect_filled(box_rect, br, bg.with_alpha(opacity));

            let border_color = if self.hovered && !self.disabled {
                palette.accent.with_alpha(0.6 * opacity)
            } else {
                palette.muted.with_alpha(0.5 * opacity)
            };
            painter.rect_stroke_sdf(box_rect, br, bw, border_color);
        }

        // -- Hover ring --
        if self.hovered && !self.disabled {
            let expand = 3.0 * s;
            let ring = Rect::new(
                box_rect.x - expand,
                box_rect.y - expand,
                sz + expand * 2.0,
                sz + expand * 2.0,
            );
            painter.rect_stroke_sdf(ring, br + expand, 1.5 * s, palette.accent.with_alpha(0.25));
        }

        // -- Label text --
        if let Some(label) = self.label {
            let text_x = box_rect.x + sz + gap;
            let text_y = self.rect.y + (self.rect.h - font) * 0.5;
            let text_color = if self.disabled {
                palette.muted
            } else {
                palette.text
            };
            let max_w = (self.rect.w - sz - gap).max(20.0);
            text_renderer.queue(
                label,
                font,
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
