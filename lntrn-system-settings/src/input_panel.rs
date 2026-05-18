use std::path::PathBuf;

use lntrn_render::{GpuContext, GpuTexture, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, Slider, Toggle};

use crate::config::LanternConfig;
use crate::panels::{
    draw_color_swatch_row, draw_section_card,
    slider_value_from_cursor, GLOW_COLORS,
    CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
};

const ZONE_MOUSE_SPEED: u32 = 800;
const ZONE_POINTER_ACCEL: u32 = 801;
const ZONE_SCROLL_SPEED: u32 = 802;
const ZONE_DOUBLE_CLICK: u32 = 803;
const ZONE_CURSOR_SIZE: u32 = 804;
const ZONE_CURSOR_OUTLINE_WIDTH: u32 = 805;
const ZONE_CURSOR_CORNER_RADIUS: u32 = 806;
const ZONE_CURSOR_BASE: u32 = 810;
// IDs 900–901 are owned by the global Save / Cancel chrome buttons, so the
// outline swatch row has to start well clear of them. Fill range stays at
// 880..890; outline jumps to 940..950; preview tile picks up at 960.
const ZONE_CURSOR_FILL_BASE:    u32 = 880; // +0..GLOW_COLORS.len()
const ZONE_CURSOR_OUTLINE_BASE: u32 = 940; // +0..GLOW_COLORS.len()
const ZONE_CURSOR_DEFAULT_TILE: u32 = 960;
const DEFAULT_PREVIEW_PX: f32 = 88.0;

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
    /// Live preview of the bundled-default cursor with the current
    /// fill/outline/border/roundness applied. Rebuilt whenever any of the
    /// inputs change.
    default_preview_tex: Option<GpuTexture>,
    default_preview_fill: String,
    default_preview_outline: String,
    default_preview_width: f32,
    default_preview_radius: f32,
    default_preview_px: u32,
}

impl InputPanelState {
    pub fn new() -> Self {
        Self {
            cursors: Vec::new(), scanned: false,
            cursor_textures: Vec::new(), textures_loaded: false,
            scroll_offset: 0.0,
            default_preview_tex: None,
            default_preview_fill: String::new(),
            default_preview_outline: String::new(),
            default_preview_width: -1.0,
            default_preview_radius: -1.0,
            default_preview_px: 0,
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

    /// (Re)build the recolored default-cursor preview texture when any of
    /// fill / outline / width / corner radius / scale change. Idempotent
    /// otherwise.
    fn ensure_default_preview(
        &mut self,
        tex_pass: &TexturePass,
        gpu: &GpuContext,
        fill: &str,
        outline: &str,
        outline_width: f32,
        corner_radius: f32,
        scale: f32,
    ) {
        let target_px = (DEFAULT_PREVIEW_PX * scale).round() as u32;
        let stale = self.default_preview_tex.is_none()
            || self.default_preview_fill != fill
            || self.default_preview_outline != outline
            || (self.default_preview_width - outline_width).abs() > 0.001
            || (self.default_preview_radius - corner_radius).abs() > 0.001
            || self.default_preview_px != target_px;
        if !stale {
            return;
        }
        // Match the compositor: prefer ~/.lantern/icons/cursors/<file>.svg
        // when present so the preview tracks the cursor the user actually
        // sees on screen. Falls back to the bundled bytes otherwise.
        let runtime_path = lntrn_theme::lantern_home()
            .map(|h| h.join("icons/cursors/lntrn-cursor.svg"))
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_default();
                std::path::PathBuf::from(home).join(".lantern/icons/cursors/lntrn-cursor.svg")
            });
        let runtime = std::fs::read(&runtime_path).ok();
        let src: &[u8] = match runtime.as_deref() {
            Some(bytes) => bytes,
            None => {
                let Some(bundled) = lntrn_icons::get("lntrn-cursor.svg") else { return };
                bundled
            }
        };
        let customized = customize_cursor_svg(src, fill, outline, outline_width, corner_radius);
        self.default_preview_tex = rasterize_svg_to_texture(&customized, target_px, tex_pass, gpu);
        self.default_preview_fill = fill.to_string();
        self.default_preview_outline = outline.to_string();
        self.default_preview_width = outline_width;
        self.default_preview_radius = corner_radius;
        self.default_preview_px = target_px;
    }
}

