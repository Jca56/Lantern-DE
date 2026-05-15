use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Button, ButtonVariant, FoxPalette, InteractionContext, ScrollArea, Scrollbar, Slider, Toggle,
};

use crate::config::LanternConfig;
use crate::panels::{
    draw_color_swatch_row, draw_section_card, slider_value_from_cursor, PanelState, CARD_GAP,
    CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V, CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
    GLOW_COLORS, LABEL_SIZE, LABEL_W, ROW_H, SLIDER_H, SLIDER_W, TOGGLE_H, VALUE_SIZE, VALUE_W,
};

const ZONE_WM_BORDER: u32 = 300;
const ZONE_WM_TITLEBAR: u32 = 301;
const ZONE_WM_GAP: u32 = 302;
const ZONE_WM_CORNER: u32 = 303;
const ZONE_WM_FOCUS: u32 = 304;
const ZONE_WM_BLUR: u32 = 306;
const ZONE_WM_TINT: u32 = 307;
const ZONE_WM_DARKEN: u32 = 308;
const ZONE_WM_BG_OPACITY: u32 = 309;
const ZONE_WM_GLOW: u32 = 310;
const ZONE_WM_GLOW_COLOR_BASE: u32 = 311; // 311..319 swatches
const ZONE_WM_GLOW_INTENSITY: u32 = 320;
const ZONE_WM_TEST_SPAWN: u32 = 321;
const ZONE_WM_ANIM_ENABLE: u32 = 322;
const ZONE_WM_ANIM_SPEED: u32 = 323;
const ZONE_WM_BORDER_COLOR_BASE: u32 = 330; // 330..339 swatches
const ZONE_WM_TINT_COLOR_BASE: u32 = 340; // 340..349 swatches

