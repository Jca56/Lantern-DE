/// Layer surface positioning: computes where a layer-shell surface should
/// be placed based on its anchor edges, margins, and the output geometry.

use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::{Anchor, LayerSurfaceCachedState};

/// Effective size of a layer surface after resolving the "0 means fill the
/// anchored span" rule. A client that anchors LEFT+RIGHT and sets width=0
/// wants full output width; same vertically.
pub fn layer_surface_effective_size(
    cached: &LayerSurfaceCachedState,
    output_geo: Rectangle<i32, Logical>,
) -> Size<i32, Logical> {
    let mut w = cached.size.w;
    let mut h = cached.size.h;
    if w <= 0 && cached.anchor.contains(Anchor::LEFT) && cached.anchor.contains(Anchor::RIGHT) {
        w = (output_geo.size.w - cached.margin.left - cached.margin.right).max(0);
    }
    if h <= 0 && cached.anchor.contains(Anchor::TOP) && cached.anchor.contains(Anchor::BOTTOM) {
        h = (output_geo.size.h - cached.margin.top - cached.margin.bottom).max(0);
    }
    Size::from((w, h))
}

/// Compute the logical (x, y) of a layer surface within the output.
fn compute_position(
    cached: &LayerSurfaceCachedState,
    output_geo: Rectangle<i32, Logical>,
) -> (i32, i32) {
    let eff = layer_surface_effective_size(cached, output_geo);
    let surf_w = eff.w;
    let surf_h = eff.h;
    let margin = &cached.margin;

    let x = if cached.anchor.contains(Anchor::LEFT) {
        output_geo.loc.x + margin.left
    } else if cached.anchor.contains(Anchor::RIGHT) {
        output_geo.loc.x + output_geo.size.w - surf_w - margin.right
    } else {
        output_geo.loc.x + (output_geo.size.w - surf_w) / 2
    };

    let y = if cached.anchor.contains(Anchor::TOP) {
        output_geo.loc.y + margin.top
    } else if cached.anchor.contains(Anchor::BOTTOM) {
        output_geo.loc.y + output_geo.size.h - surf_h - margin.bottom
    } else {
        output_geo.loc.y + (output_geo.size.h - surf_h) / 2
    };

    (x, y)
}

/// Logical position — used for pointer hit testing.
pub fn layer_surface_position_logical(
    cached: &LayerSurfaceCachedState,
    output_geo: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let (x, y) = compute_position(cached, output_geo);
    (x, y).into()
}

/// Physical position — used for rendering layer surfaces.
pub fn layer_surface_position(
    cached: &LayerSurfaceCachedState,
    output_geo: Rectangle<i32, Logical>,
    scale: f64,
) -> Point<i32, Physical> {
    let (x, y) = compute_position(cached, output_geo);
    (
        (x as f64 * scale).round() as i32,
        (y as f64 * scale).round() as i32,
    ).into()
}