/// Match the compositor's customize rules so the panel preview tracks
/// reality. Recolors fill + outline (attribute *or* CSS form), rewrites
/// `stroke-width` (attribute *or* CSS form), and parses the first polygon
/// path to apply corner rounding.
fn customize_cursor_svg(svg: &[u8], fill: &str, outline: &str, width: f32, radius: f32) -> Vec<u8> {
    let Ok(s) = std::str::from_utf8(svg) else { return svg.to_vec() };
    let mut out = s.to_string();
    out = out.replace("#0a0a0a", fill).replace("#0A0A0A", fill);
    out = out.replace("#ffffff", outline).replace("#FFFFFF", outline);
    let h = outline.trim_start_matches('#');
    if h.len() == 6 {
        let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(255);
        let rgb = format!("rgb({}, {}, {})", r, g, b);
        out = out.replace("rgb(255, 255, 255)", &rgb)
                 .replace("rgb(255,255,255)", &rgb);
    }
    out = replace_stroke_width(&out, width.max(0.0));
    if radius > 0.001 {
        out = round_first_polygon_path(&out, radius);
    }
    out.into_bytes()
}

fn replace_stroke_width(svg: &str, new_width: f32) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    let attr = "stroke-width=\"";
    while let Some(idx) = rest.find(attr) {
        out.push_str(&rest[..idx]);
        out.push_str(attr);
        let after = &rest[idx + attr.len()..];
        match after.find('"') {
            Some(end) => {
                out.push_str(&format!("{:.2}", new_width));
                rest = &after[end..];
            }
            None => { out.push_str(after); return out; }
        }
    }
    out.push_str(rest);

    let mut final_out = String::with_capacity(out.len());
    let mut rest = out.as_str();
    let css = "stroke-width:";
    while let Some(idx) = rest.find(css) {
        final_out.push_str(&rest[..idx]);
        final_out.push_str(css);
        let after = &rest[idx + css.len()..];
        let trimmed = after.trim_start_matches(' ');
        let leading_space = after.len() - trimmed.len();
        let term = trimmed.find(|c: char| c == ';' || c == '"' || c == '}' || c == '\n');
        let term_at = term.unwrap_or(trimmed.len());
        let value = &trimmed[..term_at];
        let unit_start = value
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
            .unwrap_or(value.len());
        let unit = &value[unit_start..];
        for _ in 0..leading_space { final_out.push(' '); }
        final_out.push_str(&format!("{:.2}{}", new_width, unit));
        rest = &trimmed[term_at..];
    }
    final_out.push_str(rest);
    final_out
}

fn round_first_polygon_path(svg: &str, r: f32) -> String {
    let attr = "d=\"";
    let Some(start) = svg.find(attr) else { return svg.to_string() };
    let body_start = start + attr.len();
    let Some(end_off) = svg[body_start..].find('"') else { return svg.to_string() };
    let d = &svg[body_start..body_start + end_off];
    let Some(pts) = parse_polygon_d(d) else { return svg.to_string() };
    let new_d = round_polygon_path(&pts, r);
    let mut out = String::with_capacity(svg.len() + 64);
    out.push_str(&svg[..body_start]);
    out.push_str(&new_d);
    out.push_str(&svg[body_start + end_off..]);
    out
}

fn parse_polygon_d(d: &str) -> Option<Vec<(f32, f32)>> {
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut tokens = d.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty());
    while let Some(tok) = tokens.next() {
        match tok {
            "M" | "L" | "m" | "l" => {
                let x: f32 = tokens.next()?.parse().ok()?;
                let y: f32 = tokens.next()?.parse().ok()?;
                pts.push((x, y));
            }
            "Z" | "z" => break,
            _ => return None,
        }
    }
    if pts.len() > 1 {
        let last = pts[pts.len() - 1];
        let first = pts[0];
        if (last.0 - first.0).abs() < 0.001 && (last.1 - first.1).abs() < 0.001 {
            pts.pop();
        }
    }
    if pts.len() < 3 { return None; }
    Some(pts)
}

