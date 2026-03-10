use std::collections::HashMap;

use lntrn_render::{Color, Rect, Painter, TextRenderer};
use lntrn_theme::FONT_BODY;

use crate::animation::{self};

use super::context_menu_draw::draw_panel;
use super::input::InteractionContext;
use super::palette::FoxPalette;

// Darker, slightly transparent amber hover
const HOVER_COLOR: Color = Color::rgba(0.75, 0.55, 0.10, 0.22);

/// Visual style for the context menu.
pub struct ContextMenuStyle {
    pub palette: FoxPalette,
    pub bg: Color,
    pub bg_hover: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_disabled: Color,
    pub separator: Color,
    pub border: Color,
    pub accent: Color,
    pub corner_radius: f32,
    pub padding: f32,
    pub item_height: f32,
    pub font_size: f32,
    pub min_width: f32,
    pub border_width: f32,
    pub scale: f32,
}

impl ContextMenuStyle {
    pub fn from_palette(palette: &FoxPalette) -> Self {
        Self {
            palette: palette.clone(),
            bg: palette.surface,
            bg_hover: HOVER_COLOR,
            text: palette.text_secondary,
            text_muted: palette.muted,
            text_disabled: Color::rgba(1.0, 1.0, 1.0, 0.25),
            separator: Color::rgba(1.0, 1.0, 1.0, 0.08),
            border: Color::rgba(1.0, 1.0, 1.0, 0.1),
            accent: palette.accent,
            corner_radius: 8.0,
            padding: 4.0,
            item_height: 36.0,
            font_size: FONT_BODY,
            min_width: 200.0,
            border_width: 1.0,
            scale: 1.0,
        }
    }

    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
}

/// A single entry in a context menu.
#[derive(Clone, Debug)]
pub enum MenuItem {
    Action { id: u32, label: String, shortcut: Option<String>, enabled: bool },
    Separator,
    Slider { id: u32, label: String, value: f32 },
    SubMenu { id: u32, label: String, children: Vec<MenuItem> },
    Toggle { id: u32, label: String, checked: bool, enabled: bool },
    Checkbox { id: u32, label: String, checked: bool },
    Radio { id: u32, group: u32, label: String, selected: bool },
    Button { id: u32, label: String, primary: bool },
    Progress { id: u32, label: String, value: f32 },
    Header { label: String },
}

impl MenuItem {
    pub fn action(id: u32, label: impl Into<String>) -> Self {
        Self::Action { id, label: label.into(), shortcut: None, enabled: true }
    }
    pub fn action_with(id: u32, label: impl Into<String>, shortcut: impl Into<String>) -> Self {
        Self::Action { id, label: label.into(), shortcut: Some(shortcut.into()), enabled: true }
    }
    pub fn action_disabled(id: u32, label: impl Into<String>) -> Self {
        Self::Action { id, label: label.into(), shortcut: None, enabled: false }
    }
    pub fn separator() -> Self { Self::Separator }
    pub fn slider(id: u32, label: impl Into<String>, value: f32) -> Self {
        Self::Slider { id, label: label.into(), value }
    }
    pub fn submenu(id: u32, label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self::SubMenu { id, label: label.into(), children }
    }
    pub fn toggle(id: u32, label: impl Into<String>, checked: bool) -> Self {
        Self::Toggle { id, label: label.into(), checked, enabled: true }
    }
    pub fn toggle_disabled(id: u32, label: impl Into<String>, checked: bool) -> Self {
        Self::Toggle { id, label: label.into(), checked, enabled: false }
    }
    pub fn checkbox(id: u32, label: impl Into<String>, checked: bool) -> Self {
        Self::Checkbox { id, label: label.into(), checked }
    }
    pub fn radio(id: u32, group: u32, label: impl Into<String>, selected: bool) -> Self {
        Self::Radio { id, group, label: label.into(), selected }
    }
    pub fn button(id: u32, label: impl Into<String>) -> Self {
        Self::Button { id, label: label.into(), primary: false }
    }
    pub fn button_primary(id: u32, label: impl Into<String>) -> Self {
        Self::Button { id, label: label.into(), primary: true }
    }
    pub fn progress(id: u32, label: impl Into<String>, value: f32) -> Self {
        Self::Progress { id, label: label.into(), value }
    }
    pub fn header(label: impl Into<String>) -> Self {
        Self::Header { label: label.into() }
    }
}

