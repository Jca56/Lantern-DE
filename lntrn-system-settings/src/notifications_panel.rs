use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{Button, ButtonVariant, FoxPalette, InteractionContext, ScrollArea, Scrollbar, Slider, Toggle};

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, slider_value_from_cursor, CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H,
    CARD_INNER_PAD_V, CARD_OUTER_PAD_H, CARD_OUTER_PAD_V, LABEL_SIZE, LABEL_W, ROW_H, SLIDER_H,
    SLIDER_W, TOGGLE_H, VALUE_SIZE, VALUE_W,
};

const ZONE_NOTIF_DND: u32 = 699;
const ZONE_NOTIF_SHOW: u32 = 700;
const ZONE_NOTIF_SOUND: u32 = 701;
const ZONE_NOTIF_VOLUME: u32 = 702;
const ZONE_NOTIF_DURATION: u32 = 703;
const ZONE_NOTIF_POS_TR: u32 = 710;
const ZONE_NOTIF_POS_TL: u32 = 711;
const ZONE_NOTIF_POS_BR: u32 = 712;
const ZONE_NOTIF_POS_BL: u32 = 713;
const ZONE_NOTIF_TEST: u32 = 720;

const POSITIONS: &[(u32, &str, &str)] = &[
    (ZONE_NOTIF_POS_TL, "top-left", "Top Left"),
    (ZONE_NOTIF_POS_TR, "top-right", "Top Right"),
    (ZONE_NOTIF_POS_BL, "bottom-left", "Bottom Left"),
    (ZONE_NOTIF_POS_BR, "bottom-right", "Bottom Right"),
];

pub struct NotifPanelState {
    pub scroll: f32,
}

impl NotifPanelState {
    pub fn new() -> Self {
        Self { scroll: 0.0 }
    }
}

