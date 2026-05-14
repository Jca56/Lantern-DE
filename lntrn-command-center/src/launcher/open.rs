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

/// 16:9 thumbnail tile dimensions (logical px). The whole tile is the
/// thumbnail area now — the close button is rendered by the
/// compositor on top of the thumb (no toolbar strip).
pub const OPEN_TILE_W: f32 = 256.0;
pub const OPEN_TILE_H: f32 = 144.0;
/// Deprecated — kept as 0 so existing math falls through cleanly. The
/// close button is now rendered by the compositor over the thumbnail
/// so no strip needs to be carved out of the thumbnail rect.
pub const OPEN_TILE_TOOLBAR_H: f32 = 0.0;
pub const OPEN_TILE_GAP_X: f32 = 36.0;
pub const OPEN_TILE_GAP_Y: f32 = 36.0;
pub const OPEN_LABEL_FONT: f32 = 18.0;
pub const OPEN_LABEL_GAP: f32 = 10.0;
pub const OPEN_SECTION_TOP_MARGIN: f32 = 24.0;
pub const OPEN_HEADING_GAP: f32 = 12.0;
pub const OPEN_TILE_RADIUS: f32 = 12.0;
pub const OPEN_DIVIDER_GAP_BELOW: f32 = 18.0;
pub const OPEN_DIVIDER_THICKNESS: f32 = 1.0;

const HEADING_FONT: f32 = 14.0;
const HEADING_ALPHA: f32 = 0.55;
const LABEL_ALPHA: f32 = 0.85;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
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
    divider_h + OPEN_DIVIDER_GAP_BELOW * scale
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

/// One renderable Open-section tile, representing all windows of a
/// single app. The `windows` vec is in compositor-reported order (first
/// = oldest); cycling activation walks it.
#[derive(Debug, Clone)]
pub struct AppGroup<'a> {
    pub app_id: &'a str,
    pub windows: Vec<&'a ToplevelInfo>,
}

impl<'a> AppGroup<'a> {
    /// True when at least one window in the group is currently activated.
    pub fn any_activated(&self) -> bool {
        self.windows.iter().any(|w| w.activated)
    }
    /// True when *every* window in the group is minimized — the tile is
    /// dimmed in that case so the user can spot it as "all minimized".
    pub fn all_minimized(&self) -> bool {
        !self.windows.is_empty() && self.windows.iter().all(|w| w.minimized)
    }
    /// Choose the next window to activate on click. Cycles past whichever
    /// window is currently activated; falls back to the first window.
    pub fn next_to_activate(&self) -> Option<&'a ToplevelInfo> {
        if self.windows.is_empty() {
            return None;
        }
        if let Some(cur) = self.windows.iter().position(|w| w.activated) {
            Some(self.windows[(cur + 1) % self.windows.len()])
        } else {
            Some(self.windows[0])
        }
    }
    /// Pick which window the X button should close. Prefers the currently
    /// activated window; falls back to the first.
    pub fn close_target(&self) -> Option<&'a ToplevelInfo> {
        self.windows
            .iter()
            .find(|w| w.activated)
            .copied()
            .or_else(|| self.windows.first().copied())
    }
}

/// Toplevels shown in the Open section. Each *window* gets its own
/// tile — no grouping by app — so multi-window apps can be switched
/// between without cycling. Minimized windows are included (dimmed)
/// so users can find and restore them.
pub fn visible_entries(toplevels: &[ToplevelInfo]) -> Vec<AppGroup<'_>> {
    toplevels
        .iter()
        .filter(|t| !t.app_id.is_empty())
        .map(|t| AppGroup { app_id: t.app_id.as_str(), windows: vec![t] })
        .collect()
}

const MINIMIZED_ALPHA_MULT: f32 = 0.5;

// ── X close button geometry ─────────────────────────────────────────────────

/// X close button size and inset from the tile's top-right corner.
pub const CLOSE_BTN_SIZE: f32 = 22.0;
pub const CLOSE_BTN_INSET: f32 = 6.0;

/// Hit-rect for the X close button of the i-th tile (in physical px).
pub fn close_button_rect(panel: Rect, top_y: f32, scale: f32, idx: usize) -> Rect {
    let tile = tile_rect(panel, top_y, scale, idx);
    let size = CLOSE_BTN_SIZE * scale;
    let inset = CLOSE_BTN_INSET * scale;
    Rect::new(tile.x + tile.w - size - inset, tile.y + inset, size, size)
}

// ── Badge geometry ──────────────────────────────────────────────────────────

