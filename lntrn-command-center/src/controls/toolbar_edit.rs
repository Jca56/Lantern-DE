//! Toolbar "customize" / edit mode: drag widgets between the Left /
//! Middle / Right zones, reorder within a zone, and toggle widgets on/off
//! via a disabled tray that floats below the bar.
//!
//! This module is pure geometry + drawing — all `AppState` mutation lives
//! in the click / drag handlers, which call the `hit_*` + `resolve_drop`
//! helpers here.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::app::PanelView;
use super::tile::{TileLayout, Zone};
use super::{Controls, TileId, ROW_HEIGHT, ROW_HORIZONTAL_PAD, ROW_TOP_MARGIN};
use super::tile::TILE_GAP;

// ── tunables (logical px) ─────────────────────────────────────────────────────
const TRAY_W: f32 = 780.0;
const TRAY_H: f32 = 60.0;
const TRAY_TOP_GAP: f32 = 12.0;
const TRAY_PAD: f32 = 14.0;
const TRAY_RADIUS: f32 = 14.0;
const CHIP_W: f32 = 108.0;
const CHIP_H: f32 = 32.0;
const CHIP_GAP: f32 = 8.0;
const LABEL_W: f32 = 86.0;
const DONE_W: f32 = 92.0;
const BADGE: f32 = 18.0;
const GHOST_W: f32 = 132.0;
const GHOST_H: f32 = 36.0;
const FONT: f32 = 15.0;

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const PLATE_RGB: (u8, u8, u8) = (24, 24, 24);

/// A widget being dragged in edit mode.
#[derive(Debug, Clone)]
pub struct WidgetDrag {
    pub id: TileId,
    /// True if the drag started from the disabled tray (vs a zone).
    pub from_tray: bool,
    pub cursor: (f32, f32),
}

/// Where a released drag lands.
#[derive(Debug, Clone, Copy)]
pub enum DropKind {
    Zone(Zone, usize),
    Disable,
}

// ── small geometry helpers ────────────────────────────────────────────────────

fn rect_of(l: TileLayout) -> Rect {
    Rect::new(l.x, l.y, l.w, l.h)
}

