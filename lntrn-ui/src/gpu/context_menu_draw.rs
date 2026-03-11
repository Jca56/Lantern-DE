use std::collections::HashMap;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_theme::{FONT_BODY, FONT_CAPTION, FONT_LABEL};

use crate::animation;

use super::checkbox::Checkbox;
use super::context_menu::{
    ContextMenuStyle, MenuEvent, MenuItem,
    CONTEXT_MENU_ZONE_BASE, HEADER_HEIGHT, PROGRESS_ITEM_HEIGHT, SEPARATOR_HEIGHT,
    SLIDER_ITEM_HEIGHT, SLIDER_LABEL_SIZE, SLIDER_TRACK_H,
    items_height_slice,
};
use super::controls::{Button, ButtonVariant};
use super::input::{InteractionContext, InteractionState};
use super::progress::ProgressBar;
use super::radio::RadioButton;
use super::toggle::Toggle;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_panel(
    items: &mut [MenuItem],
    px: f32,
    py: f32,
    width: f32,
    depth: usize,
    style: &ContextMenuStyle,
    painter: &mut Painter,
    text: &mut TextRenderer,
    interaction: &mut InteractionContext,
    screen_w: u32,
    screen_h: u32,
    open_submenu_ids: &mut Vec<u32>,
    hover_t: &HashMap<u32, f32>,
    hovered_zones: &mut Vec<u32>,
    pressed_zones: &mut Vec<u32>,
    alpha: f32,
    interactive: bool,
) -> Option<MenuEvent> {
    let s = style.scale;
    let total_h = items_height_slice(items, style);
    let menu_rect = Rect::new(px, py, width, total_h);
    let cr = style.corner_radius * s;
    let pal = &style.palette;

    // Shadow fades, background is opaque
    let shadow = menu_rect.expand(3.0 * s);
    painter.rect_filled(shadow, cr + 2.0 * s, Color::BLACK.with_alpha(0.25 * alpha));
    painter.rect_filled(menu_rect, cr, style.bg);
    painter.rect_stroke(menu_rect, cr, style.border_width * s, style.border.with_alpha(alpha));

    let mut event = None;
    let mut cy = py + style.padding * s;
    let inner_w = width - style.padding * 2.0 * s;
    let inner_x = px + style.padding * s;
    let zone_base = CONTEXT_MENU_ZONE_BASE + (depth as u32) * 0x1000;
    let item_h = style.item_height * s;
    let font = FONT_BODY * s;
    let pad = style.padding * s;
    let shortcut_font = FONT_LABEL * s;

    for item in items.iter_mut() {
        match item {
            MenuItem::Action { id, label, shortcut, enabled } => {
                let e = draw_action_item(
                    *id, label, shortcut.as_deref(), *enabled,
                    inner_x, cy, inner_w, item_h, cr, pad, font, shortcut_font,
                    style, s, painter, text, interaction,
                    screen_w, screen_h, zone_base, hover_t, hovered_zones,
                    pressed_zones, interactive,
                );
                if e.is_some() { event = e; }
                cy += item_h;
            }
            MenuItem::Toggle { id, label, checked, enabled } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, item_rect, interactive);

                // Hover highlight
                draw_hover_bg(state.is_hovered() && *enabled, zone_id, hover_t,
                    hovered_zones, item_rect, cr, s, style, painter);

                // Reuse Toggle widget — position the track inside the item
                let toggle_rect = Rect::new(inner_x + pad, cy, inner_w - pad * 2.0, item_h);
                Toggle::new(toggle_rect, *checked)
                    .label(label)
                    .hovered(state.is_hovered())
                    .disabled(!*enabled)
                    .draw(painter, text, pal, screen_w, screen_h);

                if *enabled && state == InteractionState::Pressed
                    && !pressed_zones.contains(&zone_id)
                {
                    pressed_zones.push(zone_id);
                    *checked = !*checked;
                    event = Some(MenuEvent::Toggled { id: *id, checked: *checked });
                }
                cy += item_h;
            }
            MenuItem::Checkbox { id, label, checked } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, item_rect, interactive);

                draw_hover_bg(state.is_hovered(), zone_id, hover_t,
                    hovered_zones, item_rect, cr, s, style, painter);

                let cb_rect = Rect::new(inner_x + pad, cy, inner_w - pad * 2.0, item_h);
                Checkbox::new(cb_rect, *checked)
                    .label(label)
                    .hovered(state.is_hovered())
                    .draw(painter, text, pal, screen_w, screen_h);

                if state == InteractionState::Pressed
                    && !pressed_zones.contains(&zone_id)
                {
                    pressed_zones.push(zone_id);
                    *checked = !*checked;
                    event = Some(MenuEvent::CheckboxToggled { id: *id, checked: *checked });
                }
                cy += item_h;
            }
            MenuItem::Radio { id, group, label, selected } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, item_rect, interactive);

                draw_hover_bg(state.is_hovered(), zone_id, hover_t,
                    hovered_zones, item_rect, cr, s, style, painter);

                let radio_rect = Rect::new(inner_x + pad, cy, inner_w - pad * 2.0, item_h);
                RadioButton::new(radio_rect, *selected)
                    .label(label)
                    .hovered(state.is_hovered())
                    .draw(painter, text, pal, screen_w, screen_h);

                if state == InteractionState::Pressed && !*selected
                    && !pressed_zones.contains(&zone_id)
                {
                    pressed_zones.push(zone_id);
                    *selected = true;
                    event = Some(MenuEvent::RadioSelected { id: *id, group: *group });
                }
                cy += item_h;
            }
            MenuItem::Button { id, label, primary } => {
                let btn_pad = pad * 3.0;
                let btn_rect = Rect::new(
                    inner_x + btn_pad, cy + 2.0 * s,
                    inner_w - btn_pad * 2.0, item_h - 4.0 * s,
                );
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, btn_rect, interactive);

                let variant = if *primary { ButtonVariant::Primary } else { ButtonVariant::Default };
                Button::new(btn_rect, label)
                    .variant(variant)
                    .hovered(state.is_hovered())
                    .pressed(state == InteractionState::Pressed)
                    .draw(painter, text, pal, screen_w, screen_h);

                if state == InteractionState::Pressed
                    && !pressed_zones.contains(&zone_id)
                {
                    pressed_zones.push(zone_id);
                    event = Some(MenuEvent::Action(*id));
                }
                cy += item_h;
            }
            MenuItem::Progress { id: _, label, value } => {
                let prog_h = PROGRESS_ITEM_HEIGHT * s;
                let label_size = FONT_CAPTION * s;

                // Label + percentage
                let label_x = inner_x + pad * 2.0;
                let label_y = cy + 4.0 * s;
                text.queue(
                    label, label_size, label_x, label_y, style.text_muted,
                    inner_w * 0.6, screen_w, screen_h,
                );
                let pct = format!("{}%", (*value * 100.0).round() as u32);
                let pct_w = pct.len() as f32 * label_size * 0.55;
                let pct_x = inner_x + inner_w - pad * 2.0 - pct_w;
                text.queue(
                    &pct, label_size, pct_x, label_y, style.text_muted,
                    pct_w + 4.0 * s, screen_w, screen_h,
                );

                // Reuse ProgressBar widget
                let bar_y = label_y + label_size + 4.0 * s;
                let bar_rect = Rect::new(
                    inner_x + pad * 2.0, bar_y,
                    inner_w - pad * 4.0, 12.0 * s,
                );
                ProgressBar::new(bar_rect)
                    .value(*value)
                    .draw(painter, text, pal, screen_w, screen_h);

                cy += prog_h;
            }
            MenuItem::Header { label } => {
                let header_h = HEADER_HEIGHT * s;
                let header_font = FONT_LABEL * s;
                let text_x = inner_x + pad * 2.0;
                let text_y = cy + (header_h - header_font) * 0.5 + 2.0 * s;
                text.queue(
                    label, header_font, text_x, text_y, style.text_muted,
                    inner_w - pad * 4.0, screen_w, screen_h,
                );
                cy += header_h;
            }
            MenuItem::Separator => {
                let sep_h = SEPARATOR_HEIGHT * s;
                let sep_y = cy + sep_h * 0.5;
                let sep_x = inner_x + pad;
                let sep_w = inner_w - pad * 2.0;
                painter.rect_filled(
                    Rect::new(sep_x, sep_y, sep_w, 1.0 * s), 0.0, style.separator,
                );
                cy += sep_h;
            }
            MenuItem::Slider { id, label, value } => {
                let slider_h = SLIDER_ITEM_HEIGHT * s;
                let label_size = FONT_CAPTION * s;
                let track_h = SLIDER_TRACK_H * s;

                let item_rect = Rect::new(inner_x, cy, inner_w, slider_h);
                let zone_id = zone_base + *id;
                let zone_state = if interactive {
                    interaction.add_zone(zone_id, item_rect)
                } else {
                    InteractionState::Idle
                };

                let label_x = inner_x + pad * 2.0;
                let label_y = cy + 6.0 * s;
                text.queue(
                    label, label_size, label_x, label_y, style.text_muted,
                    inner_w * 0.6, screen_w, screen_h,
                );
                let pct = format!("{}%", (*value * 100.0).round() as u32);
                let pct_w = pct.len() as f32 * label_size * 0.55;
                let pct_x = inner_x + inner_w - pad * 2.0 - pct_w;
                text.queue(
                    &pct, label_size, pct_x, label_y, style.text_muted,
                    pct_w + 4.0 * s, screen_w, screen_h,
                );

                let track_pad = pad * 2.0;
                let track_y = label_y + label_size + 8.0 * s;
                let track_w = inner_w - track_pad * 2.0;
                let track = Rect::new(inner_x + track_pad, track_y, track_w, track_h);

                painter.rect_filled(track, track_h * 0.5, pal.surface_2);
                let fill_w = (track_w * *value).max(track_h);
                painter.rect_filled(
                    Rect::new(track.x, track.y, fill_w, track_h),
                    track_h * 0.5, style.accent,
                );

                let thumb_x = track.x + track_w * *value;
                let thumb_cy = track.y + track_h * 0.5;
                let thumb_r = if zone_state.is_active() { 8.0 * s }
                    else if zone_state.is_hovered() { 7.0 * s }
                    else { 6.0 * s };
                painter.circle_filled(thumb_x, thumb_cy, thumb_r, Color::WHITE);
                painter.circle_stroke(
                    thumb_x, thumb_cy, thumb_r, 1.0 * s,
                    Color::rgba(0.0, 0.0, 0.0, 0.2),
                );

                if zone_state.is_active() {
                    if let Some(frac) = interaction.drag_fraction_x(&track) {
                        *value = frac;
                        event = Some(MenuEvent::SliderChanged { id: *id, value: frac });
                    }
                }
                cy += slider_h;
            }
            MenuItem::SubMenu { id, label, .. } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, item_rect, interactive);

                let is_open = open_submenu_ids.get(depth) == Some(id);
                draw_hover_bg(state.is_hovered() || is_open, zone_id, hover_t,
                    hovered_zones, item_rect, cr, s, style, painter);

                if state.is_hovered() && !is_open {
                    open_submenu_ids.truncate(depth);
                    open_submenu_ids.push(*id);
                }

                let text_x = inner_x + pad * 2.0;
                let text_y = cy + (item_h - font) * 0.5;
                text.queue(
                    label, font, text_x, text_y, style.text,
                    inner_w - pad * 6.0, screen_w, screen_h,
                );

                let arrow_x = inner_x + inner_w - pad * 2.0 - 6.0 * s;
                let arrow_cy = cy + item_h * 0.5;
                let ac = if is_open { style.accent } else { style.text_muted };
                painter.line(arrow_x, arrow_cy - 5.0 * s, arrow_x + 5.0 * s, arrow_cy, 1.5 * s, ac);
                painter.line(arrow_x + 5.0 * s, arrow_cy, arrow_x, arrow_cy + 5.0 * s, 1.5 * s, ac);

                cy += item_h;
            }
        }
    }

    event
}

