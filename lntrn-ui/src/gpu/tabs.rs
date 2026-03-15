use lntrn_render::{Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

/// Font size for tab labels (base, before scale).
const TAB_FONT_SIZE: f32 = 22.0;
/// Height of the active tab's bottom indicator bar.
const INDICATOR_HEIGHT: f32 = 3.0;
/// Corner radius for the indicator bar ends.
const INDICATOR_RADIUS: f32 = 1.5;
/// Horizontal padding inside each tab.
const TAB_PAD_X: f32 = 24.0;
/// Size of the close button hit area.
const CLOSE_SIZE: f32 = 20.0;
/// Size of the X icon lines inside the close button.
const CLOSE_ICON: f32 = 5.0;
/// Gap between label and close button.
const CLOSE_GAP: f32 = 6.0;
/// Size of the "+" new-tab button.
const NEW_TAB_SIZE: f32 = 28.0;

/// A horizontal tab bar with close buttons and a new-tab button.
///
/// ```ignore
/// TabBar::new(bar_rect)
///     .tabs(&["Home", "Documents"])
///     .selected(0)
///     .scale(1.25)
///     .closable(true)
///     .hovered_tab(Some(1))
///     .hovered_close(None)
///     .hovered_new_tab(false)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct TabBar<'a> {
    rect: Rect,
    tabs: &'a [&'a str],
    selected: usize,
    scale: f32,
    hovered_tab: Option<usize>,
    hovered_close: Option<usize>,
    closable: bool,
    hovered_new_tab: bool,
}

impl<'a> TabBar<'a> {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            tabs: &[],
            selected: 0,
            scale: 1.0,
            hovered_tab: None,
            hovered_close: None,
            closable: true,
            hovered_new_tab: false,
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

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    pub fn hovered_tab(mut self, index: Option<usize>) -> Self {
        self.hovered_tab = index;
        self
    }

    pub fn hovered_close(mut self, index: Option<usize>) -> Self {
        self.hovered_close = index;
        self
    }

    pub fn hovered_new_tab(mut self, hovered: bool) -> Self {
        self.hovered_new_tab = hovered;
        self
    }

    // Scaled helpers
    fn s(&self) -> f32 { self.scale }
    fn font_size(&self) -> f32 { TAB_FONT_SIZE * self.s() }
    fn pad_x(&self) -> f32 { TAB_PAD_X * self.s() }
    fn close_size(&self) -> f32 { CLOSE_SIZE * self.s() }
    fn close_icon(&self) -> f32 { CLOSE_ICON * self.s() }
    fn close_gap(&self) -> f32 { CLOSE_GAP * self.s() }
    fn indicator_h(&self) -> f32 { INDICATOR_HEIGHT * self.s() }
    fn indicator_r(&self) -> f32 { INDICATOR_RADIUS * self.s() }
    fn new_tab_size(&self) -> f32 { NEW_TAB_SIZE * self.s() }

    /// Width of a single tab including close button space if closable.
    fn tab_width(&self, label: &str) -> f32 {
        let text_w = label.len() as f32 * self.font_size() * 0.52;
        let close_extra = if self.closable { self.close_gap() + self.close_size() } else { 0.0 };
        text_w + self.pad_x() * 2.0 + close_extra
    }

    /// Compute the bounding rect for each tab.
    pub fn tab_rects(&self) -> Vec<Rect> {
        let mut rects = Vec::with_capacity(self.tabs.len());
        let mut x = self.rect.x;
        for label in self.tabs {
            let tab_w = self.tab_width(label);
            rects.push(Rect::new(x, self.rect.y, tab_w, self.rect.h));
            x += tab_w;
        }
        rects
    }

    /// Compute the close button rect for each tab (only meaningful if closable).
    pub fn close_rects(&self) -> Vec<Rect> {
        let cs = self.close_size();
        let tab_rects = self.tab_rects();
        tab_rects.iter().map(|tr| {
            let cx = tr.x + tr.w - self.pad_x() * 0.5 - cs;
            let cy = tr.y + (tr.h - cs) * 0.5;
            Rect::new(cx, cy, cs, cs)
        }).collect()
    }

