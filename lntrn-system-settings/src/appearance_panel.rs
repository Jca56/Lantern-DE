use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::config::LanternConfig;
use crate::panels::{
    draw_color_swatch_row, draw_section_card, CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H,
    CARD_INNER_PAD_V, CARD_OUTER_PAD_H, CARD_OUTER_PAD_V, GLOW_COLORS, LABEL_SIZE, ROW_H,
};

const ZONE_APPEARANCE_THEME_FOX: u32 = 350;
const ZONE_APPEARANCE_THEME_LANTERN: u32 = 351;
const ZONE_APPEARANCE_ACCENT_BASE: u32 = 360; // 360..368 swatches

pub fn draw_appearance_panel(
    config: &mut LanternConfig,
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
) {
    let _ = panel_h;
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    // Card 1: Theme — radio between Fox Dark and Lantern.
    let theme_rows = 2.5;
    let theme_card_h = card_chrome_h + theme_rows * row;
    let mut cy_top = y + CARD_OUTER_PAD_V * s;
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Theme",
            card_x, cy_top, card_w, theme_card_h, s, sw, sh,
        );

        let active = config.appearance.theme.as_str();
        let btn_w = 220.0 * s;
        let btn_h = 48.0 * s;
        let gap = 16.0 * s;

        let fox_dark_rect = Rect::new(card_inner_x, cy, btn_w, btn_h);
        let lantern_rect = Rect::new(card_inner_x + btn_w + gap, cy, btn_w, btn_h);
        let fox_zone = ix.add_zone(ZONE_APPEARANCE_THEME_FOX, fox_dark_rect);
        let lantern_zone = ix.add_zone(ZONE_APPEARANCE_THEME_LANTERN, lantern_rect);

        draw_theme_card(
            painter, text, fox, fox_dark_rect, "Fox Dark",
            active == "fox-dark" || active == "fox",
            fox_zone.is_hovered(), s, sw, sh,
            Color::from_hex("#181818").unwrap(),
        );
        draw_theme_card(
            painter, text, fox, lantern_rect, "Lantern",
            active == "lantern",
            lantern_zone.is_hovered(), s, sw, sh,
            Color::from_hex("#221812").unwrap(),
        );
        cy += btn_h + row * 0.5;
        let _ = cy;
    }

    cy_top += theme_card_h + CARD_GAP * s;

    // Card 2: Accent — same 9-color palette every other picker uses.
    let accent_card_h = card_chrome_h + row;
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Accent",
            card_x, cy_top, card_w, accent_card_h, s, sw, sh,
        );
        let label_x = card_inner_x;
        let ctrl_x = card_inner_x + 100.0 * s;
        draw_color_swatch_row(
            painter, text, ix, fox,
            "Color", ZONE_APPEARANCE_ACCENT_BASE,
            &config.appearance.accent,
            label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
        );
    }
}

/// Theme preview card — swatch of the variant's bg color with the name
/// centered. Selected = accent ring, hovered = muted ring.
fn draw_theme_card(
    painter: &mut Painter,
    text: &mut TextRenderer,
    fox: &FoxPalette,
    rect: Rect,
    label: &str,
    selected: bool,
    hovered: bool,
    s: f32,
    sw: u32,
    sh: u32,
    swatch_color: Color,
) {
    let r = 8.0 * s;
    painter.rect_filled(rect, r, swatch_color);
    let ring = if selected {
        fox.accent
    } else if hovered {
        fox.text_secondary.with_alpha(0.5)
    } else {
        fox.muted.with_alpha(0.2)
    };
    let thick = if selected { 2.5 * s } else { 1.0 * s };
    painter.rect_stroke_sdf(rect, r, thick, ring);
    let label_sz = 18.0 * s;
    let tw = label_sz * 0.55 * label.len() as f32;
    text.queue(
        label, label_sz,
        rect.x + (rect.w - tw) * 0.5,
        rect.y + (rect.h - label_sz) * 0.5,
        if selected { fox.accent } else { fox.text },
        rect.w, sw, sh,
    );
}

pub fn handle_appearance_click(config: &mut LanternConfig, zone_id: u32) {
    match zone_id {
        ZONE_APPEARANCE_THEME_FOX => {
            config.appearance.theme = "fox-dark".into();
        }
        ZONE_APPEARANCE_THEME_LANTERN => {
            config.appearance.theme = "lantern".into();
        }
        id if id >= ZONE_APPEARANCE_ACCENT_BASE
            && id < ZONE_APPEARANCE_ACCENT_BASE + GLOW_COLORS.len() as u32 =>
        {
            let idx = (id - ZONE_APPEARANCE_ACCENT_BASE) as usize;
            config.appearance.accent = GLOW_COLORS[idx].0.into();
        }
        _ => {}
    }
}
