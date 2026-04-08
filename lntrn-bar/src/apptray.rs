//! App tray widget — centered in the bar, shows pinned + open apps with icons.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lntrn_render::{GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::svg_icon::IconCache;
use crate::toplevel::ToplevelInfo;

const ZONE_BASE: u32 = 0xAA_0000;
pub const ICON_GAP: f32 = 8.0;
fn config_path() -> PathBuf {
    crate::bar_config_dir().join("apptray.toml")
}
/// Minimum pixels the cursor must move before a drag starts.
const DRAG_THRESHOLD: f32 = 6.0;
/// Animation duration for icons sliding into position.
const SLIDE_DURATION: f32 = 0.15;

/// A single slot in the app tray (pinned, running, or both).
#[derive(Debug, Clone)]
struct AppSlot {
    app_id: String,
    pinned: bool,
    running: bool,
    activated: bool,
    #[allow(dead_code)]
    title: String,
}

/// Active drag state.
struct DragState {
    /// Index in the current slot list where the drag started.
    src_idx: usize,
    /// The app_id being dragged.
    #[allow(dead_code)]
    app_id: String,
    /// Cursor X when the press happened (physical pixels).
    press_x: f32,
    /// Current cursor X (physical pixels).
    cursor_x: f32,
    /// Whether we've exceeded the drag threshold.
    active: bool,
    /// The current insertion index (where the dragged icon would land).
    insert_idx: usize,
}

/// The app tray widget, rendered centered in the bar.
pub struct AppTray {
    pinned: Vec<PinnedApp>,
    /// Resolved icon paths per app_id.
    icon_paths: HashMap<String, PathBuf>,
    icons_loaded: HashMap<String, bool>,
    config_modified: Option<std::time::SystemTime>,
    /// Explicit order for running-but-unpinned apps (persists across frames).
    running_order: Vec<String>,
    /// Active drag, if any.
    drag: Option<DragState>,
    /// Animated x-offsets per app_id (pixels, converging to 0).
    anim_offsets: HashMap<String, f32>,
}

#[derive(Debug, Clone)]
struct PinnedApp {
    app_id: String,
}

impl AppTray {
    pub fn new() -> Self {
        let mut tray = Self {
            pinned: Vec::new(),
            icon_paths: HashMap::new(),
            icons_loaded: HashMap::new(),
            config_modified: None,
            running_order: Vec::new(),
            drag: None,
            anim_offsets: HashMap::new(),
        };
        tray.load_config();
        tray
    }

