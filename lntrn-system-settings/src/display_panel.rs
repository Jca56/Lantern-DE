use lntrn_render::{Painter, Rect, TextureDraw, TexturePass, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, TextInput};

use crate::config::LanternConfig;
use crate::monitor_arrange::{self, MonitorArrangeState};
use crate::monitor_settings::{self, MonitorSettingsState};
use crate::output_manager::OutputManagerClient;
use crate::text_edit::TextBuffer;
use crate::wayland::OutputInfo;
use crate::wallpaper_picker::WallpaperPicker;

// ── Zone IDs ────────────────────────────────────────────────────────────────

pub const ZONE_DIR_INPUT: u32 = 600;
const ZONE_THUMB_BASE: u32 = 610;
const MAX_THUMBS: u32 = 200;

// ── Layout constants ────────────────────────────────────────────────────────

const PAD: f32 = 24.0;
const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const THUMB_GAP: f32 = 12.0;
const THUMB_W: f32 = 192.0;
const THUMB_H: f32 = 120.0;
const INPUT_H: f32 = 44.0;
const SELECTED_BORDER: f32 = 3.0;
const NAME_FONT: f32 = 14.0;

/// Display panel state (persists across frames).
pub struct DisplayPanelState {
    pub picker: WallpaperPicker,
    pub dir_buffer: TextBuffer,
    pub dir_focused: bool,
    pub scroll_offset: f32,
    pub needs_reload: bool,
    pub monitor_arrange: MonitorArrangeState,
    pub monitor_settings: MonitorSettingsState,
    /// Track which output was last selected (to detect changes).
    last_selected_output: Option<String>,
    /// Viewport for the whole panel (set during draw, used by collect_thumb_draws).
    viewport_x: f32,
    viewport_y: f32,
    viewport_w: f32,
    viewport_h: f32,
    /// Grid origin within scrollable content.
    grid_content_y_offset: f32,
    grid_x: f32,
    grid_w: f32,
    content_height: f32,
}

impl DisplayPanelState {
    pub fn new(config: &LanternConfig) -> Self {
        Self {
            picker: WallpaperPicker::new(),
            dir_buffer: TextBuffer::new(&config.appearance.wallpaper_directory),
            dir_focused: false,
            scroll_offset: 0.0,
            needs_reload: true,
            monitor_arrange: MonitorArrangeState::new(),
            monitor_settings: MonitorSettingsState::new(),
            last_selected_output: None,
            viewport_x: 0.0,
            viewport_y: 0.0,
            viewport_w: 0.0,
            viewport_h: 0.0,
            grid_content_y_offset: 0.0,
            grid_x: 0.0,
            grid_w: 0.0,
            content_height: 0.0,
        }
    }

    /// Sync the text buffer if config changed externally (e.g. cancel/load).
    pub fn sync_from_config(&mut self, config: &LanternConfig) {
        if self.dir_buffer.text != config.appearance.wallpaper_directory {
            self.dir_buffer.set(&config.appearance.wallpaper_directory);
            self.needs_reload = true;
        }
    }
}

// ── Grid layout helper ──────────────────────────────────────────────────────

fn grid_cols(grid_w: f32, thumb_w: f32, gap: f32) -> usize {
    ((grid_w + gap) / (thumb_w + gap)).floor().max(1.0) as usize
}

// ── Draw ────────────────────────────────────────────────────────────────────

