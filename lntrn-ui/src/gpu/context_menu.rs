use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::input::{InteractionContext, InteractionState};
use super::palette::FoxPalette;

/// Visual style for the context menu.
pub struct ContextMenuStyle {
    pub bg: Color,
    pub bg_hover: Color,
    pub text: Color,
    pub text_muted: Color,
    pub separator: Color,
    pub border: Color,
    pub corner_radius: f32,
    pub padding: f32,
    pub item_height: f32,
    pub font_size: f32,
    pub min_width: f32,
    pub border_width: f32,
}

impl ContextMenuStyle {
    pub fn from_palette(palette: &FoxPalette) -> Self {
        Self {
            bg: palette.surface,
            bg_hover: palette.surface_2,
            text: palette.text,
            text_muted: palette.muted,
            separator: Color::rgba(1.0, 1.0, 1.0, 0.08),
            border: Color::rgba(1.0, 1.0, 1.0, 0.1),
            corner_radius: 8.0,
            padding: 4.0,
            item_height: 32.0,
            font_size: 22.0,
            min_width: 180.0,
            border_width: 1.0,
        }
    }
}

/// A single entry in a context menu.
#[derive(Clone, Debug)]
pub enum MenuItem {
    /// Clickable action with a label.
    Action { id: u32, label: String },
    /// Visual separator line.
    Separator,
}

impl MenuItem {
    pub fn action(id: u32, label: impl Into<String>) -> Self {
        Self::Action {
            id,
            label: label.into(),
        }
    }

    pub fn separator() -> Self {
        Self::Separator
    }
}

/// A right-click context menu.
///
/// Usage:
/// 1. Create with `ContextMenu::new(style)`
/// 2. On right-click: call `open(x, y, items)`
/// 3. Each frame: call `draw(painter, text, interaction)` — returns `Some(id)` if an item was clicked
/// 4. Call `close()` or it auto-closes on click-outside
pub struct ContextMenu {
    style: ContextMenuStyle,
    items: Vec<MenuItem>,
    x: f32,
    y: f32,
    open: bool,
    /// Computed width (set on open, based on longest label).
    width: f32,
}

impl ContextMenu {
    pub fn new(style: ContextMenuStyle) -> Self {
        Self {
            style,
            items: Vec::new(),
            x: 0.0,
            y: 0.0,
            open: false,
            width: 0.0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the menu at pixel position (x, y) with the given items.
    /// Estimates width from the longest label.
    pub fn open(&mut self, x: f32, y: f32, items: Vec<MenuItem>) {
        // Estimate width from longest label (rough: char_count * font_size * 0.55)
        let max_label_w = items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { label, .. } => Some(label.len() as f32 * self.style.font_size * 0.55),
                MenuItem::Separator => None,
            })
            .fold(0.0f32, f32::max);

        self.width = (max_label_w + self.style.padding * 4.0).max(self.style.min_width);
        self.items = items;
        self.x = x;
        self.y = y;
        self.open = true;
    }

    /// Reposition the menu so it stays within the given screen bounds.
    pub fn clamp_to_screen(&mut self, screen_w: f32, screen_h: f32) {
        let total_h = self.total_height();
        if self.x + self.width > screen_w {
            self.x = (screen_w - self.width - 4.0).max(0.0);
        }
        if self.y + total_h > screen_h {
            self.y = (screen_h - total_h - 4.0).max(0.0);
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
    }

    /// Draw the menu. Returns `Some(action_id)` if an item was clicked this frame.
    ///
    /// The caller should also check for clicks outside the menu rect
    /// and call `close()` if desired.
    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        interaction: &mut InteractionContext,
        screen_w: u32,
        screen_h: u32,
    ) -> Option<u32> {
        if !self.open {
            return None;
        }

        let total_h = self.total_height();
        let menu_rect = Rect::new(self.x, self.y, self.width, total_h);

        // Background + border
        painter.rect_filled(menu_rect, self.style.corner_radius, self.style.bg);
        painter.rect_stroke(
            menu_rect,
            self.style.corner_radius,
            self.style.border_width,
            self.style.border,
        );

        let mut clicked_id = None;
        let mut cy = self.y + self.style.padding;
        let inner_w = self.width - self.style.padding * 2.0;
        let inner_x = self.x + self.style.padding;

        for item in &self.items {
            match item {
                MenuItem::Action { id, label } => {
                    let item_rect = Rect::new(inner_x, cy, inner_w, self.style.item_height);

                    // Zone ID: offset by a base to avoid collision with app zones.
                    // Use the action id directly — caller is responsible for uniqueness.
                    let zone_id = CONTEXT_MENU_ZONE_BASE + *id;
                    let state = interaction.add_zone(zone_id, item_rect);

                    // Hover highlight
                    if state == InteractionState::Hovered || state == InteractionState::Pressed {
                        painter.rect_filled(
                            item_rect,
                            self.style.corner_radius - 2.0,
                            self.style.bg_hover,
                        );
                    }

                    if state == InteractionState::Pressed {
                        clicked_id = Some(*id);
                    }

                    // Label text
                    let text_x = inner_x + self.style.padding * 2.0;
                    let text_y = cy + (self.style.item_height - self.style.font_size) * 0.5;
                    text.queue(
                        label,
                        self.style.font_size,
                        text_x,
                        text_y,
                        self.style.text,
                        inner_w - self.style.padding * 4.0,
                        screen_w,
                        screen_h,
                    );

                    cy += self.style.item_height;
                }
                MenuItem::Separator => {
                    let sep_y = cy + SEPARATOR_HEIGHT * 0.5;
                    let sep_x = inner_x + self.style.padding;
                    let sep_w = inner_w - self.style.padding * 2.0;
                    painter.rect_filled(
                        Rect::new(sep_x, sep_y, sep_w, 1.0),
                        0.0,
                        self.style.separator,
                    );
                    cy += SEPARATOR_HEIGHT;
                }
            }
        }

        clicked_id
    }

    /// Check if a point is inside the menu bounds (for dismiss-on-click-outside).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if !self.open {
            return false;
        }
        let total_h = self.total_height();
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + total_h
    }

    /// Total height of the menu including padding.
    fn total_height(&self) -> f32 {
        let content_h: f32 = self
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Action { .. } => self.style.item_height,
                MenuItem::Separator => SEPARATOR_HEIGHT,
            })
            .sum();
        content_h + self.style.padding * 2.0
    }
}

const SEPARATOR_HEIGHT: f32 = 9.0;

/// Base zone ID offset for context menu items to avoid collisions
/// with the host application's own hit zones.
const CONTEXT_MENU_ZONE_BASE: u32 = 0xCE_0000;
