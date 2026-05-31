//! The editor "page": the centered writable sheet whose width the user
//! controls by dragging its side margins. Owns the geometry math — shared by
//! the renderer, click hit-testing, and the drag logic so they always agree
//! on where the page edges are — plus the margin-handle affordance, keeping
//! `render.rs` focused on laying out text.

use lntrn_render::{Painter, Rect};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, InteractionState};

use crate::{ZONE_PAGE_HANDLE_L, ZONE_PAGE_HANDLE_R};

/// Side margin (logical px, each side) kept between the page and the window
/// edge when the writable area is dragged out to its full width.
pub const COMFY_MARGIN: f32 = 40.0;
/// Minimum writable-area width (logical px).
pub const MIN_PAGE_W: f32 = 480.0;
/// Width (logical px) of the invisible drag band sitting in the gutter at
/// each page edge.
pub const HANDLE_W: f32 = 16.0;

/// Compute the centered page geometry for the editor body from the width
/// fraction (0.0 = `MIN_PAGE_W`, 1.0 = full width minus `COMFY_MARGIN` each
/// side). Returns `(page_x, page_w)` in physical pixels.
pub fn geometry(er: Rect, frac: f32, s: f32) -> (f32, f32) {
    let max_w = (er.w - 2.0 * COMFY_MARGIN * s).max(10.0);
    let min_w = (MIN_PAGE_W * s).min(max_w);
    let page_w = (min_w + frac.clamp(0.0, 1.0) * (max_w - min_w)).min(er.w);
    let page_x = er.x + (er.w - page_w) * 0.5;
    (page_x, page_w)
}

/// Register the two margin-drag bands (one per page edge, sitting in the
/// gutter) and paint a subtle grip affordance that brightens on hover / drag.
///
/// Call AFTER `ZONE_EDITOR` is registered so the bands win the hit-test where
/// they overlap the editor body.
pub fn draw_handles(
    input: &mut InteractionContext,
    painter: &mut Painter,
    pal: &FoxPalette,
    er: Rect,
    page_x: f32,
    page_w: f32,
    s: f32,
) {
    let handle_w = HANDLE_W * s;
    let l_rect = Rect::new((page_x - handle_w).max(er.x), er.y, handle_w, er.h);
    let r_rect = Rect::new(
        page_x + page_w,
        er.y,
        handle_w.min(er.x + er.w - (page_x + page_w)).max(0.0),
        er.h,
    );
    let l_state = input.add_zone(ZONE_PAGE_HANDLE_L, l_rect);
    let r_state = input.add_zone(ZONE_PAGE_HANDLE_R, r_rect);

    let grip_h = (er.h * 0.18).clamp(40.0 * s, 140.0 * s);
    let grip_w = 5.0 * s;
    let grip_y = er.y + (er.h - grip_h) * 0.5;
    let grip_color = |state: InteractionState| {
        if state.is_active() {
            pal.accent
        } else if state.is_hovered() {
            pal.accent.with_alpha(0.75)
        } else {
            pal.muted.with_alpha(0.55)
        }
    };
    painter.rect_filled(
        Rect::new(l_rect.x + (l_rect.w - grip_w) * 0.5, grip_y, grip_w, grip_h),
        grip_w * 0.5,
        grip_color(l_state),
    );
    painter.rect_filled(
        Rect::new(r_rect.x + (r_rect.w - grip_w) * 0.5, grip_y, grip_w, grip_h),
        grip_w * 0.5,
        grip_color(r_state),
    );
}
