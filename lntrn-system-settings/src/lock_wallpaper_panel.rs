//! Lock Screen → Wallpaper panel.
//!
//! Thumbnail picker over `~/.lantern/lockscreen-wallpapers/`. Selecting an
//! image writes its absolute path to `[lockscreen] background` in lantern.toml.
//! Mirrors the wallpaper portion of `display_panel.rs` but without the monitor
//! arrangement canvas — just an "Open Folder" button and the thumbnail grid.

use lntrn_render::{Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{Button, ButtonVariant, FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V, CARD_OUTER_PAD_H,
    CARD_OUTER_PAD_V,
};
use crate::wallpaper_picker::WallpaperPicker;

// ── Zone IDs (700+ block, distinct from display_panel's 601/610+) ────────────

pub const ZONE_OPEN_FOLDER: u32 = 700;
const ZONE_THUMB_BASE: u32 = 710;
const MAX_THUMBS: u32 = 200;

// ── Layout constants ─────────────────────────────────────────────────────────

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

/// Canonical lock screen wallpaper directory.
pub fn lockscreen_wallpaper_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{}/.lantern/lockscreen-wallpapers", home)
}

/// Lock screen wallpaper panel state (persists across frames).
pub struct LockWallpaperState {
    pub picker: WallpaperPicker,
    pub scroll_offset: f32,
    pub needs_reload: bool,
    viewport_x: f32,
    viewport_y: f32,
    viewport_w: f32,
    viewport_h: f32,
    grid_content_y_offset: f32,
    grid_x: f32,
    grid_w: f32,
}

impl LockWallpaperState {
    pub fn new() -> Self {
        Self {
            picker: WallpaperPicker::new(),
            scroll_offset: 0.0,
            needs_reload: true,
            viewport_x: 0.0,
            viewport_y: 0.0,
            viewport_w: 0.0,
            viewport_h: 0.0,
            grid_content_y_offset: 0.0,
            grid_x: 0.0,
            grid_w: 0.0,
        }
    }
}

fn grid_cols(grid_w: f32, thumb_w: f32, gap: f32) -> usize {
    ((grid_w + gap) / (thumb_w + gap)).floor().max(1.0) as usize
}

