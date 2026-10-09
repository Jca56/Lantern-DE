//! Workspace-number tile: the active workspace shown as a bold digit in
//! a rounded square box. The value comes from the compositor's workspace
//! IPC (`app.workspace_ipc.active_id_for()`), passed in at draw time.
//! Clicking the box cycles through the populated workspaces.

use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

/// Logical width of the tile — the box plus a little side padding.
pub const TILE_WIDTH: f32 = 56.0;

/// Side length of the rounded square box (logical px).
const BOX_SIZE: f32 = 46.0;
/// Bold digit font inside the box.
const NUM_FONT: f32 = 30.0;
/// Accent gold #C8860A — matches the toolbar/calendar accent.
const GOLD: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    painter: &mut Painter,
    text: &mut TextRenderer,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    workspace: Option<u32>,
    lit: bool,
) {
    // Nothing to show until the compositor reports an active workspace.
    let Some(ws) = workspace else {
        return;
    };

    let box_size = BOX_SIZE * scale;
    // Center the box in the tile (both axes).
    let box_x = layout.x + (layout.w - box_size) / 2.0;
    let box_y = layout.y + (layout.h - box_size) / 2.0;
    let box_rect = Rect::new(box_x, box_y, box_size, box_size);
    let radius = 12.0 * scale;

    // Hovered with somewhere to cycle to: the box brightens and its
    // border turns gold, like the other tiles' lit state.
    let (fill, stroke) = if lit {
        (
            Color::rgba(1.0, 1.0, 1.0, 0.12 * alpha),
            Color::from_rgb8(GOLD.0, GOLD.1, GOLD.2).with_alpha(alpha),
        )
    } else {
        (
            Color::rgba(1.0, 1.0, 1.0, 0.06 * alpha),
            Color::rgba(1.0, 1.0, 1.0, 0.14 * alpha),
        )
    };
    painter.rect_filled(box_rect, radius, fill);
    painter.rect_stroke_sdf(box_rect, radius, 1.5 * scale, stroke);

    let num = ws.to_string();
    let num_font = NUM_FONT * scale;
    let num_w = text.measure_width_styled(&num, num_font, FontWeight::Bold, FontStyle::Normal);
    text.queue_styled(
        &num,
        num_font,
        box_rect.x + (box_size - num_w) / 2.0,
        box_rect.y + (box_size - num_font) / 2.0,
        Color::rgba(1.0, 1.0, 1.0, alpha),
        box_size,
        FontWeight::Bold,
        FontStyle::Normal,
        surface_w,
        surface_h,
    );
}
