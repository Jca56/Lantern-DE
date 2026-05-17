//! Unified Appearance panel.
//!
//! Combines the legacy Appearance (theme + accent) and Window Manager (layout,
//! visual effects, animations, focus + glow) panels into a single scrollable
//! tab. Sections, top to bottom:
//!
//!   1. Theme & Accent      — variant radio + accent swatch row
//!   2. Layout & Visual Effects — borders, titlebar, gap, corner, blur, opacity
//!   3. Animations          — master toggle, speed, preset picker, per-event toggles
//!   4. Focus & Glow        — focus-follows-mouse, glow toggle + color + intensity
//!
//! All click handling for these zones routes through `handle_appearance_click`.
//! The preset dropdown uses the shared ContextMenu via `PanelState::dropdown_menu`.

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, MenuEvent, ScrollArea, Scrollbar,
};

use crate::appearance_themes::ThemesPanelState;
use crate::config::LanternConfig;
use crate::panels::{
    draw_color_swatch_row, draw_section_card, make_menu_items, PanelState,
    CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V, CARD_OUTER_PAD_H,
    CARD_OUTER_PAD_V, GLOW_COLORS, LABEL_SIZE, ROW_H,
};

// ── Zone IDs ────────────────────────────────────────────────────────────────
//
// Layout (Window Frame + Visual Effects share the same card)
const ZONE_BORDER:           u32 = 300;
// 301, 302 reserved (formerly Titlebar Height + Window Gap, removed from UI)
const ZONE_CORNER:           u32 = 303;
const ZONE_BLUR:             u32 = 306;
const ZONE_TINT:             u32 = 307;
const ZONE_DARKEN:           u32 = 308;
const ZONE_BG_OPACITY:       u32 = 309;
const ZONE_BORDER_COLOR_BASE: u32 = 330; // +0..8 swatches
const ZONE_TINT_COLOR_BASE:   u32 = 340; // +0..8 swatches

// Focus & Glow
const ZONE_FOCUS:            u32 = 304;
const ZONE_GLOW:             u32 = 310;
const ZONE_GLOW_COLOR_BASE:  u32 = 311; // +0..8 swatches
const ZONE_GLOW_INTENSITY:   u32 = 320;

// Animations
const ZONE_ANIM_ENABLE:      u32 = 322;
const ZONE_ANIM_SPEED:       u32 = 323;
const ZONE_ANIM_PRESET_BTN:  u32 = 324;
const ZONE_ANIM_T_OPEN:      u32 = 325;
const ZONE_ANIM_T_STATE:     u32 = 326;
const ZONE_ANIM_T_MIN:       u32 = 327;
const ZONE_ANIM_T_TILING:    u32 = 328;
const ZONE_ANIM_T_WS:        u32 = 329;

// Background + Accent
// 350, 351 reserved (formerly Fox/Lantern radio, replaced by swatch picker)
const ZONE_ACCENT_BASE:      u32 = 360; // +0..8 swatches (GLOW_COLORS)
const ZONE_BG_COLOR_BASE:    u32 = 380; // +0..10 swatches (BG_COLORS, 11 entries)

// Dropdown menu action IDs (preset selection)
const ACT_ANIM_PRESET: u32 = 370; // +0..3

const PRESET_OPTIONS: &[&str] = &["Cinematic", "Snappy", "Springy", "Linear"];

