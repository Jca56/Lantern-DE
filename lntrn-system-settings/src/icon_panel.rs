//! App Icons panel — browse installed apps and set custom icons.

use std::path::{Path, PathBuf};

use lntrn_render::{Color, GpuContext, GpuTexture, Painter, Rect, TextRenderer, TexturePass};
use lntrn_ui::gpu::input::InteractionState;
use lntrn_ui::gpu::scroll::{ScrollArea, Scrollbar};
use lntrn_ui::gpu::text_input::TextInput;
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::text_edit::TextBuffer;

const CUSTOM_ICON_DIR: &str = "/home/alva/.config/lntrn-bar/icons";
const ICON_DIRS: &[&str] = &[
    "/usr/share/icons/Tela/scalable/apps",
    "/usr/share/icons/Tela/128/apps",
    "/usr/share/icons/hicolor/scalable/apps",
    "/usr/share/icons/hicolor/128x128/apps",
    "/usr/share/pixmaps",
];

const CELL_SIZE: f32 = 100.0;
const ICON_SZ: f32 = 48.0;
const LABEL_FONT: f32 = 14.0;
const TITLE_FONT: f32 = 20.0;
const HEADER_H: f32 = 40.0;

const ZONE_ICON_BASE: u32 = 700;
const ZONE_BACK: u32 = 950;
const ZONE_CLEAR: u32 = 951;
const ZONE_PATH_INPUT: u32 = 952;

pub struct IconPanelState {
    apps: Vec<AppEntry>,
    loaded: bool,
    pub scroll_offset: f32,
    // Editing state
    editing_app: Option<usize>,
    path_buffer: TextBuffer,
    path_focused: bool,
    // Icon textures (loaded on demand)
    icon_textures: Vec<Option<GpuTexture>>,
    icons_loaded: bool,
}

struct AppEntry {
    app_id: String,
    name: String,
    icon_name: Option<String>,
    has_custom: bool,
}

impl IconPanelState {
    pub fn new() -> Self {
        Self {
            apps: Vec::new(), loaded: false, scroll_offset: 0.0,
            editing_app: None, path_buffer: TextBuffer::new(""),
            path_focused: false,
            icon_textures: Vec::new(), icons_loaded: false,
        }
    }

    pub fn load(&mut self) {
        if self.loaded { return; }
        self.loaded = true;
        self.apps = scan_desktop_apps();
        self.icons_loaded = false;
    }

    pub fn load_icons(&mut self, tex_pass: &TexturePass, gpu: &GpuContext, scale: f32) {
        if self.icons_loaded { return; }
        self.icons_loaded = true;
        let sz = (ICON_SZ * scale) as u32;
        self.icon_textures.clear();
        for app in &self.apps {
            let icon_name = app.icon_name.as_deref().unwrap_or(&app.app_id);
            let tex = find_icon_path(icon_name, &app.app_id)
                .and_then(|path| load_icon_texture(tex_pass, gpu, &path, sz));
            self.icon_textures.push(tex);
        }
    }

    pub fn handle_key(&mut self, sym: xkbcommon::xkb::Keysym, utf8: Option<String>) -> bool {
        if !self.path_focused { return false; }
        match sym.raw() {
            0xff0d | 0xff8d => { // Enter — apply icon
                self.apply_custom_icon();
                self.path_focused = false;
                true
            }
            0xff1b => { self.path_focused = false; true } // Escape
            0xff08 => { self.path_buffer.backspace(); true }
            0xffff => { self.path_buffer.delete(); true }
            0xff51 => { self.path_buffer.left(); true }
            0xff53 => { self.path_buffer.right(); true }
            0xff50 => { self.path_buffer.home(); true }
            0xff57 => { self.path_buffer.end(); true }
            _ => {
                if let Some(ch) = utf8 { self.path_buffer.insert(&ch); true }
                else { false }
            }
        }
    }

    fn apply_custom_icon(&mut self) {
        let Some(idx) = self.editing_app else { return };
        let Some(app) = self.apps.get_mut(idx) else { return };
        let src = Path::new(&self.path_buffer.text);
        if !src.exists() { return; }

        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png");
        let dst = Path::new(CUSTOM_ICON_DIR).join(format!("{}.{ext}", app.app_id));
        let _ = std::fs::create_dir_all(CUSTOM_ICON_DIR);
        if std::fs::copy(src, &dst).is_ok() {
            app.has_custom = true;
            self.icons_loaded = false; // reload icons
        }
    }

