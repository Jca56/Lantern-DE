use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

const CORNER_RADIUS: f32 = 6.0;
const FONT_SIZE: f32 = 20.0;
const PADDING_H: f32 = 14.0;
const PADDING_V: f32 = 8.0;
const ITEM_HEIGHT: f32 = 34.0;
const CHEVRON_SIZE: f32 = 8.0;
const DROPDOWN_BORDER: f32 = 1.0;
const DROPDOWN_RADIUS: f32 = 8.0;
const DROPDOWN_SHADOW_ALPHA: f32 = 0.25;
const DROPDOWN_GAP: f32 = 4.0;

/// Event returned by [`Dropdown::draw`].
#[derive(Debug)]
pub enum DropdownEvent {
    /// The dropdown button was clicked (toggle open/close).
    Toggle,
    /// An option was selected by index.
    Selected(usize),
}

/// A dropdown / select widget.
///
/// ```ignore
/// let evt = Dropdown::new(rect, &options, selected_index)
///     .open(is_open)
///     .hovered_option(hovered_idx)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct Dropdown<'a> {
    rect: Rect,
    options: &'a [&'a str],
    selected: usize,
    open: bool,
    button_hovered: bool,
    hovered_option: Option<usize>,
    disabled: bool,
    /// Direction the dropdown opens. If true, opens upward.
    open_up: bool,
}

impl<'a> Dropdown<'a> {
    pub fn new(rect: Rect, options: &'a [&'a str], selected: usize) -> Self {
        Self {
            rect,
            options,
            selected,
            open: false,
            button_hovered: false,
            hovered_option: None,
            disabled: false,
            open_up: false,
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn button_hovered(mut self, hovered: bool) -> Self {
        self.button_hovered = hovered;
        self
    }

    pub fn hovered_option(mut self, idx: Option<usize>) -> Self {
        self.hovered_option = idx;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Open upward instead of downward.
    pub fn open_up(mut self, up: bool) -> Self {
        self.open_up = up;
        self
    }

    /// Rect for the dropdown button (for hit-testing).
    pub fn button_rect(&self) -> Rect {
        self.rect
    }

    /// Rect for a specific option in the dropdown list (for hit-testing).
    /// Only valid when open.
    pub fn option_rect(&self, index: usize) -> Rect {
        let list_rect = self.list_rect();
        let pad = PADDING_V;
        Rect::new(
            list_rect.x + pad,
            list_rect.y + pad + index as f32 * ITEM_HEIGHT,
            list_rect.w - pad * 2.0,
            ITEM_HEIGHT,
        )
    }

    /// Rect of the entire dropdown list popup (for hit-testing / dismiss).
    pub fn list_rect(&self) -> Rect {
        let list_h = self.options.len() as f32 * ITEM_HEIGHT + PADDING_V * 2.0;
        let list_y = if self.open_up {
            self.rect.y - list_h - DROPDOWN_GAP
        } else {
            self.rect.y + self.rect.h + DROPDOWN_GAP
        };
        Rect::new(self.rect.x, list_y, self.rect.w, list_h)
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

        // -- Button --
        let btn_bg = if self.open {
            palette.surface_2
        } else if self.button_hovered && !self.disabled {
            palette.surface_2
        } else {
            palette.surface
        };
        painter.rect_filled(self.rect, CORNER_RADIUS, btn_bg.with_alpha(opacity));
        painter.rect_stroke(
            self.rect,
            CORNER_RADIUS,
            DROPDOWN_BORDER,
            palette.muted.with_alpha(0.3 * opacity),
        );

        // Selected text
        let label = self
            .options
            .get(self.selected)
            .copied()
            .unwrap_or("—");
        let text_y = self.rect.y + (self.rect.h - FONT_SIZE) * 0.5;
        text_renderer.queue(
            label,
            FONT_SIZE,
            self.rect.x + PADDING_H,
            text_y,
            palette.text.with_alpha(opacity),
            self.rect.w - PADDING_H * 2.0 - CHEVRON_SIZE - 8.0,
            screen_w,
            screen_h,
        );

        // Chevron (▼ or ▲)
        let chev_x = self.rect.x + self.rect.w - PADDING_H - CHEVRON_SIZE;
        let chev_cy = self.rect.y + self.rect.h * 0.5;
        let chev_color = palette.text_secondary.with_alpha(opacity);
        if self.open {
            // ▲
            painter.line(
                chev_x,
                chev_cy + CHEVRON_SIZE * 0.35,
                chev_x + CHEVRON_SIZE * 0.5,
                chev_cy - CHEVRON_SIZE * 0.35,
                1.5,
                chev_color,
            );
            painter.line(
                chev_x + CHEVRON_SIZE * 0.5,
                chev_cy - CHEVRON_SIZE * 0.35,
                chev_x + CHEVRON_SIZE,
                chev_cy + CHEVRON_SIZE * 0.35,
                1.5,
                chev_color,
            );
        } else {
            // ▼
            painter.line(
                chev_x,
                chev_cy - CHEVRON_SIZE * 0.35,
                chev_x + CHEVRON_SIZE * 0.5,
                chev_cy + CHEVRON_SIZE * 0.35,
                1.5,
                chev_color,
            );
            painter.line(
                chev_x + CHEVRON_SIZE * 0.5,
                chev_cy + CHEVRON_SIZE * 0.35,
                chev_x + CHEVRON_SIZE,
                chev_cy - CHEVRON_SIZE * 0.35,
                1.5,
                chev_color,
            );
        }

        // -- Dropdown list (only when open) --
        if !self.open {
            return;
        }

        let list = self.list_rect();

        // Shadow
        let shadow = list.expand(4.0);
        painter.rect_filled(
            shadow,
            DROPDOWN_RADIUS + 2.0,
            Color::BLACK.with_alpha(DROPDOWN_SHADOW_ALPHA),
        );

        // Background
        painter.rect_filled(list, DROPDOWN_RADIUS, palette.surface);
        painter.rect_stroke(list, DROPDOWN_RADIUS, DROPDOWN_BORDER, palette.muted.with_alpha(0.2));

        // Items
        for (i, option) in self.options.iter().enumerate() {
            let item_rect = self.option_rect(i);
            let is_selected = i == self.selected;
            let is_hovered = self.hovered_option == Some(i);

            if is_hovered {
                painter.rect_filled(
                    item_rect,
                    CORNER_RADIUS,
                    palette.surface_2,
                );
            }

            let text_color = if is_selected {
                palette.accent
            } else {
                palette.text
            };

            let item_text_y = item_rect.y + (ITEM_HEIGHT - FONT_SIZE) * 0.5;
            text_renderer.queue(
                option,
                FONT_SIZE,
                item_rect.x + PADDING_H,
                item_text_y,
                text_color,
                item_rect.w - PADDING_H * 2.0,
                screen_w,
                screen_h,
            );

            // Checkmark for selected item
            if is_selected {
                let check_x = item_rect.x + item_rect.w - PADDING_H - 10.0;
                let check_cy = item_rect.y + ITEM_HEIGHT * 0.5;
                painter.line(
                    check_x,
                    check_cy,
                    check_x + 4.0,
                    check_cy + 4.0,
                    2.0,
                    palette.accent,
                );
                painter.line(
                    check_x + 4.0,
                    check_cy + 4.0,
                    check_x + 12.0,
                    check_cy - 4.0,
                    2.0,
                    palette.accent,
                );
            }
        }
    }
}
