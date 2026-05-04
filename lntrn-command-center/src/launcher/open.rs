//! "Open" section — live thumbnails of currently-running windows.
//!
//! Phase A: placeholder rectangles where the thumbnail will go (the
//! compositor overlay that paints real thumbnails into these rects is
//! Phase B). Title labels under each tile, click=focus, right-click=menu.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::render::IconRequest;
use crate::search::apps::AppsProvider;
use crate::search::input;
use crate::toplevel::ToplevelInfo;

/// 16:9 thumbnail tile dimensions (logical px).
pub const OPEN_TILE_W: f32 = 144.0;
pub const OPEN_TILE_H: f32 = 81.0;
pub const OPEN_TILE_GAP_X: f32 = 24.0;
pub const OPEN_TILE_GAP_Y: f32 = 24.0;
pub const OPEN_LABEL_FONT: f32 = 15.0;
pub const OPEN_LABEL_GAP: f32 = 8.0;
pub const OPEN_SECTION_TOP_MARGIN: f32 = 24.0;
pub const OPEN_HEADING_GAP: f32 = 12.0;
pub const OPEN_TILE_RADIUS: f32 = 10.0;
pub const OPEN_DIVIDER_GAP_BELOW: f32 = 18.0;
pub const OPEN_DIVIDER_THICKNESS: f32 = 1.0;

const HEADING_FONT: f32 = 14.0;
const HEADING_ALPHA: f32 = 0.55;
const LABEL_ALPHA: f32 = 0.85;
const PLACEHOLDER_BG_ALPHA: f32 = 0.10;
const PLACEHOLDER_BORDER_ALPHA: f32 = 0.06;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const ICON_INSET_FRAC: f32 = 0.30;
const DIVIDER_ALPHA: f32 = 0.10;

fn text(alpha: f32) -> Color {
    Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha)
}
fn accent(alpha: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
}

/// Vertical advance from the section's top (= `top_y` passed to `draw`)
/// to the first tile-row top: divider + gap + heading + heading→row gap.
pub fn heading_advance(scale: f32) -> f32 {
    let divider_h = (OPEN_DIVIDER_THICKNESS * scale).max(1.0);
    divider_h
        + OPEN_DIVIDER_GAP_BELOW * scale
        + HEADING_FONT * scale
        + OPEN_HEADING_GAP * scale
}

/// Total logical-pixel height the section occupies for `num` tiles in a
/// panel of `panel_w` logical px (including the top margin, heading,
/// and row wrapping). Returns 0 when `num == 0` so the panel doesn't
/// reserve space for an empty section.
pub fn section_height_logical(panel_w_logical: f32, num: usize) -> f32 {
    if num == 0 {
        return 0.0;
    }
    let pad = input::SEARCH_HORIZONTAL_PAD;
    let avail = panel_w_logical - pad * 2.0;
    let cols = (((avail + OPEN_TILE_GAP_X) / (OPEN_TILE_W + OPEN_TILE_GAP_X)).floor() as usize)
        .max(1);
    let cell_h = OPEN_TILE_H + OPEN_LABEL_GAP + OPEN_LABEL_FONT;
    let rows = num.div_ceil(cols);
    OPEN_SECTION_TOP_MARGIN
        + OPEN_DIVIDER_THICKNESS
        + OPEN_DIVIDER_GAP_BELOW
        + HEADING_FONT
        + OPEN_HEADING_GAP
        + rows as f32 * cell_h
        + (rows.saturating_sub(1)) as f32 * OPEN_TILE_GAP_Y
}

/// Number of columns that fit in `panel_w` with the given scale.
pub fn columns(panel_w: f32, scale: f32) -> usize {
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let avail = panel_w - pad * 2.0;
    let tile_w = OPEN_TILE_W * scale;
    let gap = OPEN_TILE_GAP_X * scale;
    (((avail + gap) / (tile_w + gap)).floor() as usize).max(1)
}

/// Given a tile index, return its physical-pixel rect inside `panel`.
pub fn tile_rect(panel: Rect, top_y: f32, scale: f32, idx: usize) -> Rect {
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let tile_w = OPEN_TILE_W * scale;
    let tile_h = OPEN_TILE_H * scale;
    let gap_x = OPEN_TILE_GAP_X * scale;
    let gap_y = OPEN_TILE_GAP_Y * scale;
    let label_font = OPEN_LABEL_FONT * scale;
    let label_gap = OPEN_LABEL_GAP * scale;
    let cell_h = tile_h + label_gap + label_font;
    let cols = columns(panel.w, scale);
    let col = idx % cols;
    let row = idx / cols;
    Rect::new(
        panel.x + pad + col as f32 * (tile_w + gap_x),
        top_y + row as f32 * (cell_h + gap_y),
        tile_w,
        tile_h,
    )
}

