use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, Slider, Toggle};

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
};

const ZONE_NOTIF_SHOW: u32 = 700;
const ZONE_NOTIF_SOUND: u32 = 701;
const ZONE_NOTIF_VOLUME: u32 = 702;

const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const VALUE_SIZE: f32 = 16.0;
const SLIDER_H: f32 = 36.0;
const SLIDER_W: f32 = 320.0;
const TOGGLE_H: f32 = 36.0;
const LABEL_W: f32 = 200.0;
const VALUE_W: f32 = 60.0;

pub struct NotifPanelState {
    pub scroll: f32,
}

impl NotifPanelState {
    pub fn new() -> Self {
        Self { scroll: 0.0 }
    }
}

pub fn draw_notifications_panel(
    config: &mut LanternConfig,
    state: &mut NotifPanelState,
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
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    let label_w = LABEL_W * s;
    let value_w = VALUE_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;
    let avail = (card_inner_w - label_w - value_w - 12.0 * s).max(80.0 * s);
    let ctrl_w = (SLIDER_W * s).min(avail);
    let value_x = ctrl_x + ctrl_w + 8.0 * s;

    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    // Card 1: Display — 1 toggle row
    let display_card_h = card_chrome_h + 1.0 * row;
    // Card 2: Sound — toggle + (volume only if sound enabled)
    let sound_rows: f32 = if config.notifications.play_sound { 2.0 } else { 1.0 };
    let sound_card_h = card_chrome_h + sound_rows * row;

    let content_height = CARD_OUTER_PAD_V * s
        + display_card_h
        + CARD_GAP * s
        + sound_card_h
        + CARD_OUTER_PAD_V * 2.0 * s;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(&mut state.scroll, scroll_delta * 40.0, content_height, panel_h);
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut state.scroll);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    // ── Display card ────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Display",
            card_x, cy_top, card_w, display_card_h, s, sw, sh,
        );
        let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.notifications.show_toasts)
            .label("Show Notifications")
            .scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_NOTIF_SHOW, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
        let _ = cy;
    }

    cy_top += display_card_h + CARD_GAP * s;

    // ── Sound card ──────────────────────────────────────────────────
    let mut cy = draw_section_card(
        painter, text, fox, "Sound",
        card_x, cy_top, card_w, sound_card_h, s, sw, sh,
    );
    {
        let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.notifications.play_sound)
            .label("Play Sound")
            .scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_NOTIF_SOUND, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
    }

    if config.notifications.play_sound {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Volume", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let frac = config.notifications.volume.clamp(0.0, 1.0);
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_NOTIF_VOLUME, rect);
        if let Some(f) = crate::panels::slider_value_from_cursor(ix, ZONE_NOTIF_VOLUME, &rect) {
            config.notifications.volume = (f * 100.0).round() / 100.0;
        }
        Slider::new(rect)
            .value(frac)
            .hovered(zone.is_hovered())
            .active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", config.notifications.volume * 100.0);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, state.scroll);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

pub fn handle_notifications_click(config: &mut LanternConfig, zone_id: u32) {
    match zone_id {
        ZONE_NOTIF_SHOW => {
            config.notifications.show_toasts = !config.notifications.show_toasts;
        }
        ZONE_NOTIF_SOUND => {
            config.notifications.play_sound = !config.notifications.play_sound;
        }
        _ => {}
    }
}
