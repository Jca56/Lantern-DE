use std::path::PathBuf;

use lntrn_render::{GpuContext, GpuTexture, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Slider, Toggle};

use crate::config::LanternConfig;

const ZONE_MOUSE_SPEED: u32 = 800;
const ZONE_MOUSE_ACCEL: u32 = 801;
const ZONE_CURSOR_BASE: u32 = 810;

const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const VALUE_SIZE: f32 = 16.0;
const SLIDER_H: f32 = 36.0;
const TOGGLE_H: f32 = 36.0;
const PAD_LEFT: f32 = 24.0;
const PAD_RIGHT: f32 = 32.0;
const LABEL_W: f32 = 200.0;
const VALUE_W: f32 = 60.0;
const CURSOR_ICON_SZ: f32 = 48.0;

/// A cursor SVG/PNG found in ~/.lantern/config/cursors/.
struct CursorEntry {
    /// Filename without extension (e.g. "custom1") — stored in config.
    id: String,
    /// Display name: filename with dashes/underscores replaced by spaces, title-cased.
    display_name: String,
    /// Full path to the SVG/PNG file.
    path: PathBuf,
}

// ── State ──────────────────────────────────────────────────────────────────

pub struct InputPanelState {
    cursors: Vec<CursorEntry>,
    scanned: bool,
    cursor_textures: Vec<Option<GpuTexture>>,
    textures_loaded: bool,
}

impl InputPanelState {
    pub fn new() -> Self {
        Self {
            cursors: Vec::new(), scanned: false,
            cursor_textures: Vec::new(), textures_loaded: false,
        }
    }

    fn scan(&mut self) {
        if self.scanned { return; }
        self.scanned = true;

        let cursor_dir = lntrn_theme::lantern_home()
            .map(|h| h.join("config/cursors"))
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_default();
                std::path::PathBuf::from(home).join(".lantern/config/cursors")
            });

        let Ok(entries) = std::fs::read_dir(&cursor_dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "svg" && ext != "png" { continue; }

            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            let display_name = stem.replace(['-', '_'], " ")
                .split_whitespace()
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(first) => {
                            let upper: String = first.to_uppercase().collect();
                            format!("{}{}", upper, c.as_str())
                        }
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            self.cursors.push(CursorEntry { id: stem, display_name, path });
        }

        self.cursors.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
        self.textures_loaded = false;
    }

    fn load_textures(&mut self, tex_pass: &TexturePass, gpu: &GpuContext, scale: f32) {
        if self.textures_loaded { return; }
        self.textures_loaded = true;
        let sz = (CURSOR_ICON_SZ * scale) as u32;
        self.cursor_textures.clear();
        for cursor in &self.cursors {
            self.cursor_textures.push(load_cursor_texture(tex_pass, gpu, &cursor.path, sz));
        }
    }
}

fn layout(x: f32, w: f32, s: f32) -> (f32, f32, f32, f32) {
    let pad_l = PAD_LEFT * s;
    let pad_r = PAD_RIGHT * s;
    let val_w = VALUE_W * s;
    let label_x = x + pad_l;
    let label_w = LABEL_W * s;
    let ctrl_x = label_x + label_w;
    let ctrl_w = w - pad_l - pad_r - label_w - val_w - 12.0 * s;
    let value_x = ctrl_x + ctrl_w + 8.0 * s;
    (label_x, ctrl_x, ctrl_w.max(80.0 * s), value_x)
}

fn slider_value_from_cursor(
    ix: &InteractionContext, zone_id: u32, rect: &Rect,
) -> Option<f32> {
    let state = ix.zone_state(zone_id);
    if state.is_active() {
        if let Some((cx, _)) = ix.cursor() {
            return Some(((cx - rect.x) / rect.w).clamp(0.0, 1.0));
        }
    }
    None
}