pub fn draw_wm_panel(
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

    let layout_rows: f32 = 5.0; // border, border color, titlebar, gap, corner
    let focus_base_rows: f32 = 2.0; // focus follows mouse, focus glow toggle
    let glow_extra_rows: f32 = if config.window_manager.focus_glow { 2.0 } else { 0.0 };
    let focus_rows = focus_base_rows + glow_extra_rows;
    let effects_rows: f32 = 5.0; // blur intensity, blur tint, tint color, darken, bg opacity

    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;
    let layout_card_h = card_chrome_h + layout_rows * row;
    let focus_card_h = card_chrome_h + focus_rows * row;
    let effects_card_h = card_chrome_h + effects_rows * row;
    let animations_card_h = card_chrome_h + 2.0 * row;
    let testing_card_h = card_chrome_h + row * 1.5;

    let content_height = CARD_OUTER_PAD_V * s
        + layout_card_h + CARD_GAP * s
        + focus_card_h + CARD_GAP * s
        + effects_card_h + CARD_GAP * s
        + animations_card_h + CARD_GAP * s
        + testing_card_h + CARD_OUTER_PAD_V * 2.0 * s;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(
            &mut panel_state.wm_scroll, scroll_delta * 40.0,
            content_height, panel_h,
        );
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut panel_state.wm_scroll);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    // ── Card 1: Layout ──────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Layout",
            card_x, cy_top, card_w, layout_card_h, s, sw, sh,
        );

        {
            let mut slider_row = |label: &str, frac: f32, zone_id: u32, cy: &mut f32,
                                  min: f32, max: f32, suffix: &str, config_val: &mut u32| {
                let label_y = *cy + (row - lsz) / 2.0;
                text.queue(label, lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
                let rect = Rect::new(ctrl_x, *cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
                let zone = ix.add_zone(zone_id, rect);
                if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
                    *config_val = (min + f * (max - min)).round() as u32;
                }
                Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                    .draw(painter, fox);
                let val = format!("{}{}", *config_val, suffix);
                text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
                *cy += row;
            };

            let frac = config.window_manager.border_width as f32 / 10.0;
            let mut bw = config.window_manager.border_width;
            slider_row("Border Width", frac, ZONE_WM_BORDER, &mut cy, 0.0, 10.0, "", &mut bw);
            config.window_manager.border_width = bw;

            let frac = (config.window_manager.titlebar_height as f32 - 20.0) / 40.0;
            let mut th = config.window_manager.titlebar_height;
            slider_row("Titlebar Height", frac, ZONE_WM_TITLEBAR, &mut cy, 20.0, 60.0, "px", &mut th);
            config.window_manager.titlebar_height = th;

            let frac = config.window_manager.gap as f32 / 32.0;
            let mut gap = config.window_manager.gap;
            slider_row("Window Gap", frac, ZONE_WM_GAP, &mut cy, 0.0, 32.0, "px", &mut gap);
            config.window_manager.gap = gap;

            let frac = config.window_manager.corner_radius as f32 / 20.0;
            let mut cr = config.window_manager.corner_radius;
            slider_row("Corner Radius", frac, ZONE_WM_CORNER, &mut cy, 0.0, 20.0, "px", &mut cr);
            config.window_manager.corner_radius = cr;
        }

        draw_color_swatch_row(
            painter, text, ix, fox,
            "Border Color", ZONE_WM_BORDER_COLOR_BASE,
            &config.window_manager.border_color,
            label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
        );
    }

    cy_top += layout_card_h + CARD_GAP * s;

    // ── Card 2: Focus & Glow ────────────────────────────────────────
    let mut cy = draw_section_card(
        painter, text, fox, "Focus & Glow",
        card_x, cy_top, card_w, focus_card_h, s, sw, sh,
    );
    {
        let toggle_w = card_inner_w;
        let rect = Rect::new(card_inner_x, cy, toggle_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.window_manager.focus_follows_mouse)
            .label("Focus Follows Mouse").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_WM_FOCUS, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
    }
    {
        let toggle_w = card_inner_w;
        let rect = Rect::new(card_inner_x, cy, toggle_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.window_manager.focus_glow)
            .label("Focus Glow").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_WM_GLOW, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
    }

    if config.window_manager.focus_glow {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Glow Color", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

        let swatch_size = 28.0 * s;
        let swatch_gap = 8.0 * s;
        let mut sx = ctrl_x;
        for (i, (hex, _name)) in GLOW_COLORS.iter().enumerate() {
            let color = Color::from_hex(hex).unwrap();
            let zone_id = ZONE_WM_GLOW_COLOR_BASE + i as u32;
            let swatch_rect = Rect::new(sx, cy + (row - swatch_size) / 2.0, swatch_size, swatch_size);
            let zone = ix.add_zone(zone_id, swatch_rect);

            let cx = sx + swatch_size / 2.0;
            let cy_center = swatch_rect.y + swatch_size / 2.0;
            let radius = swatch_size / 2.0;

            painter.circle_filled(cx, cy_center, radius, color);

            let is_selected = config.window_manager.focus_glow_color.eq_ignore_ascii_case(hex);
            if is_selected {
                painter.circle_stroke(cx, cy_center, radius + 3.0 * s, 2.0 * s, fox.text);
            } else if zone.is_hovered() {
                painter.circle_stroke(cx, cy_center, radius + 2.0 * s, 1.5 * s, fox.text_secondary);
            }
            sx += swatch_size + swatch_gap;
        }
        cy += row;

        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Glow Intensity", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let frac = (config.window_manager.focus_glow_intensity / 0.6).clamp(0.0, 1.0);
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_WM_GLOW_INTENSITY, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_WM_GLOW_INTENSITY, &rect) {
            config.window_manager.focus_glow_intensity = ((f * 0.6) * 100.0).round() / 100.0;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let pct = (config.window_manager.focus_glow_intensity / 0.6 * 100.0).round() as i32;
        let val = format!("{}%", pct);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
    }

    cy_top += focus_card_h + CARD_GAP * s;

    // ── Card 3: Visual Effects ──────────────────────────────────────
    let mut cy = draw_section_card(
        painter, text, fox, "Visual Effects",
        card_x, cy_top, card_w, effects_card_h, s, sw, sh,
    );

    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Blur Intensity", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let frac = config.windows.blur_intensity;
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_WM_BLUR, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_WM_BLUR, &rect) {
            config.windows.blur_intensity = (f * 100.0).round() / 100.0;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", config.windows.blur_intensity * 100.0);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        cy += row;
    }

    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Blur Tint", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let frac = config.windows.blur_tint;
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_WM_TINT, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_WM_TINT, &rect) {
            config.windows.blur_tint = (f * 100.0).round() / 100.0;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", config.windows.blur_tint * 100.0);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        cy += row;
    }

    draw_color_swatch_row(
        painter, text, ix, fox,
        "Tint Color", ZONE_WM_TINT_COLOR_BASE,
        &config.windows.blur_tint_color,
        label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
    );

    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Blur Darken", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let frac = config.windows.blur_darken;
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_WM_DARKEN, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_WM_DARKEN, &rect) {
            config.windows.blur_darken = (f * 100.0).round() / 100.0;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", config.windows.blur_darken * 100.0);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        cy += row;
    }

    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Background Opacity", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let frac = config.windows.background_opacity;
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_WM_BG_OPACITY, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_WM_BG_OPACITY, &rect) {
            config.windows.background_opacity = (f * 100.0).round() / 100.0;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", config.windows.background_opacity * 100.0);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
    }

    cy_top += effects_card_h + CARD_GAP * s;

    // ── Card 4: Animations ──────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Animations",
            card_x, cy_top, card_w, animations_card_h, s, sw, sh,
        );

        {
            let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
            let toggle = Toggle::new(rect, config.animations.enabled)
                .label("Enable Animations").scale(s);
            let track = toggle.track_rect();
            let zone = ix.add_zone(ZONE_WM_ANIM_ENABLE, track);
            toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
            cy += row;
        }

        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Speed", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
            let frac = ((config.animations.speed - 0.25) / 2.75).clamp(0.0, 1.0);
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_WM_ANIM_SPEED, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_WM_ANIM_SPEED, &rect) {
                let raw = 0.25 + f * 2.75;
                config.animations.speed = (raw / 0.05).round() * 0.05;
                config.animations.speed = config.animations.speed.clamp(0.25, 3.0);
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            let val = format!("{:.2}x", config.animations.speed);
            text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        }
    }

    cy_top += animations_card_h + CARD_GAP * s;

    // ── Card 5: Testing ─────────────────────────────────────────────
    {
        let cy = draw_section_card(
            painter, text, fox, "Testing",
            card_x, cy_top, card_w, testing_card_h, s, sw, sh,
        );
        let btn_w = 220.0 * s;
        let btn_h = 44.0 * s;
        let btn_rect = Rect::new(card_inner_x, cy, btn_w, btn_h);
        let zone = ix.add_zone(ZONE_WM_TEST_SPAWN, btn_rect);
        Button::new(btn_rect, "Open Test Window")
            .variant(ButtonVariant::Ghost)
            .hovered(zone.is_hovered())
            .pressed(zone.is_active())
            .scale(s)
            .draw(painter, text, fox, sw, sh);

        let hint = "Blank 500\u{00d7}500 window — uses Lantern's SSD so titlebar,";
        let hint2 = "corner radius, and border width sliders apply.";
        let hint_x = card_inner_x + btn_w + 16.0 * s;
        let hint_y = cy + (btn_h - vsz * 2.0 - 4.0 * s) / 2.0;
        text.queue(hint, vsz, hint_x, hint_y, fox.text_secondary,
            card_inner_w - btn_w - 16.0 * s, sw, sh);
        text.queue(hint2, vsz, hint_x, hint_y + vsz + 4.0 * s, fox.text_secondary,
            card_inner_w - btn_w - 16.0 * s, sw, sh);
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, panel_state.wm_scroll);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

