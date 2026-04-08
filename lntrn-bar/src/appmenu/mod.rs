//! App launcher menu — popup above the bar with floating tabs, search, and grid.
//! Split into mod.rs (state/logic), draw.rs (rendering), sysmon.rs (system monitor).

mod actions;
pub(crate) mod clipboard_tab;
mod draw;
pub(crate) mod notes;
mod sidebar;
pub(crate) mod sysmon;
mod sysmon_draw;

use std::collections::HashSet;

use lntrn_render::{GpuContext, Rect, TexturePass};
use lntrn_ui::gpu::InteractionContext;

use crate::apptray::find_icon;
use crate::desktop::{self, Category, DesktopEntry};
use crate::svg_icon::IconCache;

pub(crate) const CELL_SIZE: f32 = 120.0;
pub(crate) const ICON_SIZE: f32 = 56.0;
pub(crate) const PADDING: f32 = 16.0;
pub(crate) const LABEL_FONT: f32 = 16.0;
pub(crate) const FOOTER_ICON_SZ: f32 = 24.0;

pub(crate) const TAB_SIZE: f32 = 44.0;
pub(crate) const TAB_GAP: f32 = 6.0;
pub(crate) const SEARCH_FLOAT_GAP: f32 = 8.0;

pub(crate) const ZONE_BASE: u32 = 0xBB_0000;
pub(crate) const ZONE_CTX: u32 = 0xBD_0000;
pub(crate) const ZONE_POWER: u32 = 0xBE_0000;
pub(crate) const ZONE_TAB: u32 = 0xBF_0000;
pub(crate) const ZONE_CAT: u32 = 0xBC_0000;

pub(crate) const SIDEBAR_W: f32 = 140.0;

pub(crate) const RESIZE_EDGE: f32 = 6.0;

const DEFAULT_WIDTH: f32 = 800.0;
const DEFAULT_HEIGHT: f32 = 540.0;
const MIN_WIDTH: f32 = 500.0;
const MIN_HEIGHT: f32 = 360.0;
const MAX_WIDTH: f32 = 1400.0;
const MAX_HEIGHT: f32 = 900.0;

fn favorites_path() -> std::path::PathBuf { crate::bar_config_dir().join("favorites.txt") }
fn size_path() -> std::path::PathBuf { crate::bar_config_dir().join("menu_size.txt") }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuTab {
    Apps,
    SystemMonitor,
    Notes,
    Clipboard,
}

impl MenuTab {
    /// Tabs shown in the top row (next to search bar).
    pub const TOP: &[MenuTab] = &[MenuTab::Apps, MenuTab::SystemMonitor];
    /// Tabs shown on the right side (aligned with panel top).
    pub const RIGHT: &[MenuTab] = &[MenuTab::Notes, MenuTab::Clipboard];
}

#[derive(Debug, Clone, Copy)]
pub enum ResizeEdge {
    Top,
    Right,
    TopRight,
}

pub struct AppMenu {
    pub(crate) open: bool,
    pub(crate) entries: Vec<DesktopEntry>,
    pub(crate) search: String,
    pub(crate) scroll_offset: f32,
    pub(crate) icons_loaded: bool,
    pub(crate) bounds: Rect,
    pub(crate) favorites: HashSet<String>,
    pub(crate) active_tab: MenuTab,
    pub(crate) sysmon: sysmon::SystemMonitor,
    pub(crate) notes: notes::Notes,
    pub(crate) clipboard: clipboard_tab::ClipboardHistory,
    /// Selected category filter
    pub(crate) selected_category: Category,
    /// Right-click context menu state
    pub(crate) ctx_app_id: Option<String>,
    pub(crate) ctx_pos: (f32, f32),
    pub(crate) ctx_open: bool,
    /// Resizable menu dimensions (logical pixels)
    pub(crate) menu_w: f32,
    pub(crate) menu_h: f32,
    /// Drag-to-resize state
    pub(crate) dragging: Option<ResizeEdge>,
    pub(crate) drag_start: (f32, f32),
    pub(crate) drag_start_size: (f32, f32),
}

