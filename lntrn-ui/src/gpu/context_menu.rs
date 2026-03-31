use lntrn_render::{Color, Rect, Painter, TextRenderer};
use lntrn_theme::{FONT_CAPTION, FONT_LABEL};

use super::context_menu_draw::draw_panel;
use super::input::InteractionContext;
use super::palette::FoxPalette;
use super::popup::PopupSurface;

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
    pub no_shadow: bool,
}

impl ContextMenuStyle {
    pub fn from_palette(palette: &FoxPalette) -> Self {
        Self {
            palette: palette.clone(),
            bg: palette.surface_2,
            bg_hover: Color::from_rgb8(255, 200, 50).with_alpha(0.18),
            text: palette.text,
            text_muted: palette.text_secondary,
            text_disabled: palette.muted.with_alpha(0.4),
            separator: palette.muted.with_alpha(0.15),
            border: palette.muted.with_alpha(0.2),
            accent: palette.accent,
            corner_radius: 10.0,
            padding: 5.0,
            item_height: 38.0,
            font_size: FONT_LABEL,
            min_width: 200.0,
            border_width: 1.0,
            scale: 1.0,
            no_shadow: false,
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
    Action { id: u32, label: String, shortcut: Option<String>, enabled: bool, danger: bool },
    Separator,
    ColoredSeparator(Color),
    Slider { id: u32, label: String, value: f32 },
    SubMenu { id: u32, label: String, children: Vec<MenuItem> },
    Toggle { id: u32, label: String, checked: bool, enabled: bool },
    Checkbox { id: u32, label: String, checked: bool },
    Radio { id: u32, group: u32, label: String, selected: bool },
    Button { id: u32, label: String, primary: bool },
    Progress { id: u32, label: String, value: f32 },
    Header { label: String },
    /// A row of color swatches. Each swatch has an id and a color.
    ColorSwatches { label: String, swatches: Vec<(u32, Color)> },
}

impl MenuItem {
    pub fn action(id: u32, label: impl Into<String>) -> Self {
        Self::Action { id, label: label.into(), shortcut: None, enabled: true, danger: false }
    }
    pub fn action_with(id: u32, label: impl Into<String>, shortcut: impl Into<String>) -> Self {
        Self::Action { id, label: label.into(), shortcut: Some(shortcut.into()), enabled: true, danger: false }
    }
    pub fn action_disabled(id: u32, label: impl Into<String>) -> Self {
        Self::Action { id, label: label.into(), shortcut: None, enabled: false, danger: false }
    }
    pub fn action_danger(id: u32, label: impl Into<String>) -> Self {
        Self::Action { id, label: label.into(), shortcut: None, enabled: true, danger: true }
    }
    pub fn separator() -> Self { Self::Separator }
    pub fn colored_separator(color: Color) -> Self { Self::ColoredSeparator(color) }
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
    pub fn color_swatches(label: impl Into<String>, swatches: Vec<(u32, Color)>) -> Self {
        Self::ColorSwatches { label: label.into(), swatches }
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

/// A right-click context menu with nested submenu support.
///
/// 1. Create with `ContextMenu::new(style)`
/// 2. On right-click: `open(x, y, items)`
/// 3. Each frame: `update(dt)` then `draw(...)` — returns `Some(MenuEvent)`
/// 4. `close()` to dismiss
/// Delay before a submenu opens on hover (seconds).
const SUBMENU_OPEN_DELAY: f32 = 0.2;
/// Grace period before closing a submenu when cursor moves to a different item (seconds).
/// Allows cursor to travel from trigger to submenu popup without instant close.
const SUBMENU_CLOSE_DELAY: f32 = 0.3;

pub struct ContextMenu {
    style: ContextMenuStyle,
    items: Vec<MenuItem>,
    x: f32,
    y: f32,
    width: f32,
    open: bool,
    open_submenu_ids: Vec<u32>,
    /// Zones that already fired a press event this click — prevents rapid toggling.
    pressed_zones: Vec<u32>,
    /// Scroll offset for menus taller than available screen space.
    scroll_offset: f32,
    /// Maximum visible height before scrolling kicks in.
    max_height: f32,
    /// Popup IDs for each panel level (root = index 0, submenus = 1+).
    popup_ids: Vec<u32>,
    /// Whether this menu is using popup surfaces.
    uses_popups: bool,
    /// Submenu item ID being hovered (pending open).
    submenu_hover_id: Option<u32>,
    /// How long the submenu item has been hovered (seconds).
    submenu_hover_timer: f32,
    /// Depth at which the pending submenu would open.
    submenu_hover_depth: usize,
    /// Which popup depth currently has the pointer (set by the app).
    pointer_depth: Option<usize>,
    /// Timer for submenu close grace period (seconds).
    submenu_close_timer: f32,
    /// Whether the close timer is running.
    submenu_close_pending: bool,
}

impl ContextMenu {
    pub fn new(style: ContextMenuStyle) -> Self {
        Self {
            style, items: Vec::new(), x: 0.0, y: 0.0, width: 0.0,
            open: false,
            open_submenu_ids: Vec::new(),
            pressed_zones: Vec::new(),
            scroll_offset: 0.0,
            max_height: 0.0,
            popup_ids: Vec::new(),
            uses_popups: false,
            submenu_hover_id: None,
            submenu_hover_timer: 0.0,
            submenu_hover_depth: 0,
            pointer_depth: None,
            submenu_close_timer: 0.0,
            submenu_close_pending: false,
        }
    }

    pub fn set_scale(&mut self, scale: f32) { self.style.scale = scale; }
    pub fn is_open(&self) -> bool { self.open }

    /// Tell the menu which popup depth the pointer is currently on.
    /// `None` = pointer not on any popup. `Some(0)` = root, `Some(1)` = first submenu, etc.
    pub fn set_pointer_depth(&mut self, depth: Option<usize>) {
        self.pointer_depth = depth;
    }

    /// Get the popup ID at a given depth (for the app to match wl_surface → depth).
    pub fn popup_id_at_depth(&self, depth: usize) -> Option<u32> {
        self.popup_ids.get(depth).copied()
    }

    /// Number of active popup surfaces.
    pub fn popup_count(&self) -> usize { self.popup_ids.len() }

    /// Access items mutably (e.g. to update a Progress value from outside).
    pub fn items_mut(&mut self) -> &mut [MenuItem] { &mut self.items }

    /// Root popup ID (for blitting textures onto the context menu surface).
    pub fn root_popup_id(&self) -> Option<u32> { self.popup_ids.first().copied() }

    /// Returns (id, x, y, size) for each swatch in any `ColorSwatches` items on the root panel.
    /// Coordinates are relative to the popup surface (physical pixels).
    pub fn swatch_rects(&self) -> Vec<(u32, f32, f32, f32)> {
        if !self.open { return Vec::new(); }
        let s = self.style.scale;
        let pad = self.style.padding * s;
        let accent_inset = (ACCENT_BAR_WIDTH + 6.0) * s;
        let inner_w = self.width - pad * 2.0;
        let content_x = pad + pad + accent_inset;
        let content_w = inner_w - pad - accent_inset;

        let label_size = FONT_CAPTION * s;
        let icon_sz = 40.0 * s;
        let icon_gap = 6.0 * s;

        let mut cy = pad;
        let mut result = Vec::new();
        for item in &self.items {
            match item {
                MenuItem::ColorSwatches { swatches, .. } => {
                    let total_sw = swatches.len() as f32 * icon_sz
                        + (swatches.len().saturating_sub(1)) as f32 * icon_gap;
                    let start_x = content_x + (content_w - pad - total_sw) * 0.5;
                    let icon_top = cy + label_size + 12.0 * s;
                    for (i, (sid, _)) in swatches.iter().enumerate() {
                        let ix = start_x + i as f32 * (icon_sz + icon_gap);
                        result.push((*sid, ix, icon_top, icon_sz));
                    }
                    cy += COLOR_SWATCH_HEIGHT * s;
                }
                _ => { cy += item_height(item, &self.style); }
            }
        }
        result
    }

    pub fn open(&mut self, x: f32, y: f32, items: Vec<MenuItem>) {
        self.width = compute_width(&items, &self.style);
        self.items = items;
        self.x = x;
        self.y = y;
        self.open = true;
        self.open_submenu_ids.clear();
        self.pressed_zones.clear();
        self.scroll_offset = 0.0;
    }

    pub fn clamp_to_screen(&mut self, screen_w: f32, screen_h: f32) {
        let total_h = items_height(&self.items, &self.style);
        if self.x + self.width > screen_w {
            self.x = (screen_w - self.width - 4.0).max(0.0);
        }
        let margin = 4.0;
        let room_below = screen_h - self.y - margin;

        if total_h <= room_below {
            self.max_height = 0.0;
        } else {
            let flipped_y = self.y - total_h;
            if flipped_y >= margin {
                self.y = flipped_y;
                self.max_height = 0.0;
            } else if self.y > screen_h * 0.5 {
                let clamped_y = margin;
                let visible = self.y - clamped_y;
                self.y = clamped_y;
                if visible > 100.0 && total_h > visible {
                    self.max_height = visible;
                } else {
                    self.max_height = 0.0;
                }
            } else {
                if total_h > room_below && room_below > 100.0 {
                    self.max_height = room_below;
                } else {
                    self.y = (screen_h - total_h - margin).max(0.0);
                    self.max_height = 0.0;
                }
            }
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
    pub fn on_scroll(&mut self, delta: f32) {
        if !self.open || self.max_height <= 0.0 { return; }
        let total_h = items_height(&self.items, &self.style);
        let max_scroll = (total_h - self.max_height).max(0.0);
        self.scroll_offset = (self.scroll_offset - delta).clamp(0.0, max_scroll);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.open_submenu_ids.clear();
        self.pressed_zones.clear();
        self.popup_ids.clear();
        self.uses_popups = false;
    }

    pub fn close_popups(&mut self, backend: &mut dyn PopupSurface) {
        for &pid in &self.popup_ids {
            backend.destroy_popup(pid);
        }
        self.close();
    }

    pub fn open_popup(
        &mut self,
        x: f32, y: f32,
        items: Vec<MenuItem>,
        backend: &mut dyn PopupSurface,
    ) {
        self.width = compute_width(&items, &self.style);
        self.items = items;
        self.x = x;
        self.y = y;
        self.open = true;
        self.uses_popups = true;
        self.open_submenu_ids.clear();
        self.pressed_zones.clear();
        self.scroll_offset = 0.0;

        let total_h = items_height(&self.items, &self.style);
        let sc = self.style.scale;
        // Positioner wants logical size, divide physical by scale
        let log_w = (self.width / sc).ceil() as u32;
        let log_h = (total_h / sc).ceil() as u32;
        let pid = backend.create_popup(None, x as i32, y as i32, log_w, log_h);
        self.popup_ids.clear();
        self.popup_ids.push(pid);
    }

    pub fn draw_popups(
        &mut self,
        backend: &mut dyn PopupSurface,
    ) -> Option<MenuEvent> {
        if !self.open || !self.uses_popups { return None; }
        if self.popup_ids.is_empty() { return None; }

        // Popup surfaces don't need shadows — compositor handles that
        self.style.no_shadow = true;

        // Collect submenu panels that need popup surfaces
        let sc = self.style.scale;
        // (log_x, log_y, log_w, log_h, path, phys_w)
        let mut needed_popups: Vec<(i32, i32, u32, u32, Vec<usize>, f32)> = Vec::new();
        {
            let mut current_items: &[MenuItem] = &self.items;
            let mut parent_x = 0.0f32;
            let mut parent_width = self.width;
            let mut parent_y = 0.0f32;
            let mut path = Vec::new();

            for &sub_id in &self.open_submenu_ids {
                let Some((idx, children)) = find_submenu(current_items, sub_id) else { break; };
                path.push(idx);
                let sub_y_offset = item_y_offset(current_items, idx, &self.style);
                let sub_w = compute_width(children, &self.style);
                let sub_h = items_height_slice(children, &self.style);
                // For popups: position relative to parent popup surface (no overlap)
                let sub_x = parent_x + parent_width;
                let sub_y = parent_y + sub_y_offset;

                // Convert physical → logical for positioner, keep physical width for drawing
                needed_popups.push((
                    (sub_x / sc) as i32, (sub_y / sc) as i32,
                    (sub_w / sc).ceil() as u32, (sub_h / sc).ceil() as u32,
                    path.clone(), sub_w,
                ));

                parent_x = sub_x;
                parent_y = sub_y;
                parent_width = sub_w;
                current_items = children;
            }
        }

        // Create/destroy popup surfaces to match submenu state
        let target_count = 1 + needed_popups.len();
        // Destroy excess popups
        while self.popup_ids.len() > target_count {
            let pid = self.popup_ids.pop().unwrap();
            backend.destroy_popup(pid);
        }
        // Create missing popups (logical coordinates for positioner)
        for i in self.popup_ids.len()..target_count {
            let (sx, sy, sw, sh) = if i == 0 {
                (self.x as i32, self.y as i32,
                 (self.width / sc).ceil() as u32,
                 (items_height(&self.items, &self.style) / sc).ceil() as u32)
            } else {
                let p = &needed_popups[i - 1];
                (p.0, p.1, p.2, p.3)
            };
            // Root popup (i==0) is parented to the window, submenus to the parent popup
            let parent = if i == 0 { None } else { Some(self.popup_ids[i - 1]) };
            let pid = backend.create_popup(parent, sx, sy, sw, sh);
            self.popup_ids.push(pid);
        }

        // Resize existing submenu popups if needed
        for (i, np) in needed_popups.iter().enumerate() {
            let pid = self.popup_ids[i + 1];
            backend.resize_popup(pid, np.2, np.3);
        }

        // Draw each panel into its popup surface
        let mut event = None;
        let mut any_submenu_hovered: Option<(u32, usize)> = None;
        let mut any_non_submenu_hovered = false;

        // Build panel paths
        let mut panel_paths: Vec<Vec<usize>> = vec![vec![]];
        for np in &needed_popups {
            panel_paths.push(np.4.clone());
        }

        // Clear consumed presses (check root popup interaction)
        if let Some(ctx) = backend.popup_render(self.popup_ids[0]) {
            if ctx.interaction.active_zone_id().is_none() {
                self.pressed_zones.clear();
            }
        }

        for (depth, path) in panel_paths.iter().enumerate() {
            let pid = self.popup_ids[depth];
            let items = resolve_items(&mut self.items, path);
            let panel_w = if depth == 0 {
                self.width
            } else {
                needed_popups[depth - 1].5
            };

            if let Some(ctx) = backend.popup_render(pid) {
                // Set clear color to menu bg with alpha=0 so SDF edges
                // blend against the correct RGB instead of black.
                ctx.clear_color = self.style.bg.with_alpha(0.0);
                let sw = ctx.gpu.width();
                let sh = ctx.gpu.height();
                let result = draw_panel(
                    items, 0.0, 0.0, panel_w, depth,
                    &self.style, &mut ctx.painter, &mut ctx.text,
                    &mut ctx.interaction, sw, sh,
                    &mut self.open_submenu_ids, &mut self.pressed_zones,
                );
                if result.event.is_some() { event = result.event; }
                if let Some(sub_id) = result.hovered_submenu {
                    any_submenu_hovered = Some((sub_id, depth));
                }
                if result.non_submenu_hovered {
                    any_non_submenu_hovered = true;
                }
            }
        }

        self.process_submenu_hover(any_submenu_hovered, any_non_submenu_hovered);

        if let Some(MenuEvent::RadioSelected { group, id }) = &event {
            deselect_radio_group(&mut self.items, *group, *id);
        }

        event
    }

    /// Advance submenu open/close delay timers.
    pub fn update(&mut self, dt: f32) {
        if !self.open { return; }
        if let Some(pending_id) = self.submenu_hover_id {
            self.submenu_hover_timer += dt;
            if self.submenu_hover_timer >= SUBMENU_OPEN_DELAY {
                // Timer expired — open the submenu
                let depth = self.submenu_hover_depth;
                self.open_submenu_ids.truncate(depth);
                self.open_submenu_ids.push(pending_id);
                self.submenu_hover_id = None;
                self.submenu_hover_timer = 0.0;
                // Cancel any pending close since we just opened
                self.submenu_close_pending = false;
                self.submenu_close_timer = 0.0;
            }
        }
        // Submenu close delay
        if self.submenu_close_pending {
            self.submenu_close_timer += dt;
            if self.submenu_close_timer >= SUBMENU_CLOSE_DELAY {
                self.submenu_close_pending = false;
                self.submenu_close_timer = 0.0;
                if let Some(depth) = self.pointer_depth {
                    if depth < self.open_submenu_ids.len() {
                        self.open_submenu_ids.truncate(depth);
                    }
                }
            }
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
        if !self.open { return None; }

        // Clear consumed presses once mouse is released
        if interaction.active_zone_id().is_none() {
            self.pressed_zones.clear();
        }

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
        let mut any_submenu_hovered: Option<(u32, usize)> = None;
        let mut any_non_submenu_hovered = false;

        for (depth, panel) in panels.iter().enumerate() {
            let items = resolve_items(&mut self.items, &panel.path);
            let result = draw_panel(
                items, panel.x, panel.y, panel.width, depth,
                &self.style, painter, text, interaction,
                screen_w, screen_h, &mut self.open_submenu_ids,
                &mut self.pressed_zones,
            );
            if result.event.is_some() { event = result.event; }
            if let Some(sub_id) = result.hovered_submenu {
                any_submenu_hovered = Some((sub_id, depth));
            }
            if result.non_submenu_hovered {
                any_non_submenu_hovered = true;
            }
        }

        self.process_submenu_hover(any_submenu_hovered, any_non_submenu_hovered);

        // Handle radio group deselection
        if let Some(MenuEvent::RadioSelected { group, id }) = &event {
            deselect_radio_group(&mut self.items, *group, *id);
        }

        event
    }

    /// Process submenu hover state: start/reset timer, close stale submenus.
    fn process_submenu_hover(
        &mut self,
        hovered_submenu: Option<(u32, usize)>,
        non_submenu_hovered: bool,
    ) {
        match hovered_submenu {
            Some((id, depth)) => {
                // Cancel any pending close — user is on a submenu trigger
                self.submenu_close_pending = false;
                self.submenu_close_timer = 0.0;

                // Already open at this depth — nothing to do
                if self.open_submenu_ids.get(depth) == Some(&id) {
                    self.submenu_hover_id = None;
                    self.submenu_hover_timer = 0.0;
                    return;
                }
                // Start or continue timer for this submenu
                if self.submenu_hover_id == Some(id) && self.submenu_hover_depth == depth {
                    // Timer continues in update()
                } else {
                    self.submenu_hover_id = Some(id);
                    self.submenu_hover_timer = 0.0;
                    self.submenu_hover_depth = depth;
                }
            }
            None => {
                // Cancel any pending open
                self.submenu_hover_id = None;
                self.submenu_hover_timer = 0.0;

                // If cursor is on the submenu popup itself, cancel close
                if let Some(depth) = self.pointer_depth {
                    if depth >= self.open_submenu_ids.len() || depth > 0 {
                        // Pointer is on a submenu surface — keep it open
                        self.submenu_close_pending = false;
                        self.submenu_close_timer = 0.0;
                        return;
                    }
                }

                // Start close delay when cursor is on a parent panel
                // hovering a non-submenu item. Grace period allows cursor
                // to travel from trigger to the submenu popup.
                if non_submenu_hovered && !self.open_submenu_ids.is_empty() {
                    if !self.submenu_close_pending {
                        self.submenu_close_pending = true;
                        self.submenu_close_timer = 0.0;
                    }
                } else {
                    self.submenu_close_pending = false;
                    self.submenu_close_timer = 0.0;
                }
            }
        }
    }

    pub fn bounds(&self) -> Option<Rect> {
        if !self.open { return None; }
        let h = items_height(&self.items, &self.style);
        let visible_h = if self.max_height > 0.0 { self.max_height.min(h) } else { h };
        Some(Rect::new(self.x, self.y, self.width, visible_h))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        if !self.open { return false; }
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
        MenuItem::Separator | MenuItem::ColoredSeparator(_) => SEPARATOR_HEIGHT * s,
        MenuItem::Slider { .. } => SLIDER_ITEM_HEIGHT * s,
        MenuItem::Progress { .. } => PROGRESS_ITEM_HEIGHT * s,
        MenuItem::Header { .. } => HEADER_HEIGHT * s,
        MenuItem::ColorSwatches { .. } => COLOR_SWATCH_HEIGHT * s,
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
    let sc_fw = FONT_LABEL * s * 0.55;
    let cap_fw = FONT_CAPTION * s * 0.55;
    let max_w = items.iter().filter_map(|item| match item {
        MenuItem::Action { label, shortcut, .. } => {
            let sc_w = shortcut.as_ref()
                .map_or(0.0, |sc| sc.len() as f32 * sc_fw + 24.0 * s);
            Some(label.len() as f32 * fw + sc_w)
        }
        MenuItem::Toggle { label, .. } | MenuItem::Checkbox { label, .. }
        | MenuItem::Radio { label, .. } => Some(label.len() as f32 * fw + 48.0 * s),
        MenuItem::SubMenu { label, .. } => Some(label.len() as f32 * fw + 28.0 * s),
        MenuItem::Slider { label, .. } | MenuItem::Progress { label, .. } => {
            Some(label.len() as f32 * cap_fw + 80.0 * s)
        }
        MenuItem::Button { label, .. } => Some(label.len() as f32 * fw + 40.0 * s),
        MenuItem::Header { label } => Some(label.len() as f32 * sc_fw),
        MenuItem::ColorSwatches { swatches, .. } => {
            let icon = 40.0 * s;
            let gap = 6.0 * s;
            Some(swatches.len() as f32 * icon + (swatches.len().saturating_sub(1)) as f32 * gap)
        }
        MenuItem::Separator | MenuItem::ColoredSeparator(_) => None,
    }).fold(0.0f32, f32::max);
    (max_w + style.padding * s * 6.0).max(style.min_width * s)
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

// Constants (pub(super) for draw module)
pub(super) const SEPARATOR_HEIGHT: f32 = 12.0;
pub(super) const SLIDER_ITEM_HEIGHT: f32 = 60.0;
pub(super) const SLIDER_TRACK_H: f32 = 12.0;
pub(super) const HEADER_HEIGHT: f32 = 32.0;
pub(super) const PROGRESS_ITEM_HEIGHT: f32 = 50.0;
pub(super) const CONTEXT_MENU_ZONE_BASE: u32 = 0xCE_0000;
pub(super) const COLOR_SWATCH_HEIGHT: f32 = 80.0;
pub(super) const ACCENT_BAR_WIDTH: f32 = 3.5;
