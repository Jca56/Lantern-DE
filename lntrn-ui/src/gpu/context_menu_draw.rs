use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_theme::{FONT_CAPTION, FONT_LABEL};

use super::checkbox::Checkbox;
use super::context_menu::{
    ContextMenuStyle, MenuEvent, MenuItem,
    ACCENT_BAR_WIDTH, COLOR_SWATCH_HEIGHT, CONTEXT_MENU_ZONE_BASE, HEADER_HEIGHT,
    PROGRESS_ITEM_HEIGHT, SEPARATOR_HEIGHT, SLIDER_ITEM_HEIGHT, SLIDER_TRACK_H,
    items_height_slice,
};
use super::controls::{Button, ButtonVariant};
use super::input::{InteractionContext, InteractionState};
use super::progress::ProgressBar;
use super::radio::RadioButton;
use super::toggle::Toggle;

/// Result returned by `draw_panel` so the caller can process submenu hover.
pub(super) struct DrawPanelResult {
    pub event: Option<MenuEvent>,
    /// If the cursor is hovering a SubMenu trigger, its id.
    pub hovered_submenu: Option<u32>,
    /// Whether any non-submenu item is hovered on this panel.
    pub non_submenu_hovered: bool,
}

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
    pressed_zones: &mut Vec<u32>,
) -> DrawPanelResult {
    let s = style.scale;
    let total_h = items_height_slice(items, style);
    let menu_rect = Rect::new(px, py, width, total_h);
    let cr = style.corner_radius * s;
    let pal = &style.palette;

    // Multi-layer shadow (skip for popup surfaces — compositor handles shadows)
    if !style.no_shadow {
        let shadow_layers: [(f32, f32); 3] = [
            (12.0, 0.12),
            (5.0, 0.18),
            (2.0, 0.25),
        ];
        for (expand, opacity) in shadow_layers {
            let shadow = menu_rect.expand(expand * s);
            painter.rect_filled(shadow, cr + expand * s, Color::BLACK.with_alpha(opacity));
        }
    }

    // Background
    painter.rect_filled(menu_rect, cr, style.bg);
    painter.rect_stroke_sdf(menu_rect, cr, style.border_width * s, style.border);

    let mut event = None;
    let mut hovered_submenu: Option<u32> = None;
    let mut non_submenu_hovered = false;
    let mut cy = py + style.padding * s;
    let inner_w = width - style.padding * 2.0 * s;
    let inner_x = px + style.padding * s;
    let zone_base = CONTEXT_MENU_ZONE_BASE + (depth as u32) * 0x1000;
    let item_h = style.item_height * s;
    let font = style.font_size * s;
    let pad = style.padding * s;
    let shortcut_font = FONT_LABEL * s;
    // Extra left inset so content clears the accent bar
    let accent_inset = (ACCENT_BAR_WIDTH + 6.0) * s;
    let content_x = inner_x + pad + accent_inset;
    let content_w = inner_w - pad - accent_inset;

    for item in items.iter_mut() {
        match item {
            MenuItem::Action { id, label, shortcut, enabled, danger } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, item_rect);
                let hovered = *enabled && state.is_hovered();
                if hovered { non_submenu_hovered = true; }

                draw_hover_bg(hovered, item_rect, cr, s, style, painter);

                let text_color = if !*enabled { style.text_disabled } else if *danger { pal.danger } else { style.text };
                let text_x = content_x;
                let text_y = cy + (item_h - font) * 0.5;
                text.queue(
                    label, font, text_x, text_y, text_color,
                    content_w - pad, screen_w, screen_h,
                );

                if let Some(sc_text) = shortcut {
                    let sc_w = sc_text.len() as f32 * shortcut_font * 0.55;
                    let sc_x = inner_x + inner_w - pad * 2.0 - sc_w;
                    let sc_y = cy + (item_h - shortcut_font) * 0.5;
                    let sc_color = if *enabled { style.text_muted } else { style.text_disabled };
                    text.queue(
                        sc_text, shortcut_font, sc_x, sc_y, sc_color,
                        sc_w + 4.0 * s, screen_w, screen_h,
                    );
                }

                if *enabled && state == InteractionState::Pressed
                    && !pressed_zones.contains(&zone_id)
                {
                    pressed_zones.push(zone_id);
                    event = Some(MenuEvent::Action(*id));
                }
                cy += item_h;
            }
            MenuItem::Toggle { id, label, checked, enabled } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, item_rect);
                if state.is_hovered() && *enabled { non_submenu_hovered = true; }

                draw_hover_bg(state.is_hovered() && *enabled, item_rect, cr, s, style, painter);

                let toggle_rect = Rect::new(content_x, cy, content_w - pad, item_h);
                Toggle::new(toggle_rect, *checked)
                    .label(label)
                    .scale(s)
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
                let state = zone_state(interaction, zone_id, item_rect);
                if state.is_hovered() { non_submenu_hovered = true; }

                draw_hover_bg(state.is_hovered(), item_rect, cr, s, style, painter);

                let cb_rect = Rect::new(content_x, cy, content_w - pad, item_h);
                Checkbox::new(cb_rect, *checked)
                    .label(label)
                    .scale(s)
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
                let state = zone_state(interaction, zone_id, item_rect);
                if state.is_hovered() { non_submenu_hovered = true; }

                draw_hover_bg(state.is_hovered(), item_rect, cr, s, style, painter);

                let radio_rect = Rect::new(content_x, cy, content_w - pad, item_h);
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
                    inner_x + btn_pad, cy + 3.0 * s,
                    inner_w - btn_pad * 2.0, item_h - 6.0 * s,
                );
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, btn_rect);

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

                let label_x = content_x;
                let label_y = cy + 6.0 * s;
                text.queue(
                    label, label_size, label_x, label_y, style.text_muted,
                    content_w * 0.6, screen_w, screen_h,
                );
                let pct = format!("{}%", (*value * 100.0).round() as u32);
                let pct_w = pct.len() as f32 * label_size * 0.55;
                let pct_x = inner_x + inner_w - pad * 2.0 - pct_w;
                text.queue(
                    &pct, label_size, pct_x, label_y, style.text_muted,
                    pct_w + 4.0 * s, screen_w, screen_h,
                );

                let bar_y = label_y + label_size + 6.0 * s;
                let bar_rect = Rect::new(
                    content_x, bar_y,
                    content_w - pad, 14.0 * s,
                );
                ProgressBar::new(bar_rect)
                    .value(*value)
                    .draw(painter, text, pal, screen_w, screen_h);

                cy += prog_h;
            }
            MenuItem::Header { label } => {
                let header_h = HEADER_HEIGHT * s;
                let header_font = FONT_LABEL * s;
                let text_x = content_x;
                let text_y = cy + (header_h - header_font) * 0.5 + 2.0 * s;
                text.queue(
                    label, header_font, text_x, text_y, style.accent,
                    content_w - pad, screen_w, screen_h,
                );
                cy += header_h;
            }
            MenuItem::Separator => {
                let sep_h = SEPARATOR_HEIGHT * s;
                let sep_y = cy + sep_h * 0.5;
                let sep_x = content_x;
                let sep_w = content_w - pad;
                painter.rect_filled(
                    Rect::new(sep_x, sep_y, sep_w, 1.0 * s), 0.0, style.separator,
                );
                cy += sep_h;
            }
            MenuItem::ColoredSeparator(color) => {
                let sep_h = SEPARATOR_HEIGHT * s;
                let sep_y = cy + sep_h * 0.5;
                let sep_x = content_x;
                let sep_w = content_w - pad;
                painter.rect_filled(
                    Rect::new(sep_x, sep_y, sep_w, 2.0 * s), 1.0 * s, *color,
                );
                cy += sep_h;
            }
            MenuItem::Slider { id, label, value } => {
                let slider_h = SLIDER_ITEM_HEIGHT * s;
                let label_size = FONT_CAPTION * s;
                let track_h = SLIDER_TRACK_H * s;

                let item_rect = Rect::new(inner_x, cy, inner_w, slider_h);
                let zone_id = zone_base + *id;
                let zone_state = interaction.add_zone(zone_id, item_rect);

                let label_x = content_x;
                let label_y = cy + 8.0 * s;
                text.queue(
                    label, label_size, label_x, label_y, style.text_muted,
                    content_w * 0.6, screen_w, screen_h,
                );
                let pct = format!("{}%", (*value * 100.0).round() as u32);
                let pct_w = pct.len() as f32 * label_size * 0.55;
                let pct_x = inner_x + inner_w - pad * 2.0 - pct_w;
                text.queue(
                    &pct, label_size, pct_x, label_y, style.text_muted,
                    pct_w + 4.0 * s, screen_w, screen_h,
                );

                let track_y = label_y + label_size + 10.0 * s;
                let track_w = content_w - pad * 2.0;
                let track = Rect::new(content_x, track_y, track_w, track_h);

                painter.rect_filled(track, track_h * 0.5, pal.surface);
                let fill_w = (track_w * *value).max(track_h);
                painter.rect_filled(
                    Rect::new(track.x, track.y, fill_w, track_h),
                    track_h * 0.5, style.accent,
                );

                let thumb_x = track.x + track_w * *value;
                let thumb_cy = track.y + track_h * 0.5;
                let thumb_r = if zone_state.is_active() { 9.0 * s }
                    else if zone_state.is_hovered() { 8.0 * s }
                    else { 7.0 * s };
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
            MenuItem::ColorSwatches { label, swatches } => {
                let row_h = COLOR_SWATCH_HEIGHT * s;
                let label_size = FONT_CAPTION * s;

                // Draw label
                let label_x = content_x;
                let label_y = cy + 6.0 * s;
                text.queue(
                    label, label_size, label_x, label_y, style.text_muted,
                    content_w * 0.8, screen_w, screen_h,
                );

                // Draw mini folder icons
                let icon_sz = 40.0 * s;
                let icon_gap = 6.0 * s;
                let total_sw = swatches.len() as f32 * icon_sz
                    + (swatches.len().saturating_sub(1)) as f32 * icon_gap;
                let start_x = content_x + (content_w - pad - total_sw) * 0.5;
                let icon_top = cy + label_size + 18.0 * s;

                for (i, (sid, _color)) in swatches.iter().enumerate() {
                    let ix = start_x + i as f32 * (icon_sz + icon_gap);
                    let hit_rect = Rect::new(ix, icon_top, icon_sz, icon_sz);
                    let zone_id = zone_base + *sid;
                    let state = zone_state(interaction, zone_id, hit_rect);
                    let hovered = state.is_hovered();

                    // Hover highlight
                    if hovered {
                        painter.rect_filled(hit_rect, 4.0 * s, style.bg_hover);
                        painter.rect_stroke(hit_rect, 4.0 * s, 1.5 * s, style.accent.with_alpha(0.5));
                    }

                    // Actual folder icon textures are rendered by the app via swatch_rects()

                    if state == InteractionState::Pressed
                        && !pressed_zones.contains(&zone_id)
                    {
                        pressed_zones.push(zone_id);
                        event = Some(MenuEvent::Action(*sid));
                    }
                }

                cy += row_h;
            }
            MenuItem::SubMenu { id, label, .. } => {
                let item_rect = Rect::new(inner_x, cy, inner_w, item_h);
                let zone_id = zone_base + *id;
                let state = zone_state(interaction, zone_id, item_rect);

                let is_open = open_submenu_ids.get(depth) == Some(id);
                draw_hover_bg(state.is_hovered() || is_open, item_rect, cr, s, style, painter);

                if state.is_hovered() {
                    hovered_submenu = Some(*id);
                }

                let text_x = content_x;
                let text_y = cy + (item_h - font) * 0.5;
                text.queue(
                    label, font, text_x, text_y, style.text,
                    content_w - pad * 2.0, screen_w, screen_h,
                );

                // Arrow chevron
                let arrow_x = inner_x + inner_w - pad * 2.0 - 7.0 * s;
                let arrow_cy = cy + item_h * 0.5;
                let ac = if is_open { style.accent } else { style.text_muted };
                painter.line(
                    arrow_x, arrow_cy - 6.0 * s,
                    arrow_x + 6.0 * s, arrow_cy, 2.0 * s, ac,
                );
                painter.line(
                    arrow_x + 6.0 * s, arrow_cy,
                    arrow_x, arrow_cy + 6.0 * s, 2.0 * s, ac,
                );

                cy += item_h;
            }
        }
    }

    DrawPanelResult { event, hovered_submenu, non_submenu_hovered }
}

// ── Shared helpers ───────────────────────────────────────────────────────────

fn zone_state(
    interaction: &mut InteractionContext,
    zone_id: u32,
    rect: Rect,
) -> InteractionState {
    interaction.add_zone(zone_id, rect)
}

/// Draw hover highlight with left accent bar.
fn draw_hover_bg(
    is_hovered: bool,
    rect: Rect,
    _cr: f32,
    s: f32,
    style: &ContextMenuStyle,
    painter: &mut Painter,
) {
    if !is_hovered { return; }
    // Left accent bar
    let bar_w = ACCENT_BAR_WIDTH * s;
    let bar_inset = 3.0 * s;
    let bar_rect = Rect::new(
        rect.x + bar_inset, rect.y + bar_inset,
        bar_w, rect.h - bar_inset * 2.0,
    );
    painter.rect_filled(bar_rect, bar_w * 0.5, style.accent);
}