fn hit(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

fn row_band(panel: Rect, scale: f32) -> (f32, f32) {
    let top = panel.y + ROW_TOP_MARGIN * scale;
    (top, top + ROW_HEIGHT * scale)
}

/// Content x-range available to the three zones, excluding the pinned
/// Collapse chevron at the far right.
fn content_bounds(layouts: &[(TileId, TileLayout)], panel: Rect, scale: f32) -> (f32, f32) {
    let left = panel.x + ROW_HORIZONTAL_PAD * scale;
    let right = layouts
        .iter()
        .find(|(id, _)| *id == TileId::Collapse)
        .map(|(_, l)| l.x - TILE_GAP * scale)
        .unwrap_or(panel.x + panel.w - ROW_HORIZONTAL_PAD * scale);
    (left, right.max(left + 1.0))
}

fn zone_of_x(content_l: f32, content_r: f32, x: f32) -> Zone {
    let third = (content_r - content_l) / 3.0;
    if x < content_l + third {
        Zone::Left
    } else if x < content_l + third * 2.0 {
        Zone::Middle
    } else {
        Zone::Right
    }
}

fn zone_region(content_l: f32, content_r: f32, zone: Zone, panel: Rect, scale: f32) -> Rect {
    let third = (content_r - content_l) / 3.0;
    let (top, bot) = row_band(panel, scale);
    let x = match zone {
        Zone::Left => content_l,
        Zone::Middle => content_l + third,
        Zone::Right => content_l + third * 2.0,
    };
    Rect::new(x, top, third, bot - top)
}

/// Widgets currently shown in `zone`, in toolbar order, excluding
/// `skip` and any not-present widget, paired with their live layout.
fn zone_widgets(
    controls: &Controls,
    layouts: &[(TileId, TileLayout)],
    zone: Zone,
    skip: TileId,
) -> Vec<TileLayout> {
    controls
        .toolbar
        .zone(zone)
        .iter()
        .filter(|&&id| id != skip && controls.widget_present(id))
        .filter_map(|id| layouts.iter().find(|(wid, _)| wid == id).map(|(_, l)| *l))
        .collect()
}

fn insertion_index(zone_w: &[TileLayout], cursor_x: f32) -> usize {
    zone_w.iter().filter(|l| l.x + l.w / 2.0 < cursor_x).count()
}

fn caret_x(zone_w: &[TileLayout], idx: usize, region: Rect, scale: f32) -> f32 {
    let g = TILE_GAP * scale;
    if zone_w.is_empty() {
        region.x + region.w / 2.0
    } else if idx == 0 {
        zone_w[0].x - g / 2.0
    } else if idx >= zone_w.len() {
        let last = zone_w[zone_w.len() - 1];
        last.x + last.w + g / 2.0
    } else {
        (zone_w[idx - 1].x + zone_w[idx - 1].w + zone_w[idx].x) / 2.0
    }
}

// ── tray geometry ─────────────────────────────────────────────────────────────

pub fn tray_rect(panel: Rect, scale: f32) -> Rect {
    let w = TRAY_W * scale;
    let h = TRAY_H * scale;
    let x = panel.x + (panel.w - w) / 2.0;
    // Below the bottom outer-zone band so the tray doesn't cover the
    // sliders/media drop targets while editing.
    let band = crate::outer_zones::BOTTOM_BAND_GAP + crate::outer_zones::BOTTOM_BAND_H;
    let y = panel.y + panel.h + (band + TRAY_TOP_GAP) * scale;
    Rect::new(x, y, w, h)
}

fn done_rect(panel: Rect, scale: f32) -> Rect {
    let tray = tray_rect(panel, scale);
    let pad = TRAY_PAD * scale;
    let w = DONE_W * scale;
    let h = CHIP_H * scale;
    Rect::new(tray.x + tray.w - pad - w, tray.y + (tray.h - h) / 2.0, w, h)
}

/// Present, disabled widgets and their chip rects inside the tray.
fn tray_chips(controls: &Controls, panel: Rect, scale: f32) -> Vec<(TileId, Rect)> {
    let tray = tray_rect(panel, scale);
    let pad = TRAY_PAD * scale;
    let chip_w = CHIP_W * scale;
    let chip_h = CHIP_H * scale;
    let gap = CHIP_GAP * scale;
    let mut x = tray.x + pad + LABEL_W * scale;
    let y = tray.y + (tray.h - chip_h) / 2.0;
    let limit = done_rect(panel, scale).x - gap;
    let mut out = Vec::new();
    for &id in &controls.toolbar.disabled {
        if !controls.widget_present(id) {
            continue;
        }
        if x + chip_w > limit {
            break; // out of room — rare; overflow chips are clipped.
        }
        out.push((id, Rect::new(x, y, chip_w, chip_h)));
        x += chip_w + gap;
    }
    out
}

fn close_badge_rect(layout: TileLayout, scale: f32) -> Rect {
    let s = BADGE * scale;
    Rect::new(layout.x + layout.w - s - 2.0 * scale, layout.y + 2.0 * scale, s, s)
}

// ── hit tests (called from click handler) ─────────────────────────────────────

pub fn hit_done(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    hit(done_rect(panel, scale), px, py)
}

pub fn hit_close_badge(controls: &Controls, panel: Rect, scale: f32, px: f32, py: f32) -> Option<TileId> {
    for (id, layout) in controls.widget_layouts(panel, scale, PanelView::Default) {
        if id == TileId::Collapse {
            continue;
        }
        if hit(close_badge_rect(layout, scale), px, py) {
            return Some(id);
        }
    }
    None
}

pub fn hit_tray_chip(controls: &Controls, panel: Rect, scale: f32, px: f32, py: f32) -> Option<TileId> {
    tray_chips(controls, panel, scale)
        .into_iter()
        .find(|(_, r)| hit(*r, px, py))
        .map(|(id, _)| id)
}

pub fn hit_widget(controls: &Controls, panel: Rect, scale: f32, px: f32, py: f32) -> Option<TileId> {
    for (id, layout) in controls.widget_layouts(panel, scale, PanelView::Default) {
        if id == TileId::Collapse {
            continue;
        }
        if hit(rect_of(layout), px, py) {
            return Some(id);
        }
    }
    None
}

/// Resolve where a released drag should land. `None` = cancel (no change).
pub fn resolve_drop(
    controls: &Controls,
    panel: Rect,
    scale: f32,
    drag: &WidgetDrag,
    cursor: (f32, f32),
) -> Option<DropKind> {
    let (cx, cy) = cursor;
    // Tray = disable target.
    if hit(tray_rect(panel, scale), cx, cy) {
        return if drag.from_tray { None } else { Some(DropKind::Disable) };
    }
    // Row band → a zone + insertion index.
    let (top, bot) = row_band(panel, scale);
    if cy >= top && cy <= bot {
        let layouts = controls.widget_layouts(panel, scale, PanelView::Default);
        let (cl, cr) = content_bounds(&layouts, panel, scale);
        let zone = zone_of_x(cl, cr, cx);
        let zw = zone_widgets(controls, &layouts, zone, drag.id);
        return Some(DropKind::Zone(zone, insertion_index(&zw, cx)));
    }
    None
}

// ── drawing ───────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    controls: &Controls,
    drag: &Option<WidgetDrag>,
    settings_open: Option<TileId>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let layouts = controls.widget_layouts(panel, scale, PanelView::Default);
    let (cl, cr) = content_bounds(&layouts, panel, scale);
    let dragged = drag.as_ref().map(|d| d.id);

    // Faint zone dividers + a label in any empty zone.
    let (top, bot) = row_band(panel, scale);
    let third = (cr - cl) / 3.0;
    let div = Color::rgba(1.0, 1.0, 1.0, 0.14 * alpha);
    for k in 1..3 {
        let x = cl + third * k as f32;
        painter.rect_filled(Rect::new(x - 0.5 * scale, top, 1.0 * scale, bot - top), 0.0, div);
    }
    for (i, zone) in [Zone::Left, Zone::Middle, Zone::Right].into_iter().enumerate() {
        if controls.toolbar.zone(zone).iter().any(|&id| controls.widget_present(id)) {
            continue; // only label empty zones
        }
        let region = Rect::new(cl + third * i as f32, top, third, bot - top);
        let label = match zone {
            Zone::Left => "LEFT",
            Zone::Middle => "MIDDLE",
            Zone::Right => "RIGHT",
        };
        let f = FONT * scale;
        let w = text.measure_width(label, f);
        text.queue(
            label,
            f,
            region.x + (region.w - w) / 2.0,
            region.y + (region.h - f) / 2.0,
            Color::rgba(1.0, 1.0, 1.0, 0.3 * alpha),
            w + 8.0 * scale,
            surface_w,
            surface_h,
        );
    }

    // Per-widget edit chrome: a grab outline + ✕ remove badge. The widget
    // being dragged is dimmed in place.
    for (id, layout) in &layouts {
        if *id == TileId::Collapse {
            continue;
        }
        let is_dragged = dragged == Some(*id) && drag.as_ref().map(|d| !d.from_tray).unwrap_or(false);
        let a = if is_dragged { alpha * 0.25 } else { alpha };
        painter.rect_stroke_sdf(rect_of(*layout), 6.0 * scale, 1.5 * scale, Color::rgba(1.0, 1.0, 1.0, 0.45 * a));
        draw_gear_badge(painter, text, gear_badge_rect(*layout, scale), a, scale, surface_w, surface_h);
        draw_close_badge(painter, close_badge_rect(*layout, scale), a, scale);
    }

    // Drop indicator for the active drag.
    if let Some(d) = drag {
        if let Some(kind) = resolve_drop(controls, panel, scale, d, d.cursor) {
            match kind {
                DropKind::Zone(zone, idx) => {
                    let region = zone_region(cl, cr, zone, panel, scale);
                    painter.rect_filled(region, 6.0 * scale, Color::rgba(ACCENT_RGB.0 as f32 / 255.0, ACCENT_RGB.1 as f32 / 255.0, ACCENT_RGB.2 as f32 / 255.0, 0.12 * alpha));
                    let zw = zone_widgets(controls, &layouts, zone, d.id);
                    let cx = caret_x(&zw, idx, region, scale);
                    painter.rect_filled(
                        Rect::new(cx - 1.5 * scale, top, 3.0 * scale, bot - top),
                        1.5 * scale,
                        accent(alpha),
                    );
                }
                DropKind::Disable => {
                    let tray = tray_rect(panel, scale);
                    painter.rect_stroke_sdf(tray, TRAY_RADIUS * scale, 2.0 * scale, accent(0.9 * alpha));
                }
            }
        }
    }

    // Either the per-widget settings popover (if one is open) or the
    // disabled tray. The popover takes over the below-bar space.
    if let Some(id) = settings_open {
        let opts = controls.toolbar.opts(id);
        super::widget_settings::draw(painter, text, id, &opts, controls, panel, scale, alpha, surface_w, surface_h);
    } else {
        draw_tray(painter, text, controls, drag, panel, scale, alpha, surface_w, surface_h);
    }

    // Ghost following the cursor.
    if let Some(d) = drag {
        draw_ghost(painter, text, d, scale, alpha, surface_w, surface_h);
    }
}

