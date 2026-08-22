//! Focus Glow card for the Appearance / Windows subpanel.
//!
//! Focus-glow master toggle plus glow color + intensity controls revealed when
//! glow is enabled. (Focus-follows-mouse lives on the Input / Mouse subpanel
//! since it's a pointer behavior, not a visual effect.)

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Slider, Toggle};

use crate::appearance_panel::FocusZones;
use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, slider_value_from_cursor, CARD_INNER_PAD_H, GLOW_COLORS, LABEL_SIZE,
    LABEL_W, ROW_H, SLIDER_H, SLIDER_W, TOGGLE_H, VALUE_SIZE, VALUE_W,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_focus_card(
    config: &mut LanternConfig,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    card_x: f32,
    card_y: f32,
    card_w: f32,
    card_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
    z: &FocusZones,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;
    let label_w = LABEL_W * s;
    let value_w = VALUE_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;
    let avail = (card_inner_w - label_w - value_w - 12.0 * s).max(80.0 * s);
    let ctrl_w = (SLIDER_W * s).min(avail);
    let value_x = ctrl_x + ctrl_w + 8.0 * s;

    let mut cy = draw_section_card(
        painter,
        text,
        fox,
        "Focus Glow",
        card_x,
        card_y,
        card_w,
        card_h,
        s,
        sw,
        sh,
    );
    // `z.focus` no longer rendered here — see input_panel Pointer card.
    let _ = z.focus;

    // Glow master toggle
    {
        let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.window_manager.focus_glow)
            .label("Focus Glow")
            .scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(z.glow, track);
        toggle
            .hovered(zone.is_hovered())
            .draw(painter, text, fox, sw, sh);
        cy += row;
    }

    if !config.window_manager.focus_glow {
        return;
    }

    // Glow color swatches (custom drawn — uses circles, not the square swatches)
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue(
            "Glow Color",
            lsz,
            label_x,
            label_y,
            fox.text,
            ctrl_x - label_x,
            sw,
            sh,
        );

        let swatch_size = 28.0 * s;
        let swatch_gap = 8.0 * s;
        let mut sx = ctrl_x;
        for (i, (hex, _name)) in GLOW_COLORS.iter().enumerate() {
            let color = Color::from_hex(hex).unwrap();
            let zone_id = z.glow_color_base + i as u32;
            let swatch_rect =
                Rect::new(sx, cy + (row - swatch_size) / 2.0, swatch_size, swatch_size);
            let zone = ix.add_zone(zone_id, swatch_rect);

            let cx = sx + swatch_size / 2.0;
            let cy_center = swatch_rect.y + swatch_size / 2.0;
            let radius = swatch_size / 2.0;
            painter.circle_filled(cx, cy_center, radius, color);

            let is_selected = config
                .window_manager
                .focus_glow_color
                .eq_ignore_ascii_case(hex);
            if is_selected {
                painter.circle_stroke(cx, cy_center, radius + 3.0 * s, 2.0 * s, fox.text);
            } else if zone.is_hovered() {
                painter.circle_stroke(cx, cy_center, radius + 2.0 * s, 1.5 * s, fox.text_secondary);
            }
            sx += swatch_size + swatch_gap;
        }
        cy += row;
    }

    // Glow intensity slider
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue(
            "Glow Intensity",
            lsz,
            label_x,
            label_y,
            fox.text,
            ctrl_x - label_x,
            sw,
            sh,
        );
        let frac = (config.window_manager.focus_glow_intensity / 0.6).clamp(0.0, 1.0);
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(z.glow_intensity, rect);
        if let Some(f) = slider_value_from_cursor(ix, z.glow_intensity, &rect) {
            config.window_manager.focus_glow_intensity = ((f * 0.6) * 100.0).round() / 100.0;
        }
        Slider::new(rect)
            .value(frac)
            .hovered(zone.is_hovered())
            .active(zone.is_active())
            .draw(painter, fox);
        let pct = (config.window_manager.focus_glow_intensity / 0.6 * 100.0).round() as i32;
        let val = format!("{}%", pct);
        text.queue(
            &val,
            vsz,
            value_x,
            label_y,
            fox.text_secondary,
            VALUE_W * s,
            sw,
            sh,
        );
    }

    // Suppress unused-var warning when the early-return path is taken
    let _ = SLIDER_W;
}
