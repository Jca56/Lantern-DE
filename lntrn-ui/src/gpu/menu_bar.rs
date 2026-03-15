use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_theme::FONT_BODY;

use super::context_menu::{ContextMenu, MenuItem};
use super::input::InteractionContext;
use super::palette::FoxPalette;

/// A horizontal menu bar (File, Edit, View, Help, etc.) that lives in the
/// title bar's content area. Each label opens a ContextMenu when clicked.
///
/// ```ignore
/// let menus = vec![
///     ("File", vec![MenuItem::action(1, "New"), MenuItem::action(2, "Open")]),
///     ("Edit", vec![MenuItem::action(3, "Undo"), MenuItem::action(4, "Redo")]),
/// ];
/// menu_bar.update(&mut ix, &menus, &fox, s);
/// menu_bar.draw(&mut painter, &mut text, &fox, sw, sh, s);
/// ```
pub struct MenuBar {
    pub context_menu: ContextMenu,
    open_index: Option<usize>,
    /// Cached rects for each label, set during draw.
    label_rects: Vec<Rect>,
}

/// Base zone ID for menu bar labels. Each label uses ZONE_BASE + index.
const ZONE_BASE: u32 = 500;
const LABEL_PAD_H: f32 = 8.0;
const LABEL_PAD_V: f32 = 4.0;

impl MenuBar {
    pub fn new(palette: &FoxPalette) -> Self {
        let style = super::context_menu::ContextMenuStyle::from_palette(palette);
        Self {
            context_menu: ContextMenu::new(style),
            open_index: None,
            label_rects: Vec::new(),
        }
    }

    pub fn is_open(&self) -> bool { self.open_index.is_some() }

    pub fn open_index(&self) -> Option<usize> { self.open_index }

    /// Call each frame before draw. Handles click-to-open, hover-to-switch,
    /// and click-outside-to-close.
    pub fn update(
        &mut self,
        ix: &mut InteractionContext,
        menus: &[(&str, Vec<MenuItem>)],
        rect: Rect,
        scale: f32,
    ) {
        // Compute label rects
        self.label_rects.clear();
        let font = FONT_BODY * scale;
        let pad_h = LABEL_PAD_H * scale;
        let pad_v = LABEL_PAD_V * scale;
        let mut x = rect.x + pad_h * 0.5;

        for (label, _) in menus {
            let text_w = label.len() as f32 * font * 0.52;
            let w = text_w + pad_h * 2.0;
            let h = font + pad_v * 2.0;
            let y = rect.y + (rect.h - h) * 0.5;
            self.label_rects.push(Rect::new(x, y, w, h));
            x += w;
        }

        // Register zones and check for hover-to-switch
        let mut switch_to = None;
        for (i, lr) in self.label_rects.iter().enumerate() {
            let zone = ix.add_zone(ZONE_BASE + i as u32, *lr);
            if self.open_index.is_some() && zone.is_hovered() && self.open_index != Some(i) {
                switch_to = Some(i);
            }
        }
        if let Some(i) = switch_to {
            self.open_menu(i, menus, scale);
        }
    }

    /// Handle a left click. Returns true if the menu bar consumed it.
    pub fn on_click(
        &mut self,
        ix: &mut InteractionContext,
        menus: &[(&str, Vec<MenuItem>)],
        scale: f32,
    ) -> bool {
        // Check if any label was clicked
        for (i, lr) in self.label_rects.iter().enumerate() {
            if let Some((cx, cy)) = ix.cursor() {
                if lr.contains(cx, cy) {
                    if self.open_index == Some(i) {
                        // Clicking the same label closes it
                        self.close();
                    } else {
                        self.open_menu(i, menus, scale);
                    }
                    return true;
                }
            }
        }

        // If menu is open and click is outside both labels and menu, close
        if self.open_index.is_some() {
            if let Some((cx, cy)) = ix.cursor() {
                if !self.context_menu.contains(cx, cy) {
                    self.close();
                    return true;
                }
            }
        }

        false
    }

    pub fn close(&mut self) {
        self.context_menu.close();
        self.open_index = None;
    }

    fn open_menu(&mut self, index: usize, menus: &[(&str, Vec<MenuItem>)], scale: f32) {
        if let (Some(lr), Some((_, items))) = (self.label_rects.get(index), menus.get(index)) {
            self.context_menu.set_scale(scale);
            self.context_menu.open(lr.x, lr.y + lr.h, items.clone());
            self.open_index = Some(index);
        }
    }

    /// Draw the menu bar labels. Call context_menu.draw() separately in the
    /// overlay pass.
    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        palette: &FoxPalette,
        sw: u32,
        sh: u32,
        scale: f32,
    ) {
        let font = FONT_BODY * scale;
        let pad_h = LABEL_PAD_H * scale;
        let r = 6.0 * scale;

        for (i, lr) in self.label_rects.iter().enumerate() {
            let is_open = self.open_index == Some(i);

            // Hover/active background
            if is_open {
                painter.rect_filled(*lr, r, palette.accent.with_alpha(0.15));
            }

            // Label text — centered in rect
            // We don't have the label text here, so we derive x from rect
            let text_x = lr.x + pad_h;
            let text_y = lr.y + (lr.h - font) * 0.5;
            let color = if is_open { palette.accent } else { palette.text };
            text.queue(
                "", font, text_x, text_y, color,
                lr.w, sw, sh,
            );
        }
    }

    /// Draw the labels with the actual label strings. Preferred over `draw()`.
    pub fn draw_with_labels(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        palette: &FoxPalette,
        labels: &[&str],
        sw: u32,
        sh: u32,
        scale: f32,
    ) {
        let font = FONT_BODY * scale;
        let pad_h = LABEL_PAD_H * scale;
        let r = 6.0 * scale;

        for (i, lr) in self.label_rects.iter().enumerate() {
            let is_open = self.open_index == Some(i);

            if is_open {
                painter.rect_filled(*lr, r, palette.accent.with_alpha(0.15));
            }

            let label = labels.get(i).copied().unwrap_or("");
            let text_x = lr.x + pad_h;
            let text_y = lr.y + (lr.h - font) * 0.5;
            let color = if is_open { palette.accent } else { palette.text };
            text.queue(label, font, text_x, text_y, color, lr.w, sw, sh);
        }
    }
}
