//! Decorative lantern mascot — sits OUTSIDE the panel on the left,
//! bottom-aligned with the panel's bottom edge. Visible only while
//! the panel is expanded enough to be a window (fades with collapse).
//! Purely visual: no hover, no click, no hit-test.
//!
//! The source PNG at `~/.lantern/icons/lantern-mascot.png` is padded
//! to a 1000×1000 square so it rasterizes cleanly through the shared
//! icon cache (which stores square textures). The lantern itself
//! occupies the centred ~528×1000 column; the rest is transparent.

use lntrn_render::Rect;

use crate::render::IconRequest;

/// Logical-px height of the visible lantern. The square tile drawn
/// is `HEIGHT × HEIGHT`; the lantern lives in the middle of it.
pub const HEIGHT: f32 = 120.0;

/// Lantern aspect (width / height) from the source PNG.
const ASPECT: f32 = 528.0 / 1000.0;

/// Gap (logical px) between the lantern's right edge and the panel's
/// left edge.
const RIGHT_GAP: f32 = 8.0;

/// Bottom inset (logical px) so the lantern's base lines up just
/// slightly above the panel's bottom edge.
const BOTTOM_INSET: f32 = 0.0;

const CACHE_KEY: &str = "__mascot_lantern";
const ICON_NAME: &str = "lantern-mascot";

/// Push an IconRequest for the lantern mascot next to the expanded
/// panel. No-op when CC is collapsed — the dock-side mascot covers
/// that state (see [`draw_beside_dock`]).
pub fn draw(
    icons: &mut Vec<IconRequest>,
    panel: Rect,
    scale: f32,
    collapse_progress: f32,
    alpha: f32,
) {
    let expanded_alpha = (1.0 - collapse_progress).clamp(0.0, 1.0);
    let a = alpha * expanded_alpha;
    if a < 0.005 {
        return;
    }
    push_request(
        icons,
        scale,
        a,
        panel.x - RIGHT_GAP * scale,
        panel.y + panel.h - BOTTOM_INSET * scale,
    );
}

/// Push an IconRequest for the dock-side lantern: anchored just to
/// the left of the mini-dock plate, bottom-aligned with the plate.
pub fn draw_beside_dock(icons: &mut Vec<IconRequest>, plate: Rect, scale: f32, alpha: f32) {
    if alpha < 0.005 {
        return;
    }
    push_request(
        icons,
        scale,
        alpha,
        plate.x - RIGHT_GAP * scale,
        plate.y + plate.h,
    );
}

/// Shared placement: lantern's right edge lands at `right_x` and the
/// bottom of the lantern sits at `bottom_y`. The square tile is
/// re-centered so the visible lantern column lines up correctly.
fn push_request(icons: &mut Vec<IconRequest>, scale: f32, alpha: f32, right_x: f32, bottom_y: f32) {
    let tile = HEIGHT * scale;
    let visible_w = tile * ASPECT;
    let tile_x = right_x - (tile + visible_w) / 2.0;
    let tile_y = bottom_y - tile;
    icons.push(IconRequest {
        app_id: CACHE_KEY.into(),
        icon_name: Some(ICON_NAME.into()),
        x: tile_x,
        y: tile_y,
        size: tile,
        opacity: alpha,
        clip: None,
    });
}