fn load_cursor_texture(
    tex_pass: &TexturePass, gpu: &GpuContext, path: &std::path::Path, sz: u32,
) -> Option<GpuTexture> {
    let ext = path.extension()?.to_str()?;
    if ext == "svg" {
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &Default::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(sz, sz)?;
        let sx = sz as f32 / tree.size().width();
        let sy = sz as f32 / tree.size().height();
        let scale = sx.min(sy);
        let tx = (sz as f32 - tree.size().width() * scale) * 0.5;
        let ty = (sz as f32 - tree.size().height() * scale) * 0.5;
        let xf = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
        resvg::render(&tree, xf, &mut pixmap.as_mut());
        Some(tex_pass.upload(gpu, pixmap.data(), sz, sz))
    } else {
        let img = image::open(path).ok()?
            .resize_exact(sz, sz, image::imageops::FilterType::Triangle)
            .to_rgba8();
        Some(tex_pass.upload(gpu, &img, sz, sz))
    }
}

// ── Input panel ─────────────────────────────────────────────────────────────

pub fn draw_input_panel<'a>(
    config: &mut LanternConfig,
    state: &'a mut InputPanelState,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    tex_pass: &TexturePass, fox: &FoxPalette, gpu: &GpuContext,
    x: f32, y: f32, w: f32, s: f32, sw: u32, sh: u32,
    tex_draws: &mut Vec<TextureDraw<'a>>,
) {
    state.scan();
    state.load_textures(tex_pass, gpu, s);

    let (label_x, ctrl_x, ctrl_w, value_x) = layout(x, w, s);
    let mut cy = y;
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    // ── Section: Mouse ──────────────────────────────────────────────
    text.queue("Mouse", lsz, label_x, cy, fox.text_secondary, LABEL_W * s, sw, sh);
    cy += lsz + 8.0 * s;

    // Mouse Speed slider (-1.0 to 1.0, displayed as percentage)
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Mouse Speed", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

        // Map -1..1 to 0..1 for slider fraction
        let frac = (config.input.mouse_speed + 1.0) / 2.0;
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_MOUSE_SPEED, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_MOUSE_SPEED, &rect) {
            // Map 0..1 back to -1..1, snap to nearest 0.05
            let raw = f * 2.0 - 1.0;
            config.input.mouse_speed = (raw / 0.05).round() * 0.05;
            config.input.mouse_speed = config.input.mouse_speed.clamp(-1.0, 1.0);
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);

        // Display as percentage (-100% to +100%)
        let pct = (config.input.mouse_speed * 100.0).round() as i32;
        let val = if pct == 0 {
            "0%".to_string()
        } else if pct > 0 {
            format!("+{}%", pct)
        } else {
            format!("{}%", pct)
        };
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        cy += row;
    }

    // Mouse Acceleration toggle
    {
        let rect = Rect::new(label_x, cy, w - PAD_LEFT * s - PAD_RIGHT * s, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.input.mouse_acceleration)
            .label("Mouse Acceleration").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_MOUSE_ACCEL, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
    }

    // ── Section separator ───────────────────────────────────────────
    cy += 8.0 * s;
    painter.rect_filled(
        Rect::new(label_x, cy, w - PAD_LEFT * s - PAD_RIGHT * s, 1.0 * s),
        0.0, fox.muted.with_alpha(0.2),
    );
    cy += 16.0 * s;

    // ── Section: Cursor Theme ───────────────────────────────────────
    text.queue("Cursor Theme", lsz, label_x, cy, fox.text_secondary, LABEL_W * s, sw, sh);
    cy += lsz + 12.0 * s;

    let card_size = 100.0 * s;
    let card_gap = 16.0 * s;
    let card_r = 8.0 * s;
    let avail_w = w - PAD_LEFT * s - PAD_RIGHT * s;
    let cols = ((avail_w + card_gap) / (card_size + card_gap)).floor().max(1.0) as usize;

    for (i, cursor) in state.cursors.iter().enumerate() {
        let col = i % cols;
        let row_idx = i / cols;
        let card_x = label_x + col as f32 * (card_size + card_gap);
        let card_y = cy + row_idx as f32 * (card_size + card_gap);
        let card_rect = Rect::new(card_x, card_y, card_size, card_size);

        let zone_id = ZONE_CURSOR_BASE + i as u32;
        let zone = ix.add_zone(zone_id, card_rect);

        let is_selected = config.input.cursor_theme == cursor.id;

        // Card background
        let bg = if is_selected {
            fox.accent.with_alpha(0.15)
        } else if zone.is_hovered() {
            fox.surface_2
        } else {
            fox.surface
        };
        painter.rect_filled(card_rect, card_r, bg);

        // Border
        let border_color = if is_selected {
            fox.accent
        } else {
            fox.muted.with_alpha(0.3)
        };
        let border_w = if is_selected { 2.0 * s } else { 1.0 * s };
        painter.rect_stroke_sdf(card_rect, card_r, border_w, border_color);

        // Draw cursor icon from rasterized SVG/PNG
        let icon_size = CURSOR_ICON_SZ * s;
        let icon_x = card_x + (card_size - icon_size) / 2.0;
        let icon_y = card_y + (card_size - icon_size) / 2.0 - 8.0 * s;
        if let Some(Some(tex)) = state.cursor_textures.get(i) {
            tex_draws.push(TextureDraw::new(tex, icon_x, icon_y, icon_size, icon_size));
        } else {
            let color = if is_selected { fox.accent } else { fox.text };
            draw_cursor_preview(painter, icon_x, icon_y, icon_size, color);
        }

        // Label below the icon — full name when selected, truncated otherwise
        let label_font = 14.0 * s;
        let label_y = card_y + card_size - label_font - 8.0 * s;
        let label_color = if is_selected { fox.accent } else { fox.text };
        let display = if is_selected {
            cursor.display_name.clone()
        } else if cursor.display_name.len() > 12 {
            format!("{}...", &cursor.display_name[..10])
        } else {
            cursor.display_name.clone()
        };
        text.queue(&display, label_font, card_x + 4.0 * s, label_y, label_color,
            card_size - 8.0 * s, sw, sh);
    }
}

