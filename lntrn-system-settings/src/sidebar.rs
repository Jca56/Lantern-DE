//! Left-edge panel list: icon + label per panel, active pill + accent stripe.

use lntrn_render::{Color, GpuTexture, Painter, Rect, TextRenderer, TextureDraw};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::wayland::{Panel, PANELS, ZONE_SIDEBAR_BASE};

pub(crate) const SIDEBAR_W: f32 = 300.0;
pub(crate) const SIDEBAR_ITEM_H: f32 = 76.0;
pub(crate) const SIDEBAR_ICON_DRAW: f32 = 36.0;

/// Render the sidebar's divider + items. Pushes icon `TextureDraw`s into
/// `tex_draws` so the caller batches them with the rest of the frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_sidebar<'a>(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    icon_textures: &'a [GpuTexture],
    tex_draws: &mut Vec<TextureDraw<'a>>,
    active_panel: Panel,
    sidebar_w: f32,
    body_y: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let item_h = SIDEBAR_ITEM_H * s;
    let label_size = 22.0 * s;
    let icon_draw = SIDEBAR_ICON_DRAW * s;

    // Divider line between sidebar and content — subtle in both modes.
    painter.rect_filled(
        Rect::new(sidebar_w, body_y, 1.0 * s, hf - body_y),
        0.0,
        fox.muted.with_alpha(0.35),
    );

    for (i, (panel, label)) in PANELS.iter().enumerate() {
        let y = body_y + 8.0 * s + i as f32 * item_h;
        let zone_id = ZONE_SIDEBAR_BASE + i as u32;
        let rect = Rect::new(0.0, y, sidebar_w, item_h);
        let zone_state = ix.add_zone(zone_id, rect);
        let is_active = *panel == active_panel;

        // Active items get a bright-gold pill instead of `fox.accent.with_alpha`
        // because low-alpha gold over the near-black bg blends to muddy brown.
        if is_active {
            let inset_x = 10.0 * s;
            let inset_y = 6.0 * s;
            let pill = Rect::new(
                inset_x,
                y + inset_y,
                sidebar_w - inset_x * 2.0,
                item_h - inset_y * 2.0,
            );
            let radius = 8.0 * s;
            painter.rect_filled(pill, radius, Color::from_rgba8(255, 180, 30, 56));
            painter.rect_stroke_sdf(pill, radius, 1.0 * s, Color::from_rgba8(255, 180, 30, 110));
            painter.rect_filled(
                Rect::new(0.0, y + 8.0 * s, 4.0 * s, item_h - 16.0 * s),
                2.0 * s,
                fox.accent,
            );
        } else if zone_state.is_hovered() {
            let inset_x = 10.0 * s;
            let inset_y = 6.0 * s;
            let pill = Rect::new(
                inset_x,
                y + inset_y,
                sidebar_w - inset_x * 2.0,
                item_h - inset_y * 2.0,
            );
            painter.rect_filled(pill, 8.0 * s, fox.text.with_alpha(0.06));
        }

        let icon_x = 24.0 * s;
        let icon_y = y + (item_h - icon_draw) / 2.0;
        let draw = TextureDraw::new(&icon_textures[i], icon_x, icon_y, icon_draw, icon_draw);
        tex_draws.push(draw);

        let text_x = icon_x + icon_draw + 16.0 * s;
        let text_y = y + (item_h - label_size) / 2.0;
        let text_color = if is_active { fox.accent } else { fox.text };
        text.queue(label, label_size, text_x, text_y, text_color, sidebar_w - text_x, sw, sh);
    }
}
