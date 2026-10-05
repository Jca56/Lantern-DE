//! Drag-to-reorder for the dock's pinned section. A press on a pinned
//! icon parks a drag candidate on `AppState` (`dock_drag`); once the
//! cursor has moved past the threshold the icon lifts — its slot dims,
//! a ghost follows the cursor, and a gold marker shows where it lands.
//! Running-only icons have no order of their own and can't be dragged.
//!
//! Everything here reads the frame's `DockLayout`, magnification
//! included, so the marker, the drop slot, and what's on screen agree.

use lntrn_render::{Color, Painter, Rect};

use super::{DockLayout, ACCENT_RGB, ICON_SIZE, MAG_PEAK};
use crate::app::PinDrag;
use crate::render::IconRequest;

/// Opacity of the dragged icon's own slot while its ghost is in the air.
pub(super) const LIFTED_ALPHA: f32 = 0.3;
const GHOST_ALPHA: f32 = 0.85;
/// Drop-marker pill width (logical px).
const MARKER_W: f32 = 4.0;
const MARKER_ALPHA: f32 = 0.9;

/// Drop-target index (0..=`pinned_count`) for a drag whose cursor is at
/// physical-pixel x `px`: an insertion point, counted as the number of
/// pinned icons whose centre lies left of the cursor.
pub fn drop_slot(layout: &DockLayout, px: f32) -> usize {
    pinned_icons(layout)
        .iter()
        .filter(|r| r.x + r.w / 2.0 < px)
        .count()
}

fn pinned_icons(layout: &DockLayout) -> &[Rect] {
    &layout.icons[..layout.pinned_count.min(layout.icons.len())]
}

/// Horizontal centre of the gap that insertion point `slot` falls in.
/// The dock's ends count as gaps too: the marker sits in the plate's
/// padding (or on the tray divider) instead of on the plate's edge.
fn marker_center_x(layout: &DockLayout, slot: usize) -> f32 {
    let plate = layout.plate;
    let left = match slot.checked_sub(1).and_then(|i| layout.icons.get(i)) {
        Some(r) => r.x + r.w,
        None => plate.x,
    };
    let right = match layout.icons.get(slot) {
        Some(r) => r.x,
        None => layout
            .tray_icons
            .first()
            .map(|r| r.x)
            .unwrap_or(plate.x + plate.w),
    };
    (left + right) / 2.0
}

/// Marker + ghost for a drag that has started. The caller (`draw`) has
/// already checked `drag.from_idx` is a pinned slot.
pub(super) fn draw_overlay(
    painter: &mut Painter,
    icons: &mut Vec<IconRequest>,
    layout: &DockLayout,
    drag: &PinDrag,
    alpha: f32,
) {
    let scale = layout.scale;
    let plate = layout.plate;
    let accent = Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2);

    let slot = drop_slot(layout, drag.current_x);
    let marker_w = MARKER_W * scale;
    let inset = 4.0 * scale;
    painter.rect_filled(
        Rect::new(
            marker_center_x(layout, slot) - marker_w / 2.0,
            plate.y + inset,
            marker_w,
            plate.h - inset * 2.0,
        ),
        marker_w * 0.5,
        accent.with_alpha(MARKER_ALPHA * alpha),
    );

    // Ghost at the size the icon had under the cursor when it was
    // grabbed (fully magnified), so lifting it doesn't change its size.
    let entry = &layout.entries[drag.from_idx];
    let ghost = ICON_SIZE * MAG_PEAK * scale;
    icons.push(IconRequest {
        app_id: entry.app_id.clone(),
        icon_name: entry.icon_name.clone(),
        x: drag.current_x - ghost / 2.0,
        y: drag.current_y - ghost / 2.0,
        size: ghost,
        opacity: GHOST_ALPHA * alpha,
        clip: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three pinned icons + one running one, 10 px wide on a 20 px pitch.
    fn layout() -> DockLayout {
        DockLayout {
            entries: Vec::new(),
            plate: Rect::new(0.0, 0.0, 90.0, 20.0),
            icons: (0..4)
                .map(|i| Rect::new(10.0 + i as f32 * 20.0, 5.0, 10.0, 10.0))
                .collect(),
            pinned_count: 3,
            tray: Vec::new(),
            tray_icons: Vec::new(),
            scale: 1.0,
        }
    }

    #[test]
    fn drop_slot_counts_pinned_centres_left_of_cursor() {
        let l = layout();
        assert_eq!(drop_slot(&l, 0.0), 0);
        assert_eq!(drop_slot(&l, 14.0), 0); // left half of icon 0
        assert_eq!(drop_slot(&l, 16.0), 1); // right half of icon 0
        assert_eq!(drop_slot(&l, 40.0), 2);
        // Past the pinned section — the running icon is never a target.
        assert_eq!(drop_slot(&l, 500.0), 3);
    }

    #[test]
    fn marker_sits_between_neighbours_and_inside_the_plate() {
        let l = layout();
        assert_eq!(marker_center_x(&l, 0), 5.0); // plate edge ↔ icon 0
        assert_eq!(marker_center_x(&l, 1), 25.0); // icon 0 ↔ icon 1
        assert_eq!(marker_center_x(&l, 3), 65.0); // icon 2 ↔ running icon
        let mut only_pinned = layout();
        only_pinned.icons.truncate(3);
        assert_eq!(marker_center_x(&only_pinned, 3), 75.0); // icon 2 ↔ plate edge
    }
}
