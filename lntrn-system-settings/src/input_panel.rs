use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Slider, Toggle};

use crate::config::LanternConfig;

const ZONE_MOUSE_SPEED: u32 = 800;
const ZONE_MOUSE_ACCEL: u32 = 801;
const ZONE_CURSOR_DEFAULT: u32 = 810;
const ZONE_CURSOR_CUSTOM1: u32 = 811;
const ZONE_CURSOR_CUSTOM2: u32 = 812;

const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const VALUE_SIZE: f32 = 16.0;
const SLIDER_H: f32 = 36.0;
const TOGGLE_H: f32 = 36.0;
const PAD_LEFT: f32 = 24.0;
const PAD_RIGHT: f32 = 32.0;
const LABEL_W: f32 = 200.0;
const VALUE_W: f32 = 60.0;

const CURSOR_THEMES: &[(&str, &str)] = &[
    ("default", "Default"),
    ("custom1", "Cursor 1"),
    ("custom2", "Cursor 2"),
];

fn layout(x: f32, w: f32, s: f32) -> (f32, f32, f32, f32) {
    let pad_l = PAD_LEFT * s;
    let pad_r = PAD_RIGHT * s;
    let val_w = VALUE_W * s;
    let label_x = x + pad_l;
    let label_w = LABEL_W * s;
    let ctrl_x = label_x + label_w;
    let ctrl_w = w - pad_l - pad_r - label_w - val_w - 12.0 * s;
    let value_x = ctrl_x + ctrl_w + 8.0 * s;
    (label_x, ctrl_x, ctrl_w.max(80.0 * s), value_x)
}

fn slider_value_from_cursor(
    ix: &InteractionContext, zone_id: u32, rect: &Rect,
) -> Option<f32> {
    let state = ix.zone_state(zone_id);
    if state.is_active() {
        if let Some((cx, _)) = ix.cursor() {
            return Some(((cx - rect.x) / rect.w).clamp(0.0, 1.0));
        }
    }
    None
}

// ── Input panel ─────────────────────────────────────────────────────────────

pub fn draw_input_panel(
    config: &mut LanternConfig,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, x: f32, y: f32, w: f32, s: f32, sw: u32, sh: u32,
) {
    let (label_x, ctrl_x, ctrl_w, value_x) = layout(x, w, s);
    let mut cy = y;
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    // ── Section: Mouse ──────────────────────────────────────────────
    text.queue("Mouse", lsz, label_x, cy, fox.text_secondary, LABEL_W * s, sw, sh);
    cy += lsz + 8.0 * s;

    // Mouse Speed slider (-1.0 to 1.0, displayed as percentage)
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Mouse Speed", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

        // Map -1..1 to 0..1 for slider fraction
        let frac = (config.input.mouse_speed + 1.0) / 2.0;
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(ZONE_MOUSE_SPEED, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_MOUSE_SPEED, &rect) {
            // Map 0..1 back to -1..1, snap to nearest 0.05
            let raw = f * 2.0 - 1.0;
            config.input.mouse_speed = (raw / 0.05).round() * 0.05;
            config.input.mouse_speed = config.input.mouse_speed.clamp(-1.0, 1.0);
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);

        // Display as percentage (-100% to +100%)
        let pct = (config.input.mouse_speed * 100.0).round() as i32;
        let val = if pct == 0 {
            "0%".to_string()
        } else if pct > 0 {
            format!("+{}%", pct)
        } else {
            format!("{}%", pct)
        };
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        cy += row;
    }

    // Mouse Acceleration toggle
    {
        let rect = Rect::new(label_x, cy, w - PAD_LEFT * s - PAD_RIGHT * s, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.input.mouse_acceleration)
            .label("Mouse Acceleration").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_MOUSE_ACCEL, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
    }

    // ── Section separator ───────────────────────────────────────────
    cy += 8.0 * s;
    painter.rect_filled(
        Rect::new(label_x, cy, w - PAD_LEFT * s - PAD_RIGHT * s, 1.0 * s),
        0.0, fox.muted.with_alpha(0.2),
    );
    cy += 16.0 * s;

    // ── Section: Cursor Theme ───────────────────────────────────────
    text.queue("Cursor Theme", lsz, label_x, cy, fox.text_secondary, LABEL_W * s, sw, sh);
    cy += lsz + 12.0 * s;

    let card_size = 100.0 * s;
    let card_gap = 16.0 * s;
    let card_r = 8.0 * s;
    let zone_ids = [ZONE_CURSOR_DEFAULT, ZONE_CURSOR_CUSTOM1, ZONE_CURSOR_CUSTOM2];

    for (i, (theme_id, theme_label)) in CURSOR_THEMES.iter().enumerate() {
        let card_x = label_x + i as f32 * (card_size + card_gap);
        let card_rect = Rect::new(card_x, cy, card_size, card_size);
        let zone = ix.add_zone(zone_ids[i], card_rect);

        let is_selected = config.input.cursor_theme == *theme_id;

        // Card background
        let bg = if is_selected {
            fox.accent.with_alpha(0.15)
        } else if zone.is_hovered() {
            fox.surface_2
        } else {
            fox.surface
        };
        painter.rect_filled(card_rect, card_r, bg);

        // Border
        let border_color = if is_selected {
            fox.accent
        } else {
            fox.muted.with_alpha(0.3)
        };
        let border_w = if is_selected { 2.0 * s } else { 1.0 * s };
        painter.rect_stroke_sdf(card_rect, card_r, border_w, border_color);

        // Draw a simple cursor icon placeholder in the card
        let icon_size = 32.0 * s;
        let icon_x = card_x + (card_size - icon_size) / 2.0;
        let icon_y = cy + (card_size - icon_size) / 2.0 - 8.0 * s;
        draw_cursor_preview(painter, icon_x, icon_y, icon_size, fox, i);

        // Label below the icon
        let label_font = 14.0 * s;
        let label_y = cy + card_size - label_font - 8.0 * s;
        let label_color = if is_selected { fox.accent } else { fox.text };
        text.queue(theme_label, label_font, card_x + 4.0 * s, label_y, label_color,
            card_size - 8.0 * s, sw, sh);
    }
}