pub fn handle_wm_click(config: &mut LanternConfig, zone_id: u32) {
    if zone_id == ZONE_WM_FOCUS {
        config.window_manager.focus_follows_mouse = !config.window_manager.focus_follows_mouse;
    } else if zone_id == ZONE_WM_GLOW {
        config.window_manager.focus_glow = !config.window_manager.focus_glow;
    } else if zone_id >= ZONE_WM_GLOW_COLOR_BASE
        && zone_id < ZONE_WM_GLOW_COLOR_BASE + GLOW_COLORS.len() as u32
    {
        let idx = (zone_id - ZONE_WM_GLOW_COLOR_BASE) as usize;
        config.window_manager.focus_glow_color = GLOW_COLORS[idx].0.into();
    } else if zone_id >= ZONE_WM_BORDER_COLOR_BASE
        && zone_id < ZONE_WM_BORDER_COLOR_BASE + GLOW_COLORS.len() as u32
    {
        let idx = (zone_id - ZONE_WM_BORDER_COLOR_BASE) as usize;
        config.window_manager.border_color = GLOW_COLORS[idx].0.into();
    } else if zone_id >= ZONE_WM_TINT_COLOR_BASE
        && zone_id < ZONE_WM_TINT_COLOR_BASE + GLOW_COLORS.len() as u32
    {
        let idx = (zone_id - ZONE_WM_TINT_COLOR_BASE) as usize;
        config.windows.blur_tint_color = GLOW_COLORS[idx].0.into();
    } else if zone_id == ZONE_WM_ANIM_ENABLE {
        config.animations.enabled = !config.animations.enabled;
    } else if zone_id == ZONE_WM_TEST_SPAWN {
        let _ = std::process::Command::new(std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("lntrn-system-settings")))
            .arg("--test-window")
            .spawn();
    }
}
