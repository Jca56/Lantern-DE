use lntrn_render::{Painter, Rect, TextureDraw, TexturePass, TextRenderer};
use lntrn_ui::gpu::{Button, ButtonVariant, FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::config::LanternConfig;
use crate::monitor_arrange::{self, MonitorArrangeState};
use crate::monitor_settings::{self, MonitorSettingsState};
use crate::output_manager::OutputManagerClient;
use crate::panels::{
    draw_section_card,
    CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
};
use crate::wayland::OutputInfo;
use crate::wallpaper_picker::WallpaperPicker;

// ── Zone IDs ────────────────────────────────────────────────────────────────

pub const ZONE_OPEN_FOLDER: u32 = 601;
const ZONE_THUMB_BASE: u32 = 610;
const MAX_THUMBS: u32 = 200;

// ── Layout constants ────────────────────────────────────────────────────────

const LABEL_SIZE: f32 = 18.0;
const CURRENT_LABEL_SIZE: f32 = 24.0;
const CURRENT_VALUE_SIZE: f32 = 22.0;
const CURRENT_ROW_H: f32 = 56.0;
const THUMB_GAP: f32 = 22.0;
const THUMB_W: f32 = 240.0;
const THUMB_H: f32 = 150.0;
const SELECTED_BORDER: f32 = 3.0;
const NAME_FONT: f32 = 14.0;
const OPEN_FOLDER_BTN_W: f32 = 160.0;
const OPEN_FOLDER_BTN_H: f32 = 40.0;

/// Canonical Lantern wallpaper directory.
pub fn wallpaper_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{}/.lantern/wallpapers", home)
}

/// Display panel state (persists across frames).
pub struct DisplayPanelState {
    pub picker: WallpaperPicker,
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
    pub fn new(_config: &LanternConfig) -> Self {
        Self {
            picker: WallpaperPicker::new(),
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
            last_arrange_h: 330.0,
            last_settings_h: 0.0,
        }
    }
}

// ── Grid layout helper ──────────────────────────────────────────────────────

fn grid_cols(grid_w: f32, thumb_w: f32, gap: f32) -> usize {
    ((grid_w + gap) / (thumb_w + gap)).floor().max(1.0) as usize
}

// ── Draw ────────────────────────────────────────────────────────────────────

