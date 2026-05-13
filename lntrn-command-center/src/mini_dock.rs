//! Mini-dock of pinned apps that floats just under the panel while it
//! is collapsed (or animating into collapse). Click an icon to launch.
//!
//! Lives outside the panel rect so the panel can stay tiny while these
//! shortcuts remain a single click away.

use lntrn_render::{Color, Painter, Rect};

use crate::render::IconRequest;
use crate::search::apps::{AppsProvider, DesktopEntry};

/// Icon side length (logical px).
pub const ICON_SIZE: f32 = 56.0;
/// Gap between icons (logical px).
pub const ICON_GAP: f32 = 12.0;
/// Vertical gap from the panel's bottom edge to the dock.
pub const TOP_GAP: f32 = 16.0;
/// Plate corner radius and padding around the icons.
pub const PLATE_RADIUS: f32 = 18.0;
pub const PLATE_PAD: f32 = 10.0;

const PLATE_RGB: (u8, u8, u8) = (24, 24, 24);
const PLATE_ALPHA: f32 = 0.85;
const PLATE_BORDER_ALPHA: f32 = 0.08;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

/// Compute the plate rect (rounded background behind the icons) for
/// the given pin count. Returns None when there are no pins.
pub fn plate_rect(panel: Rect, scale: f32, count: usize) -> Option<Rect> {
    if count == 0 {
        return None;
    }
    let icon = ICON_SIZE * scale;
    let gap = ICON_GAP * scale;
    let pad = PLATE_PAD * scale;
    let top_gap = TOP_GAP * scale;
    let icons_w = count as f32 * icon + (count as f32 - 1.0) * gap;
    let plate_w = icons_w + pad * 2.0;
    let plate_h = icon + pad * 2.0;
    let center_x = panel.x + panel.w / 2.0;
    let x = center_x - plate_w / 2.0;
    let y = panel.y + panel.h + top_gap;
    Some(Rect::new(x, y, plate_w, plate_h))
}

/// Rect of the i-th icon (in physical px). `count` should be the total
/// number of icons in the dock so the layout matches `plate_rect`.
pub fn icon_rect(panel: Rect, scale: f32, count: usize, idx: usize) -> Option<Rect> {
    if idx >= count {
        return None;
    }
    let plate = plate_rect(panel, scale, count)?;
    let icon = ICON_SIZE * scale;
    let gap = ICON_GAP * scale;
    let pad = PLATE_PAD * scale;
    let x = plate.x + pad + idx as f32 * (icon + gap);
    let y = plate.y + pad;
    Some(Rect::new(x, y, icon, icon))
}

/// Hit-test the dock. Returns the pin index under (px, py) if any.
pub fn hit_test(panel: Rect, scale: f32, count: usize, px: f32, py: f32) -> Option<usize> {
    for i in 0..count {
        let r = icon_rect(panel, scale, count, i)?;
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            return Some(i);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    icons: &mut Vec<IconRequest>,
    pinned: &[&DesktopEntry],
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered_idx: Option<usize>,
    _apps: &AppsProvider,
) {
    let Some(plate) = plate_rect(panel, scale, pinned.len()) else { return };
    let radius = PLATE_RADIUS * scale;

    // Plate background + faint border.
    painter.rect_filled(
        plate,
        radius,
        Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2)
            .with_alpha(PLATE_ALPHA * alpha),
    );
    painter.rect_stroke_sdf(
        plate,
        radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(PLATE_BORDER_ALPHA * alpha),
    );

    // Icons.
    for (i, entry) in pinned.iter().enumerate() {
        let Some(r) = icon_rect(panel, scale, pinned.len(), i) else { continue };

        // Hover ring around the icon (accent gold).
        if hovered_idx == Some(i) {
            painter.rect_stroke_sdf(
                Rect::new(r.x - 4.0 * scale, r.y - 4.0 * scale, r.w + 8.0 * scale, r.h + 8.0 * scale),
                (r.w * 0.4) + 4.0 * scale,
                2.0 * scale,
                Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.65 * alpha),
            );
        }

        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name: entry.icon_name.clone(),
            x: r.x,
            y: r.y,
            size: r.w,
            opacity: alpha,
            clip: None,
        });
    }
}