// ── Public entry ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_appearance_panel(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    themes_state: &mut ThemesPanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    tex_pass: &TexturePass,
    gpu: &GpuContext,
    fox: &FoxPalette,
    tex_draws: &mut Vec<TextureDraw>,
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
    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;

    // ── Card sizing ──────────────────────────────────────────────────
    let themes_card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;
    let themes_card_h = crate::appearance_themes::themes_card_height(
        themes_state, themes_card_inner_w, s,
    );
    let theme_card_h = card_chrome_h + 1.6 * row + row; // theme cards + accent row
    // Layout & Visual Effects: 2 sliders (Border Width, Corner Radius) +
    // Border Color swatch + 2 sliders (Blur Intensity, Blur Tint) + Tint
    // Color swatch + 2 sliders (Blur Darken, Background Opacity) = 7 rows.
    let layout_card_h = card_chrome_h + 7.0 * row;
    let anim_card_h = card_chrome_h + 4.0 * row + 3.0 * (ROW_H * 0.75) * s;
    let focus_base_rows = 2.0;
    let glow_extra_rows = if config.window_manager.focus_glow { 2.0 } else { 0.0 };
    let focus_card_h = card_chrome_h + (focus_base_rows + glow_extra_rows) * row;

    let content_height = CARD_OUTER_PAD_V * s
        + themes_card_h + CARD_GAP * s
        + theme_card_h + CARD_GAP * s
        + layout_card_h + CARD_GAP * s
        + anim_card_h + CARD_GAP * s
        + focus_card_h + CARD_OUTER_PAD_V * 2.0 * s;

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

    // ── Card 0: Themes ─────────────────────────────────────────────
    crate::appearance_themes::draw_themes_card(
        themes_state, config, painter, text, ix, tex_pass, gpu, fox, tex_draws,
        card_x, cy_top, card_w, themes_card_h, s, sw, sh,
    );
    cy_top += themes_card_h + CARD_GAP * s;

    // ── Card 1: Background Color ──────────────────────────────────
    draw_background_color_card(
        config, painter, text, ix, fox,
        card_x, cy_top, card_w, theme_card_h, s, sw, sh,
    );
    cy_top += theme_card_h + CARD_GAP * s;

    // ── Card 2: Layout & Visual Effects ────────────────────────────
    crate::appearance_layout::draw_layout_card(
        config, painter, text, ix, fox,
        card_x, cy_top, card_w, layout_card_h, s, sw, sh,
        &ZONE_IDS_LAYOUT,
    );
    cy_top += layout_card_h + CARD_GAP * s;

    // ── Card 3: Animations ─────────────────────────────────────────
    crate::appearance_animations::draw_animations_card(
        config, panel_state, painter, text, ix, fox,
        card_x, cy_top, card_w, anim_card_h, s, sw, sh,
        &ZONE_IDS_ANIM,
    );
    cy_top += anim_card_h + CARD_GAP * s;

    // ── Card 4: Focus & Glow ───────────────────────────────────────
    crate::appearance_focus::draw_focus_card(
        config, painter, text, ix, fox,
        card_x, cy_top, card_w, focus_card_h, s, sw, sh,
        &ZONE_IDS_FOCUS,
    );

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, panel_state.wm_scroll);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }

    // Preset + theme context dropdown — shared menu, drawn last so it sits on top.
    panel_state.dropdown_menu.set_scale(s);
    panel_state.dropdown_menu.update(0.016);
    if let Some(evt) = panel_state.dropdown_menu.draw(painter, text, ix, sw, sh) {
        if let MenuEvent::Action(id) = evt {
            if id >= ACT_ANIM_PRESET && id < ACT_ANIM_PRESET + PRESET_OPTIONS.len() as u32 {
                config.animations.preset =
                    PRESET_OPTIONS[(id - ACT_ANIM_PRESET) as usize].to_lowercase();
            } else {
                crate::appearance_themes::dispatch_theme_menu_action(
                    themes_state, config, id,
                );
            }
            panel_state.close_dropdown();
        }
    }
}

// ── Zone-id bundles passed into section modules ────────────────────────────

pub(crate) struct LayoutZones {
    pub border: u32,
    pub corner: u32,
    pub blur: u32,
    pub tint: u32,
    pub darken: u32,
    pub bg_opacity: u32,
    pub border_color_base: u32,
    pub tint_color_base: u32,
}