    /// Compute the "+" new-tab button rect (positioned after the last tab).
    pub fn new_tab_rect(&self) -> Rect {
        let nts = self.new_tab_size();
        let tab_rects = self.tab_rects();
        let x = tab_rects.last()
            .map(|r| r.x + r.w + 4.0 * self.s())
            .unwrap_or(self.rect.x);
        let y = self.rect.y + (self.rect.h - nts) * 0.5;
        Rect::new(x, y, nts, nts)
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

        let s = self.s();
        let font = self.font_size();
        let ind_h = self.indicator_h();

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
        let close_rects = if self.closable { self.close_rects() } else { Vec::new() };

        for (i, (label, tab_rect)) in self.tabs.iter().zip(&tab_rects).enumerate() {
            let is_selected = i == self.selected;
            let is_tab_hovered = self.hovered_tab == Some(i) && !is_selected;
            let is_close_hovered = self.closable && self.hovered_close == Some(i);

            // Hover highlight background
            if is_tab_hovered {
                painter.rect_filled(
                    *tab_rect,
                    0.0,
                    palette.text.with_alpha(0.04),
                );
            }

            // Label text
            let text_color = if is_selected {
                palette.text
            } else if is_tab_hovered {
                palette.text_secondary
            } else {
                palette.muted
            };

            let text_w = label.len() as f32 * font * 0.52;
            let label_area_w = if self.closable {
                tab_rect.w - self.close_gap() - self.close_size() - self.pad_x()
            } else {
                tab_rect.w
            };
            let text_x = tab_rect.x + (label_area_w - text_w) * 0.5;
            let text_y = tab_rect.y + (tab_rect.h - font) * 0.5 - ind_h * 0.5;
            text_renderer.queue(
                label,
                font,
                text_x,
                text_y,
                text_color,
                label_area_w,
                screen_w,
                screen_h,
            );

            // Close button
            if self.closable {
                let cr = close_rects[i];
                let close_color = if is_close_hovered {
                    painter.rect_filled(cr, 4.0 * s, palette.danger.with_alpha(0.15));
                    palette.danger
                } else if is_selected || is_tab_hovered {
                    palette.text_secondary
                } else {
                    palette.muted.with_alpha(0.5)
                };

                let ccx = cr.x + cr.w * 0.5;
                let ccy = cr.y + cr.h * 0.5;
                let half = self.close_icon() * 0.5;
                let stroke = 1.5 * s;
                painter.line(ccx - half, ccy - half, ccx + half, ccy + half, stroke, close_color);
                painter.line(ccx + half, ccy - half, ccx - half, ccy + half, stroke, close_color);
            }

            // Active tab indicator bar
            if is_selected {
                let ind_w = tab_rect.w - self.pad_x() * 0.5;
                let ind_x = tab_rect.x + (tab_rect.w - ind_w) * 0.5;
                let ind_y = tab_rect.y + tab_rect.h - ind_h;
                painter.rect_filled(
                    Rect::new(ind_x, ind_y, ind_w, ind_h),
                    self.indicator_r(),
                    palette.accent,
                );
            }
        }

        // -- New tab "+" button --
        let nt = self.new_tab_rect();
        let plus_color = if self.hovered_new_tab {
            painter.rect_filled(nt, 6.0 * s, palette.text.with_alpha(0.06));
            palette.text
        } else {
            palette.muted
        };
        let pcx = nt.x + nt.w * 0.5;
        let pcy = nt.y + nt.h * 0.5;
        let arm = 6.0 * s;
        let stroke = 2.0 * s;
        painter.line(pcx - arm, pcy, pcx + arm, pcy, stroke, plus_color);
        painter.line(pcx, pcy - arm, pcx, pcy + arm, stroke, plus_color);
    }
}
