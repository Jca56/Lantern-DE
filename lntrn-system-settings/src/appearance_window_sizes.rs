//! Window Sizes subpanel — its own page under Appearance.
//!
//! Five big sections (Default, Small, Medium, Large, Extra Large). Each shows
//! a live mini-monitor preview of the window proportion plus a slider to set
//! it (10–100% of the work area, snapped to 5%). Small→Medium→Large→Extra
//! Large are the rungs of the Super+Arrow resize ladder; Default is the size
//! new windows open at.
//!
//! Sliders are dragged live during draw via `slider_value_from_cursor`, so
//! there is no separate click handler — the page just needs the panel to be
//! routed here.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, Slider};

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, slider_value_from_cursor, PanelState, CARD_GAP, CARD_HEADER_H,
    CARD_INNER_PAD_H, CARD_INNER_PAD_V, CARD_OUTER_PAD_H, CARD_OUTER_PAD_V, SLIDER_H,
};

/// One zone id per size slider.
const ZONE_WSIZE_SLIDER_BASE: u32 = 800;

/// Slider range + snap step (percent of the work area).
const PCT_MIN: f32 = 10.0;
const PCT_MAX: f32 = 100.0;
const PCT_STEP: u32 = 5;

// Mini-monitor preview dimensions (logical px, scaled by `s`).
const PREVIEW_W: f32 = 168.0;
const PREVIEW_H: f32 = 100.0;

const VALUE_SIZE: f32 = 26.0;
const DESC_SIZE: f32 = 16.0;

/// (label, one-line description) for each rung, in ladder order.
const SIZES: [(&str, &str); 5] = [
    ("Default", "Size new windows open at"),
    ("Small", "Smallest step in the resize ladder"),
    ("Medium", "Middle step in the resize ladder"),
    ("Large", "Large step in the resize ladder"),
    ("Extra Large", "Largest step — near fullscreen"),
];

/// Read the stored percentage for rung `idx`.
fn size_pct(config: &LanternConfig, idx: usize) -> u32 {
    match idx {
        0 => config.windows.default_size_pct,
        1 => config.windows.size_small_pct,
        2 => config.windows.size_medium_pct,
        3 => config.windows.size_large_pct,
        _ => config.windows.size_xlarge_pct,
    }
}

/// Write the percentage for rung `idx`.
fn set_size_pct(config: &mut LanternConfig, idx: usize, pct: u32) {
    match idx {
        0 => config.windows.default_size_pct = pct,
        1 => config.windows.size_small_pct = pct,
        2 => config.windows.size_medium_pct = pct,
        3 => config.windows.size_large_pct = pct,
        _ => config.windows.size_xlarge_pct = pct,
    }
}

/// Height of one size card.
fn card_height(s: f32) -> f32 {
    CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s + PREVIEW_H * s
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_window_sizes_page(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32,
    y: f32,
    w: f32,
    panel_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
    scroll_delta: f32,
) {
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_h = card_height(s);
    let n = SIZES.len();

    let content_height =
        CARD_OUTER_PAD_V * 2.0 * s + card_h * n as f32 + CARD_GAP * s * (n - 1) as f32;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(
            &mut panel_state.scroll_offset,
            scroll_delta * 40.0,
            content_height,
            panel_h,
        );
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut panel_state.scroll_offset);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;
    for (i, (label, desc)) in SIZES.iter().enumerate() {
        draw_size_card(
            config, painter, text, ix, fox, i, label, desc, card_x, cy_top, card_w, card_h, s, sw,
            sh,
        );
        cy_top += card_h + CARD_GAP * s;
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, panel_state.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_size_card(
    config: &mut LanternConfig,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    idx: usize,
    label: &str,
    desc: &str,
    card_x: f32,
    card_y: f32,
    card_w: f32,
    card_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let body_top = draw_section_card(
        painter, text, fox, label, card_x, card_y, card_w, card_h, s, sw, sh,
    );

    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_right = card_x + card_w - CARD_INNER_PAD_H * s;
    let preview_w = PREVIEW_W * s;
    let preview_h = PREVIEW_H * s;

    // Live mini-monitor preview on the left.
    let pct = size_pct(config, idx);
    let preview = Rect::new(card_inner_x, body_top, preview_w, preview_h);
    draw_preview(painter, fox, preview, pct, s);

    // Right column: description, slider, big % readout.
    let right_x = card_inner_x + preview_w + 28.0 * s;
    let right_w = (card_inner_right - right_x).max(120.0 * s);

    // Description sits near the top of the body.
    let desc_sz = DESC_SIZE * s;
    text.queue(
        desc,
        desc_sz,
        right_x,
        body_top + 6.0 * s,
        fox.text_secondary,
        right_w,
        sw,
        sh,
    );

    // Big percentage value, baseline-aligned with the slider center.
    let val_sz = VALUE_SIZE * s;
    let value_w = (val_sz * 3.0).ceil();
    let slider_w = (right_w - value_w - 16.0 * s).max(80.0 * s);
    let slider_h = SLIDER_H * s;
    let slider_y = body_top + preview_h - slider_h - 8.0 * s;

    let rect = Rect::new(right_x, slider_y, slider_w, slider_h);
    let zone_id = ZONE_WSIZE_SLIDER_BASE + idx as u32;
    let zone = ix.add_zone(zone_id, rect);
    if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
        let raw = PCT_MIN + f * (PCT_MAX - PCT_MIN);
        let snapped = ((raw / PCT_STEP as f32).round() as u32 * PCT_STEP)
            .clamp(PCT_MIN as u32, PCT_MAX as u32);
        set_size_pct(config, idx, snapped);
    }
    let frac = ((size_pct(config, idx) as f32 - PCT_MIN) / (PCT_MAX - PCT_MIN)).clamp(0.0, 1.0);
    Slider::new(rect)
        .value(frac)
        .hovered(zone.is_hovered())
        .active(zone.is_active())
        .draw(painter, fox);

    let val = format!("{}%", size_pct(config, idx));
    let val_x = right_x + slider_w + 16.0 * s;
    let val_y = slider_y + (slider_h - val_sz) / 2.0;
    text.queue(&val, val_sz, val_x, val_y, fox.text, value_w, sw, sh);
}

/// Draw a small monitor with a window rectangle sized to `pct` of the screen,
/// centered. Purely illustrative of the proportion — the real footprint uses
/// the live output's work area + aspect at runtime.
fn draw_preview(painter: &mut Painter, fox: &FoxPalette, area: Rect, pct: u32, s: f32) {
    painter.rect_filled(area, 8.0 * s, fox.surface);
    painter.rect_stroke_sdf(area, 8.0 * s, 1.5 * s, fox.muted.with_alpha(0.5));

    let f = (pct as f32 / 100.0).clamp(0.05, 1.0);
    // Inset the usable area slightly so a 100% window still reads inside the bezel.
    let inset = 6.0 * s;
    let usable_w = area.w - inset * 2.0;
    let usable_h = area.h - inset * 2.0;
    let win_w = usable_w * f;
    let win_h = usable_h * f;
    let win = Rect::new(
        area.x + (area.w - win_w) / 2.0,
        area.y + (area.h - win_h) / 2.0,
        win_w,
        win_h,
    );
    painter.rect_filled(win, 5.0 * s, fox.accent.with_alpha(0.28));
    painter.rect_stroke_sdf(win, 5.0 * s, 1.5 * s, fox.accent);
}