pub(crate) const ZONE_IDS_LAYOUT: LayoutZones = LayoutZones {
    border: ZONE_BORDER,
    corner: ZONE_CORNER,
    blur: ZONE_BLUR,
    tint: ZONE_TINT,
    darken: ZONE_DARKEN,
    bg_opacity: ZONE_BG_OPACITY,
    border_color_base: ZONE_BORDER_COLOR_BASE,
    tint_color_base: ZONE_TINT_COLOR_BASE,
};

pub(crate) struct AnimZones {
    pub enable: u32,
    pub speed: u32,
    pub preset_btn: u32,
    pub t_open: u32,
    pub t_state: u32,
    pub t_min: u32,
    pub t_tiling: u32,
    pub t_ws: u32,
}

pub(crate) const ZONE_IDS_ANIM: AnimZones = AnimZones {
    enable: ZONE_ANIM_ENABLE,
    speed: ZONE_ANIM_SPEED,
    preset_btn: ZONE_ANIM_PRESET_BTN,
    t_open: ZONE_ANIM_T_OPEN,
    t_state: ZONE_ANIM_T_STATE,
    t_min: ZONE_ANIM_T_MIN,
    t_tiling: ZONE_ANIM_T_TILING,
    t_ws: ZONE_ANIM_T_WS,
};

pub(crate) struct FocusZones {
    pub focus: u32,
    pub glow: u32,
    pub glow_color_base: u32,
    pub glow_intensity: u32,
}

pub(crate) const ZONE_IDS_FOCUS: FocusZones = FocusZones {
    focus: ZONE_FOCUS,
    glow: ZONE_GLOW,
    glow_color_base: ZONE_GLOW_COLOR_BASE,
    glow_intensity: ZONE_GLOW_INTENSITY,
};

// ── Background Color card ──────────────────────────────────────────────────
//
// Two swatch rows: the window's background color (11 swatches incl. Black +
// Brown) and the accent (9 swatches, the existing GLOW_COLORS palette). The
// bg picker writes `appearance.background_color`; chrome.rs reads it via
// `lntrn_theme::active_background_color()`.