/// Draw a simple cursor arrow preview shape in each card.
fn draw_cursor_preview(painter: &mut Painter, x: f32, y: f32, size: f32, fox: &FoxPalette, variant: usize) {
    let color = match variant {
        0 => fox.text,              // Default: white arrow
        1 => fox.accent,            // Custom 1: accent colored
        2 => fox.text_secondary,    // Custom 2: muted color
        _ => fox.text,
    };

    // Simple arrow shape using lines
    let tip_x = x + size * 0.3;
    let tip_y = y;
    let bottom_y = y + size * 0.85;
    let right_x = x + size * 0.65;
    let mid_y = y + size * 0.55;
    let lw = 2.0;

    // Left edge
    painter.line(tip_x, tip_y, tip_x, bottom_y, lw, color);
    // Bottom-left to middle
    painter.line(tip_x, bottom_y, tip_x + size * 0.15, mid_y, lw, color);
    // Diagonal handle
    painter.line(tip_x + size * 0.15, mid_y, right_x, y + size * 0.85, lw, color);
    // Handle right edge
    painter.line(right_x, y + size * 0.85, right_x - size * 0.1, mid_y + size * 0.05, lw, color);
    // Back to top-right
    painter.line(right_x - size * 0.1, mid_y + size * 0.05, tip_x + size * 0.25, mid_y, lw, color);
    // Top-right back to tip
    painter.line(tip_x + size * 0.25, mid_y, tip_x, tip_y, lw, color);
}

// ── Click handling ──────────────────────────────────────────────────────────

pub fn handle_input_click(config: &mut LanternConfig, zone_id: u32) {
    match zone_id {
        ZONE_MOUSE_ACCEL => {
            config.input.mouse_acceleration = !config.input.mouse_acceleration;
        }
        ZONE_CURSOR_DEFAULT => {
            config.input.cursor_theme = "default".into();
        }
        ZONE_CURSOR_CUSTOM1 => {
            config.input.cursor_theme = "custom1".into();
        }
        ZONE_CURSOR_CUSTOM2 => {
            config.input.cursor_theme = "custom2".into();
        }
        _ => {}
    }
}
