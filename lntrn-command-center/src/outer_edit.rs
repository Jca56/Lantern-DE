//! Edit-mode chrome for the outer zones — drawn alongside the toolbar
//! edit overlay so one right-click unlocks both the in-bar widgets and
//! the floating chrome around the bar.
//!
//! Pure geometry + drawing; all `AppState` mutation lives in the click
//! and drag handlers, which call `outer_zones::{hit_widget,
//! resolve_drop}` directly.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::outer_zones::{self, OuterId, ALL_ZONES};
use crate::view_indicator::BTN_GAP;

const FONT: f32 = 15.0;
const GHOST_W: f32 = 140.0;
const GHOST_H: f32 = 36.0;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const PLATE_RGB: (u8, u8, u8) = (24, 24, 24);

/// An outer widget being dragged in edit mode.
#[derive(Debug, Clone)]
pub struct OuterDrag {
    pub id: OuterId,
    pub cursor: (f32, f32),
}

fn accent(a: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(a)
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    layout: &crate::outer_zones::OuterLayout,
    drag: &Option<OuterDrag>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    // Media is always present in edit mode so the card stays grabbable
    // even when nothing is playing.
    let positions = outer_zones::positions(layout, panel, scale, true);
    let dragged = drag.as_ref().map(|d| d.id);

    // Zone hint boxes. While dragging, zones the widget may land in
    // glow gold; disallowed ones go nearly invisible so the constraint
    // reads at a glance.
    for zone in ALL_ZONES {
        let r = outer_zones::zone_rect(zone, panel, scale);
        let (fill_a, stroke, label_a) = match dragged {
            Some(id) if id.allowed(zone) => (0.08, accent(0.45 * alpha), 0.5),
            Some(_) => (0.01, Color::rgba(1.0, 1.0, 1.0, 0.05 * alpha), 0.12),
            None => (0.03, Color::rgba(1.0, 1.0, 1.0, 0.14 * alpha), 0.3),
        };
        painter.rect_filled(r, 8.0 * scale, Color::rgba(1.0, 1.0, 1.0, fill_a * alpha));
        painter.rect_stroke_sdf(r, 8.0 * scale, 1.0 * scale, stroke);
        // Label empty zones so they read as drop targets.
        let empty = layout.zone(zone).is_empty();
        if empty {
            let f = FONT * scale;
            let label = zone.label();
            let w = text.measure_width(label, f);
            text.queue(
                label,
                f,
                r.x + (r.w - w) / 2.0,
                r.y + (r.h - f) / 2.0,
                Color::rgba(1.0, 1.0, 1.0, label_a * alpha),
                w + 8.0 * scale,
                surface_w,
                surface_h,
            );
        }
    }

    // Per-widget grab chrome. Buttons/dots keep drawing their real
    // glyphs underneath; the sliders and media card are hidden during
    // edit mode, so their slots get a labeled plate instead.
    for (id, r) in &positions {
        let is_dragged = dragged == Some(*id);
        let a = if is_dragged { alpha * 0.25 } else { alpha };
        if matches!(id, OuterId::Sliders | OuterId::Media) {
            painter.rect_filled(
                *r,
                10.0 * scale,
                Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2).with_alpha(0.75 * a),
            );
            let f = FONT * scale;
            let name = id.display_name();
            let w = text.measure_width(name, f);
            text.queue(
                name,
                f,
                r.x + (r.w - w) / 2.0,
                r.y + (r.h - f) / 2.0,
                Color::rgba(1.0, 1.0, 1.0, 0.85 * a),
                w + 8.0 * scale,
                surface_w,
                surface_h,
            );
        }
        painter.rect_stroke_sdf(
            *r,
            6.0 * scale,
            1.5 * scale,
            Color::rgba(1.0, 1.0, 1.0, 0.45 * a),
        );
    }

    // Drop caret + ghost for the active drag.
    if let Some(d) = drag {
        if let Some((zone, idx)) = outer_zones::resolve_drop(layout, panel, scale, d.id, d.cursor) {
            let region = outer_zones::zone_rect(zone, panel, scale);
            let others: Vec<Rect> = layout
                .zone(zone)
                .iter()
                .filter(|&&w| w != d.id)
                .filter_map(|&w| outer_zones::rect_in(&positions, w))
                .collect();
            let cx = caret_x(&others, idx, region, scale);
            painter.rect_filled(
                Rect::new(cx - 1.5 * scale, region.y, 3.0 * scale, region.h),
                1.5 * scale,
                accent(alpha),
            );
        }
        draw_ghost(painter, text, d, scale, alpha, surface_w, surface_h);
    }
}

fn caret_x(others: &[Rect], idx: usize, region: Rect, scale: f32) -> f32 {
    let g = BTN_GAP * scale;
    if others.is_empty() {
        region.x + region.w / 2.0
    } else if idx == 0 {
        others[0].x - g / 2.0
    } else if idx >= others.len() {
        let last = others[others.len() - 1];
        last.x + last.w + g / 2.0
    } else {
        (others[idx - 1].x + others[idx - 1].w + others[idx].x) / 2.0
    }
}

fn draw_ghost(
    painter: &mut Painter,
    text: &mut TextRenderer,
    drag: &OuterDrag,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let w = GHOST_W * scale;
    let h = GHOST_H * scale;
    let r = Rect::new(drag.cursor.0 - w / 2.0, drag.cursor.1 - h / 2.0, w, h);
    painter.rect_filled(
        r,
        h * 0.3,
        Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2).with_alpha(0.92 * alpha),
    );
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