// ── Shared helpers ───────────────────────────────────────────────────────────

fn zone_state(
    interaction: &mut InteractionContext,
    zone_id: u32,
    rect: Rect,
    interactive: bool,
) -> InteractionState {
    if interactive { interaction.add_zone(zone_id, rect) }
    else { InteractionState::Idle }
}

/// Draw animated hover background highlight.
fn draw_hover_bg(
    is_hovered: bool,
    zone_id: u32,
    hover_t: &HashMap<u32, f32>,
    hovered_zones: &mut Vec<u32>,
    rect: Rect,
    cr: f32,
    s: f32,
    style: &ContextMenuStyle,
    painter: &mut Painter,
) {
    if is_hovered {
        hovered_zones.push(zone_id);
    }
    let t = animation::ease_out(*hover_t.get(&zone_id).unwrap_or(&0.0));
    if t > 0.001 {
        painter.rect_filled(
            rect, cr - 2.0 * s,
            style.bg_hover.with_alpha(style.bg_hover.a * t),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_action_item(
    id: u32,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    inner_x: f32, cy: f32, inner_w: f32, item_h: f32,
    cr: f32, pad: f32, font: f32, shortcut_font: f32,
    style: &ContextMenuStyle, s: f32,
    painter: &mut Painter, text: &mut TextRenderer,
    interaction: &mut InteractionContext,
    screen_w: u32, screen_h: u32,
    zone_base: u32,
    hover_t: &HashMap<u32, f32>,
    hovered_zones: &mut Vec<u32>,
    pressed_zones: &mut Vec<u32>,
    interactive: bool,
) -> Option<MenuEvent> {
    let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
    let zone_id = zone_base + id;
    let state = zone_state(interaction, zone_id, item_rect, interactive);

    let text_color = if enabled { style.text } else { style.text_disabled };
    let hovered = enabled
        && (state == InteractionState::Hovered || state == InteractionState::Pressed);

    draw_hover_bg(hovered, zone_id, hover_t, hovered_zones, item_rect, cr, s, style, painter);

    let event = if enabled && state == InteractionState::Pressed
        && !pressed_zones.contains(&zone_id)
    {
        pressed_zones.push(zone_id);
        Some(MenuEvent::Action(id))
    } else {
        None
    };

    let text_x = inner_x + pad * 2.0;
    let text_y = cy + (item_h - font) * 0.5;
    text.queue(label, font, text_x, text_y, text_color, inner_w - pad * 4.0, screen_w, screen_h);

    if let Some(sc_text) = shortcut {
        let sc_w = sc_text.len() as f32 * shortcut_font * 0.55;
        let sc_x = inner_x + inner_w - pad * 2.0 - sc_w;
        let sc_y = cy + (item_h - shortcut_font) * 0.5;
        let sc_color = if enabled { style.text_muted } else { style.text_disabled };
        text.queue(sc_text, shortcut_font, sc_x, sc_y, sc_color, sc_w + 4.0 * s, screen_w, screen_h);
    }

    event
}
