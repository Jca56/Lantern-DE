use std::path::PathBuf;

use lntrn_render::{GpuContext, GpuTexture, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, Slider, Toggle};

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, slider_value_from_cursor,
    CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
};

const ZONE_MOUSE_SPEED: u32 = 800;
const ZONE_POINTER_ACCEL: u32 = 801;
const ZONE_SCROLL_SPEED: u32 = 802;
const ZONE_SINGLE_CLICK: u32 = 803;
const ZONE_CURSOR_SIZE: u32 = 804;
const ZONE_CURSOR_BASE: u32 = 810;

const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const VALUE_SIZE: f32 = 16.0;
const SLIDER_H: f32 = 36.0;
const SLIDER_W: f32 = 320.0;
const TOGGLE_H: f32 = 36.0;
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
    pub scroll_offset: f32,
}

impl InputPanelState {
    pub fn new() -> Self {
        Self {
            cursors: Vec::new(), scanned: false,
            cursor_textures: Vec::new(), textures_loaded: false,
            scroll_offset: 0.0,
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
    x: f32, y: f32, w: f32, panel_h: f32, s: f32, sw: u32, sh: u32,
    scroll_delta: f32,
    tex_draws: &mut Vec<TextureDraw<'a>>,
) {
    state.scan();
    state.load_textures(tex_pass, gpu, s);

    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    // Card geometry — match the WM panel layout.
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    // Inner control layout — labels, fixed-width slider, value column inside the card
    let label_w = LABEL_W * s;
    let value_w = VALUE_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;
    let avail = (card_inner_w - label_w - value_w - 12.0 * s).max(80.0 * s);
    let ctrl_w = (SLIDER_W * s).min(avail);
    let value_x = ctrl_x + ctrl_w + 8.0 * s;

    // ── Card sizing ─────────────────────────────────────────────────
    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    // Pointer card: Speed slider + Pointer Acceleration toggle.
    let pointer_card_h = card_chrome_h + 2.0 * row;

    // Scrolling card: just Scroll Speed for now.
    let scrolling_card_h = card_chrome_h + 1.0 * row;

    // Clicking card: Single-click activate toggle.
    let clicking_card_h = card_chrome_h + 1.0 * row;

    // Cursor Theme card: Cursor Size slider + cursor grid.
    let cursor_card_size = 100.0 * s;
    let cursor_card_gap = 16.0 * s;
    let cursor_cols = ((card_inner_w + cursor_card_gap)
        / (cursor_card_size + cursor_card_gap))
        .floor().max(1.0) as usize;
    let cursor_grid_rows = if state.cursors.is_empty() {
        1
    } else {
        (state.cursors.len() + cursor_cols - 1) / cursor_cols
    };
    let cursor_grid_h = cursor_grid_rows as f32 * (cursor_card_size + cursor_card_gap)
        - cursor_card_gap; // last row has no trailing gap
    let cursor_card_h = card_chrome_h + row + 8.0 * s
        + cursor_grid_h.max(cursor_card_size);

    let content_height = CARD_OUTER_PAD_V * s
        + pointer_card_h + CARD_GAP * s
        + scrolling_card_h + CARD_GAP * s
        + clicking_card_h + CARD_GAP * s
        + cursor_card_h + CARD_OUTER_PAD_V * 2.0 * s;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(
            &mut state.scroll_offset, scroll_delta * 40.0,
            content_height, panel_h,
        );
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut state.scroll_offset);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 1: Pointer
    // ─────────────────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Pointer",
            card_x, cy_top, card_w, pointer_card_h, s, sw, sh,
        );

        // Speed slider (-1.0 to 1.0, displayed as percentage)
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Speed", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