/// Draw a simple cursor arrow preview shape.
fn draw_cursor_preview(painter: &mut Painter, x: f32, y: f32, size: f32, color: lntrn_render::Color) {
    let tip_x = x + size * 0.3;
    let tip_y = y;
    let bottom_y = y + size * 0.85;
    let right_x = x + size * 0.65;
    let mid_y = y + size * 0.55;
    let lw = 2.0;

    painter.line(tip_x, tip_y, tip_x, bottom_y, lw, color);
    painter.line(tip_x, bottom_y, tip_x + size * 0.15, mid_y, lw, color);
    painter.line(tip_x + size * 0.15, mid_y, right_x, y + size * 0.85, lw, color);
    painter.line(right_x, y + size * 0.85, right_x - size * 0.1, mid_y + size * 0.05, lw, color);
    painter.line(right_x - size * 0.1, mid_y + size * 0.05, tip_x + size * 0.25, mid_y, lw, color);
    painter.line(tip_x + size * 0.25, mid_y, tip_x, tip_y, lw, color);
}

// ── Click handling ──────────────────────────────────────────────────────────

pub fn handle_input_click(config: &mut LanternConfig, state: &InputPanelState, zone_id: u32) {
    match zone_id {
        ZONE_MOUSE_ACCEL => {
            config.input.mouse_acceleration = !config.input.mouse_acceleration;
        }
        id if id >= ZONE_CURSOR_BASE => {
            let idx = (id - ZONE_CURSOR_BASE) as usize;
            if let Some(cursor) = state.cursors.get(idx) {
                config.input.cursor_theme = cursor.id.clone();
            }
        }
        _ => {}
    }
}
