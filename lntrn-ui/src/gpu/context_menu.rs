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
    pub accent: Color,
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
            accent: palette.accent,
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
    /// Draggable slider (0.0–1.0).
    Slider { id: u32, label: String, value: f32 },
    /// A submenu that expands on hover.
    SubMenu { id: u32, label: String, children: Vec<MenuItem> },
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

    pub fn slider(id: u32, label: impl Into<String>, value: f32) -> Self {
        Self::Slider {
            id,
            label: label.into(),
            value,
        }
    }

    pub fn submenu(id: u32, label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self::SubMenu {
            id,
            label: label.into(),
            children,
        }
    }
}

/// Event returned by [`ContextMenu::draw`].
#[derive(Debug)]
pub enum MenuEvent {
    /// A menu action was clicked.
    Action(u32),
    /// A slider value changed during drag.
    SliderChanged { id: u32, value: f32 },
}

/// Tracks position and items for a single menu panel (root or submenu).
struct MenuPanel {
    x: f32,
    y: f32,
    width: f32,
    /// Index path to this panel's children in the root item tree.
    /// Empty = root, [2] = children of root item 2, [2,1] = grandchildren, etc.
    path: Vec<usize>,
}

/// A right-click context menu with nested submenu support.
///
/// Usage:
/// 1. Create with `ContextMenu::new(style)`
/// 2. On right-click: call `open(x, y, items)`
/// 3. Each frame: call `draw(...)` — returns `Some(MenuEvent)` on interaction
/// 4. Call `close()` or it auto-closes on click-outside
pub struct ContextMenu {
    style: ContextMenuStyle,
    items: Vec<MenuItem>,
    x: f32,
    y: f32,
    open: bool,
    width: f32,
    /// Stack of open submenu ids (from root to deepest).
    open_submenu_ids: Vec<u32>,
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
            open_submenu_ids: Vec::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the menu at pixel position (x, y) with the given items.
    pub fn open(&mut self, x: f32, y: f32, items: Vec<MenuItem>) {
        self.width = compute_width(&items, &self.style);
        self.items = items;
        self.x = x;
        self.y = y;
        self.open = true;
        self.open_submenu_ids.clear();
    }

    /// Reposition the menu so it stays within the given screen bounds.
    pub fn clamp_to_screen(&mut self, screen_w: f32, screen_h: f32) {
        let total_h = items_height(&self.items, &self.style);
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
        self.open_submenu_ids.clear();
    }

    /// Draw the menu and all open submenus. Returns an event if an item was clicked.
    pub fn draw(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        interaction: &mut InteractionContext,
        screen_w: u32,
        screen_h: u32,
    ) -> Option<MenuEvent> {
        if !self.open {
            return None;
        }

        // Build the panel stack: root + each open submenu level
        let mut panels: Vec<MenuPanel> = Vec::with_capacity(self.open_submenu_ids.len() + 1);
        panels.push(MenuPanel {
            x: self.x,
            y: self.y,
            width: self.width,
            path: Vec::new(),
        });

        // Walk the open submenu chain to build panel positions
        let mut current_items: &[MenuItem] = &self.items;
        let mut parent_x = self.x;
        let mut parent_width = self.width;
        let mut path = Vec::new();

        for &sub_id in &self.open_submenu_ids {
            // Find which item index matches this submenu id
            let Some((idx, children)) = find_submenu(current_items, sub_id) else {
                break;
            };
            path.push(idx);

            let sub_y_offset = item_y_offset(current_items, idx, &self.style);
            let sub_w = compute_width(children, &self.style);
            let sub_x = parent_x + parent_width - 4.0; // slight overlap
            let sub_y = panels.last().unwrap().y + sub_y_offset;

            panels.push(MenuPanel {
                x: sub_x,
                y: sub_y,
                width: sub_w,
                path: path.clone(),
            });

            parent_x = sub_x;
            parent_width = sub_w;
            current_items = children;
        }

        // Draw each panel and collect events
        let mut event = None;
        let mut new_submenu_ids = self.open_submenu_ids.clone();

        for (depth, panel) in panels.iter().enumerate() {
            let items = resolve_items(&mut self.items, &panel.path);
            let panel_event = draw_panel(
                items,
                panel.x,
                panel.y,
                panel.width,
                depth,
                &self.style,
                painter,
                text,
                interaction,
                screen_w,
                screen_h,
                &mut new_submenu_ids,
            );
            if panel_event.is_some() {
                event = panel_event;
            }
        }

        self.open_submenu_ids = new_submenu_ids;
        event
    }

