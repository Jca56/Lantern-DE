use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_theme::{FONT_CAPTION, FONT_LABEL};

use super::checkbox::Checkbox;
use super::context_menu::{
    ContextMenuStyle, MenuEvent, MenuItem,
    ACCENT_BAR_WIDTH, COLOR_SWATCH_HEIGHT, CONTEXT_MENU_ZONE_BASE, HEADER_HEIGHT,
    PROGRESS_ITEM_HEIGHT, SEPARATOR_HEIGHT, SLIDER_ITEM_HEIGHT, SLIDER_TRACK_H,
    TAB_DOTS_OVERHANG, WINDOW_CONTROLS_HEIGHT, items_height_slice,
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

    // Background. The border strokes inward (rect_border) rather than
    // centered on the edge — popup surfaces are sized exactly to the menu,
    // so a centered stroke would have its outer half clipped away.
    painter.rect_filled(menu_rect, cr, style.bg);
    painter.rect_border(menu_rect, cr, style.border_width * s, style.border);

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
    let shortcut_font = FONT_LABEL * 0.85 * s;
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
                    .font_size(style.font_size)
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
            MenuItem::WindowControls { minimize_id, maximize_id, close_id, title, nav, tabs } => {
                let row_h = WINDOW_CONTROLS_HEIGHT * s;
                let radius = 10.0 * s;
                let icon = 4.0 * s;
                let thick = 1.5 * s;
                let mid_y = cy + row_h * 0.5;

                // Tab-indicator dots in a capsule floating above the panel's
                // top-left corner. Active tab = accent, others = muted;
                // clicking a dot jumps to that tab.
                if let Some((base_id, count, active)) = tabs {
                    let dot_r = 4.0 * s;
                    let gap = 16.0 * s;
                    let cap_h = 20.0 * s;
                    let cap_pad = 10.0 * s;
                    let cap_w = (*count as f32 - 1.0) * gap + dot_r * 2.0 + cap_pad * 2.0;
                    let cap_y = py - TAB_DOTS_OVERHANG * s;
                    let cap = Rect::new(px, cap_y, cap_w, cap_h);
                    painter.rect_filled(cap, cap_h * 0.5, style.bg);
                    if style.border_width > 0.0 {
                        painter.rect_stroke_sdf(cap, cap_h * 0.5, style.border_width, style.border);
                    }
                    let dot_cy = cap_y + cap_h * 0.5;
                    for i in 0..*count {
                        let dx = px + cap_pad + dot_r + i as f32 * gap;
                        let zone_id = zone_base + *base_id + i as u32;
                        let state = zone_state(
                            interaction, zone_id,
                            Rect::new(dx - gap * 0.5, cap_y, gap, cap_h),
                        );
                        let hov = state.is_hovered();
                        if hov { non_submenu_hovered = true; }
                        let is_active = i == *active;
                        let color = if is_active {
                            style.accent
                        } else if hov {
                            style.text
                        } else {
                            style.text_muted
                        };
                        let r = if is_active { dot_r } else { dot_r * 0.7 };
                        painter.circle_filled(dx, dot_cy, r, color);
                        if state == InteractionState::Pressed && !pressed_zones.contains(&zone_id) {
                            pressed_zones.push(zone_id);
                            event = Some(MenuEvent::Action(*base_id + i as u32));
                        }
                    }
                }
                // Right-aligned like a real title bar: close at the far
                // right, then maximize, then minimize, 30px apart.
                let right = inner_x + inner_w - pad * 2.0;
                let close_x = right - radius;
                let max_x = close_x - 30.0 * s;
                let min_x = max_x - 30.0 * s;

                // Clickable title label on the left (accent-colored brand mark)
                if let Some((tid, label)) = title {
                    let title_font = style.font_size * s;
                    let label_w = label.len() as f32 * title_font * 0.55;
                    let pill = Rect::new(
                        content_x - 4.0 * s, mid_y - row_h * 0.5 + 2.0 * s,
                        label_w + 8.0 * s, row_h - 4.0 * s,
                    );
                    let zone_id = zone_base + *tid;
                    let state = zone_state(interaction, zone_id, pill);
                    if state.is_hovered() {
                        non_submenu_hovered = true;
                        painter.rect_filled(pill, 6.0 * s, style.bg_hover);
                    }
                    text.queue(
                        label, title_font, content_x, mid_y - title_font * 0.5,
                        style.accent, label_w + 8.0 * s, screen_w, screen_h,
                    );
                    if state == InteractionState::Pressed && !pressed_zones.contains(&zone_id) {
                        pressed_zones.push(zone_id);
                        event = Some(MenuEvent::Action(*tid));
                    }
                }

                // Prev / next nav chevrons just left of the buttons
                if let Some((prev_id, next_id)) = nav {
                    let next_x = min_x - 30.0 * s;
                    let prev_x = next_x - 26.0 * s;
                    let ch = 5.0 * s;
                    for (bx, id, dir) in [(prev_x, *prev_id, -1.0f32), (next_x, *next_id, 1.0f32)] {
                        let zone_id = zone_base + id;
                        let state = zone_state(
                            interaction, zone_id,
                            Rect::new(bx - radius, mid_y - radius, radius * 2.0, radius * 2.0),
                        );
                        let hov = state.is_hovered();
                        if hov {
                            non_submenu_hovered = true;
                            painter.circle_filled(bx, mid_y, radius, style.bg_hover);
                        }
                        let ic = if hov { style.text } else { style.text_muted };
                        let tip_x = bx + ch * 0.5 * dir;
                        let base_x = bx - ch * 0.5 * dir;
                        painter.line(base_x, mid_y - ch, tip_x, mid_y, thick, ic);
                        painter.line(tip_x, mid_y, base_x, mid_y + ch, thick, ic);
                        if state == InteractionState::Pressed && !pressed_zones.contains(&zone_id) {
                            pressed_zones.push(zone_id);
                            event = Some(MenuEvent::Action(id));
                        }
                    }
                }

                // Close — X, danger-tinted on hover
                let zone_id = zone_base + *close_id;
                let state = zone_state(
                    interaction, zone_id,
                    Rect::new(close_x - radius, mid_y - radius, radius * 2.0, radius * 2.0),
                );
                let hov = state.is_hovered();
                if hov {
                    non_submenu_hovered = true;
                    painter.circle_filled(close_x, mid_y, radius, pal.danger.with_alpha(0.30));
                }
                let ic = if hov { pal.danger } else { style.text_muted };
                painter.line(close_x - icon, mid_y - icon, close_x + icon, mid_y + icon, thick, ic);
                painter.line(close_x - icon, mid_y + icon, close_x + icon, mid_y - icon, thick, ic);
                if state == InteractionState::Pressed && !pressed_zones.contains(&zone_id) {
                    pressed_zones.push(zone_id);
                    event = Some(MenuEvent::Action(*close_id));
                }

                // Maximize — square
                let zone_id = zone_base + *maximize_id;
                let state = zone_state(
                    interaction, zone_id,
                    Rect::new(max_x - radius, mid_y - radius, radius * 2.0, radius * 2.0),
                );
                let hov = state.is_hovered();
                if hov {
                    non_submenu_hovered = true;
                    painter.circle_filled(max_x, mid_y, radius, style.bg_hover);
                }
                let ic = if hov { style.text } else { style.text_muted };
                painter.rect_stroke_sdf(
                    Rect::new(max_x - icon, mid_y - icon, icon * 2.0, icon * 2.0),
                    1.5 * s, thick, ic,
                );
                if state == InteractionState::Pressed && !pressed_zones.contains(&zone_id) {
                    pressed_zones.push(zone_id);
                    event = Some(MenuEvent::Action(*maximize_id));
                }

                // Minimize — line
                let zone_id = zone_base + *minimize_id;
                let state = zone_state(
                    interaction, zone_id,
                    Rect::new(min_x - radius, mid_y - radius, radius * 2.0, radius * 2.0),
                );
                let hov = state.is_hovered();
                if hov {
                    non_submenu_hovered = true;
                    painter.circle_filled(min_x, mid_y, radius, style.bg_hover);
                }
                let ic = if hov { style.text } else { style.text_muted };
                painter.line(min_x - icon, mid_y, min_x + icon, mid_y, thick, ic);
                if state == InteractionState::Pressed && !pressed_zones.contains(&zone_id) {
                    pressed_zones.push(zone_id);
                    event = Some(MenuEvent::Action(*minimize_id));
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