    fn clear_custom_icon(&mut self, idx: usize) {
        let Some(app) = self.apps.get_mut(idx) else { return };
        let dir = Path::new(CUSTOM_ICON_DIR);
        for ext in &["svg", "png"] {
            let p = dir.join(format!("{}.{ext}", app.app_id));
            let _ = std::fs::remove_file(&p);
        }
        app.has_custom = false;
        self.icons_loaded = false;
    }

    pub fn on_click(&mut self, zone_id: u32) {
        match zone_id {
            ZONE_BACK => {
                self.editing_app = None;
                self.path_buffer.set("");
                self.path_focused = false;
                self.scroll_offset = 0.0;
            }
            ZONE_CLEAR => {
                if let Some(idx) = self.editing_app {
                    self.clear_custom_icon(idx);
                }
            }
            ZONE_PATH_INPUT => {
                self.path_focused = true;
            }
            z if z >= ZONE_ICON_BASE && z < ZONE_ICON_BASE + 200 => {
                let idx = (z - ZONE_ICON_BASE) as usize;
                if idx < self.apps.len() {
                    self.editing_app = Some(idx);
                    self.path_buffer.set("");
                    self.path_focused = false;
                    self.scroll_offset = 0.0;
                }
            }
            _ => {}
        }
    }

    pub fn wants_keyboard(&self) -> bool { self.path_focused }
}

// ── Drawing ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_icon_panel<'a>(
    state: &'a mut IconPanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    tex_pass: &TexturePass,
    fox: &FoxPalette,
    gpu: &GpuContext,
    x: f32, y: f32, w: f32, h: f32,
    s: f32, sw: u32, sh: u32,
    scroll_delta: f32,
    tex_draws: &mut Vec<lntrn_render::TextureDraw<'a>>,
) {
    state.load();
    state.load_icons(tex_pass, gpu, s);

    if state.scroll_offset != 0.0 || scroll_delta != 0.0 {
        // handled externally
    }
    state.scroll_offset = (state.scroll_offset + scroll_delta).max(0.0);

    if state.editing_app.is_some() {
        draw_editor(state, painter, text, ix, fox, x, y, w, h, s, sw, sh, tex_draws);
    } else {
        draw_grid(state, painter, text, ix, fox, x, y, w, h, s, sw, sh, tex_draws);
    }
}

fn draw_grid<'a>(
    state: &'a mut IconPanelState,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32, y: f32, w: f32, h: f32, s: f32, sw: u32, _sh: u32,
    tex_draws: &mut Vec<lntrn_render::TextureDraw<'a>>,
) {
    let pad = 16.0 * s;
    let cell = CELL_SIZE * s;
    let icon_sz = ICON_SZ * s;
    let lf = LABEL_FONT * s;

    let grid_x = x + pad;
    let grid_w = w - pad * 2.0;
    let cols = (grid_w / cell).floor().max(1.0) as usize;
    let rows = (state.apps.len() + cols - 1) / cols.max(1);
    let content_h = rows as f32 * cell;

    let grid_rect = Rect::new(grid_x, y, grid_w, h);
    let scroll = ScrollArea::new(grid_rect, content_h, &mut state.scroll_offset);
    scroll.begin(painter);
    let clip = [grid_rect.x, grid_rect.y, grid_rect.w, grid_rect.h];

    for (i, app) in state.apps.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let cx = grid_x + col as f32 * cell;
        let cy = scroll.content_y() + row as f32 * cell;
        if cy + cell < y || cy > y + h { continue; }

        let cell_rect = Rect::new(cx, cy, cell, cell);
        let zone_id = ZONE_ICON_BASE + i as u32;
        let zone_state = ix.add_zone(zone_id, cell_rect);

        if zone_state.is_hovered() {
            painter.rect_filled(cell_rect, 6.0 * s, fox.surface_2);
        }

        // Custom icon badge
        if app.has_custom {
            let badge_r = 4.0 * s;
            painter.circle_filled(cx + cell - 8.0 * s, cy + 8.0 * s, badge_r, fox.accent);
        }

        // Icon
        let icon_x = cx + (cell - icon_sz) * 0.5;
        let icon_y = cy + 8.0 * s;
        if let Some(Some(tex)) = state.icon_textures.get(i) {
            tex_draws.push(lntrn_render::TextureDraw {
                texture: tex, x: icon_x, y: icon_y, w: icon_sz, h: icon_sz,
                opacity: 1.0, uv: [0.0, 0.0, 1.0, 1.0], clip: Some(clip),
            });
        } else {
            // Fallback circle with initial
            let initial = app.name.chars().next().unwrap_or('?');
            let hue = app.app_id.bytes().fold(0u32, |a, b| a.wrapping_add(b as u32));
            let bg = fallback_color(hue);
            painter.circle_filled(icon_x + icon_sz * 0.5, icon_y + icon_sz * 0.5, icon_sz * 0.4, bg);
            let init_str = initial.to_uppercase().to_string();
            let init_f = icon_sz * 0.4;
            text.queue_clipped(&init_str, init_f,
                icon_x + icon_sz * 0.5 - init_f * 0.26, icon_y + icon_sz * 0.5 - init_f * 0.5,
                Color::WHITE, init_f, clip);
        }

        // Label
        let label = if app.name.len() > 12 { format!("{}...", &app.name[..10]) } else { app.name.clone() };
        let lw = text.measure_width(&label, lf).min(cell - 4.0 * s);
        text.queue_clipped(&label, lf, cx + (cell - lw) * 0.5, icon_y + icon_sz + 6.0 * s,
            fox.text_secondary, cell - 4.0 * s, clip);
    }

    scroll.end(painter);
    if scroll.is_scrollable() {
        let sb = Scrollbar::new(&grid_rect, content_h, state.scroll_offset);
        sb.draw(painter, InteractionState::Idle, fox);
    }
}