    /// Check if a point is inside any open menu panel.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if !self.open {
            return false;
        }
        // Check root
        let root_h = items_height(&self.items, &self.style);
        if contains_rect(x, y, self.x, self.y, self.width, root_h) {
            return true;
        }
        // Check each open submenu level
        let mut current_items: &[MenuItem] = &self.items;
        let mut parent_x = self.x;
        let mut parent_y = self.y;
        let mut parent_width = self.width;

        for &sub_id in &self.open_submenu_ids {
            let Some((idx, children)) = find_submenu(current_items, sub_id) else {
                break;
            };
            let sub_y_offset = item_y_offset(current_items, idx, &self.style);
            let sub_x = parent_x + parent_width - 4.0;
            let sub_y = parent_y + sub_y_offset;
            let sub_w = compute_width(children, &self.style);
            let sub_h = items_height(children, &self.style);

            if contains_rect(x, y, sub_x, sub_y, sub_w, sub_h) {
                return true;
            }

            parent_x = sub_x;
            parent_y = sub_y;
            parent_width = sub_w;
            current_items = children;
        }
        false
    }
}

// ── Drawing helpers ──────────────────────────────────────────────────────────

fn draw_panel(
    items: &mut [MenuItem],
    px: f32,
    py: f32,
    width: f32,
    depth: usize,
    style: &ContextMenuStyle,
    painter: &mut Painter,
    text: &mut TextRenderer,
    interaction: &mut InteractionContext,
    screen_w: u32,
    screen_h: u32,
    open_submenu_ids: &mut Vec<u32>,
) -> Option<MenuEvent> {
    let total_h = items_height_slice(items, style);
    let menu_rect = Rect::new(px, py, width, total_h);

    // Shadow
    let shadow = menu_rect.expand(3.0);
    painter.rect_filled(shadow, style.corner_radius + 2.0, Color::BLACK.with_alpha(0.2));

    painter.rect_filled(menu_rect, style.corner_radius, style.bg);
    painter.rect_stroke(menu_rect, style.corner_radius, style.border_width, style.border);

    let mut event = None;
    let mut cy = py + style.padding;
    let inner_w = width - style.padding * 2.0;
    let inner_x = px + style.padding;
    let zone_base = CONTEXT_MENU_ZONE_BASE + (depth as u32) * 0x1000;

    for item in items.iter_mut() {
        match item {
            MenuItem::Action { id, label } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, style.item_height);
                let zone_id = zone_base + *id;
                let state = interaction.add_zone(zone_id, item_rect);

                if state == InteractionState::Hovered || state == InteractionState::Pressed {
                    painter.rect_filled(item_rect, style.corner_radius - 2.0, style.bg_hover);
                }
                if state == InteractionState::Pressed {
                    event = Some(MenuEvent::Action(*id));
                }

                let text_x = inner_x + style.padding * 2.0;
                let text_y = cy + (style.item_height - style.font_size) * 0.5;
                text.queue(
                    label, style.font_size, text_x, text_y, style.text,
                    inner_w - style.padding * 4.0, screen_w, screen_h,
                );
                cy += style.item_height;
            }
            MenuItem::Separator => {
                let sep_y = cy + SEPARATOR_HEIGHT * 0.5;
                let sep_x = inner_x + style.padding;
                let sep_w = inner_w - style.padding * 2.0;
                painter.rect_filled(Rect::new(sep_x, sep_y, sep_w, 1.0), 0.0, style.separator);
                cy += SEPARATOR_HEIGHT;
            }
            MenuItem::Slider { id, label, value } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, SLIDER_ITEM_HEIGHT);
                let zone_id = zone_base + *id;
                let zone_state = interaction.add_zone(zone_id, item_rect);

                let label_x = inner_x + style.padding * 2.0;
                let label_y = cy + 6.0;
                text.queue(
                    label, SLIDER_LABEL_SIZE, label_x, label_y, style.text_muted,
                    inner_w * 0.6, screen_w, screen_h,
                );
                let pct = format!("{}%", (*value * 100.0).round() as u32);
                let pct_w = pct.len() as f32 * SLIDER_LABEL_SIZE * 0.55;
                let pct_x = inner_x + inner_w - style.padding * 2.0 - pct_w;
                text.queue(
                    &pct, SLIDER_LABEL_SIZE, pct_x, label_y, style.text_muted,
                    pct_w + 4.0, screen_w, screen_h,
                );

                let track_pad = style.padding * 2.0;
                let track_y = label_y + SLIDER_LABEL_SIZE + 8.0;
                let track_w = inner_w - track_pad * 2.0;
                let track = Rect::new(inner_x + track_pad, track_y, track_w, SLIDER_TRACK_H);

                painter.rect_filled(track, SLIDER_TRACK_H * 0.5, style.bg_hover);
                let fill_w = (track_w * *value).max(SLIDER_TRACK_H);
                painter.rect_filled(
                    Rect::new(track.x, track.y, fill_w, SLIDER_TRACK_H),
                    SLIDER_TRACK_H * 0.5, style.accent,
                );

                let thumb_x = track.x + track_w * *value;
                let thumb_cy = track.y + SLIDER_TRACK_H * 0.5;
                let thumb_r = if zone_state.is_active() { 8.0 }
                    else if zone_state.is_hovered() { 7.0 }
                    else { 6.0 };
                painter.circle_filled(thumb_x, thumb_cy, thumb_r, Color::WHITE);
                painter.circle_stroke(thumb_x, thumb_cy, thumb_r, 1.0, Color::rgba(0.0, 0.0, 0.0, 0.2));

                if zone_state.is_active() {
                    if let Some(frac) = interaction.drag_fraction_x(&track) {
                        *value = frac;
                        event = Some(MenuEvent::SliderChanged { id: *id, value: frac });
                    }
                }
                cy += SLIDER_ITEM_HEIGHT;
            }
            MenuItem::SubMenu { id, label, .. } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, style.item_height);
                let zone_id = zone_base + *id;
                let state = interaction.add_zone(zone_id, item_rect);

                let is_open = open_submenu_ids.get(depth) == Some(id);

                if state.is_hovered() || is_open {
                    painter.rect_filled(item_rect, style.corner_radius - 2.0, style.bg_hover);
                    // Open this submenu (trim deeper levels if switching)
                    if !is_open {
                        open_submenu_ids.truncate(depth);
                        open_submenu_ids.push(*id);
                    }
                }

                let text_x = inner_x + style.padding * 2.0;
                let text_y = cy + (style.item_height - style.font_size) * 0.5;
                text.queue(
                    label, style.font_size, text_x, text_y, style.text,
                    inner_w - style.padding * 6.0, screen_w, screen_h,
                );

                // Arrow chevron ›
                let arrow_x = inner_x + inner_w - style.padding * 2.0 - 6.0;
                let arrow_cy = cy + style.item_height * 0.5;
                let arrow_color = if is_open { style.accent } else { style.text_muted };
                painter.line(arrow_x, arrow_cy - 5.0, arrow_x + 5.0, arrow_cy, 1.5, arrow_color);
                painter.line(arrow_x + 5.0, arrow_cy, arrow_x, arrow_cy + 5.0, 1.5, arrow_color);

                cy += style.item_height;
            }
        }
    }

    event
}