/// Event returned by [`ContextMenu::draw`].
#[derive(Debug)]
pub enum MenuEvent {
    Action(u32),
    Toggled { id: u32, checked: bool },
    CheckboxToggled { id: u32, checked: bool },
    RadioSelected { id: u32, group: u32 },
    SliderChanged { id: u32, value: f32 },
}

struct MenuPanel { x: f32, y: f32, width: f32, path: Vec<usize> }

#[derive(Clone, Copy, PartialEq)]
enum MenuState { Closed, Opening, Open, Closing }

const OPEN_DURATION: f32 = 0.75;
const CLOSE_DURATION: f32 = 1.0;

/// A right-click context menu with nested submenu support and animations.
///
/// 1. Create with `ContextMenu::new(style)`
/// 2. On right-click: `open(x, y, items)`
/// 3. Each frame: `update(dt)` then `draw(...)` — returns `Some(MenuEvent)`
/// 4. `close()` to begin close animation (auto-finishes)
pub struct ContextMenu {
    style: ContextMenuStyle,
    items: Vec<MenuItem>,
    x: f32,
    y: f32,
    width: f32,
    state: MenuState,
    open_t: f32,
    open_submenu_ids: Vec<u32>,
    hover_t: HashMap<u32, f32>,
    hovered_zones: Vec<u32>,
    /// Zones that already fired a press event this click — prevents rapid toggling.
    pressed_zones: Vec<u32>,
    /// Scroll offset for menus taller than available screen space.
    scroll_offset: f32,
    /// Maximum visible height before scrolling kicks in.
    max_height: f32,
}

impl ContextMenu {
    pub fn new(style: ContextMenuStyle) -> Self {
        Self {
            style, items: Vec::new(), x: 0.0, y: 0.0, width: 0.0,
            state: MenuState::Closed, open_t: 0.0,
            open_submenu_ids: Vec::new(),
            hover_t: HashMap::new(), hovered_zones: Vec::new(),
            pressed_zones: Vec::new(),
            scroll_offset: 0.0,
            max_height: 0.0,
        }
    }

    pub fn set_scale(&mut self, scale: f32) { self.style.scale = scale; }
    pub fn is_open(&self) -> bool { self.state != MenuState::Closed }
    fn is_interactive(&self) -> bool {
        self.state == MenuState::Open || self.state == MenuState::Opening
    }

    /// Access items mutably (e.g. to update a Progress value from outside).
    pub fn items_mut(&mut self) -> &mut [MenuItem] { &mut self.items }

    pub fn open(&mut self, x: f32, y: f32, items: Vec<MenuItem>) {
        self.width = compute_width(&items, &self.style);
        self.items = items;
        self.x = x;
        self.y = y;
        self.state = MenuState::Opening;
        self.open_t = 0.0;
        self.open_submenu_ids.clear();
        self.hover_t.clear();
        self.hovered_zones.clear();
        self.pressed_zones.clear();
        self.scroll_offset = 0.0;
    }

    pub fn clamp_to_screen(&mut self, screen_w: f32, screen_h: f32) {
        let total_h = items_height(&self.items, &self.style);
        if self.x + self.width > screen_w {
            self.x = (screen_w - self.width - 4.0).max(0.0);
        }
        // If menu is taller than screen, cap visible height and enable scrolling
        let available_h = screen_h - self.y - 8.0;
        if total_h > available_h && available_h > 100.0 {
            self.max_height = available_h;
        } else {
            if self.y + total_h > screen_h {
                self.y = (screen_h - total_h - 4.0).max(0.0);
            }
            self.max_height = 0.0;
        }
    }