/// Gear (settings) badge — sits just left of the ✕ remove badge.
fn gear_badge_rect(layout: TileLayout, scale: f32) -> Rect {
    let close = close_badge_rect(layout, scale);
    Rect::new(close.x - close.w - 3.0 * scale, close.y, close.w, close.h)
}

/// The widget whose ⚙ badge is under the cursor, if any.
pub fn hit_gear_badge(controls: &Controls, panel: Rect, scale: f32, px: f32, py: f32) -> Option<TileId> {
    for (id, layout) in controls.widget_layouts(panel, scale, PanelView::Default) {
        if id == TileId::Collapse {
            continue;
        }
        if hit(gear_badge_rect(layout, scale), px, py) {
            return Some(id);
        }
    }
    None
}

fn draw_gear_badge(
    painter: &mut Painter,
    text: &mut TextRenderer,
    r: Rect,
    alpha: f32,
    scale: f32,
    surface_w: u32,
    surface_h: u32,
) {
    painter.circle_filled(r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, accent(0.92 * alpha));
    let glyph = "⚙";
    let f = r.h * 0.78;
    let w = text.measure_width(glyph, f);
    let _ = scale;
    text.queue(
        glyph,
        f,
        r.x + (r.w - w) / 2.0,
        r.y + (r.h - f) / 2.0,
        Color::from_rgb8(20, 16, 6).with_alpha(alpha),
        r.w,
        surface_w,
        surface_h,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_tray(
    painter: &mut Painter,
    text: &mut TextRenderer,
    controls: &Controls,
    drag: &Option<WidgetDrag>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let tray = tray_rect(panel, scale);
    painter.shadow(tray, TRAY_RADIUS * scale, 18.0 * scale, Color::BLACK.with_alpha(0.35 * alpha), 0.0, 4.0 * scale);
    painter.rect_filled(tray, TRAY_RADIUS * scale, Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2).with_alpha(0.85 * alpha));
    painter.rect_stroke_sdf(tray, TRAY_RADIUS * scale, 1.0 * scale, Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha));

    let pad = TRAY_PAD * scale;
    let f = FONT * scale;
    let chips = tray_chips(controls, panel, scale);
    let label = if chips.is_empty() {
        "Drag a widget here to hide it"
    } else {
        "Disabled"
    };
    text.queue(
        label,
        f,
        tray.x + pad,
        tray.y + (tray.h - f) / 2.0,
        Color::rgba(1.0, 1.0, 1.0, 0.55 * alpha),
        LABEL_W * scale + 40.0 * scale,
        surface_w,
        surface_h,
    );

    let dragged_tray = drag.as_ref().filter(|d| d.from_tray).map(|d| d.id);
    for (id, r) in &chips {
        let a = if dragged_tray == Some(*id) { alpha * 0.25 } else { alpha };
        draw_chip(painter, text, id.display_name(), *r, scale, a, surface_w, surface_h);
    }

    // Done pill (gold).
    let done = done_rect(panel, scale);
    painter.rect_filled(done, done.h * 0.5, accent(alpha));
    let dl = "Done";
    let dw = text.measure_width(dl, f);
    text.queue(
        dl,
        f,
        done.x + (done.w - dw) / 2.0,
        done.y + (done.h - f) / 2.0,
        Color::from_rgb8(20, 16, 6).with_alpha(alpha),
        dw + 8.0 * scale,
        surface_w,
        surface_h,
    );
}