// ── Item tree helpers ────────────────────────────────────────────────────────

fn compute_width(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
    let max_label_w = items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Action { label, .. } => Some(label.len() as f32 * style.font_size * 0.55),
            MenuItem::SubMenu { label, .. } => {
                Some(label.len() as f32 * style.font_size * 0.55 + 20.0) // extra for arrow
            }
            MenuItem::Slider { label, .. } => {
                Some(label.len() as f32 * SLIDER_LABEL_SIZE * 0.55 + 60.0)
            }
            MenuItem::Separator => None,
        })
        .fold(0.0f32, f32::max);
    (max_label_w + style.padding * 4.0).max(style.min_width)
}

fn item_height(item: &MenuItem, style: &ContextMenuStyle) -> f32 {
    match item {
        MenuItem::Action { .. } | MenuItem::SubMenu { .. } => style.item_height,
        MenuItem::Separator => SEPARATOR_HEIGHT,
        MenuItem::Slider { .. } => SLIDER_ITEM_HEIGHT,
    }
}

fn items_height(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
    items_height_slice(items, style)
}

fn items_height_slice(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
    items.iter().map(|i| item_height(i, style)).sum::<f32>() + style.padding * 2.0
}

fn item_y_offset(items: &[MenuItem], index: usize, style: &ContextMenuStyle) -> f32 {
    let mut offset = style.padding;
    for item in items.iter().take(index) {
        offset += item_height(item, style);
    }
    offset
}

fn find_submenu(items: &[MenuItem], id: u32) -> Option<(usize, &[MenuItem])> {
    items.iter().enumerate().find_map(|(i, item)| match item {
        MenuItem::SubMenu { id: sid, children, .. } if *sid == id => Some((i, children.as_slice())),
        _ => None,
    })
}

/// Walk the item tree by index path to get a mutable slice.
fn resolve_items<'a>(items: &'a mut [MenuItem], path: &[usize]) -> &'a mut [MenuItem] {
    let mut current: &mut [MenuItem] = items;
    for &idx in path {
        let item = &mut current[idx];
        current = match item {
            MenuItem::SubMenu { children, .. } => children.as_mut_slice(),
            _ => return &mut [],
        };
    }
    current
}

fn contains_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    px >= x && px <= x + w && py >= y && py <= y + h
}

const SEPARATOR_HEIGHT: f32 = 9.0;
const SLIDER_ITEM_HEIGHT: f32 = 50.0;
const SLIDER_LABEL_SIZE: f32 = 16.0;
const SLIDER_TRACK_H: f32 = 6.0;

/// Base zone ID offset for context menu items to avoid collisions
/// with the host application's own hit zones.
const CONTEXT_MENU_ZONE_BASE: u32 = 0xCE_0000;