    pub fn clamp_bottom_bar(&mut self, surface_w: f32, _surface_h: f32) {
        let total_h = items_height(&self.items, &self.style);
        self.y = (self.y - total_h).max(0.0);
        if self.x + self.width > surface_w {
            self.x = (surface_w - self.width - 4.0).max(0.0);
        }
    }

    /// Apply scroll delta (from trackpad/mouse wheel) to the context menu.
    /// `delta` is in pixels (positive = scroll up / content moves down).
    pub fn on_scroll(&mut self, delta: f32) {
        if self.state == MenuState::Closed || self.max_height <= 0.0 { return; }
        let total_h = items_height(&self.items, &self.style);
        let max_scroll = (total_h - self.max_height).max(0.0);
        self.scroll_offset = (self.scroll_offset - delta).clamp(0.0, max_scroll);
    }

    pub fn close(&mut self) {
        if self.state == MenuState::Closed { return; }
        self.state = MenuState::Closing;
        self.open_submenu_ids.clear();
    }

    /// Advance all animations by `dt` seconds. Call once per frame before `draw`.
    pub fn update(&mut self, dt: f32) {
        match self.state {
            MenuState::Closed => return,
            MenuState::Opening => {
                self.open_t += dt / OPEN_DURATION;
                if self.open_t >= 1.0 {
                    self.open_t = 1.0;
                    self.state = MenuState::Open;
                }
            }
            MenuState::Open => {}
            MenuState::Closing => {
                self.open_t -= dt / CLOSE_DURATION;
                if self.open_t <= 0.0 {
                    self.open_t = 0.0;
                    self.state = MenuState::Closed;
                    self.items.clear();
                    self.hover_t.clear();
                    self.hovered_zones.clear();
                    return;
                }
            }
        }

        let hover_speed = dt / HOVER_DURATION;
        let hovered = &self.hovered_zones;
        self.hover_t.retain(|id, t| {
            let target = if hovered.contains(id) { 1.0 } else { 0.0 };
            *t = move_toward(*t, target, hover_speed);
            *t > 0.001
        });
        for &id in &self.hovered_zones {
            self.hover_t.entry(id).or_insert(0.0);
        }
    }

