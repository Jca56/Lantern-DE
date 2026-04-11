use lntrn_render::{Painter, Rect, TextureDraw, TexturePass, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, TextInput};

use crate::config::LanternConfig;
use crate::monitor_arrange::{self, MonitorArrangeState};
use crate::monitor_settings::{self, MonitorSettingsState};
use crate::output_manager::OutputManagerClient;
use crate::panels::{
    draw_section_card,
    CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
};
use crate::text_edit::TextBuffer;
use crate::wayland::OutputInfo;
use crate::wallpaper_picker::WallpaperPicker;

// ── Zone IDs ────────────────────────────────────────────────────────────────

pub const ZONE_DIR_INPUT: u32 = 600;
const ZONE_THUMB_BASE: u32 = 610;
const MAX_THUMBS: u32 = 200;

// ── Layout constants ────────────────────────────────────────────────────────

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
    /// Height of the monitor arrangement section, captured from the previous
    /// frame so we can size the scroll area before drawing.
    last_arrange_h: f32,
    /// Height of the per-monitor settings section, captured from the previous
    /// frame.
    last_settings_h: f32,
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
            last_arrange_h: 330.0, // estimate: matches monitor_arrange canvas + padding
            last_settings_h: 0.0,
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
    let lsz = LABEL_SIZE * s;

    // Reset per-monitor settings if selection changed
    let selected_name = dps.monitor_arrange.selected_output_name();
    if selected_name != dps.last_selected_output {
        dps.monitor_settings.reset();
        dps.last_selected_output = selected_name.clone();
    }
    let selected_head_idx = selected_name.as_ref().and_then(|name| {
        output_mgr.heads.iter().position(|h| &h.name == name)
    });

    // Load thumbnails if needed
    if dps.needs_reload {
        dps.needs_reload = false;
        dps.picker.load_directory(&dps.dir_buffer.text, tex_pass, gpu, true);
    }

    // ── Card geometry ──────────────────────────────────────────────
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    // ── Wallpaper card sizing ──────────────────────────────────────
    let header_row_h = ROW_H * s;       // current wallpaper label row
    let input_row_h = ROW_H * s + 8.0 * s; // directory input + gap
    let thumb_w = THUMB_W * s;
    let thumb_h = THUMB_H * s;
    let gap = THUMB_GAP * s;
    let cols = grid_cols(card_inner_w, thumb_w, gap);
    let entry_count = dps.picker.entries.len();
    let rows = if entry_count > 0 { (entry_count + cols - 1) / cols } else { 0 };
    let grid_content_h = if entry_count > 0 {
        rows as f32 * (thumb_h + gap)
    } else {
        80.0 * s // empty-state message height
    };
    let wallpaper_card_h = card_chrome_h + header_row_h + input_row_h + grid_content_h;

    // ── Display Settings card sizing ───────────────────────────────
    // Combined arrange canvas + per-monitor settings (when one is selected).
    // Heights snapshotted from the previous frame so the scroll area sizes
    // correctly even though both inner sections are dynamic.
    let arrange_h_est = dps.last_arrange_h.max(280.0 * s);
    let has_settings = selected_head_idx.is_some();
    let settings_h_est = if has_settings {
        dps.last_settings_h.max(180.0 * s) + 12.0 * s // small gap above
    } else {
        0.0
    };
    let display_card_h = card_chrome_h + arrange_h_est + settings_h_est;

    let content_height = CARD_OUTER_PAD_V * s
        + display_card_h + CARD_GAP * s
        + wallpaper_card_h + CARD_OUTER_PAD_V * 2.0 * s;

    // ── Single ScrollArea wrapping the whole panel ─────────────────
    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(&mut dps.scroll_offset, scroll_delta * 40.0, content_height, h);
    }

    let viewport = Rect::new(x, y, w, h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut dps.scroll_offset);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 1: Display Settings (arrangement canvas + per-monitor settings)
    // ─────────────────────────────────────────────────────────────────
    let inner_y = draw_section_card(
        painter, text, fox, "Display Settings",
        card_x, cy_top, card_w, display_card_h, s, sw, sh,
    );
    // monitor_arrange uses x + PAD as its content origin; PAD == 24 ==
    // CARD_INNER_PAD_H, so passing card_x lines content up with the card.
    let arrange_h = monitor_arrange::draw_monitor_arrange(
        &mut dps.monitor_arrange, outputs, &config.monitors,
        painter, text, ix, fox,
        card_x, inner_y, card_w, s, sw, sh,
        false, // header is provided by the card
    );
    dps.last_arrange_h = arrange_h;

    // Per-monitor settings drawn directly under the canvas, only when one
    // is selected.
    let mut settings_h = 0.0;
    if let Some(hi) = selected_head_idx {
        settings_h = monitor_settings::draw_monitor_settings(
            output_mgr, &mut dps.monitor_settings, hi,
            painter, text, ix, fox,
            card_x, inner_y + arrange_h + 12.0 * s, card_w, s, sw, sh,
            true, // show "Settings: <name>" inline header to identify which display
        );
    }
    dps.last_settings_h = settings_h;
    cy_top += display_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 3: Wallpaper
    // ─────────────────────────────────────────────────────────────────
    let wp_inner_y = draw_section_card(
        painter, text, fox, "Wallpaper",
        card_x, cy_top, card_w, wallpaper_card_h, s, sw, sh,
    );
    let mut cy = wp_inner_y;

    // Row 1: Current wallpaper label
    {
        let label_y = cy + (ROW_H * s - lsz) / 2.0;
        text.queue("Current", lsz, card_inner_x, label_y, fox.text, 140.0 * s, sw, sh);
        let val = if config.appearance.wallpaper.is_empty() {
            "(default)"
        } else {
            std::path::Path::new(&config.appearance.wallpaper)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&config.appearance.wallpaper)
        };
        let val_x = card_inner_x + 140.0 * s;
        text.queue(val, lsz, val_x, label_y, fox.text_secondary,
            card_inner_w - 140.0 * s, sw, sh);
        cy += ROW_H * s;
    }

    // Row 2: Directory input
    draw_dir_input_in_card(config, dps, painter, text, ix, fox,
        card_inner_x, cy, card_inner_w, s, sw, sh);
    cy += ROW_H * s + 8.0 * s;

    // ── Thumbnail grid (or empty-state message) ────────────────────
    if entry_count == 0 {
        let msg = if !std::path::Path::new(&dps.dir_buffer.text).is_dir() {
            "Directory not found"
        } else {
            "No images found"
        };
        text.queue(msg, lsz, card_inner_x, cy + 40.0 * s, fox.text_secondary,
            card_inner_w, sw, sh);
    } else {
        let name_sz = NAME_FONT * s;
        let name_pad = 4.0 * s;

        for (i, entry) in dps.picker.entries.iter().enumerate() {
            if i as u32 >= MAX_THUMBS { break; }
            let col = i % cols;
            let row = i / cols;
            let tx = card_inner_x + col as f32 * (thumb_w + gap);
            let ty = cy + row as f32 * (thumb_h + gap);

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

            if let Some(name) = entry.path.file_stem().and_then(|n| n.to_str()) {
                let scrim_h = name_sz + name_pad * 2.0;
                let scrim_rect = Rect::new(tx, ty, thumb_w, scrim_h);
                painter.rect_4corner(scrim_rect, [corner, corner, 0.0, 0.0], fox.bg.with_alpha(0.6));
                text.queue(name, name_sz, tx + name_pad, ty + name_pad, fox.text, thumb_w - name_pad * 2.0, sw, sh);
            }
        }
    }

    scroll_area.end(painter, text);

    // ── Stash layout for collect_thumb_draws ───────────────────────
    // collect_thumb_draws uses these to position textures in the
    // separate texture pass. The grid origin is wp_inner_y + 2 rows.
    dps.viewport_x = x;
    dps.viewport_y = y;
    dps.viewport_w = w;
    dps.viewport_h = h;
    dps.grid_x = card_inner_x;
    dps.grid_w = card_inner_w;
    dps.grid_content_y_offset = CARD_OUTER_PAD_V * s
        + display_card_h + CARD_GAP * s
        + (CARD_HEADER_H * s + CARD_INNER_PAD_V * s)
        + header_row_h + input_row_h;
    dps.content_height = content_height;

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, dps.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

/// Draw the directory text input row laid out within a card.
fn draw_dir_input_in_card(
    config: &LanternConfig,
    dps: &mut DisplayPanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    inner_x: f32, cy: f32, inner_w: f32, s: f32, sw: u32, sh: u32,
) {
    let _ = config;
    let lsz = LABEL_SIZE * s;
    let label_y = cy + (ROW_H * s - lsz) / 2.0;
    text.queue("Directory", lsz, inner_x, label_y, fox.text, 140.0 * s, sw, sh);

    let input_x = inner_x + 140.0 * s;
    let input_w = inner_w - 140.0 * s;
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