    fn load_config(&mut self) {
        let path = config_path();

        // Check if file changed
        let modified = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok();
        if modified == self.config_modified && self.config_modified.is_some() {
            return;
        }
        self.config_modified = modified;

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                // Create default config
                let default = "# Pinned apps (by app_id)\npinned = []\n";
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                let _ = std::fs::write(&path, default);
                return;
            }
        };

        self.pinned.clear();
        // Simple TOML parsing — just look for pinned = ["app1", "app2"]
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("pinned") {
                if let Some(start) = line.find('[') {
                    if let Some(end) = line.find(']') {
                        let inner = &line[start + 1..end];
                        for item in inner.split(',') {
                            let item = item.trim().trim_matches('"').trim_matches('\'');
                            if !item.is_empty() {
                                self.pinned.push(PinnedApp {
                                    app_id: item.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// Save current pinned list to config.
    fn save_config(&self) {
        let ids: Vec<String> = self.pinned.iter().map(|p| format!("\"{}\"", p.app_id)).collect();
        let content = format!(
            "# Pinned apps (by app_id)\npinned = [{}]\n",
            ids.join(", ")
        );
        let _ = std::fs::create_dir_all(crate::bar_config_dir());
        let _ = std::fs::write(config_path(), content);
    }

    /// Pin an app_id (persists to config).
    pub fn pin(&mut self, app_id: &str) {
        if !self.pinned.iter().any(|p| p.app_id == app_id) {
            self.pinned.push(PinnedApp { app_id: app_id.to_string() });
            self.save_config();
        }
    }

    /// Unpin an app_id (persists to config).
    pub fn unpin(&mut self, app_id: &str) {
        self.pinned.retain(|p| p.app_id != app_id);
        self.save_config();
    }

    // ── Drag-to-reorder ──────────────────────────────────────────────

    /// Call on left mouse press. Returns true if press was on a tray icon.
    pub fn on_press(
        &mut self,
        ix: &InteractionContext,
        phys_cx: f32,
        phys_cy: f32,
        toplevels: &[ToplevelInfo],
    ) -> bool {
        let zone = match ix.zone_at(phys_cx, phys_cy) {
            Some(z) if z >= ZONE_BASE && z < ZONE_BASE + 256 => z,
            _ => return false,
        };
        let idx = (zone - ZONE_BASE) as usize;
        let slots = self.build_slots(toplevels);
        if idx >= slots.len() { return false; }
        self.drag = Some(DragState {
            src_idx: idx,
            app_id: slots[idx].app_id.clone(),
            press_x: phys_cx,
            cursor_x: phys_cx,
            active: false,
            insert_idx: idx,
        });
        true
    }

    /// Call on pointer motion while left button is held.
    /// Returns true if a drag is active (caller should keep animating).
    pub fn on_motion(
        &mut self,
        phys_cx: f32,
        toplevels: &[ToplevelInfo],
        icon_size: f32,
        gap: f32,
        bar_x: f32,
        bar_w: f32,
    ) -> bool {
        let drag = match &mut self.drag {
            Some(d) => d,
            None => return false,
        };
        drag.cursor_x = phys_cx;

        // Check drag threshold
        if !drag.active {
            if (phys_cx - drag.press_x).abs() < DRAG_THRESHOLD {
                return false;
            }
            drag.active = true;
        }

        // Extract values we need from drag before releasing the borrow
        let press_x = drag.press_x;
        let src_idx = drag.src_idx;
        let prev_insert = drag.insert_idx;

        // Compute insertion index from cursor position
        let slots = self.build_slots(toplevels);
        let n = slots.len();
        let stride = icon_size + gap;
        let total_w = n as f32 * stride - gap;
        let start_x = bar_x + (bar_w - total_w) / 2.0;

        // Where is the center of the dragged icon right now?
        let drag_offset = phys_cx - press_x;
        let src_center = start_x + src_idx as f32 * stride + icon_size / 2.0 + drag_offset;

        // Find which slot the center is closest to
        let mut best = 0usize;
        let mut best_dist = f32::MAX;
        for i in 0..n {
            let slot_center = start_x + i as f32 * stride + icon_size / 2.0;
            let dist = (src_center - slot_center).abs();
            if dist < best_dist {
                best_dist = dist;
                best = i;
            }
        }

        // Update drag insert index
        if let Some(d) = &mut self.drag {
            d.insert_idx = best;
        }

        // If insertion changed, set up animation offsets for shifting icons
        if prev_insert != best {
            for (i, slot) in slots.iter().enumerate() {
                if i == src_idx { continue; }
                let old_vis = visual_index(i, src_idx, prev_insert);
                let new_vis = visual_index(i, src_idx, best);
                let delta = (old_vis as f32 - new_vis as f32) * stride;
                let current = self.anim_offsets.get(&slot.app_id).copied().unwrap_or(0.0);
                self.anim_offsets.insert(slot.app_id.clone(), current + delta);
            }
        }

        true
    }

    /// Call on left mouse release. Returns true if drag completed a reorder.
    pub fn on_release(&mut self, toplevels: &[ToplevelInfo]) -> bool {
        let drag = match self.drag.take() {
            Some(d) => d,
            None => return false,
        };
        if !drag.active || drag.src_idx == drag.insert_idx {
            return false;
        }

        // Apply the reorder
        let mut slots = self.build_slots(toplevels);
        if drag.src_idx >= slots.len() || drag.insert_idx >= slots.len() {
            return false;
        }
        let removed = slots.remove(drag.src_idx);
        slots.insert(drag.insert_idx, removed);

        // Rebuild pinned list and running_order from new slot order
        self.pinned = slots.iter()
            .filter(|s| s.pinned)
            .map(|s| PinnedApp { app_id: s.app_id.clone() })
            .collect();
        self.running_order = slots.iter()
            .filter(|s| !s.pinned)
            .map(|s| s.app_id.clone())
            .collect();
        self.save_config();

        true
    }

    /// Whether a drag is currently active (past threshold).
    pub fn is_dragging(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.active)
    }

    /// Advance slide animations. Returns true if still animating.
    pub fn update_anim(&mut self, dt: f32) -> bool {
        if self.anim_offsets.is_empty() {
            return false;
        }
        let speed = 1.0 / SLIDE_DURATION;
        let mut any = false;
        self.anim_offsets.retain(|_, offset| {
            let step = offset.abs() * speed * dt + 0.5; // proportional + minimum
            if *offset > step {
                *offset -= step;
                any = true;
                true
            } else if *offset < -step {
                *offset += step;
                any = true;
                true
            } else {
                false // animation done, remove
            }
        });
        any
    }

    /// Build the merged slot list from pinned + running toplevels.
    fn build_slots(&self, toplevels: &[ToplevelInfo]) -> Vec<AppSlot> {
        let mut slots: Vec<AppSlot> = Vec::new();

        // Start with pinned apps
        for pin in &self.pinned {
            let any_running = toplevels.iter().any(|t| t.app_id == pin.app_id);
            let any_activated = toplevels.iter().any(|t| t.app_id == pin.app_id && t.activated);
            let title = toplevels.iter()
                .find(|t| t.app_id == pin.app_id)
                .map_or(String::new(), |t| t.title.clone());
            slots.push(AppSlot {
                app_id: pin.app_id.clone(),
                pinned: true,
                running: any_running,
                activated: any_activated,
                title,
            });
        }

        // Add running apps that aren't pinned, respecting running_order
        let mut unpinned: Vec<AppSlot> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for t in toplevels {
            if t.app_id.is_empty() || seen.contains(&t.app_id) {
                continue;
            }
            if !slots.iter().any(|s| s.app_id == t.app_id) {
                seen.insert(t.app_id.clone());
                let any_activated = toplevels.iter()
                    .any(|t2| t2.app_id == t.app_id && t2.activated);
                unpinned.push(AppSlot {
                    app_id: t.app_id.clone(),
                    pinned: false,
                    running: true,
                    activated: any_activated,
                    title: t.title.clone(),
                });
            }
        }
        // Sort unpinned by running_order (known order first, then newcomers)
        unpinned.sort_by_key(|s| {
            self.running_order.iter().position(|id| *id == s.app_id)
                .unwrap_or(usize::MAX)
        });
        slots.extend(unpinned);

        slots
    }

    /// Resolve icon path for an app_id (freedesktop lookup with custom fallback).
    fn resolve_icon(&mut self, app_id: &str) -> Option<PathBuf> {
        if let Some(path) = self.icon_paths.get(app_id) {
            return Some(path.clone());
        }

        let path = find_icon(app_id)?;
        self.icon_paths.insert(app_id.to_string(), path.clone());
        Some(path)
    }

    /// Load icon textures for all visible apps.
    pub fn load_icons(
        &mut self,
        toplevels: &[ToplevelInfo],
        icons: &mut IconCache,
        tex_pass: &TexturePass,
        gpu: &GpuContext,
        icon_size: u32,
    ) {
        let slots = self.build_slots(toplevels);
        for slot in &slots {
            let key = format!("app-{}", slot.app_id);
            if self.icons_loaded.contains_key(&slot.app_id) {
                continue;
            }
            // Try embedded icon first (our Lantern apps)
            let svg_name = format!("{}.svg", slot.app_id);
            let png_name = format!("{}.png", slot.app_id);
            if lntrn_icons::get(&svg_name).is_some() {
                if icons.load_embedded(tex_pass, gpu, &key, &svg_name, icon_size, icon_size).is_some() {
                    self.icons_loaded.insert(slot.app_id.clone(), true);
                }
            } else if lntrn_icons::get(&png_name).is_some() {
                if icons.load_embedded(tex_pass, gpu, &key, &png_name, icon_size, icon_size).is_some() {
                    self.icons_loaded.insert(slot.app_id.clone(), true);
                }
            } else if let Some(path) = self.resolve_icon(&slot.app_id) {
                if icons.load(tex_pass, gpu, &key, &path, icon_size, icon_size).is_some() {
                    self.icons_loaded.insert(slot.app_id.clone(), true);
                }
            }
        }
    }

    /// Measure the total width the app tray would occupy (without drawing).
    pub fn measure_width(&self, toplevels: &[ToplevelInfo], bar_h: f32, scale: f32) -> f32 {
        let slots = self.build_slots(toplevels);
        if slots.is_empty() { return 0.0; }
        let icon_size = (bar_h * 0.75).max(36.0);
        let gap = ICON_GAP * scale;
        let stride = icon_size + gap;
        slots.len() as f32 * stride - gap
    }

    /// Draw the app tray in the bar.
    /// When `left_x` is Some, left-aligns at that x position; otherwise centers.
    /// Returns (total_width, Vec<TextureDraw>).
    pub fn draw<'a>(
        &self,
        painter: &mut Painter,
        _text: &mut TextRenderer,
        ix: &mut InteractionContext,
        icons: &'a IconCache,
        palette: &FoxPalette,
        toplevels: &[ToplevelInfo],
        bar_x: f32,
        bar_y: f32,
        bar_w: f32,
        bar_h: f32,
        scale: f32,
        _screen_w: u32,
        _screen_h: u32,
        left_x: Option<f32>,
    ) -> (f32, Vec<TextureDraw<'a>>) {
        let slots = self.build_slots(toplevels);
        if slots.is_empty() {
            return (0.0, Vec::new());
        }

        let icon_size = (bar_h * 0.75).max(36.0);
        let gap = ICON_GAP * scale;
        let stride = icon_size + gap;
        let total_w = slots.len() as f32 * stride - gap;

        let start_x = match left_x {
            Some(x) => x,
            None => bar_x + (bar_w - total_w) / 2.0,
        };
        let center_y = bar_y + (bar_h - icon_size) / 2.0;

        let mut tex_draws = Vec::new();
        let indicator_h = 3.0 * scale;

        let dragging = &self.drag;

        // Draw non-dragged icons first, then the dragged icon on top
        for pass in 0..2u8 {
            for (i, slot) in slots.iter().enumerate() {
                let is_dragged = dragging.as_ref()
                    .is_some_and(|d| d.active && i == d.src_idx);
                if (pass == 0) == is_dragged { continue; }

                let base_x = if is_dragged {
                    // Dragged icon follows cursor
                    let d = dragging.as_ref().unwrap();
                    start_x + d.src_idx as f32 * stride + (d.cursor_x - d.press_x)
                } else if let Some(d) = dragging.as_ref().filter(|d| d.active) {
                    // Shift this icon to make room for the dragged one
                    let vis = visual_index(i, d.src_idx, d.insert_idx);
                    let target_x = start_x + vis as f32 * stride;
                    let anim_off = self.anim_offsets.get(&slot.app_id).copied().unwrap_or(0.0);
                    target_x + anim_off
                } else {
                    let anim_off = self.anim_offsets.get(&slot.app_id).copied().unwrap_or(0.0);
                    start_x + i as f32 * stride + anim_off
                };

                let x = base_x;
                let rect = Rect::new(x, center_y, icon_size, icon_size);
                let zone_id = ZONE_BASE + i as u32;

                // Only register zones for non-dragged icons (dragged follows cursor)
                if !is_dragged {
                    ix.add_zone(zone_id, rect);
                }
                let hovered = !is_dragged && ix.is_hovered(&rect);

                // Hover highlight (not while dragging another icon)
                if hovered && !self.is_dragging() {
                    let pad = 4.0 * scale;
                    let hover_rect = Rect::new(
                        x - pad, bar_y + 2.0 * scale,
                        icon_size + pad * 2.0, bar_h - 4.0 * scale,
                    );
                    painter.rect_filled(hover_rect, 4.0 * scale, palette.muted.with_alpha(0.35));
                }

                // Draw icon or fallback
                let key = format!("app-{}", slot.app_id);
                if let Some(tex) = icons.get(&key) {
                    let mut draw = TextureDraw::new(tex, x, center_y, icon_size, icon_size);
                    if is_dragged { draw.opacity = 0.8; }
                    tex_draws.push(draw);
                } else {
                    painter.rect_filled(rect, icon_size * 0.2, palette.accent);
                }

                // Active indicator line across top of icon
                if slot.running {
                    let line_color = if slot.activated { palette.accent } else { palette.muted };
                    painter.rect_filled(
                        Rect::new(x, center_y - indicator_h - 2.0 * scale, icon_size, indicator_h),
                        indicator_h / 2.0, line_color,
                    );
                }
            }
        }

        (total_w, tex_draws)
    }

    /// Handle a click on the app tray. Returns the app_id if one was clicked.
    /// Returns None if a drag was active (consumed by on_release instead).
    pub fn handle_click(
        &self,
        ix: &InteractionContext,
        phys_cx: f32,
        phys_cy: f32,
        toplevels: &[ToplevelInfo],
    ) -> Option<String> {
        // Don't fire click if we just finished a drag
        if self.drag.as_ref().is_some_and(|d| d.active) {
            return None;
        }
        let zone = ix.zone_at(phys_cx, phys_cy)?;
        if zone < ZONE_BASE || zone >= ZONE_BASE + 256 {
            return None;
        }
        let idx = (zone - ZONE_BASE) as usize;
        let slots = self.build_slots(toplevels);
        slots.get(idx).map(|s| s.app_id.clone())
    }

    /// Return the hovered app_id and its (x, w) in physical pixels, if any.
    /// Must be called after draw() so zones are registered.
    pub fn hovered_app(
        &self,
        ix: &InteractionContext,
        phys_cx: f32,
        phys_cy: f32,
        toplevels: &[ToplevelInfo],
        bar_x: f32,
        bar_w: f32,
        bar_h: f32,
        scale: f32,
        left_x: Option<f32>,
    ) -> Option<(String, f32, f32)> {
        // Don't show preview while dragging
        if self.is_dragging() { return None; }
        let zone = ix.zone_at(phys_cx, phys_cy)?;
        if zone < ZONE_BASE || zone >= ZONE_BASE + 256 {
            return None;
        }
        let idx = (zone - ZONE_BASE) as usize;
        let slots = self.build_slots(toplevels);
        let slot = slots.get(idx)?;
        // Only show preview for running apps (including minimized)
        if !slot.running { return None; }

        let icon_size = (bar_h * 0.75).max(36.0);
        let gap = ICON_GAP * scale;
        let stride = icon_size + gap;
        let total_w = slots.len() as f32 * stride - gap;
        let start_x = match left_x {
            Some(x) => x,
            None => bar_x + (bar_w - total_w) / 2.0,
        };
        let icon_x = start_x + idx as f32 * stride;
        // Convert from physical to logical for the compositor
        let logical_x = icon_x / scale;
        let logical_w = icon_size / scale;
        Some((slot.app_id.clone(), logical_x, logical_w))
    }

    /// Check if an app_id is pinned.
    pub fn is_pinned(&self, app_id: &str) -> bool {
        self.pinned.iter().any(|p| p.app_id == app_id)
    }

    /// Handle a right-click on the app tray. Returns (app_id, is_pinned, is_running).
    pub fn handle_right_click(
        &self,
        ix: &InteractionContext,
        phys_cx: f32,
        phys_cy: f32,
        toplevels: &[ToplevelInfo],
    ) -> Option<(String, bool, bool)> {
        let zone = ix.zone_at(phys_cx, phys_cy)?;
        if zone < ZONE_BASE || zone >= ZONE_BASE + 256 {
            return None;
        }
        let idx = (zone - ZONE_BASE) as usize;
        let slots = self.build_slots(toplevels);
        slots.get(idx).map(|s| (s.app_id.clone(), s.pinned, s.running))
    }
}

/// Compute the visual position of slot `i` when slot `src` is being dragged to `dst`.
/// Icons between src and dst shift by one to fill the gap.
fn visual_index(i: usize, src: usize, dst: usize) -> usize {
    if i == src {
        return dst;
    }
    if src < dst {
        // Dragging right: icons in (src, dst] shift left by 1
        if i > src && i <= dst { i - 1 } else { i }
    } else if src > dst {
        // Dragging left: icons in [dst, src) shift right by 1
        if i >= dst && i < src { i + 1 } else { i }
    } else {
        i
    }
}

// ── Freedesktop icon lookup ─────────────────────────────────────────────────

/// Search for an icon by app_id in standard locations.
pub(crate) fn find_icon(app_id: &str) -> Option<PathBuf> {
    // 1. Check custom icons dir
    let custom_dir = crate::lantern_icons_dir();
    let custom_names = [
        format!("{}.svg", app_id),
        format!("{}.png", app_id),
    ];
    for name in &custom_names {
        let p = custom_dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Freedesktop icon theme lookup
    let home = std::env::var("HOME").unwrap_or_default();
    let user_tela = format!("{home}/.local/share/icons/Tela/scalable/apps");
    let user_hicolor = format!("{home}/.local/share/icons/hicolor/scalable/apps");
    let search_dirs = [
        // User-local icons first
        user_tela.as_str(),
        user_hicolor.as_str(),
        // System icons
        "/usr/share/icons/Tela/scalable/apps",
        "/usr/share/icons/Tela/128/apps",
        "/usr/share/icons/Tela/64/apps",
        "/usr/share/icons/Tela/48/apps",
        "/usr/share/icons/hicolor/scalable/apps",
        "/usr/share/icons/hicolor/256x256/apps",
        "/usr/share/icons/hicolor/128x128/apps",
        "/usr/share/icons/hicolor/64x64/apps",
        "/usr/share/icons/hicolor/48x48/apps",
        "/usr/share/icons/Adwaita/scalable/apps",
        "/usr/share/icons/breeze/apps/48",
        "/usr/share/pixmaps",
    ];

    // Try exact app_id match, then lowercase
    let candidates = [
        app_id.to_string(),
        app_id.to_lowercase(),
    ];

    for dir in &search_dirs {
        let dir = Path::new(dir);
        if !dir.exists() {
            continue;
        }
        for name in &candidates {
            for ext in &["svg", "svgz", "png"] {
                let p = dir.join(format!("{}.{}", name, ext));
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }

    // 3. Search .desktop files for Icon= field
    find_icon_from_desktop(app_id)
}

/// Try to find icon by reading .desktop files.
fn find_icon_from_desktop(app_id: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let user_apps = format!("{home}/.local/share/applications");
    let desktop_dirs = [
        "/usr/share/applications",
        user_apps.as_str(),
        "/usr/local/share/applications",
    ];

    for dir in &desktop_dirs {
        let desktop_file = Path::new(dir).join(format!("{}.desktop", app_id));
        if let Ok(content) = std::fs::read_to_string(&desktop_file) {
            for line in content.lines() {
                if let Some(icon_name) = line.strip_prefix("Icon=") {
                    let icon_name = icon_name.trim();
                    // If it's an absolute path, use it directly
                    if icon_name.starts_with('/') {
                        let p = PathBuf::from(icon_name);
                        if p.exists() {
                            return Some(p);
                        }
                    }
                    // Otherwise search icon themes for this name (avoid recursion)
                    if icon_name != app_id {
                        return find_icon(icon_name);
                    }
                }
            }
        }
    }

    None
}