    pub fn draw(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        interaction: &mut InteractionContext,
        screen_w: u32,
        screen_h: u32,
    ) -> Option<MenuEvent> {
        if self.state == MenuState::Closed { return None; }

        self.hovered_zones.clear();

        // Clear consumed presses once mouse is released
        if interaction.active_zone_id().is_none() {
            self.pressed_zones.clear();
        }

        let ease_t = animation::ease_out(self.open_t);
        // Menu background is fully opaque — only shadow/border fade
        let alpha = ease_t;

        // Clip-reveal: menu unrolls from top to bottom
        let total_h = items_height(&self.items, &self.style);
        let visible_h = if self.max_height > 0.0 { self.max_height } else { total_h };
        let clip_h = visible_h * ease_t;
        // Clip covers full screen so submenus aren't cut off.
        // The open animation uses clip_h only during opening/closing.
        let sc = self.style.scale;
        let clip_y = self.y;
        let clip_total_h = if self.state == MenuState::Open {
            screen_h as f32 - clip_y
        } else {
            clip_h + 8.0 * sc
        };
        let clip_rect = Rect::new(
            0.0, clip_y - 8.0 * sc,
            screen_w as f32, clip_total_h + 8.0 * sc,
        );
        painter.push_clip(clip_rect);

        // Apply scroll offset — shift the root panel up
        let scroll_y = self.y - self.scroll_offset;

        let mut panels: Vec<MenuPanel> = Vec::with_capacity(self.open_submenu_ids.len() + 1);
        panels.push(MenuPanel {
            x: self.x, y: scroll_y, width: self.width, path: Vec::new(),
        });

        let mut current_items: &[MenuItem] = &self.items;
        let mut parent_x = self.x;
        let mut parent_width = self.width;
        let mut path = Vec::new();
        let sc = self.style.scale;

        for &sub_id in &self.open_submenu_ids {
            let Some((idx, children)) = find_submenu(current_items, sub_id) else { break; };
            path.push(idx);
            let sub_y_offset = item_y_offset(current_items, idx, &self.style);
            let sub_w = compute_width(children, &self.style);
            let sub_x = parent_x + parent_width - 4.0 * sc;
            let sub_y = panels.last().unwrap().y + sub_y_offset;
            panels.push(MenuPanel { x: sub_x, y: sub_y, width: sub_w, path: path.clone() });
            parent_x = sub_x;
            parent_width = sub_w;
            current_items = children;
        }

        let mut event = None;
        let mut new_submenu_ids = self.open_submenu_ids.clone();
        let interactive = self.is_interactive();

        for (depth, panel) in panels.iter().enumerate() {
            let items = resolve_items(&mut self.items, &panel.path);
            let e = draw_panel(
                items, panel.x, panel.y, panel.width, depth,
                &self.style, painter, text, interaction,
                screen_w, screen_h, &mut new_submenu_ids,
                &self.hover_t, &mut self.hovered_zones,
                &mut self.pressed_zones, alpha, interactive,
            );
            if e.is_some() { event = e; }
        }

        painter.pop_clip();

        // Handle radio group deselection
        if let Some(MenuEvent::RadioSelected { group, id }) = &event {
            deselect_radio_group(&mut self.items, *group, *id);
        }

        self.open_submenu_ids = new_submenu_ids;
        event
    }

    pub fn bounds(&self) -> Option<Rect> {
        if self.state == MenuState::Closed { return None; }
        let h = items_height(&self.items, &self.style);
        let visible_h = if self.max_height > 0.0 { self.max_height.min(h) } else { h };
        Some(Rect::new(self.x, self.y, self.width, visible_h))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        if self.state == MenuState::Closed { return false; }
        let sc = self.style.scale;
        let root_h = items_height(&self.items, &self.style);
        let visible_h = if self.max_height > 0.0 { self.max_height.min(root_h) } else { root_h };
        if contains_rect(x, y, self.x, self.y, self.width, visible_h) { return true; }

        let mut current_items: &[MenuItem] = &self.items;
        let mut px = self.x;
        let mut py = self.y;
        let mut pw = self.width;
        for &sub_id in &self.open_submenu_ids {
            let Some((idx, children)) = find_submenu(current_items, sub_id) else { break; };
            let sy = item_y_offset(current_items, idx, &self.style);
            let sx = px + pw - 4.0 * sc;
            let sub_y = py + sy;
            let sw = compute_width(children, &self.style);
            let sh = items_height(children, &self.style);
            if contains_rect(x, y, sx, sub_y, sw, sh) { return true; }
            px = sx; py = sub_y; pw = sw;
            current_items = children;
        }
        false
    }
}

// ── Helpers (pub(super) for draw module) ─────────────────────────────────────

pub(super) fn items_height_slice(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
    items.iter().map(|i| item_height(i, style)).sum::<f32>() + style.padding * style.scale * 2.0
}

pub(super) fn item_height(item: &MenuItem, style: &ContextMenuStyle) -> f32 {
    let s = style.scale;
    match item {
        MenuItem::Action { .. } | MenuItem::SubMenu { .. }
        | MenuItem::Toggle { .. } | MenuItem::Checkbox { .. }
        | MenuItem::Radio { .. } | MenuItem::Button { .. } => style.item_height * s,
        MenuItem::Separator => SEPARATOR_HEIGHT * s,
        MenuItem::Slider { .. } => SLIDER_ITEM_HEIGHT * s,
        MenuItem::Progress { .. } => PROGRESS_ITEM_HEIGHT * s,
        MenuItem::Header { .. } => HEADER_HEIGHT * s,
    }
}