const BADGE_FONT: f32 = 14.0;
const BADGE_H: f32 = 22.0;
const BADGE_INSET: f32 = 6.0;

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
    let label_font = OPEN_LABEL_FONT * scale;
    let label_gap = OPEN_LABEL_GAP * scale;
    let tile_w = OPEN_TILE_W * scale;
    let tile_h = OPEN_TILE_H * scale;
    let radius = OPEN_TILE_RADIUS * scale;

    let mut y = top_y + OPEN_SECTION_TOP_MARGIN * scale;

    // Slim divider between the pinned row and the open thumbnails — no
    // header text any more; the thumbnails themselves are the section.
    let divider_h = (OPEN_DIVIDER_THICKNESS * scale).max(1.0);
    painter.rect_filled(
        Rect::new(panel.x + pad, y, panel.w - pad * 2.0, divider_h),
        0.0,
        text(DIVIDER_ALPHA * alpha),
    );
    y += divider_h + OPEN_DIVIDER_GAP_BELOW * scale;

    let row_top = y;
    let cols = columns(panel.w, scale);
    let cell_h = tile_h + label_gap + label_font;
    let gap_y = OPEN_TILE_GAP_Y * scale;

    for (i, group) in entries.iter().enumerate() {
        let r = tile_rect(panel, row_top, scale, i);
        let dim = group.all_minimized();
        let group_mult = if dim { MINIMIZED_ALPHA_MULT } else { 1.0 };
        let group_alpha = alpha * group_mult;
        let is_activated = group.any_activated();

        // No placeholder icon — the compositor fills the entire tile
        // with the live thumbnail and renders the close button overlay
        // on top.

        // Selection / activated ring.
        let is_selected = selected == Some(i);
        if is_selected || is_activated {
            let ring_alpha = if is_selected { 0.65 } else { 0.45 };
            let ring_base = if is_selected { alpha } else { group_alpha };
            painter.rect_stroke_sdf(r, radius, 2.0 * scale, accent(ring_alpha * ring_base));
        }

        // ×N counter badge top-left when grouped.
        if group.windows.len() > 1 {
            draw_count_badge(
                painter,
                text_r,
                r,
                group.windows.len(),
                scale,
                group_alpha,
                surface_w,
                surface_h,
            );
        }

        // The close button is rendered by the compositor on top of the
        // thumbnail (see `cc_thumbs` in lntrn-compositor) — nothing to
        // draw here.

        // Title label centered under the tile. When the group has one
        // window we show its title; with multiple we show the app name.
        let label = if group.windows.len() == 1 {
            let t = group.windows[0];
            if t.title.is_empty() { group.app_id.to_string() } else { t.title.clone() }
        } else {
            group.app_id.to_string()
        };
        let label = truncate_label(&label, 28);
        let lw = text_r.measure_width(&label, label_font);
        let lx = r.x + (tile_w - lw) / 2.0;
        text_r.queue(
            &label,
            label_font,
            lx,
            r.y + tile_h + label_gap,
            text(LABEL_ALPHA * group_alpha),
            tile_w + OPEN_TILE_GAP_X * scale,
            surface_w,
            surface_h,
        );
    }

    let rows = entries.len().div_ceil(cols);
    row_top + rows as f32 * cell_h + (rows.saturating_sub(1)) as f32 * gap_y
}

#[allow(clippy::too_many_arguments)]
fn draw_count_badge(
    painter: &mut Painter,
    text_r: &mut TextRenderer,
    tile: Rect,
    count: usize,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let label = format!("×{count}");
    let badge_font = BADGE_FONT * scale;
    let badge_h = BADGE_H * scale;
    let pad_x = 8.0 * scale;
    let lw = text_r.measure_width(&label, badge_font);
    let badge_w = lw + pad_x * 2.0;
    let inset = BADGE_INSET * scale;
    let bx = tile.x + inset;
    let by = tile.y + inset;
    let radius = badge_h * 0.5;
    painter.rect_filled(
        Rect::new(bx, by, badge_w, badge_h),
        radius,
        accent(0.85 * alpha),
    );
    text_r.queue(
        &label,
        badge_font,
        bx + pad_x,
        by + (badge_h - badge_font) / 2.0,
        Color::BLACK.with_alpha(alpha),
        lw,
        surface_w,
        surface_h,
    );
}

#[allow(dead_code)] // compositor renders the close button now
fn draw_close_button(painter: &mut Painter, tile: Rect, scale: f32, alpha: f32) {
    let size = CLOSE_BTN_SIZE * scale;
    let inset = CLOSE_BTN_INSET * scale;
    let bx = tile.x + tile.w - size - inset;
    let by = tile.y + inset;
    let bg = Color::BLACK.with_alpha(0.55 * alpha);
    painter.rect_filled(Rect::new(bx, by, size, size), size * 0.5, bg);

    // Draw a small X using two thin rounded lines.
    let cx = bx + size * 0.5;
    let cy = by + size * 0.5;
    let arm = size * 0.26;
    let stroke = (2.0 * scale).max(1.5);
    let xcol = text(0.92 * alpha);
    painter.line_round(cx - arm, cy - arm, cx + arm, cy + arm, stroke, xcol);
    painter.line_round(cx - arm, cy + arm, cx + arm, cy - arm, stroke, xcol);
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