fn draw_editor<'a>(
    state: &'a mut IconPanelState,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32, y: f32, w: f32, _h: f32, s: f32, sw: u32, sh: u32,
    _tex_draws: &mut Vec<lntrn_render::TextureDraw<'a>>,
) {
    let pad = 16.0 * s;
    let tf = TITLE_FONT * s;
    let nf = 18.0 * s;
    let header_h = HEADER_H * s;
    let Some(idx) = state.editing_app else { return };
    let Some(app) = state.apps.get(idx) else { return };

    let gx = x + pad;
    let gw = w - pad * 2.0;

    // Back button
    let back_label = "< Back";
    let back_w = text.measure_width(back_label, tf) + pad;
    let back_rect = Rect::new(gx, y, back_w, header_h);
    let back_s = ix.add_zone(ZONE_BACK, back_rect);
    if back_s.is_hovered() { painter.rect_filled(back_rect, 0.0, fox.surface_2); }
    text.queue(back_label, tf, gx + 4.0 * s, y + (header_h - tf) * 0.5,
        if back_s.is_hovered() { fox.text } else { fox.text_secondary }, back_w, sw, sh);

    // App name
    let name_w = text.measure_width(&app.name, tf);
    text.queue(&app.name, tf, gx + (gw - name_w) * 0.5, y + (header_h - tf) * 0.5, fox.text, name_w + 4.0, sw, sh);

    let mut cy = y + header_h + pad;

    // Current status
    let status = if app.has_custom { "Custom icon set" } else { "Using default icon" };
    let status_color = if app.has_custom { fox.accent } else { fox.text_secondary };
    text.queue(status, nf, gx, cy, status_color, gw, sw, sh);
    cy += nf + pad;

    // Path input label
    text.queue("Icon path (SVG or PNG):", nf, gx, cy, fox.text_secondary, gw, sw, sh);
    cy += nf + 8.0 * s;

    // Path text input
    let input_rect = Rect::new(gx, cy, gw, 40.0 * s);
    let input_zone = ix.add_zone(ZONE_PATH_INPUT, input_rect);
    let mut ti = TextInput::new(input_rect)
        .text(&state.path_buffer.text)
        .placeholder("/path/to/icon.svg")
        .focused(state.path_focused)
        .hovered(input_zone.is_hovered());
    if state.path_focused {
        ti = ti.cursor_pos(state.path_buffer.cursor);
    }
    ti.scale(s).draw(painter, text, fox, sw, sh);
    cy += 40.0 * s + pad;

    // Instructions
    text.queue("Press Enter to apply. The icon will be copied to:", 16.0 * s, gx, cy, fox.muted, gw, sw, sh);
    cy += 18.0 * s;
    let dest = format!("{}/{}.svg/png", CUSTOM_ICON_DIR, app.app_id);
    text.queue(&dest, 14.0 * s, gx, cy, fox.muted, gw, sw, sh);
    cy += 20.0 * s + pad;

    // Clear custom icon button (only if has custom)
    if app.has_custom {
        let clear_label = "Remove Custom Icon";
        let clear_w = text.measure_width(clear_label, nf) + pad * 2.0;
        let clear_rect = Rect::new(gx, cy, clear_w, 36.0 * s);
        let clear_s = ix.add_zone(ZONE_CLEAR, clear_rect);
        if clear_s.is_hovered() {
            painter.rect_filled(clear_rect, 0.0, Color::from_rgb8(239, 68, 68).with_alpha(0.15));
        }
        painter.rect_stroke(clear_rect, 0.0, 1.0 * s, Color::from_rgb8(239, 68, 68).with_alpha(0.5));
        text.queue(clear_label, nf, gx + pad, cy + (36.0 * s - nf) * 0.5,
            if clear_s.is_hovered() { fox.danger } else { Color::from_rgb8(239, 68, 68).with_alpha(0.6) },
            clear_w, sw, sh);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn scan_desktop_apps() -> Vec<AppEntry> {
    let dirs = [
        "/usr/share/applications",
        &format!("{}/.local/share/applications", std::env::var("HOME").unwrap_or_default()),
    ];
    let custom_dir = Path::new(CUSTOM_ICON_DIR);
    let mut seen = std::collections::HashSet::new();
    let mut apps = Vec::new();

    for dir in &dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "desktop") { continue; }
            let Ok(content) = std::fs::read_to_string(&path) else { continue; };
            let mut name = String::new();
            let mut icon = None;
            let mut no_display = false;

            for line in content.lines() {
                if line.starts_with("Name=") && name.is_empty() {
                    name = line[5..].to_string();
                } else if line.starts_with("Icon=") {
                    icon = Some(line[5..].to_string());
                } else if line == "NoDisplay=true" {
                    no_display = true;
                }
            }
            if name.is_empty() || no_display { continue; }

            let app_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if !seen.insert(app_id.clone()) { continue; }

            let has_custom = custom_dir.join(format!("{app_id}.svg")).exists()
                || custom_dir.join(format!("{app_id}.png")).exists();

            apps.push(AppEntry { app_id, name, icon_name: icon, has_custom });
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn find_icon_path(icon_name: &str, app_id: &str) -> Option<PathBuf> {
    // Custom first
    let custom = Path::new(CUSTOM_ICON_DIR);
    for ext in &["svg", "png"] {
        let p = custom.join(format!("{app_id}.{ext}"));
        if p.exists() { return Some(p); }
    }
    // Freedesktop search
    for dir in ICON_DIRS {
        for ext in &["svg", "png"] {
            let p = Path::new(dir).join(format!("{icon_name}.{ext}"));
            if p.exists() { return Some(p); }
        }
    }
    None
}

fn load_icon_texture(tex_pass: &TexturePass, gpu: &GpuContext, path: &Path, sz: u32) -> Option<GpuTexture> {
    let ext = path.extension()?.to_str()?;
    if ext == "svg" {
        // Use resvg to rasterize SVG
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &Default::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(sz, sz)?;
        let sx = sz as f32 / tree.size().width();
        let sy = sz as f32 / tree.size().height();
        let scale = sx.min(sy);
        let tx = (sz as f32 - tree.size().width() * scale) * 0.5;
        let ty = (sz as f32 - tree.size().height() * scale) * 0.5;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty), &mut pixmap.as_mut());
        Some(tex_pass.upload(gpu, pixmap.data(), sz, sz))
    } else {
        let img = image::open(path).ok()?.resize_exact(sz, sz, image::imageops::FilterType::Triangle).to_rgba8();
        Some(tex_pass.upload(gpu, &img, sz, sz))
    }
}

fn fallback_color(hash: u32) -> Color {
    let colors = [
        Color::from_rgb8(200, 134, 10), Color::from_rgb8(59, 130, 246),
        Color::from_rgb8(34, 197, 94), Color::from_rgb8(239, 68, 68),
        Color::from_rgb8(168, 85, 247), Color::from_rgb8(236, 72, 153),
    ];
    colors[(hash as usize) % colors.len()]
}