#[allow(clippy::too_many_arguments)]
fn draw_background_color_card(
    config: &mut LanternConfig,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    card_x: f32, card_y: f32, card_w: f32, card_h: f32,
    s: f32, sw: u32, sh: u32,
) {
    use crate::panels::BG_COLORS;

    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + 130.0 * s;

    let mut cy = draw_section_card(
        painter, text, fox, "Background Color",
        card_x, card_y, card_w, card_h, s, sw, sh,
    );

    // Background swatch row (custom because BG_COLORS is a different palette
    // than draw_color_swatch_row's hardcoded GLOW_COLORS).
    draw_bg_swatch_row(
        painter, text, ix, fox,
        "Background", ZONE_BG_COLOR_BASE, BG_COLORS,
        &config.appearance.background_color,
        label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
    );

    // Accent row (existing helper handles GLOW_COLORS).
    draw_color_swatch_row(
        painter, text, ix, fox,
        "Accent", ZONE_ACCENT_BASE,
        &config.appearance.accent,
        label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_bg_swatch_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    label: &str,
    zone_base: u32,
    colors: &[(&str, &str)],
    selected_hex: &str,
    label_x: f32,
    ctrl_x: f32,
    cy: &mut f32,
    row: f32,
    lsz: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(label, lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

    let swatch_size = 28.0 * s;
    let swatch_gap = 8.0 * s;
    let mut sx = ctrl_x;
    for (i, (hex, _name)) in colors.iter().enumerate() {
        let color = Color::from_hex(hex).unwrap();
        let zone_id = zone_base + i as u32;
        let swatch_rect = Rect::new(sx, *cy + (row - swatch_size) / 2.0, swatch_size, swatch_size);
        let zone = ix.add_zone(zone_id, swatch_rect);

        let cx = sx + swatch_size / 2.0;
        let cy_center = swatch_rect.y + swatch_size / 2.0;
        let radius = swatch_size / 2.0;
        painter.circle_filled(cx, cy_center, radius, color);

        let is_selected = selected_hex.eq_ignore_ascii_case(hex);
        if is_selected {
            painter.circle_stroke(cx, cy_center, radius + 3.0 * s, 2.0 * s, fox.text);
        } else if zone.is_hovered() {
            painter.circle_stroke(cx, cy_center, radius + 2.0 * s, 1.5 * s, fox.text_secondary);
        }
        sx += swatch_size + swatch_gap;
    }
    *cy += row;
}

// ── Click handling ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn handle_appearance_click(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    zone_id: u32,
    cursor_x: f32,
    cursor_y: f32,
) {
    match zone_id {
        // Focus & glow toggles
        ZONE_FOCUS => config.window_manager.focus_follows_mouse =
            !config.window_manager.focus_follows_mouse,
        ZONE_GLOW => config.window_manager.focus_glow = !config.window_manager.focus_glow,
        // Animations master + per-event toggles
        ZONE_ANIM_ENABLE => config.animations.enabled = !config.animations.enabled,
        ZONE_ANIM_T_OPEN => config.animations.open_close = !config.animations.open_close,
        ZONE_ANIM_T_STATE => config.animations.state = !config.animations.state,
        ZONE_ANIM_T_MIN => config.animations.minimize = !config.animations.minimize,
        ZONE_ANIM_T_TILING => config.animations.tiling = !config.animations.tiling,
        ZONE_ANIM_T_WS => config.animations.workspace = !config.animations.workspace,
        // Preset dropdown — open menu under cursor
        ZONE_ANIM_PRESET_BTN => {
            if panel_state.active_dropdown == Some(ZONE_ANIM_PRESET_BTN) {
                panel_state.close_dropdown();
            } else {
                let items = make_menu_items(
                    PRESET_OPTIONS, ACT_ANIM_PRESET, &config.animations.preset,
                );
                panel_state.dropdown_menu.open(cursor_x, cursor_y + 16.0, items);
                panel_state.active_dropdown = Some(ZONE_ANIM_PRESET_BTN);
            }
        }
        // Accent swatches
        id if id >= ZONE_ACCENT_BASE
            && id < ZONE_ACCENT_BASE + GLOW_COLORS.len() as u32 =>
        {
            let idx = (id - ZONE_ACCENT_BASE) as usize;
            config.appearance.accent = GLOW_COLORS[idx].0.into();
        }
        // Background-color swatches
        id if id >= ZONE_BG_COLOR_BASE
            && id < ZONE_BG_COLOR_BASE + crate::panels::BG_COLORS.len() as u32 =>
        {
            let idx = (id - ZONE_BG_COLOR_BASE) as usize;
            config.appearance.background_color = crate::panels::BG_COLORS[idx].0.into();
        }
        // Border color swatches
        id if id >= ZONE_BORDER_COLOR_BASE
            && id < ZONE_BORDER_COLOR_BASE + GLOW_COLORS.len() as u32 =>
        {
            let idx = (id - ZONE_BORDER_COLOR_BASE) as usize;
            config.window_manager.border_color = GLOW_COLORS[idx].0.into();
        }
        // Tint color swatches
        id if id >= ZONE_TINT_COLOR_BASE
            && id < ZONE_TINT_COLOR_BASE + GLOW_COLORS.len() as u32 =>
        {
            let idx = (id - ZONE_TINT_COLOR_BASE) as usize;
            config.windows.blur_tint_color = GLOW_COLORS[idx].0.into();
        }
        // Glow color swatches
        id if id >= ZONE_GLOW_COLOR_BASE
            && id < ZONE_GLOW_COLOR_BASE + GLOW_COLORS.len() as u32 =>
        {
            let idx = (id - ZONE_GLOW_COLOR_BASE) as usize;
            config.window_manager.focus_glow_color = GLOW_COLORS[idx].0.into();
        }
        _ => {
            // Click landed somewhere benign (slider drag handled during render).
            if panel_state.dropdown_menu.is_open() {
                panel_state.close_dropdown();
            }
        }
    }
}

