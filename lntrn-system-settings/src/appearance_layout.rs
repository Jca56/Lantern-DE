//! Borders + Blur Effects cards for the Appearance / Windows subpanel.
//!
//! Split into two cards so the Windows page can render them in a two-column
//! layout: Borders on the left, Blur Effects on the right.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Slider};

use crate::appearance_panel::LayoutZones;
use crate::config::LanternConfig;
use crate::panels::{
    draw_color_swatch_row, draw_section_card, slider_value_from_cursor, CARD_INNER_PAD_H,
    LABEL_SIZE, LABEL_W, ROW_H, SLIDER_H, SLIDER_W, VALUE_SIZE, VALUE_W,
};

/// Number of rows in the Borders card (Border Width, Corner Radius, Border Color).
pub const BORDER_ROWS: f32 = 3.0;
/// Number of rows in the Effects card (Blur Intensity, Tint, Tint Color, Darken, BG Opacity).
pub const EFFECT_ROWS: f32 = 5.0;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_borders_card(
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
    z: &LayoutZones,
) {
    let (label_x, ctrl_x, ctrl_w, value_x, row, lsz, _vsz, slider_h) = card_geom(card_x, card_w, s);

    let mut cy = draw_section_card(
        painter, text, fox, "Borders", card_x, card_y, card_w, card_h, s, sw, sh,
    );

    {
        let mut slider_row = |label: &str,
                              frac: f32,
                              zone_id: u32,
                              cy: &mut f32,
                              min: f32,
                              max: f32,
                              suffix: &str,
                              config_val: &mut u32| {
            let label_y = *cy + (row - lsz) / 2.0;
            text.queue(
                label,
                lsz,
                label_x,
                label_y,
                fox.text,
                ctrl_x - label_x,
                sw,
                sh,
            );
            let rect = Rect::new(ctrl_x, *cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(zone_id, rect);
            if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
                *config_val = (min + f * (max - min)).round() as u32;
            }
            Slider::new(rect)
                .value(frac)
                .hovered(zone.is_hovered())
                .active(zone.is_active())
                .draw(painter, fox);
            let val = format!("{}{}", *config_val, suffix);
            text.queue(
                &val,
                VALUE_SIZE * s,
                value_x,
                label_y,
                fox.text_secondary,
                VALUE_W * s,
                sw,
                sh,
            );
            *cy += row;
        };

        let frac = config.window_manager.border_width as f32 / 10.0;
        let mut bw = config.window_manager.border_width;
        slider_row(
            "Border Width",
            frac,
            z.border,
            &mut cy,
            0.0,
            10.0,
            "",
            &mut bw,
        );
        config.window_manager.border_width = bw;

        let frac = config.window_manager.corner_radius as f32 / 20.0;
        let mut cr = config.window_manager.corner_radius;
        slider_row(
            "Corner Radius",
            frac,
            z.corner,
            &mut cy,
            0.0,
            20.0,
            "px",
            &mut cr,
        );
        config.window_manager.corner_radius = cr;
    }

    // Swatch rows use a tighter label column than the sliders so all 10
    // chips have room in a narrow two-column card.
    let sw_ctrl_x = label_x + 130.0 * s;
    let end_x = card_x + card_w - CARD_INNER_PAD_H * s;
    draw_color_swatch_row(
        painter,
        text,
        ix,
        fox,
        "Border Color",
        z.border_color_base,
        &config.window_manager.border_color,
        label_x,
        sw_ctrl_x,
        end_x,
        &mut cy,
        row,
        lsz,
        s,
        sw,
        sh,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_effects_card(
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
    z: &LayoutZones,
) {
    let (label_x, ctrl_x, ctrl_w, value_x, row, lsz, vsz, slider_h) = card_geom(card_x, card_w, s);

    let mut cy = draw_section_card(
        painter,
        text,
        fox,
        "Blur & Effects",
        card_x,
        card_y,
        card_w,
        card_h,
        s,
        sw,
        sh,
    );

    pct_slider(
        painter,
        text,
        ix,
        fox,
        "Blur Intensity",
        &mut config.windows.blur_intensity,
        z.blur,
        label_x,
        ctrl_x,
        ctrl_w,
        value_x,
        slider_h,
        row,
        lsz,
        vsz,
        &mut cy,
        s,
        sw,
        sh,
    );
    pct_slider(
        painter,
        text,
        ix,
        fox,
        "Blur Tint",
        &mut config.windows.blur_tint,
        z.tint,
        label_x,
        ctrl_x,
        ctrl_w,
        value_x,
        slider_h,
        row,
        lsz,
        vsz,
        &mut cy,
        s,
        sw,
        sh,
    );

    let sw_ctrl_x = label_x + 130.0 * s;
    let end_x = card_x + card_w - CARD_INNER_PAD_H * s;
    draw_color_swatch_row(
        painter,
        text,
        ix,
        fox,
        "Tint Color",
        z.tint_color_base,
        &config.windows.blur_tint_color,
        label_x,
        sw_ctrl_x,
        end_x,
        &mut cy,
        row,
        lsz,
        s,
        sw,
        sh,
    );

    pct_slider(
        painter,
        text,
        ix,
        fox,
        "Blur Darken",
        &mut config.windows.blur_darken,
        z.darken,
        label_x,
        ctrl_x,
        ctrl_w,
        value_x,
        slider_h,
        row,
        lsz,
        vsz,
        &mut cy,
        s,
        sw,
        sh,
    );
    pct_slider(
        painter,
        text,
        ix,
        fox,
        "Background Opacity",
        &mut config.windows.background_opacity,
        z.bg_opacity,
        label_x,
        ctrl_x,
        ctrl_w,
        value_x,
        slider_h,
        row,
        lsz,
        vsz,
        &mut cy,
        s,
        sw,
        sh,
    );
}

/// (label_x, ctrl_x, ctrl_w, value_x, row, lsz, vsz, slider_h)
fn card_geom(card_x: f32, card_w: f32, s: f32) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
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
    (label_x, ctrl_x, ctrl_w, value_x, row, lsz, vsz, slider_h)
}

#[allow(clippy::too_many_arguments)]
fn pct_slider(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    label: &str,
    val_ref: &mut f32,
    zone_id: u32,
    label_x: f32,
    ctrl_x: f32,
    ctrl_w: f32,
    value_x: f32,
    slider_h: f32,
    row: f32,
    lsz: f32,
    vsz: f32,
    cy: &mut f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(
        label,
        lsz,
        label_x,
        label_y,
        fox.text,
        ctrl_x - label_x,
        sw,
        sh,
    );
    let rect = Rect::new(ctrl_x, *cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
    let zone = ix.add_zone(zone_id, rect);
    if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
        *val_ref = (f * 100.0).round() / 100.0;
    }
    Slider::new(rect)
        .value(*val_ref)
        .hovered(zone.is_hovered())
        .active(zone.is_active())
        .draw(painter, fox);
    let val = format!("{:.0}%", *val_ref * 100.0);
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
    *cy += row;
}
