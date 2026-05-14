//! Chevron button that toggles the panel between full and "just the
//! controls row" modes. Lives at the top-right of the controls row.

use lntrn_render::{Color, Painter, TextRenderer};

use crate::controls::tile::TileLayout;

/// Logical width the collapse button claims in the row.
pub const TILE_WIDTH: f32 = 36.0;

/// Logical chevron arm length.
const CHEVRON_ARM: f32 = 9.0;
/// Stroke width (logical).
const STROKE: f32 = 2.5;

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    collapsed: bool,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    lit: bool,
) {
    let arm = CHEVRON_ARM * scale;
    let stroke = STROKE * scale;
    let cx = layout.x + layout.w / 2.0;
    let cy = layout.y + layout.h / 2.0;

    let color = if lit || collapsed {
        Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(0.95 * alpha)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.85 * alpha)
    };

    // Chevron points DOWN when collapsed (so the user reads it as
    // "expand downward"), UP when expanded ("collapse upward").
    if collapsed {
        painter.line_round(cx - arm, cy - arm * 0.5, cx, cy + arm * 0.5, stroke, color);
        painter.line_round(cx, cy + arm * 0.5, cx + arm, cy - arm * 0.5, stroke, color);
    } else {
        painter.line_round(cx - arm, cy + arm * 0.5, cx, cy - arm * 0.5, stroke, color);
        painter.line_round(cx, cy - arm * 0.5, cx + arm, cy + arm * 0.5, stroke, color);
    }
}