fn round_polygon_path(pts: &[(f32, f32)], r: f32) -> String {
    let r = r.clamp(0.0, 1.0);
    if r < 0.001 {
        let mut s = String::new();
        for (i, p) in pts.iter().enumerate() {
            let cmd = if i == 0 { "M" } else { "L" };
            s.push_str(&format!("{} {:.3} {:.3} ", cmd, p.0, p.1));
        }
        s.push('Z');
        return s;
    }
    let n = pts.len();
    let mut s = String::new();
    for i in 0..n {
        let prev = pts[(i + n - 1) % n];
        let cur = pts[i];
        let next = pts[(i + 1) % n];
        let (dx1, dy1) = (prev.0 - cur.0, prev.1 - cur.1);
        let l1 = (dx1 * dx1 + dy1 * dy1).sqrt().max(0.001);
        let (dx2, dy2) = (next.0 - cur.0, next.1 - cur.1);
        let l2 = (dx2 * dx2 + dy2 * dy2).sqrt().max(0.001);
        let r_eff = r * 0.45 * l1.min(l2);
        let p_in = (cur.0 + dx1 / l1 * r_eff, cur.1 + dy1 / l1 * r_eff);
        let p_out = (cur.0 + dx2 / l2 * r_eff, cur.1 + dy2 / l2 * r_eff);
        let cmd = if i == 0 { "M" } else { "L" };
        s.push_str(&format!("{} {:.3} {:.3} ", cmd, p_in.0, p_in.1));
        s.push_str(&format!("Q {:.3} {:.3} {:.3} {:.3} ", cur.0, cur.1, p_out.0, p_out.1));
    }
    s.push('Z');
    s
}