/// Toplevels shown in the Open section. Includes minimized windows so users
/// can find and restore them; minimized tiles render dimmed.
pub fn visible_entries(toplevels: &[ToplevelInfo]) -> Vec<&ToplevelInfo> {
    toplevels
        .iter()
        .filter(|t| !t.app_id.is_empty())
        .collect()
}

const MINIMIZED_ALPHA_MULT: f32 = 0.5;

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text_r: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    toplevels: &[ToplevelInfo],
    apps: &AppsProvider,
    selected: Option<usize>,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let entries = visible_entries(toplevels);
    if entries.is_empty() {
        return top_y;
    }

    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let heading_font = HEADING_FONT * scale;
    let heading_gap = OPEN_HEADING_GAP * scale;
    let label_font = OPEN_LABEL_FONT * scale;
    let label_gap = OPEN_LABEL_GAP * scale;
    let tile_w = OPEN_TILE_W * scale;
    let tile_h = OPEN_TILE_H * scale;
    let radius = OPEN_TILE_RADIUS * scale;

    let mut y = top_y + OPEN_SECTION_TOP_MARGIN * scale;

    // Divider above the heading separating Pinned from Open.
    let divider_h = (OPEN_DIVIDER_THICKNESS * scale).max(1.0);
    painter.rect_filled(
        Rect::new(panel.x + pad, y, panel.w - pad * 2.0, divider_h),
        0.0,
        text(DIVIDER_ALPHA * alpha),
    );
    y += divider_h + OPEN_DIVIDER_GAP_BELOW * scale;

    text_r.queue(
        "Open",
        heading_font,
        panel.x + pad,
        y,
        text(HEADING_ALPHA * alpha),
        panel.w - pad * 2.0,
        surface_w,
        surface_h,
    );
    y += heading_font + heading_gap;

    let row_top = y;
    let cols = columns(panel.w, scale);
    let cell_h = tile_h + label_gap + label_font;
    let gap_y = OPEN_TILE_GAP_Y * scale;

    for (i, entry) in entries.iter().enumerate() {
        let r = tile_rect(panel, row_top, scale, i);
        let entry_mult = if entry.minimized { MINIMIZED_ALPHA_MULT } else { 1.0 };
        let entry_alpha = alpha * entry_mult;

        // Placeholder plate: subtle dark fill + faint border. The
        // compositor will (Phase B) paint a live thumbnail on top.
        painter.rect_filled(r, radius, text(PLACEHOLDER_BG_ALPHA * entry_alpha));
        painter.rect_stroke_sdf(r, radius, 1.0 * scale, text(PLACEHOLDER_BORDER_ALPHA * entry_alpha));

        // App icon centered on the placeholder so the user has a hint
        // of what's inside until live thumbs land.
        let icon_size = (tile_h.min(tile_w) * (1.0 - ICON_INSET_FRAC)).max(32.0 * scale);
        let icon_x = r.x + (tile_w - icon_size) / 2.0;
        let icon_y = r.y + (tile_h - icon_size) / 2.0;
        let icon_name = Some(lookup_icon_name(apps, &entry.app_id));
        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name,
            x: icon_x,
            y: icon_y,
            size: icon_size,
            opacity: entry_alpha * 0.85,
            clip: None,
        });

        // Selection / activated ring. Selection ring uses the un-dimmed
        // alpha so a focused minimized tile still reads as selected.
        let is_selected = selected == Some(i);
        if is_selected || entry.activated {
            let ring_alpha = if is_selected { 0.65 } else { 0.45 };
            let ring_base = if is_selected { alpha } else { entry_alpha };
            painter.rect_stroke_sdf(r, radius, 2.0 * scale, accent(ring_alpha * ring_base));
        }

        // Title label centered under the tile.
        let label = if entry.title.is_empty() {
            entry.app_id.clone()
        } else {
            entry.title.clone()
        };
        let label = truncate_label(&label, 28);
        let lw = text_r.measure_width(&label, label_font);
        let lx = r.x + (tile_w - lw) / 2.0;
        text_r.queue(
            &label,
            label_font,
            lx,
            r.y + tile_h + label_gap,
            text(LABEL_ALPHA * entry_alpha),
            tile_w + OPEN_TILE_GAP_X * scale,
            surface_w,
            surface_h,
        );
    }

    let rows = entries.len().div_ceil(cols);
    row_top + rows as f32 * cell_h + (rows.saturating_sub(1)) as f32 * gap_y
}

fn lookup_icon_name(apps: &AppsProvider, app_id: &str) -> String {
    for i in 0..apps.count() {
        if let Some(e) = apps.get(i) {
            if e.app_id.eq_ignore_ascii_case(app_id) {
                if let Some(n) = e.icon_name.as_ref() {
                    return n.clone();
                }
            }
        }
    }
    app_id.to_string()
}

fn truncate_label(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}