impl AppMenu {
    pub fn new() -> Self {
        let (w, h) = load_size();
        let mut menu = Self {
            open: false,
            entries: Vec::new(),
            search: String::new(),
            scroll_offset: 0.0,
            icons_loaded: false,
            bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
            favorites: HashSet::new(),
            active_tab: MenuTab::Apps,
            selected_category: Category::Favorites,
            sysmon: sysmon::SystemMonitor::new(),
            notes: notes::Notes::new(),
            clipboard: clipboard_tab::ClipboardHistory::new(),
            ctx_app_id: None,
            ctx_pos: (0.0, 0.0),
            ctx_open: false,
            menu_w: w,
            menu_h: h,
            dragging: None,
            drag_start: (0.0, 0.0),
            drag_start_size: (0.0, 0.0),
        };
        menu.load_favorites();
        menu
    }

    pub fn is_open(&self) -> bool { self.open }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.entries = desktop::scan_apps();
            self.icons_loaded = false;
            tracing::info!("app menu: scanned {} apps", self.entries.len());
            self.search.clear();
            self.scroll_offset = 0.0;
            self.ctx_open = false;
            self.active_tab = MenuTab::Apps;
            self.selected_category = Category::Favorites;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.ctx_open = false;
        self.sysmon.filter_focused = false;
        self.clipboard.search_focused = false;
        self.notes.save_all();
    }

    /// Returns the context menu rect if it's open, for texture occlusion.
    #[allow(dead_code)]
    pub fn ctx_menu_rect(&self, scale: f32) -> Option<[f32; 4]> {
        if !self.ctx_open { return None; }
        let item_h = 40.0 * scale;
        let menu_w = 250.0 * scale;
        let menu_h = item_h * 4.0 + 8.0 * scale;
        let pad = 4.0 * scale;
        let ctx_x = self.ctx_pos.0.min(self.bounds.x + self.bounds.w - menu_w - pad);
        let ctx_y = self.ctx_pos.1.min(self.bounds.y + self.bounds.h - menu_h - pad);
        Some([ctx_x, ctx_y, menu_w, menu_h])
    }

    /// Hit-test: checks main panel, floating search bar, and floating tabs.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if !self.open { return false; }
        if self.ctx_open { return true; }

        let edge = RESIZE_EDGE * 2.0;
        let b = &self.bounds;

        // Main panel (with resize edge expansion)
        if x >= b.x - edge && x <= b.x + b.w + edge
            && y >= b.y - edge && y <= b.y + b.h + edge
        {
            return true;
        }

        // Floating row above panel (search bar + tabs, one row)
        let float_h = (TAB_SIZE + SEARCH_FLOAT_GAP) * 2.0; // generous
        if x >= b.x && x <= b.x + b.w
            && y >= b.y - float_h && y < b.y
        {
            return true;
        }

        // Floating power icons to the right of panel
        let power_right = (TAB_SIZE + TAB_GAP) * 2.0; // generous
        if x > b.x + b.w && x <= b.x + b.w + power_right
            && y >= b.y && y <= b.y + b.h
        {
            return true;
        }

        false
    }

    pub fn resize_edge_at(&self, px: f32, py: f32, scale: f32) -> Option<ResizeEdge> {
        if !self.open { return None; }
        let e = RESIZE_EDGE * scale;
        let bx = self.bounds.x;
        let by = self.bounds.y;
        let bw = self.bounds.w;

        let on_top = py >= by - e && py <= by + e;
        let on_right = px >= bx + bw - e && px <= bx + bw + e;

        match (on_top, on_right) {
            (true, true) => Some(ResizeEdge::TopRight),
            (true, false) => Some(ResizeEdge::Top),
            (false, true) => Some(ResizeEdge::Right),
            _ => None,
        }
    }

    pub fn start_resize(&mut self, edge: ResizeEdge, px: f32, py: f32) {
        self.dragging = Some(edge);
        self.drag_start = (px, py);
        self.drag_start_size = (self.menu_w, self.menu_h);
    }

    pub fn update_resize(&mut self, px: f32, py: f32, scale: f32) {
        let Some(edge) = self.dragging else { return };
        let dx = (px - self.drag_start.0) / scale;
        let dy = (py - self.drag_start.1) / scale;
        match edge {
            ResizeEdge::Right => {
                self.menu_w = (self.drag_start_size.0 + dx).clamp(MIN_WIDTH, MAX_WIDTH);
            }
            ResizeEdge::Top => {
                self.menu_h = (self.drag_start_size.1 - dy).clamp(MIN_HEIGHT, MAX_HEIGHT);
            }
            ResizeEdge::TopRight => {
                self.menu_w = (self.drag_start_size.0 + dx).clamp(MIN_WIDTH, MAX_WIDTH);
                self.menu_h = (self.drag_start_size.1 - dy).clamp(MIN_HEIGHT, MAX_HEIGHT);
            }
        }
    }

    pub fn end_resize(&mut self) {
        if self.dragging.is_some() {
            self.dragging = None;
            save_size(self.menu_w, self.menu_h);
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// Handle keyboard input — returns true if consumed.
    pub fn on_key(&mut self, key: u32, shift: bool) -> bool {
        if !self.open || self.ctx_open { return false; }
        // Notes editor gets priority when editing
        if self.active_tab == MenuTab::Notes && self.notes.wants_keyboard() {
            return self.notes.on_key(key, shift);
        }
        // Clipboard search filter
        if self.active_tab == MenuTab::Clipboard && self.clipboard.wants_keyboard() {
            return self.clipboard.on_key(key, shift);
        }
        // Sysmon filter gets keyboard when on SystemMonitor tab
        if self.active_tab == MenuTab::SystemMonitor && self.sysmon.filter_focused {
            match key {
                1 => { // Esc — unfocus filter or close menu
                    if !self.sysmon.proc_filter.is_empty() {
                        self.sysmon.proc_filter.clear();
                        self.sysmon.filter_cursor = 0;
                    } else {
                        self.close();
                    }
                    return true;
                }
                14 => { // Backspace
                    if self.sysmon.filter_cursor > 0 {
                        self.sysmon.filter_cursor -= 1;
                        self.sysmon.proc_filter.remove(self.sysmon.filter_cursor);
                    }
                    return true;
                }
                _ => {
                    if let Some(ch) = keycode_to_char(key, shift) {
                        self.sysmon.proc_filter.insert(self.sysmon.filter_cursor, ch);
                        self.sysmon.filter_cursor += 1;
                        return true;
                    }
                }
            }
            return false;
        }
        match key {
            1 => { self.close(); true } // Esc
            14 => { self.search.pop(); self.scroll_offset = 0.0; true } // Backspace
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    self.search.push(ch);
                    self.scroll_offset = 0.0;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn wants_keyboard(&self) -> bool {
        self.open && !self.ctx_open
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if !self.open || self.ctx_open { return; }
        match self.active_tab {
            MenuTab::Apps => self.scroll_offset = (self.scroll_offset + delta).max(0.0),
            MenuTab::SystemMonitor => self.sysmon.scroll_offset = (self.sysmon.scroll_offset + delta).max(0.0),
            MenuTab::Notes => self.notes.scroll_offset = (self.notes.scroll_offset + delta).max(0.0),
            MenuTab::Clipboard => self.clipboard.scroll_offset = (self.clipboard.scroll_offset + delta).max(0.0),
        }
    }

    pub fn on_right_click(&mut self, phys_x: f32, phys_y: f32, ix: &InteractionContext) {
        if !self.open { return; }
        if let Some(zone_id) = ix.zone_at(phys_x, phys_y) {
            if zone_id >= ZONE_BASE && zone_id < ZONE_BASE + 0x10000 {
                let idx = (zone_id - ZONE_BASE) as usize;
                let filtered = self.filtered_indices();
                if let Some(&entry_idx) = filtered.get(idx) {
                    self.ctx_app_id = Some(self.entries[entry_idx].app_id.clone());
                    self.ctx_pos = (phys_x, phys_y);
                    self.ctx_open = true;
                    return;
                }
            }
        }
        self.ctx_open = false;
    }

    pub fn on_left_click(&mut self, phys_x: f32, phys_y: f32, ix: &InteractionContext, scale: f32) {
        if self.ctx_open {
            let menu_w = 250.0 * scale;
            let menu_h = (40.0 * 4.0 + 8.0) * scale;
            let (cx, cy) = self.ctx_pos;
            if phys_x < cx || phys_x > cx + menu_w || phys_y < cy || phys_y > cy + menu_h {
                self.ctx_open = false;
            } else if let Some(app_id) = self.ctx_app_id.clone() {
                // Click inside context menu — handle actions directly
                if let Some(zone) = ix.zone_at(phys_x, phys_y) {
                    match zone {
                        ZONE_CTX => {
                            self.toggle_favorite(&app_id);
                        }
                        z if z == ZONE_CTX + 1 => {
                            launch_app("lntrn-system-settings --panel app-icons");
                        }
                        z if z == ZONE_CTX + 2 => {
                            crate::desktop::hide_app(&app_id);
                            self.entries.retain(|e| e.app_id != app_id);
                        }
                        z if z == ZONE_CTX + 3 => {
                            uninstall_app(&app_id);
                            self.entries.retain(|e| e.app_id != app_id);
                        }
                        _ => {}
                    }
                }
                self.ctx_open = false;
            }
            return;
        }
        // Category sidebar clicks (Apps tab)
        if self.active_tab == MenuTab::Apps {
            if let Some(zone) = ix.zone_at(phys_x, phys_y) {
                if zone >= ZONE_CAT && zone < ZONE_CAT + 0x100 {
                    let idx = (zone - ZONE_CAT) as usize;
                    if let Some(&cat) = Category::SIDEBAR_ORDER.get(idx) {
                        self.selected_category = cat;
                        self.scroll_offset = 0.0;
                    }
                    return;
                }
            }
        }
        // Sysmon cores toggle + kill button + sort headers
        if self.active_tab == MenuTab::SystemMonitor {
            if self.sysmon.on_kill_click(ix, phys_x, phys_y) {
                return;
            }
            if self.sysmon.on_sort_click(ix, phys_x, phys_y) {
                return;
            }
            if let Some(zone) = ix.zone_at(phys_x, phys_y) {
                if zone == sysmon::ZONE_CORES_TOGGLE {
                    self.sysmon.cores_expanded = !self.sysmon.cores_expanded;
                }
            }
            // Toggle filter focus
            self.sysmon.filter_focused = true;
        }
        // Notes click
        if self.active_tab == MenuTab::Notes {
            self.notes.on_left_click(ix, phys_x, phys_y);
        }
        // Clipboard click
        if self.active_tab == MenuTab::Clipboard {
            self.clipboard.on_left_click(ix, phys_x, phys_y);
        }
    }

    // -- Favorites --

    fn load_favorites(&mut self) {
        if let Ok(content) = std::fs::read_to_string(favorites_path()) {
            self.favorites = content.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
        }
    }

    fn save_favorites(&self) {
        let _ = std::fs::create_dir_all(crate::bar_config_dir());
        let content: String = self.favorites.iter()
            .map(|s| format!("{s}\n"))
            .collect();
        let _ = std::fs::write(favorites_path(), content);
    }

    pub(crate) fn toggle_favorite(&mut self, app_id: &str) {
        if self.favorites.contains(app_id) {
            self.favorites.remove(app_id);
        } else {
            self.favorites.insert(app_id.to_string());
        }
        self.save_favorites();
    }

    // -- Filtering (search only, no categories) --

    pub(crate) fn filtered_indices(&self) -> Vec<usize> {
        let q = self.search.to_lowercase();
        self.entries.iter().enumerate()
            .filter(|(_, e)| {
                // Search filter
                if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                    return false;
                }
                // Category filter
                match self.selected_category {
                    Category::All => true,
                    Category::Favorites => self.favorites.contains(&e.app_id),
                    cat => e.category == cat,
                }
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Load icon textures for visible apps + power footer icons.
    pub fn load_icons(
        &mut self,
        icon_cache: &mut IconCache,
        tex_pass: &TexturePass,
        gpu: &GpuContext,
        scale: f32,
    ) {
        if !self.open || self.icons_loaded { return; }
        self.icons_loaded = true;

        // Clear old appmenu icons so custom icons get picked up
        for entry in &self.entries {
            icon_cache.remove(&format!("appmenu_{}", entry.app_id));
        }

        let icon_sz = (ICON_SIZE * scale) as u32;
        for entry in &self.entries {
            let key = format!("appmenu_{}", entry.app_id);
            // Try embedded icon first (our Lantern apps)
            let embedded = lntrn_icons::get(&format!("{}.svg", entry.app_id))
                .or_else(|| lntrn_icons::get(&format!("{}.png", entry.app_id)));
            if embedded.is_some() {
                let file = if lntrn_icons::get(&format!("{}.svg", entry.app_id)).is_some() {
                    format!("{}.svg", entry.app_id)
                } else {
                    format!("{}.png", entry.app_id)
                };
                icon_cache.load_embedded(tex_pass, gpu, &key, &file, icon_sz, icon_sz);
                continue;
            }
            // Fall back to disk (third-party apps)
            let icon_name = entry.icon.as_deref().unwrap_or(&entry.app_id);
            let path = if icon_name.starts_with('/') {
                let p = std::path::PathBuf::from(icon_name);
                if p.exists() { Some(p) } else { None }
            } else {
                find_icon(icon_name)
            };
            if let Some(path) = path {
                icon_cache.load(tex_pass, gpu, &key, &path, icon_sz, icon_sz);
            }
        }

        // Category sidebar icons
        let cat_sz = (18.0 * scale) as u32;
        let icons_dir = crate::lantern_icons_dir();
        for &cat in Category::SIDEBAR_ORDER {
            let key = format!("cat_{}", cat.label());
            if icon_cache.get(&key).is_some() { continue; }
            let path = icons_dir.join(cat.icon_file());
            if path.exists() {
                icon_cache.load(tex_pass, gpu, &key, &path, cat_sz, cat_sz);
            }
        }

        // Power footer icons (all embedded)
        let pwr_sz = (FOOTER_ICON_SZ * scale) as u32;
        for (key_name, _label, svg_file) in draw::POWER_ICONS {
            let key = format!("power_{key_name}");
            if icon_cache.get(&key).is_some() { continue; }
            icon_cache.load_embedded(tex_pass, gpu, &key, svg_file, pwr_sz, pwr_sz);
        }
    }
}

fn load_size() -> (f32, f32) {
    std::fs::read_to_string(size_path()).ok().and_then(|s| {
        let mut parts = s.trim().split('x');
        let w: f32 = parts.next()?.parse().ok()?;
        let h: f32 = parts.next()?.parse().ok()?;
        Some((w.clamp(MIN_WIDTH, MAX_WIDTH), h.clamp(MIN_HEIGHT, MAX_HEIGHT)))
    }).unwrap_or((DEFAULT_WIDTH, DEFAULT_HEIGHT))
}

fn save_size(w: f32, h: f32) {
    let _ = std::fs::create_dir_all(crate::bar_config_dir());
    let _ = std::fs::write(size_path(), format!("{}x{}", w as u32, h as u32));
}

pub(crate) use actions::launch_app;
use actions::uninstall_app;

pub(crate) fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    let ch = match key {
        2..=11 => {
            let base = b"1234567890"[(key - 2) as usize];
            if shift { b"!@#$%^&*()"[(key - 2) as usize] } else { base }
        }
        12 => if shift { b'_' } else { b'-' },
        13 => if shift { b'+' } else { b'=' },
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        57 => b' ',
        _ => return None,
    };
    Some(ch as char)
}