            let frac = (config.input.mouse_speed + 1.0) / 2.0;
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_MOUSE_SPEED, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_MOUSE_SPEED, &rect) {
                let raw = f * 2.0 - 1.0;
                config.input.mouse_speed = (raw / 0.05).round() * 0.05;
                config.input.mouse_speed = config.input.mouse_speed.clamp(-1.0, 1.0);
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);

            let pct = (config.input.mouse_speed * 100.0).round() as i32;
            let val = if pct == 0 {
                "0%".to_string()
            } else if pct > 0 {
                format!("+{}%", pct)
            } else {
                format!("{}%", pct)
            };
            text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            cy += row;
        }

        // Pointer Acceleration toggle (true = adaptive, false = flat)
        {
            let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
            let toggle = Toggle::new(rect, config.input.pointer_acceleration)
                .label("Pointer Acceleration").scale(s);
            let track = toggle.track_rect();
            let zone = ix.add_zone(ZONE_POINTER_ACCEL, track);
            toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        }
    }

    cy_top += pointer_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 2: Scrolling
    // ─────────────────────────────────────────────────────────────────
    {
        let cy = draw_section_card(
            painter, text, fox, "Scrolling",
            card_x, cy_top, card_w, scrolling_card_h, s, sw, sh,
        );

        // Scroll Speed slider (0.25x to 3.0x)
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Speed", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

        let frac = ((config.input.scroll_speed - 0.25) / 2.75).clamp(0.0, 1.0);
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_SCROLL_SPEED, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_SCROLL_SPEED, &rect) {
            let raw = 0.25 + f * 2.75;
            // Snap to nearest 0.05x
            config.input.scroll_speed = (raw / 0.05).round() * 0.05;
            config.input.scroll_speed = config.input.scroll_speed.clamp(0.25, 3.0);
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.2}x", config.input.scroll_speed);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
    }

    cy_top += scrolling_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 3: Clicking
    // ─────────────────────────────────────────────────────────────────
    {
        let cy = draw_section_card(
            painter, text, fox, "Clicking",
            card_x, cy_top, card_w, clicking_card_h, s, sw, sh,
        );

        // Single-click activate toggle (true = single click, false = double click)
        let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.input.single_click_activate)
            .label("Single-click to activate").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_SINGLE_CLICK, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
    }

    cy_top += clicking_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 4: Cursor Theme (with size slider above the grid)
    // ─────────────────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Cursor Theme",
            card_x, cy_top, card_w, cursor_card_h, s, sw, sh,
        );

        // Cursor Size slider (16 – 64 px)
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Size", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
            let frac = ((config.input.cursor_size as f32 - 16.0) / 48.0).clamp(0.0, 1.0);
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_CURSOR_SIZE, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_CURSOR_SIZE, &rect) {
                config.input.cursor_size = (16.0 + f * 48.0).round() as u32;
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            let val = format!("{}px", config.input.cursor_size);
            text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            cy += row + 8.0 * s;
        }

        let grid_origin_y = cy;

        let card_r = 8.0 * s;
        for (i, cursor) in state.cursors.iter().enumerate() {
            let col = i % cursor_cols;
            let row_idx = i / cursor_cols;
            let cx_card = card_inner_x + col as f32 * (cursor_card_size + cursor_card_gap);
            let cy_card = grid_origin_y + row_idx as f32 * (cursor_card_size + cursor_card_gap);
            let card_rect = Rect::new(cx_card, cy_card, cursor_card_size, cursor_card_size);

            let zone_id = ZONE_CURSOR_BASE + i as u32;
            let zone = ix.add_zone(zone_id, card_rect);

            let is_selected = config.input.cursor_theme == cursor.id;

            // Card background
            let bg = if is_selected {
                fox.accent.with_alpha(0.18)
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

            // Cursor icon
            let icon_size = CURSOR_ICON_SZ * s;
            let icon_x = cx_card + (cursor_card_size - icon_size) / 2.0;
            let icon_y = cy_card + (cursor_card_size - icon_size) / 2.0 - 8.0 * s;
            if let Some(Some(tex)) = state.cursor_textures.get(i) {
                tex_draws.push(TextureDraw::new(tex, icon_x, icon_y, icon_size, icon_size));
            } else {
                let color = if is_selected { fox.accent } else { fox.text };
                draw_cursor_preview(painter, icon_x, icon_y, icon_size, color);
            }

            // Label
            let label_font = 14.0 * s;
            let label_y = cy_card + cursor_card_size - label_font - 8.0 * s;
            let label_color = if is_selected { fox.accent } else { fox.text };
            let display = if is_selected {
                cursor.display_name.clone()
            } else if cursor.display_name.len() > 12 {
                format!("{}...", &cursor.display_name[..10])
            } else {
                cursor.display_name.clone()
            };
            text.queue(&display, label_font, cx_card + 4.0 * s, label_y, label_color,
                cursor_card_size - 8.0 * s, sw, sh);
        }
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, state.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
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
        ZONE_POINTER_ACCEL => {
            config.input.pointer_acceleration = !config.input.pointer_acceleration;
        }
        ZONE_SINGLE_CLICK => {
            config.input.single_click_activate = !config.input.single_click_activate;
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
