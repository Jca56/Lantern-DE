//! Header button for the Terminal view — a small "clear" icon that
//! drops the most recent command output. The actual terminal state
//! lives in `crate::terminal::TerminalState`.

use lntrn_render::{Color, Painter, TextRenderer};

use crate::controls::tile::TileLayout;

#[allow(dead_code)] // kept in case we re-add a terminal header tile
pub const TILE_WIDTH: f32 = 44.0;

pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
) {
    // Draw a stylized "broom"/"clear" icon: a small chevron + bar.
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.92 * alpha);
    let cx = layout.x + layout.w / 2.0;
    let cy = layout.y + layout.h / 2.0;
    let arm = layout.h * 0.22;
    let stroke = 2.5 * scale;

    // Circular outline that suggests "reset / clear".
    let steps = 14;
    let r = arm * 1.1;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let a0 = t0 * std::f32::consts::TAU * 0.78 + 0.4;
        let a1 = t1 * std::f32::consts::TAU * 0.78 + 0.4;
        let x0 = cx + a0.cos() * r;
        let y0 = cy + a0.sin() * r;
        let x1 = cx + a1.cos() * r;
        let y1 = cy + a1.sin() * r;
        painter.line_round(x0, y0, x1, y1, stroke, color);
    }
    // Arrow tip at the top so it reads as "rotate".
    let a_tip: f32 = 0.4;
    let tip_x = cx + a_tip.cos() * r;
    let tip_y = cy + a_tip.sin() * r;
    painter.line_round(
        tip_x,
        tip_y,
        tip_x - stroke * 2.5,
        tip_y - stroke * 1.5,
        stroke,
        color,
    );
    painter.line_round(
        tip_x,
        tip_y,
        tip_x + stroke * 0.5,
        tip_y - stroke * 2.8,
        stroke,
        color,
    );
}