pub fn draw_display_panel(
    config: &mut LanternConfig,
    dps: &mut DisplayPanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    tex_pass: &TexturePass,
    fox: &FoxPalette,
    gpu: &lntrn_render::GpuContext,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    s: f32,
    sw: u32,
    sh: u32,
    scroll_delta: f32,
    outputs: &[(u32, OutputInfo)],
    output_mgr: &OutputManagerClient,
) {
    let pad = PAD * s;
    let lsz = LABEL_SIZE * s;

    // ── Monitor arrangement (fixed at top, not scrolled) ───────────
    let mon_h = monitor_arrange::draw_monitor_arrange(
        &mut dps.monitor_arrange, outputs, &config.monitors,
        painter, text, ix, fox, x, y, w, s, sw, sh,
    );
    // ── Per-monitor settings (resolution, refresh, scale) ────────
    let mut settings_h = 0.0;
    let selected_name = dps.monitor_arrange.selected_output_name();
    // Reset monitor settings if selection changed
    if selected_name != dps.last_selected_output {
        dps.monitor_settings.reset();
        dps.last_selected_output = selected_name.clone();
    }
    let selected_head_idx = selected_name.as_ref().and_then(|name| {
        output_mgr.heads.iter().position(|h| &h.name == name)
    });
    if let Some(hi) = selected_head_idx {
        settings_h = monitor_settings::draw_monitor_settings(
            output_mgr, &mut dps.monitor_settings, hi,
            painter, text, ix, fox,
            x, y + mon_h, w, s, sw, sh,
        );
    }

    let wp_y = y + mon_h + settings_h;
    let wp_h = h - mon_h - settings_h;

    // Load thumbnails if needed
    if dps.needs_reload {
        dps.needs_reload = false;
        dps.picker.load_directory(&dps.dir_buffer.text, tex_pass, gpu, true);
    }

    // ── Compute total content height ────────────────────────────────
    // Row 1: "Wallpaper" label row
    // Row 2: "Directory" input row + gap
    // Then the thumbnail grid
    let header_h = ROW_H * s;          // wallpaper label
    let input_row_h = ROW_H * s + 8.0 * s; // directory input + gap

    let grid_x = x + pad;
    let grid_w = w - pad * 2.0;
    let thumb_w = THUMB_W * s;
    let thumb_h = THUMB_H * s;
    let gap = THUMB_GAP * s;
    let cols = grid_cols(grid_w, thumb_w, gap);
    let entry_count = dps.picker.entries.len();
    let rows = if entry_count > 0 { (entry_count + cols - 1) / cols } else { 0 };
    let grid_content_h = rows as f32 * (thumb_h + gap);

    let content_height = header_h + input_row_h + grid_content_h;
    let viewport = Rect::new(x, wp_y, w, wp_h);

    // Store for collect_thumb_draws
    dps.viewport_x = x;
    dps.viewport_y = wp_y;
    dps.viewport_w = w;
    dps.viewport_h = wp_h;
    dps.grid_x = grid_x;
    dps.grid_w = grid_w;
    dps.grid_content_y_offset = header_h + input_row_h;
    dps.content_height = content_height;

    // Handle scrolling
    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(&mut dps.scroll_offset, scroll_delta * 40.0, content_height, wp_h);
    }

    let scroll_area = ScrollArea::new(viewport, content_height, &mut dps.scroll_offset);

    // Empty state (show before scroll area so it's not clipped weirdly)
    if entry_count == 0 {
        scroll_area.begin(painter, text);
        let cy = scroll_area.content_y();

        // Still draw headers inside scroll
        let label_y = cy + (ROW_H * s - lsz) / 2.0;
        text.queue("Wallpaper", lsz, x + pad, label_y, fox.text, 140.0 * s, sw, sh);
        let val = if config.appearance.wallpaper.is_empty() { "(default)" } else {
            std::path::Path::new(&config.appearance.wallpaper)
                .file_name().and_then(|n| n.to_str())
                .unwrap_or(&config.appearance.wallpaper)
        };
        text.queue(val, lsz, x + pad + 140.0 * s, label_y, fox.text_secondary, w - pad - 140.0 * s, sw, sh);

        let input_cy = cy + header_h;
        draw_dir_input(config, dps, painter, text, ix, fox, x, input_cy, w, s, sw, sh);

        let msg_cy = cy + header_h + input_row_h;
        let msg = if !std::path::Path::new(&dps.dir_buffer.text).is_dir() {
            "Directory not found"
        } else {
            "No images found"
        };
        text.queue(msg, lsz, grid_x, msg_cy + 40.0 * s, fox.text_secondary, grid_w, sw, sh);

        scroll_area.end(painter, text);
        return;
    }

    // ── Draw everything inside the scroll area ──────────────────────
    scroll_area.begin(painter, text);
    let base_y = scroll_area.content_y();
    let mut cy = base_y;

    // Row 1: Current wallpaper label
    {
        let label_y = cy + (ROW_H * s - lsz) / 2.0;
        text.queue("Wallpaper", lsz, x + pad, label_y, fox.text, 140.0 * s, sw, sh);
        let val = if config.appearance.wallpaper.is_empty() {
            "(default)"
        } else {
            std::path::Path::new(&config.appearance.wallpaper)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&config.appearance.wallpaper)
        };
        let val_x = x + pad + 140.0 * s;
        text.queue(val, lsz, val_x, label_y, fox.text_secondary, w - pad - 140.0 * s, sw, sh);
        cy += ROW_H * s;
    }

    // Row 2: Directory input
    draw_dir_input(config, dps, painter, text, ix, fox, x, cy, w, s, sw, sh);
    cy += ROW_H * s + 8.0 * s;

    // ── Thumbnail grid ──────────────────────────────────────────────
    let name_sz = NAME_FONT * s;
    let name_pad = 4.0 * s;

    for (i, entry) in dps.picker.entries.iter().enumerate() {
        if i as u32 >= MAX_THUMBS { break; }
        let col = i % cols;
        let row = i / cols;
        let tx = grid_x + col as f32 * (thumb_w + gap);
        let ty = cy + row as f32 * (thumb_h + gap);

        // Skip if outside visible area
        if ty + thumb_h < y || ty > y + h { continue; }

        let zone_id = ZONE_THUMB_BASE + i as u32;
        let rect = Rect::new(tx, ty, thumb_w, thumb_h);
        let zone = ix.add_zone(zone_id, rect);

        let is_selected = !config.appearance.wallpaper.is_empty()
            && entry.path.to_str().map(|p| p == config.appearance.wallpaper).unwrap_or(false);

        let corner = 6.0 * s;

        if is_selected {
            let b = SELECTED_BORDER * s;
            let outer = Rect::new(tx - b, ty - b, thumb_w + b * 2.0, thumb_h + b * 2.0);
            painter.rect_filled(outer, corner + b, fox.accent);
        } else if zone.is_hovered() {
            let b = 2.0 * s;
            let outer = Rect::new(tx - b, ty - b, thumb_w + b * 2.0, thumb_h + b * 2.0);
            painter.rect_filled(outer, corner + b, fox.text.with_alpha(0.3));
        }

        // Draw filename at top-left of thumbnail with a dark scrim behind it
        if let Some(name) = entry.path.file_stem().and_then(|n| n.to_str()) {
            let scrim_h = name_sz + name_pad * 2.0;
            let scrim_rect = Rect::new(tx, ty, thumb_w, scrim_h);
            painter.rect_4corner(scrim_rect, [corner, corner, 0.0, 0.0], fox.bg.with_alpha(0.6));
            text.queue(name, name_sz, tx + name_pad, ty + name_pad, fox.text, thumb_w - name_pad * 2.0, sw, sh);
        }
    }

    scroll_area.end(painter, text);

    // Scrollbar outside the clip region
    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, dps.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