pub fn draw_display_panel(
    subpanel: crate::wayland::Panel,
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
    hdr_client: &crate::hdr_client::HdrClient,
) {
    use crate::wayland::Panel;
    // The Display panel now hosts both the monitor arrangement and the
    // wallpaper picker: pick a display in the canvas, then set its wallpaper
    // in the card below.
    let show_display   = matches!(subpanel, Panel::Monitors);
    let show_wallpaper = show_display;
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
        dps.picker.load_directory(&wallpaper_dir(), tex_pass, gpu, true);
    }

    // ── Card geometry ──────────────────────────────────────────────
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    // ── Wallpaper card sizing ──────────────────────────────────────
    let header_row_h = CURRENT_ROW_H * s;       // current wallpaper label row
    let input_row_h = OPEN_FOLDER_BTN_H * s + 12.0 * s; // open-folder button + gap
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

    let mut content_height = CARD_OUTER_PAD_V * 2.0 * s;
    if show_display   { content_height += display_card_h; }
    if show_wallpaper { content_height += wallpaper_card_h; }
    if show_display && show_wallpaper { content_height += CARD_GAP * s; }

    // ── Single ScrollArea wrapping the whole panel ─────────────────
    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(&mut dps.scroll_offset, scroll_delta * 20.0, content_height, h);
    }

    let viewport = Rect::new(x, y, w, h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut dps.scroll_offset);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    if show_display {
        // ── Card: Display Settings (arrangement canvas + per-monitor settings) ──
        let inner_y = draw_section_card(
            painter, text, fox, "Display Settings",
            card_x, cy_top, card_w, display_card_h, s, sw, sh,
        );
        let arrange_h = monitor_arrange::draw_monitor_arrange(
            &mut dps.monitor_arrange, outputs, &config.monitors, output_mgr,
            painter, text, ix, fox,
            card_x, inner_y, card_w, s, sw, sh,
            false,
        );
        dps.last_arrange_h = arrange_h;

        let mut settings_h = 0.0;
        if let Some(hi) = selected_head_idx {
            let cfg_entry = selected_name.as_ref()
                .and_then(|name| config.monitors.iter().find(|m| &m.name == name));
            let hdr_caps = selected_name.as_ref()
                .and_then(|name| hdr_client.caps_for(name));
            let hdr_pending_secs = selected_name.as_ref()
                .and_then(|name| hdr_client.pending_for(name))
                .map(|p| p.secs_left());
            settings_h = monitor_settings::draw_monitor_settings(
                output_mgr, &mut dps.monitor_settings, hi, cfg_entry,
                painter, text, ix, fox,
                hdr_caps, hdr_pending_secs,
                card_x, inner_y + arrange_h + 12.0 * s, card_w, s, sw, sh,
                true,
            );
        }
        dps.last_settings_h = settings_h;
        cy_top += display_card_h + CARD_GAP * s;
    }

    // Wallpaper card — sits below the display settings. Name the selected
    // display in the header so it's clear which monitor we're changing.
    let wp_label = match &selected_name {
        Some(name) => format!("Wallpaper — {}", name),
        None => "Wallpaper".to_string(),
    };
    let wp_inner_y = if show_wallpaper {
        let inner = draw_section_card(
            painter, text, fox, &wp_label,
            card_x, cy_top, card_w, wallpaper_card_h, s, sw, sh,
        );
        inner
    } else {
        // Inert placeholder so the rest of the function compiles unchanged;
        // when `show_wallpaper` is false we skip every wallpaper-card draw.
        cy_top
    };
    let mut cy = wp_inner_y;

    if show_wallpaper {
    // Row 1: Current wallpaper label (bigger text)
    {
        let label_sz = CURRENT_LABEL_SIZE * s;
        let value_sz = CURRENT_VALUE_SIZE * s;
        let row_h = CURRENT_ROW_H * s;
        let label_w = 160.0 * s;
        let label_y = cy + (row_h - label_sz) / 2.0;
        text.queue("Current", label_sz, card_inner_x, label_y, fox.text, label_w, sw, sh);
        let val = if config.appearance.wallpaper.is_empty() {
            "(default)"
        } else {
            std::path::Path::new(&config.appearance.wallpaper)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&config.appearance.wallpaper)
        };
        let val_x = card_inner_x + label_w;
        let val_y = cy + (row_h - value_sz) / 2.0;
        text.queue(val, value_sz, val_x, val_y, fox.text_secondary,
            card_inner_w - label_w, sw, sh);
        cy += row_h;
    }

    // Row 2: Open Folder button
    {
        let btn_w = OPEN_FOLDER_BTN_W * s;
        let btn_h = OPEN_FOLDER_BTN_H * s;
        let btn_rect = Rect::new(card_inner_x, cy, btn_w, btn_h);
        let zone = ix.add_zone(ZONE_OPEN_FOLDER, btn_rect);
        Button::new(btn_rect, "Open Folder")
            .variant(ButtonVariant::Ghost)
            .hovered(zone.is_hovered())
            .pressed(zone.is_active())
            .scale(s)
            .draw(painter, text, fox, sw, sh);
        cy += btn_h + 12.0 * s;
    }

    // ── Thumbnail grid (or empty-state message) ────────────────────
    if entry_count == 0 {
        let dir = wallpaper_dir();
        let msg = if !std::path::Path::new(&dir).is_dir() {
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
    } // end `if show_wallpaper`

    scroll_area.end(painter, text);

    // ── Stash layout for collect_thumb_draws ───────────────────────
    dps.viewport_x = x;
    dps.viewport_y = y;
    dps.viewport_w = w;
    dps.viewport_h = h;
    dps.grid_x = card_inner_x;
    dps.grid_w = card_inner_w;
    let preceding_offset = if show_display {
        display_card_h + CARD_GAP * s
    } else { 0.0 };
    dps.grid_content_y_offset = CARD_OUTER_PAD_V * s
        + preceding_offset
        + (CARD_HEADER_H * s + CARD_INNER_PAD_V * s)
        + header_row_h + input_row_h;
    dps.content_height = content_height;

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, dps.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
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

    if zone_id == ZONE_OPEN_FOLDER {
        let dir = wallpaper_dir();
        let _ = std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn();
        return;
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