fn items_height(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
    items_height_slice(items, style)
}

fn item_y_offset(items: &[MenuItem], index: usize, style: &ContextMenuStyle) -> f32 {
    let mut offset = style.padding * style.scale;
    for item in items.iter().take(index) { offset += item_height(item, style); }
    offset
}

fn compute_width(items: &[MenuItem], style: &ContextMenuStyle) -> f32 {
    let s = style.scale;
    let fw = style.font_size * s * 0.55;
    let max_w = items.iter().filter_map(|item| match item {
        MenuItem::Action { label, shortcut, .. } => {
            let sc_w = shortcut.as_ref()
                .map_or(0.0, |sc| sc.len() as f32 * fw * 0.75 + 16.0 * s);
            Some(label.len() as f32 * fw + sc_w)
        }
        MenuItem::Toggle { label, .. } | MenuItem::Checkbox { label, .. }
        | MenuItem::Radio { label, .. } => Some(label.len() as f32 * fw + 40.0 * s),
        MenuItem::SubMenu { label, .. } => Some(label.len() as f32 * fw + 20.0 * s),
        MenuItem::Slider { label, .. } | MenuItem::Progress { label, .. } => {
            Some(label.len() as f32 * SLIDER_LABEL_SIZE * s * 0.55 + 60.0 * s)
        }
        MenuItem::Button { label, .. } => Some(label.len() as f32 * fw + 32.0 * s),
        MenuItem::Header { label } => Some(label.len() as f32 * fw * 0.7),
        MenuItem::Separator => None,
    }).fold(0.0f32, f32::max);
    (max_w + style.padding * s * 4.0).max(style.min_width * s)
}

fn find_submenu(items: &[MenuItem], id: u32) -> Option<(usize, &[MenuItem])> {
    items.iter().enumerate().find_map(|(i, item)| match item {
        MenuItem::SubMenu { id: sid, children, .. } if *sid == id => {
            Some((i, children.as_slice()))
        }
        _ => None,
    })
}

fn resolve_items<'a>(items: &'a mut [MenuItem], path: &[usize]) -> &'a mut [MenuItem] {
    let mut current: &mut [MenuItem] = items;
    for &idx in path {
        current = match &mut current[idx] {
            MenuItem::SubMenu { children, .. } => children.as_mut_slice(),
            _ => return &mut [],
        };
    }
    current
}

/// Deselect all radios in the same group except the one with `selected_id`.
fn deselect_radio_group(items: &mut [MenuItem], group: u32, selected_id: u32) {
    for item in items.iter_mut() {
        match item {
            MenuItem::Radio { id, group: g, selected, .. } if *g == group => {
                *selected = *id == selected_id;
            }
            MenuItem::SubMenu { children, .. } => {
                deselect_radio_group(children, group, selected_id);
            }
            _ => {}
        }
    }
}

fn contains_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    px >= x && px <= x + w && py >= y && py <= y + h
}

fn move_toward(current: f32, target: f32, step: f32) -> f32 {
    if (target - current).abs() <= step { target }
    else if target > current { current + step }
    else { current - step }
}

// Constants (pub(super) for draw module)
pub(super) const SEPARATOR_HEIGHT: f32 = 9.0;
pub(super) const SLIDER_ITEM_HEIGHT: f32 = 50.0;
pub(super) const SLIDER_LABEL_SIZE: f32 = 16.0;
pub(super) const SLIDER_TRACK_H: f32 = 6.0;
pub(super) const HEADER_HEIGHT: f32 = 24.0;
pub(super) const PROGRESS_ITEM_HEIGHT: f32 = 40.0;
pub(super) const CONTEXT_MENU_ZONE_BASE: u32 = 0xCE_0000;

const HOVER_DURATION: f32 = 0.12;