/// Draw the directory text input row.
fn draw_dir_input(
    config: &LanternConfig,
    dps: &mut DisplayPanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32, cy: f32, w: f32, s: f32, sw: u32, sh: u32,
) {
    let pad = PAD * s;
    let lsz = LABEL_SIZE * s;
    let label_y = cy + (ROW_H * s - lsz) / 2.0;
    text.queue("Directory", lsz, x + pad, label_y, fox.text, 140.0 * s, sw, sh);

    let input_x = x + pad + 140.0 * s;
    let input_w = w - pad * 2.0 - 140.0 * s;
    let input_h = INPUT_H * s;
    let input_y = cy + (ROW_H * s - input_h) / 2.0;
    let input_rect = Rect::new(input_x, input_y, input_w, input_h);
    let zone = ix.add_zone(ZONE_DIR_INPUT, input_rect);

    let mut ti = TextInput::new(input_rect)
        .text(&dps.dir_buffer.text)
        .placeholder("~/Pictures/Wallpapers")
        .focused(dps.dir_focused)
        .hovered(zone.is_hovered());
    if dps.dir_focused {
        ti = ti.cursor_pos(dps.dir_buffer.cursor);
    }
    ti.scale(s).draw(painter, text, fox, sw, sh);
}

/// Collect texture draws for thumbnail images. Call after draw_display_panel.
pub fn collect_thumb_draws<'a>(
    dps: &'a DisplayPanelState,
    s: f32,
) -> Vec<TextureDraw<'a>> {
    let thumb_w = THUMB_W * s;
    let thumb_h = THUMB_H * s;
    let gap = THUMB_GAP * s;
    let cols = grid_cols(dps.grid_w, thumb_w, gap);

    // Grid starts at viewport_y - scroll_offset + grid_content_y_offset
    let base_y = dps.viewport_y - dps.scroll_offset + dps.grid_content_y_offset;
    let clip = [dps.viewport_x, dps.viewport_y, dps.viewport_w, dps.viewport_h];

    let mut draws = Vec::new();
    for (i, entry) in dps.picker.entries.iter().enumerate() {
        if i as u32 >= MAX_THUMBS { break; }
        let col = i % cols;
        let row = i / cols;
        let tx = dps.grid_x + col as f32 * (thumb_w + gap);
        let ty = base_y + row as f32 * (thumb_h + gap);

        if ty + thumb_h < dps.viewport_y || ty > dps.viewport_y + dps.viewport_h { continue; }

        let mut draw = TextureDraw::new(&entry.texture, tx, ty, thumb_w, thumb_h);
        draw.clip = Some(clip);
        draws.push(draw);
    }
    draws
}

