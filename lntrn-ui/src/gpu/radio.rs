use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

const OUTER_RADIUS: f32 = 12.0;
const INNER_RADIUS: f32 = 5.0;
const BORDER_WIDTH: f32 = 2.0;
const LABEL_GAP: f32 = 12.0;
const LABEL_FONT_SIZE: f32 = 20.0;
/// Vertical spacing between items when drawn as a group.
const ITEM_HEIGHT: f32 = 36.0;

/// A single radio button with optional label.
///
/// ```ignore
/// RadioButton::new(rect, is_selected)
///     .label("Option A")
///     .hovered(true)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct RadioButton<'a> {
    rect: Rect,
    selected: bool,
    label: Option<&'a str>,
    hovered: bool,
    disabled: bool,
}

impl<'a> RadioButton<'a> {
    pub fn new(rect: Rect, selected: bool) -> Self {
        Self {
            rect,
            selected,
            label: None,
            hovered: false,
            disabled: false,
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

    /// Returns the rect of the clickable circle area (for external hit-testing).
    pub fn circle_rect(&self) -> Rect {
        let cx = self.rect.x + OUTER_RADIUS;
        let cy = self.rect.y + self.rect.h * 0.5;
        Rect::new(
            cx - OUTER_RADIUS,
            cy - OUTER_RADIUS,
            OUTER_RADIUS * 2.0,
            OUTER_RADIUS * 2.0,
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
        let opacity = if self.disabled { 0.4 } else { 1.0 };
        let cx = self.rect.x + OUTER_RADIUS;
        let cy = self.rect.y + self.rect.h * 0.5;

        // -- Outer circle --
        if self.selected {
            // Accent-filled ring
            painter.circle_filled(cx, cy, OUTER_RADIUS, palette.accent.with_alpha(opacity));
            // Inner dot (contrasting)
            painter.circle_filled(cx, cy, INNER_RADIUS, Color::from_rgb8(20, 20, 20).with_alpha(opacity));
        } else {
            // Empty circle with border
            let bg = if self.hovered && !self.disabled {
                palette.surface_2
            } else {
                palette.surface
            };
            painter.circle_filled(cx, cy, OUTER_RADIUS, bg.with_alpha(opacity));

            let border_color = if self.hovered && !self.disabled {
                palette.accent.with_alpha(0.6 * opacity)
            } else {
                palette.muted.with_alpha(0.5 * opacity)
            };
            painter.circle_stroke(cx, cy, OUTER_RADIUS, BORDER_WIDTH, border_color);
        }

        // -- Hover ring --
        if self.hovered && !self.disabled {
            painter.circle_stroke(
                cx,
                cy,
                OUTER_RADIUS + 3.0,
                1.5,
                palette.accent.with_alpha(0.25),
            );
        }

        // -- Label --
        if let Some(label) = self.label {
            let text_x = self.rect.x + OUTER_RADIUS * 2.0 + LABEL_GAP;
            let text_y = self.rect.y + (self.rect.h - LABEL_FONT_SIZE) * 0.5;
            let text_color = if self.disabled {
                palette.muted
            } else {
                palette.text
            };
            let max_w = (self.rect.w - OUTER_RADIUS * 2.0 - LABEL_GAP).max(20.0);
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

/// A vertical group of radio buttons. Selection logic is external —
/// this just draws them and reports which index was clicked.
///
/// ```ignore
/// let clicked = RadioGroup::new(x, y, &["Small", "Medium", "Large"], selected_idx)
///     .hovered_index(hovered)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct RadioGroup<'a> {
    x: f32,
    y: f32,
    options: &'a [&'a str],
    selected: usize,
    hovered_index: Option<usize>,
    disabled: bool,
    width: f32,
}

impl<'a> RadioGroup<'a> {
    pub fn new(x: f32, y: f32, options: &'a [&'a str], selected: usize) -> Self {
        Self {
            x,
            y,
            options,
            selected,
            hovered_index: None,
            disabled: false,
            width: 250.0,
        }
    }

    pub fn hovered_index(mut self, idx: Option<usize>) -> Self {
        self.hovered_index = idx;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    /// Returns the rect for a specific option index (for hit-testing).
    pub fn item_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.x,
            self.y + index as f32 * ITEM_HEIGHT,
            self.width,
            ITEM_HEIGHT,
        )
    }

    /// Total height of the group.
    pub fn total_height(&self) -> f32 {
        self.options.len() as f32 * ITEM_HEIGHT
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        for (i, label) in self.options.iter().enumerate() {
            let rect = self.item_rect(i);
            RadioButton::new(rect, i == self.selected)
                .label(label)
                .hovered(self.hovered_index == Some(i))
                .disabled(self.disabled)
                .draw(painter, text_renderer, palette, screen_w, screen_h);
        }
    }
}