#[allow(clippy::too_many_arguments)]
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

    // Card 1: Do Not Disturb — single toggle
    let dnd_card_h = card_chrome_h + 1.0 * row;
    // Card 2: Display — Show toggle + Duration slider + Position picker (4 buttons in 2x2)
    let display_card_h = card_chrome_h + 4.0 * row;
    // Card 3: Sound — toggle + (volume only if sound enabled)
    let sound_rows: f32 = if config.notifications.play_sound { 2.0 } else { 1.0 };
    let sound_card_h = card_chrome_h + sound_rows * row;
    // Card 4: Testing — fire a test notification
    let testing_card_h = card_chrome_h + row * 1.5;

    let content_height = CARD_OUTER_PAD_V * s
        + dnd_card_h + CARD_GAP * s
        + display_card_h + CARD_GAP * s
        + sound_card_h + CARD_GAP * s
        + testing_card_h
        + CARD_OUTER_PAD_V * 2.0 * s;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(&mut state.scroll, scroll_delta * 40.0, content_height, panel_h);
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut state.scroll);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    // ── Do Not Disturb ──────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Do Not Disturb",
            card_x, cy_top, card_w, dnd_card_h, s, sw, sh,
        );
        let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.notifications.do_not_disturb)
            .label("Mute all notifications")
            .scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_NOTIF_DND, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
        let _ = cy;
    }

    cy_top += dnd_card_h + CARD_GAP * s;

    // ── Display ─────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Display",
            card_x, cy_top, card_w, display_card_h, s, sw, sh,
        );

        // Show toggle
        {
            let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
            let toggle = Toggle::new(rect, config.notifications.show_toasts)
                .label("Show Notifications")
                .scale(s);
            let track = toggle.track_rect();
            let zone = ix.add_zone(ZONE_NOTIF_SHOW, track);
            toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
            cy += row;
        }

        // Duration slider (1.0 – 10.0 s)
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Duration", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
            let frac = ((config.notifications.default_duration_secs - 1.0) / 9.0).clamp(0.0, 1.0);
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_NOTIF_DURATION, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_NOTIF_DURATION, &rect) {
                let raw = 1.0 + f * 9.0;
                config.notifications.default_duration_secs = (raw * 10.0).round() / 10.0;
            }
            Slider::new(rect)
                .value(frac)
                .hovered(zone.is_hovered())
                .active(zone.is_active())
                .draw(painter, fox);
            let val = format!("{:.1}s", config.notifications.default_duration_secs);
            text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
            cy += row;
        }

        // Position picker — 2x2 grid of buttons (labels in screen-corner layout)
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Position", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
            let btn_w = 130.0 * s;
            let btn_h = 36.0 * s;
            let btn_gap = 8.0 * s;
            let row_y = cy + (row - btn_h) / 2.0;
            for (col, (zone_id, value, label)) in POSITIONS.iter().take(2).enumerate() {
                let bx = ctrl_x + col as f32 * (btn_w + btn_gap);
                let rect = Rect::new(bx, row_y, btn_w, btn_h);
                let zone = ix.add_zone(*zone_id, rect);
                let selected = config.notifications.position.eq_ignore_ascii_case(value);
                Button::new(rect, label)
                    .variant(if selected { ButtonVariant::Primary } else { ButtonVariant::Ghost })
                    .hovered(zone.is_hovered())
                    .pressed(zone.is_active())
                    .scale(s)
                    .draw(painter, text, fox, sw, sh);
            }
            cy += row;
            let row_y = cy + (row - btn_h) / 2.0 - row;
            // Second row of buttons sits below; we draw on this row position.
            let row2_y = row_y + row;
            for (col, (zone_id, value, label)) in POSITIONS.iter().skip(2).enumerate() {
                let bx = ctrl_x + col as f32 * (btn_w + btn_gap);
                let rect = Rect::new(bx, row2_y, btn_w, btn_h);
                let zone = ix.add_zone(*zone_id, rect);
                let selected = config.notifications.position.eq_ignore_ascii_case(value);
                Button::new(rect, label)
                    .variant(if selected { ButtonVariant::Primary } else { ButtonVariant::Ghost })
                    .hovered(zone.is_hovered())
                    .pressed(zone.is_active())
                    .scale(s)
                    .draw(painter, text, fox, sw, sh);
            }
        }
    }

    cy_top += display_card_h + CARD_GAP * s;

    // ── Sound ───────────────────────────────────────────────────────
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
        if let Some(f) = slider_value_from_cursor(ix, ZONE_NOTIF_VOLUME, &rect) {
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

    cy_top += sound_card_h + CARD_GAP * s;

    // ── Testing ─────────────────────────────────────────────────────
    {
        let cy = draw_section_card(
            painter, text, fox, "Testing",
            card_x, cy_top, card_w, testing_card_h, s, sw, sh,
        );
        let btn_w = 220.0 * s;
        let btn_h = 44.0 * s;
        let btn_rect = Rect::new(card_inner_x, cy, btn_w, btn_h);
        let zone = ix.add_zone(ZONE_NOTIF_TEST, btn_rect);
        Button::new(btn_rect, "Send Test")
            .variant(ButtonVariant::Ghost)
            .hovered(zone.is_hovered())
            .pressed(zone.is_active())
            .scale(s)
            .draw(painter, text, fox, sw, sh);

        let hint = "Fires a notification via notify-send so you can preview";
        let hint2 = "the current duration, position, and sound.";
        let hint_x = card_inner_x + btn_w + 16.0 * s;
        let hint_y = cy + (btn_h - vsz * 2.0 - 4.0 * s) / 2.0;
        text.queue(hint, vsz, hint_x, hint_y, fox.text_secondary,
            card_inner_w - btn_w - 16.0 * s, sw, sh);
        text.queue(hint2, vsz, hint_x, hint_y + vsz + 4.0 * s, fox.text_secondary,
            card_inner_w - btn_w - 16.0 * s, sw, sh);
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, state.scroll);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

pub fn handle_notifications_click(config: &mut LanternConfig, zone_id: u32) {
    match zone_id {
        ZONE_NOTIF_DND => {
            config.notifications.do_not_disturb = !config.notifications.do_not_disturb;
        }
        ZONE_NOTIF_SHOW => {
            config.notifications.show_toasts = !config.notifications.show_toasts;
        }
        ZONE_NOTIF_SOUND => {
            config.notifications.play_sound = !config.notifications.play_sound;
        }
        ZONE_NOTIF_POS_TR => config.notifications.position = "top-right".into(),
        ZONE_NOTIF_POS_TL => config.notifications.position = "top-left".into(),
        ZONE_NOTIF_POS_BR => config.notifications.position = "bottom-right".into(),
        ZONE_NOTIF_POS_BL => config.notifications.position = "bottom-left".into(),
        ZONE_NOTIF_TEST => {
            let _ = std::process::Command::new("notify-send")
                .arg("Lantern Notifications")
                .arg("This is a preview toast \u{2014} duration, position, and sound apply.")
                .spawn();
        }
        _ => {}
    }
}
