use lntrn_render::{Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

/// Font size for tab labels (Small = 22px).
const TAB_FONT_SIZE: f32 = 22.0;
/// Height of the active tab's bottom indicator bar.
const INDICATOR_HEIGHT: f32 = 3.0;
/// Corner radius for the indicator bar ends.
const INDICATOR_RADIUS: f32 = 1.5;
/// Horizontal padding inside each tab.
const TAB_PAD_X: f32 = 24.0;

/// A horizontal tab bar.
///
/// ```ignore
/// TabBar::new(bar_rect)
///     .tabs(&["General", "Appearance", "Advanced"])
///     .selected(0)
///     .hovered(Some(1))
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct TabBar<'a> {
    rect: Rect,
    tabs: &'a [&'a str],
    selected: usize,
    hovered: Option<usize>,
}

impl<'a> TabBar<'a> {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            tabs: &[],
            selected: 0,
            hovered: None,
        }
    }

    pub fn tabs(mut self, tabs: &'a [&'a str]) -> Self {
        self.tabs = tabs;
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    pub fn hovered(mut self, index: Option<usize>) -> Self {
        self.hovered = index;
        self
    }

    /// Compute the bounding rect for each tab. Tabs are sized to their
    /// label width plus padding, laid out left-to-right.
    pub fn tab_rects(&self) -> Vec<Rect> {
        let mut rects = Vec::with_capacity(self.tabs.len());
        let mut x = self.rect.x;
        for label in self.tabs {
            let text_w = label.len() as f32 * TAB_FONT_SIZE * 0.52;
            let tab_w = text_w + TAB_PAD_X * 2.0;
            rects.push(Rect::new(x, self.rect.y, tab_w, self.rect.h));
            x += tab_w;
        }
        rects
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        if self.tabs.is_empty() {
            return;
        }

        // -- Bar background --
        painter.rect_filled(self.rect, 0.0, palette.surface);

        // -- Bottom separator line across the full bar --
        let sep_y = self.rect.y + self.rect.h - 1.0;
        painter.rect_filled(
            Rect::new(self.rect.x, sep_y, self.rect.w, 1.0),
            0.0,
            palette.muted.with_alpha(0.2),
        );

        // -- Tabs --
        let tab_rects = self.tab_rects();
        for (i, (label, tab_rect)) in self.tabs.iter().zip(&tab_rects).enumerate() {
            let is_selected = i == self.selected;
            let is_hovered = self.hovered == Some(i) && !is_selected;

            // Hover highlight background
            if is_hovered {
                painter.rect_filled(
                    *tab_rect,
                    0.0,
                    palette.text.with_alpha(0.04),
                );
            }

            // Label text
            let text_color = if is_selected {
                palette.text
            } else if is_hovered {
                palette.text_secondary
            } else {
                palette.muted
            };

            let text_w = label.len() as f32 * TAB_FONT_SIZE * 0.52;
            let text_x = tab_rect.x + (tab_rect.w - text_w) * 0.5;
            let text_y = tab_rect.y + (tab_rect.h - TAB_FONT_SIZE) * 0.5 - INDICATOR_HEIGHT * 0.5;
            text_renderer.queue(
                label,
                TAB_FONT_SIZE,
                text_x,
                text_y,
                text_color,
                tab_rect.w,
                screen_w,
                screen_h,
            );

            // Active tab indicator bar (accent-colored bottom border)
            if is_selected {
                let ind_w = tab_rect.w - TAB_PAD_X * 0.5;
                let ind_x = tab_rect.x + (tab_rect.w - ind_w) * 0.5;
                let ind_y = tab_rect.y + tab_rect.h - INDICATOR_HEIGHT;
                painter.rect_filled(
                    Rect::new(ind_x, ind_y, ind_w, INDICATOR_HEIGHT),
                    INDICATOR_RADIUS,
                    palette.accent,
                );
            }
        }
    }
}