fn draw_chip(
    painter: &mut Painter,
    text: &mut TextRenderer,
    name: &str,
    r: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    painter.rect_filled(r, r.h * 0.3, Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha));
    painter.rect_stroke_sdf(r, r.h * 0.3, 1.0 * scale, Color::rgba(1.0, 1.0, 1.0, 0.18 * alpha));
    let f = FONT * scale;
    let w = text.measure_width(name, f).min(r.w - 12.0 * scale);
    text.queue(
        name,
        f,
        r.x + (r.w - w) / 2.0,
        r.y + (r.h - f) / 2.0,
        Color::rgba(1.0, 1.0, 1.0, 0.92 * alpha),
        r.w - 8.0 * scale,
        surface_w,
        surface_h,
    );
}

fn draw_ghost(
    painter: &mut Painter,
    text: &mut TextRenderer,
    drag: &WidgetDrag,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let w = GHOST_W * scale;
    let h = GHOST_H * scale;
    let r = Rect::new(drag.cursor.0 - w / 2.0, drag.cursor.1 - h / 2.0, w, h);
    painter.rect_filled(r, h * 0.3, Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2).with_alpha(0.92 * alpha));
    painter.rect_stroke_sdf(r, h * 0.3, 1.5 * scale, accent(alpha));
    let name = drag.id.display_name();
    let f = FONT * scale;
    let tw = text.measure_width(name, f);
    text.queue(
        name,
        f,
        r.x + (r.w - tw) / 2.0,
        r.y + (r.h - f) / 2.0,
        Color::rgba(1.0, 1.0, 1.0, alpha),
        r.w,
        surface_w,
        surface_h,
    );
}

fn draw_close_badge(painter: &mut Painter, r: Rect, alpha: f32, scale: f32) {
    painter.circle_filled(r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, Color::rgba(0.85, 0.18, 0.18, 0.9 * alpha));
    // × glyph (two strokes).
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let s = r.w * 0.24;
    let col = Color::rgba(1.0, 1.0, 1.0, alpha);
    painter.line_round(cx - s, cy - s, cx + s, cy + s, 1.6 * scale, col);
    painter.line_round(cx - s, cy + s, cx + s, cy - s, 1.6 * scale, col);
}

fn accent(a: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(a)
}