fn rasterize_svg_to_texture(
    data: &[u8], sz: u32, tex_pass: &TexturePass, gpu: &GpuContext,
) -> Option<GpuTexture> {
    let tree = resvg::usvg::Tree::from_data(data, &Default::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(sz, sz)?;
    let sx = sz as f32 / tree.size().width();
    let sy = sz as f32 / tree.size().height();
    let scale = sx.min(sy);
    let tx = (sz as f32 - tree.size().width() * scale) * 0.5;
    let ty = (sz as f32 - tree.size().height() * scale) * 0.5;
    let xf = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    resvg::render(&tree, xf, &mut pixmap.as_mut());
    Some(tex_pass.upload(gpu, pixmap.data(), sz, sz))
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
    state.ensure_default_preview(
        tex_pass, gpu,
        &config.input.cursor_fill,
        &config.input.cursor_outline,
        config.input.cursor_outline_width,
        config.input.cursor_corner_radius,
        s,
    );

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
    // Cursor card rows: Size + Border Width + Corner Roundness sliders,
    // Fill swatch, Outline swatch, then theme grid below.
    let cursor_card_h = card_chrome_h + row * 5.0 + 8.0 * s
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

        // Double-click toggle (true = double-click required, false = single-click)
        let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.input.double_click_to_open)
            .label("Double-click to open").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_DOUBLE_CLICK, track);
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

        // Snapshot where the full 5-row control block begins so the preview
        // tile can be vertically centered across it.
        let control_block_top = cy;

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
            cy += row;
        }

        // Border Width slider (0 – 8 SVG px). Drives `stroke-width` in the
        // recolored default-cursor SVG.
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Border Width", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
            let frac = (config.input.cursor_outline_width / 8.0).clamp(0.0, 1.0);
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_CURSOR_OUTLINE_WIDTH, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_CURSOR_OUTLINE_WIDTH, &rect) {
                let raw = f * 8.0;
                config.input.cursor_outline_width = (raw * 4.0).round() / 4.0; // snap 0.25
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            let val = format!("{:.2}", config.input.cursor_outline_width);
            text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            cy += row;
        }

        // Corner Roundness slider (0 – 100%). Reshapes the default pointer
        // path with smooth bezier corners.
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Roundness", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
            let frac = config.input.cursor_corner_radius.clamp(0.0, 1.0);
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_CURSOR_CORNER_RADIUS, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_CURSOR_CORNER_RADIUS, &rect) {
                config.input.cursor_corner_radius = (f * 20.0).round() / 20.0; // snap 5%
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            let val = format!("{}%", (config.input.cursor_corner_radius * 100.0).round() as i32);
            text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            cy += row;
        }

        // Fill + Outline swatch rows — same GLOW_COLORS palette for both so
        // the picker reads as one consistent set of accent options.
        draw_color_swatch_row(
            painter, text, ix, fox,
            "Fill", ZONE_CURSOR_FILL_BASE,
            &config.input.cursor_fill,
            label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
        );

        draw_color_swatch_row(
            painter, text, ix, fox,
            "Outline", ZONE_CURSOR_OUTLINE_BASE,
            &config.input.cursor_outline,
            label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
        );

        // Live preview tile of the bundled default cursor with all current
        // settings applied. Vertically centered across the full 5-row
        // control block so it sits near the top of the card.
        {
            let tile_px = DEFAULT_PREVIEW_PX * s;
            let swatch_run_w = GLOW_COLORS.len() as f32 * (28.0 + 8.0) * s;
            let tile_x = ctrl_x + swatch_run_w + 12.0 * s;
            let tile_y = control_block_top + (row * 5.0 - tile_px) / 2.0;
            let tile_rect = Rect::new(tile_x, tile_y, tile_px, tile_px);
            let zone = ix.add_zone(ZONE_CURSOR_DEFAULT_TILE, tile_rect);

            let r = 10.0 * s;
            painter.rect_filled(tile_rect, r, fox.surface);

            let is_selected = config.input.cursor_theme == "default";
            let border_w = if is_selected { 3.0 * s } else { 1.5 * s };
            let border_color = if is_selected {
                fox.accent
            } else if zone.is_hovered() {
                fox.text_secondary
            } else {
                fox.muted
            };
            painter.rect_stroke_sdf(tile_rect, r, border_w, border_color);

            if let Some(tex) = state.default_preview_tex.as_ref() {
                let pad = 10.0 * s;
                let inner = tile_px - pad * 2.0;
                tex_draws.push(
                    TextureDraw::new(tex, tile_x + pad, tile_y + pad, inner, inner),
                );
            }
        }

        cy += 8.0 * s;
        let grid_origin_y = cy;
        let _ = GLOW_COLORS.len();

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
    // Cursor fill / outline swatch ranges — must be matched BEFORE the
    // open-ended `id >= ZONE_CURSOR_BASE` arm or those zones get swallowed
    // by the cursor-theme grid (and lookup-misses silently no-op).
    if zone_id >= ZONE_CURSOR_FILL_BASE
        && zone_id < ZONE_CURSOR_FILL_BASE + GLOW_COLORS.len() as u32
    {
        let idx = (zone_id - ZONE_CURSOR_FILL_BASE) as usize;
        if let Some((hex, _)) = GLOW_COLORS.get(idx) {
            config.input.cursor_fill = (*hex).into();
        }
        return;
    }
    if zone_id >= ZONE_CURSOR_OUTLINE_BASE
        && zone_id < ZONE_CURSOR_OUTLINE_BASE + GLOW_COLORS.len() as u32
    {
        let idx = (zone_id - ZONE_CURSOR_OUTLINE_BASE) as usize;
        if let Some((hex, _)) = GLOW_COLORS.get(idx) {
            config.input.cursor_outline = (*hex).into();
        }
        return;
    }

    match zone_id {
        ZONE_POINTER_ACCEL => {
            config.input.pointer_acceleration = !config.input.pointer_acceleration;
        }
        ZONE_DOUBLE_CLICK => {
            config.input.double_click_to_open = !config.input.double_click_to_open;
        }
        ZONE_CURSOR_DEFAULT_TILE => {
            // "Use the bundled default cursor" — swatches still drive its
            // fill / outline live via the compositor's tick_colors path.
            config.input.cursor_theme = "default".into();
        }
        id if id >= ZONE_CURSOR_BASE
            && id < ZONE_CURSOR_FILL_BASE =>
        {
            let idx = (id - ZONE_CURSOR_BASE) as usize;
            if let Some(cursor) = state.cursors.get(idx) {
                config.input.cursor_theme = cursor.id.clone();
            }
        }
        _ => {}
    }
}
