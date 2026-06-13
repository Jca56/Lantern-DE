//! Gaming Mode toggle tile — shows the compositor's Gaming Mode state
//! (primary output dropped to native scale 1.0 so fullscreen games get
//! true physical resolution) and flips it on click via the gaming IPC
//! socket. Always in lock-step with the Super+G keybind.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::gaming_ipc::GamingIpc;
use crate::controls::tile::TileLayout;

/// Logical width reserved for the gaming tile in the controls row.
pub const TILE_WIDTH: f32 = 84.0;

const LABEL_FONT: f32 = 17.0;
const SUB_FONT: f32 = 13.0;
/// Accent gold #C8860A — matches the toolbar/calendar accent.
const GOLD: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
/// Brighter gold for the active label (matches the battery bolt).
const GOLD_BRIGHT: (u8, u8, u8) = (0xff, 0xd0, 0x40);

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    painter: &mut Painter,
    text: &mut TextRenderer,
    gaming: &GamingIpc,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    lit: bool,
) {
    let enabled = gaming.gaming_mode == Some(true);
    // Unknown state (compositor socket not connected) renders dimmed.
    let known = gaming.gaming_mode.is_some();

    // Gold pill behind the tile while gaming mode is engaged, so the
    // "your desktop scale is currently different" state is unmissable.
    if enabled {
        painter.rect_filled(
            Rect::new(layout.x, layout.y, layout.w, layout.h),
            10.0 * scale,
            Color::from_rgb8(GOLD.0, GOLD.1, GOLD.2).with_alpha(0.16 * alpha),
        );
    }

    let text_alpha = if !known {
        0.4
    } else if lit || enabled {
        1.0
    } else {
        0.85
    };
    let label_color = if enabled {
        Color::from_rgb8(GOLD_BRIGHT.0, GOLD_BRIGHT.1, GOLD_BRIGHT.2)
            .with_alpha(alpha * text_alpha)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * text_alpha)
    };

    let label_font = LABEL_FONT * scale;
    let sub_font = SUB_FONT * scale;
    let cy = layout.y + layout.h / 2.0;
    let gap = 3.0 * scale;

    let label = "GAME";
    let lw = text.measure_width(label, label_font);
    text.queue(
        label,
        label_font,
        layout.x + (layout.w - lw) / 2.0,
        cy - gap / 2.0 - label_font,
        label_color,
        lw + 8.0 * scale,
        surface_w,
        surface_h,
    );

    let sub = if !known {
        "—"
    } else if enabled {
        "ON"
    } else {
        "OFF"
    };
    let sub_color = if enabled {
        label_color
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * text_alpha * 0.7)
    };
    let sw = text.measure_width(sub, sub_font);
    text.queue(
        sub,
        sub_font,
        layout.x + (layout.w - sw) / 2.0,
        cy + gap / 2.0,
        sub_color,
        sw + 8.0 * scale,
        surface_w,
        surface_h,
    );
}