// ── Click handling ──────────────────────────────────────────────────────────

pub fn handle_display_click(
    config: &mut LanternConfig,
    dps: &mut DisplayPanelState,
    zone_id: u32,
    cursor_x: f32,
    cursor_y: f32,
    output_mgr: &OutputManagerClient,
) {
    // Monitor arrangement clicks
    if monitor_arrange::handle_arrange_click(&mut dps.monitor_arrange, zone_id, cursor_x, cursor_y) {
        return;
    }

    // Per-monitor settings clicks
    let selected_head_idx = dps.monitor_arrange.selected_output_name().and_then(|name| {
        output_mgr.heads.iter().position(|h| h.name == name)
    });
    if let Some(hi) = selected_head_idx {
        if monitor_settings::handle_monitor_settings_click(output_mgr, &mut dps.monitor_settings, hi, zone_id) {
            return;
        }
    }

    if zone_id == ZONE_DIR_INPUT {
        dps.dir_focused = true;
        return;
    }

    // Clicking anywhere else unfocuses the text input
    if dps.dir_focused {
        dps.dir_focused = false;
        if dps.dir_buffer.text != config.appearance.wallpaper_directory {
            config.appearance.wallpaper_directory = dps.dir_buffer.text.clone();
            dps.needs_reload = true;
        }
    }

    // Thumbnail click — write to per-monitor config if a monitor is selected
    if zone_id >= ZONE_THUMB_BASE && zone_id < ZONE_THUMB_BASE + MAX_THUMBS {
        let idx = (zone_id - ZONE_THUMB_BASE) as usize;
        if let Some(entry) = dps.picker.entries.get(idx) {
            let wp_path = entry.path.to_string_lossy().to_string();
            if let Some(selected_name) = dps.monitor_arrange.selected_output_name() {
                // Write to per-monitor config entry
                if let Some(mon) = config.monitors.iter_mut().find(|m| m.name == selected_name) {
                    mon.wallpaper = wp_path.clone();
                }
            }
            // Also update global wallpaper
            config.appearance.wallpaper = wp_path;
        }
    }
}

/// Handle keyboard input when the directory text input is focused.
/// Returns true if the key was consumed.
pub fn handle_display_key(
    config: &mut LanternConfig,
    dps: &mut DisplayPanelState,
    sym: xkbcommon::xkb::Keysym,
    utf8: Option<String>,
) -> bool {
    if !dps.dir_focused {
        return false;
    }

    match sym.raw() {
        0xff0d | 0xff8d => {
            // Return/Enter — apply directory change
            dps.dir_focused = false;
            if dps.dir_buffer.text != config.appearance.wallpaper_directory {
                config.appearance.wallpaper_directory = dps.dir_buffer.text.clone();
                dps.needs_reload = true;
            }
            true
        }
        0xff1b => {
            // Escape — cancel editing, revert
            dps.dir_focused = false;
            dps.dir_buffer.set(&config.appearance.wallpaper_directory);
            true
        }
        0xff08 => { dps.dir_buffer.backspace(); true }
        0xffff => { dps.dir_buffer.delete(); true }
        0xff51 => { dps.dir_buffer.left(); true }
        0xff53 => { dps.dir_buffer.right(); true }
        0xff50 => { dps.dir_buffer.home(); true }
        0xff57 => { dps.dir_buffer.end(); true }
        _ => {
            if let Some(ch) = utf8 {
                dps.dir_buffer.insert(&ch);
                true
            } else {
                false
            }
        }
    }
}
