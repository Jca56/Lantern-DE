//! Category sidebar — vertical list of category filters with SVG icons.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};
use std::f32::consts::FRAC_PI_2;

use crate::desktop::Category;
use crate::svg_icon::IconCache;

use super::{AppMenu, ZONE_CAT};

impl AppMenu {
    /// Draw category sidebar on the left side of the Apps tab.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_sidebar(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        icon_cache: &IconCache,
        palette: &FoxPalette,
        sx: f32, sy: f32, sw: f32, sh: f32,
        scale: f32, screen_w: u32, screen_h: u32,
        icon_draws: &mut Vec<(String, f32, f32, f32, f32, Option<[f32; 4]>)>,
    ) {
        // 4px vertical gradient divider on the right edge — matches lntrn-file-manager
        let grad_w = 4.0 * scale;
        let grad_colors = palette.file_manager_gradient_stops();
        let grad_stops: Vec<(f32, lntrn_render::Color)> = grad_colors.iter().enumerate()
            .map(|(i, &c)| (i as f32 / (grad_colors.len() - 1) as f32, c))
            .collect();
        painter.rect_gradient_multi(
            Rect::new(sx + sw - grad_w, sy, grad_w, sh),
            0.0, FRAC_PI_2, &grad_stops,
        );

        let font = 22.0 * scale;
        let item_h = 48.0 * scale;
        let pad_x = 16.0 * scale;
        let icon_sz = 22.0 * scale;
        let icon_text_gap = 12.0 * scale;
        let cr = 10.0 * scale;
        let top_pad = 14.0 * scale;

        for (i, &cat) in Category::SIDEBAR_ORDER.iter().enumerate() {
            let iy = sy + top_pad + i as f32 * item_h;
            let item_rect = Rect::new(sx + 6.0 * scale, iy, sw - 12.0 * scale, item_h - 4.0 * scale);
            let zone_id = ZONE_CAT + i as u32;
            let state = ix.add_zone(zone_id, item_rect);
            let is_active = self.selected_category == cat;

            if is_active {
                painter.rect_filled(item_rect, cr, palette.accent.with_alpha(0.2));
            } else if state.is_hovered() {
                painter.rect_filled(item_rect, cr, palette.surface_2);
            }

            // SVG icon
            let icon_key = format!("cat_{}", cat.label());
            let icon_x = sx + pad_x;
            let icon_y = iy + (item_h - 4.0 * scale - icon_sz) * 0.5;
            if icon_cache.get(&icon_key).is_some() {
                icon_draws.push((icon_key, icon_x, icon_y, icon_sz, icon_sz, None));
            }

            // Label
            let label = cat.label();
            let text_x = icon_x + icon_sz + icon_text_gap;
            let icon_cy = iy + (item_h - 4.0 * scale) * 0.5;
            let text_y = icon_cy - font * 0.5;
            let text_color = if is_active { palette.text } else { palette.text_secondary };
            text.queue(label, font, text_x, text_y, text_color, sw - pad_x * 2.0, screen_w, screen_h);
        }
    }
}