// ── Draw ──────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_lock_wallpaper_panel(
    config: &mut LanternConfig,
    lws: &mut LockWallpaperState,
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
) {
    let lsz = LABEL_SIZE * s;

    if lws.needs_reload {
        lws.needs_reload = false;
        lws.picker
            .load_directory(&lockscreen_wallpaper_dir(), tex_pass, gpu, true);
    }

    // ── Card geometry ──────────────────────────────────────────────
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    let header_row_h = CURRENT_ROW_H * s;
    let input_row_h = OPEN_FOLDER_BTN_H * s + 12.0 * s;
    let thumb_w = THUMB_W * s;
    let thumb_h = THUMB_H * s;
    let gap = THUMB_GAP * s;
    let cols = grid_cols(card_inner_w, thumb_w, gap);
    let entry_count = lws.picker.entries.len();
    let rows = if entry_count > 0 {
        (entry_count + cols - 1) / cols
    } else {
        0
    };
    let grid_content_h = if entry_count > 0 {
        rows as f32 * (thumb_h + gap)
    } else {
        80.0 * s
    };
    let card_h = card_chrome_h + header_row_h + input_row_h + grid_content_h;

    let mut content_height = CARD_OUTER_PAD_V * 2.0 * s;
    content_height += card_h;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(
            &mut lws.scroll_offset,
            scroll_delta * 20.0,
            content_height,
            h,
        );
    }

    let viewport = Rect::new(x, y, w, h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut lws.scroll_offset);
    scroll_area.begin(painter, text);

    let cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    let inner_y = draw_section_card(
        painter,
        text,
        fox,
        "Wallpaper",
        card_x,
        cy_top,
        card_w,
        card_h,
        s,
        sw,
        sh,
    );
    let mut cy = inner_y;

    // Row 1: Current selection label
    {
        let label_sz = CURRENT_LABEL_SIZE * s;
        let value_sz = CURRENT_VALUE_SIZE * s;
        let row_h = CURRENT_ROW_H * s;
        let label_w = 160.0 * s;
        let label_y = cy + (row_h - label_sz) / 2.0;
        text.queue(
            "Current",
            label_sz,
            card_inner_x,
            label_y,
            fox.text,
            label_w,
            sw,
            sh,
        );
        let val = if config.lockscreen.background.is_empty() {
            "(default)"
        } else {
            std::path::Path::new(&config.lockscreen.background)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&config.lockscreen.background)
        };
        let val_x = card_inner_x + label_w;
        let val_y = cy + (row_h - value_sz) / 2.0;
        text.queue(
            val,
            value_sz,
            val_x,
            val_y,
            fox.text_secondary,
            card_inner_w - label_w,
            sw,
            sh,
        );
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
        let dir = lockscreen_wallpaper_dir();
        let msg = if !std::path::Path::new(&dir).is_dir() {
            "Directory not found"
        } else {
            "No images found"
        };
        text.queue(
            msg,
            lsz,
            card_inner_x,
            cy + 40.0 * s,
            fox.text_secondary,
            card_inner_w,
            sw,
            sh,
        );
    } else {
        let name_sz = NAME_FONT * s;
        let name_pad = 4.0 * s;

        for (i, entry) in lws.picker.entries.iter().enumerate() {
            if i as u32 >= MAX_THUMBS {
                break;
            }
            let col = i % cols;
            let row = i / cols;
            let tx = card_inner_x + col as f32 * (thumb_w + gap);
            let ty = cy + row as f32 * (thumb_h + gap);

            if ty + thumb_h < y || ty > y + h {
                continue;
            }

            let zone_id = ZONE_THUMB_BASE + i as u32;
            let rect = Rect::new(tx, ty, thumb_w, thumb_h);
            let zone = ix.add_zone(zone_id, rect);

            let is_selected = !config.lockscreen.background.is_empty()
                && entry
                    .path
                    .to_str()
                    .map(|p| p == config.lockscreen.background)
                    .unwrap_or(false);

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
                painter.rect_4corner(
                    scrim_rect,
                    [corner, corner, 0.0, 0.0],
                    fox.bg.with_alpha(0.6),
                );
                text.queue(
                    name,
                    name_sz,
                    tx + name_pad,
                    ty + name_pad,
                    fox.text,
                    thumb_w - name_pad * 2.0,
                    sw,
                    sh,
                );
            }
        }
    }

    scroll_area.end(painter, text);

    // ── Stash layout for collect_thumb_draws ───────────────────────
    lws.viewport_x = x;
    lws.viewport_y = y;
    lws.viewport_w = w;
    lws.viewport_h = h;
    lws.grid_x = card_inner_x;
    lws.grid_w = card_inner_w;
    lws.grid_content_y_offset = CARD_OUTER_PAD_V * s
        + (CARD_HEADER_H * s + CARD_INNER_PAD_V * s)
        + header_row_h
        + input_row_h;

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, lws.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

/// Collect texture draws for thumbnail images. Call after draw_lock_wallpaper_panel.
pub fn collect_thumb_draws<'a>(lws: &'a LockWallpaperState, s: f32) -> Vec<TextureDraw<'a>> {
    let thumb_w = THUMB_W * s;
    let thumb_h = THUMB_H * s;
    let gap = THUMB_GAP * s;
    let cols = grid_cols(lws.grid_w, thumb_w, gap);

    let base_y = lws.viewport_y - lws.scroll_offset + lws.grid_content_y_offset;
    let clip = [
        lws.viewport_x,
        lws.viewport_y,
        lws.viewport_w,
        lws.viewport_h,
    ];

    let mut draws = Vec::new();
    for (i, entry) in lws.picker.entries.iter().enumerate() {
        if i as u32 >= MAX_THUMBS {
            break;
        }
        let col = i % cols;
        let row = i / cols;
        let tx = lws.grid_x + col as f32 * (thumb_w + gap);
        let ty = base_y + row as f32 * (thumb_h + gap);

        if ty + thumb_h < lws.viewport_y || ty > lws.viewport_y + lws.viewport_h {
            continue;
        }

        let mut draw = TextureDraw::new(&entry.texture, tx, ty, thumb_w, thumb_h);
        draw.clip = Some(clip);
        draws.push(draw);
    }
    draws
}

// ── Click handling ──────────────────────────────────────────────────────────

pub fn handle_lock_wallpaper_click(
    config: &mut LanternConfig,
    lws: &mut LockWallpaperState,
    zone_id: u32,
) {
    if zone_id == ZONE_OPEN_FOLDER {
        let dir = lockscreen_wallpaper_dir();
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
        return;
    }

    if zone_id >= ZONE_THUMB_BASE && zone_id < ZONE_THUMB_BASE + MAX_THUMBS {
        let idx = (zone_id - ZONE_THUMB_BASE) as usize;
        if let Some(entry) = lws.picker.entries.get(idx) {
            config.lockscreen.background = entry.path.to_string_lossy().to_string();
        }
    }
}
