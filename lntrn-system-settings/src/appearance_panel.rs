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
use lntrn_ui::gpu::{FoxPalette, InteractionContext, MenuEvent, ScrollArea, Scrollbar, Slider};

use crate::appearance_themes::ThemesPanelState;
use crate::config::LanternConfig;
use crate::panels::{
    draw_color_swatch_row, draw_section_card, draw_select_button, make_menu_items,
    slider_value_from_cursor, PanelState, CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H,
    CARD_INNER_PAD_V, CARD_OUTER_PAD_H, CARD_OUTER_PAD_V, GLOW_COLORS, LABEL_SIZE, ROW_H, SLIDER_H,
    SLIDER_W, VALUE_SIZE, VALUE_W,
};

// ── Zone IDs ────────────────────────────────────────────────────────────────
//
// Layout (Window Frame + Visual Effects share the same card)
const ZONE_BORDER: u32 = 300;
// 301, 302 reserved (formerly Titlebar Height + Window Gap, removed from UI)
const ZONE_CORNER: u32 = 303;
const ZONE_BLUR: u32 = 306;
const ZONE_TINT: u32 = 307;
const ZONE_DARKEN: u32 = 308;
const ZONE_BG_OPACITY: u32 = 309;
const ZONE_BORDER_COLOR_BASE: u32 = 330; // +0..9 swatches (10 entries)
const ZONE_TINT_COLOR_BASE: u32 = 340; // +0..9 swatches (10 entries)

// Focus & Glow — ZONE_FOCUS is rendered on the Input/Mouse page now (Pointer
// card) but still lives here so both modules see the same id.
pub(crate) const ZONE_FOCUS: u32 = 304;
const ZONE_GLOW: u32 = 310;
const ZONE_GLOW_COLOR_BASE: u32 = 311; // +0..9 swatches (10 entries, 311..320)
const ZONE_GLOW_INTENSITY: u32 = 321;

// Animations
const ZONE_ANIM_ENABLE: u32 = 322;
const ZONE_ANIM_SPEED: u32 = 323;
const ZONE_ANIM_PRESET_BTN: u32 = 324;
const ZONE_ANIM_T_OPEN: u32 = 325;
const ZONE_ANIM_T_STATE: u32 = 326;
const ZONE_ANIM_T_MIN: u32 = 327;
const ZONE_ANIM_T_TILING: u32 = 328;
const ZONE_ANIM_T_WS: u32 = 329;

// Background + Accent
// 350, 351 reserved (formerly Fox/Lantern radio, replaced by swatch picker)
const ZONE_ACCENT_BASE: u32 = 360; // +0..9 swatches (GLOW_COLORS, 10 entries)
const ZONE_BG_COLOR_BASE: u32 = 380; // +0..9 swatches (BG_COLORS, 10 entries)

// Window Gradient card — 4 stop rows × N swatches (BG_COLORS) plus a
// direction-picker row (4 presets) and a "Clear / Disable" button.
// BG_COLORS now has 11 entries; stop block reserves 60 IDs (4 stops × 15)
// to leave headroom for future swatch additions.
const ZONE_GRADIENT_STOP_BASE: u32 = 410; // +0..(BG_COLORS.len()-1) — shared color swatch row
const ZONE_GRADIENT_CLEAR: u32 = 470; // "Disable All" button
const ZONE_GRADIENT_CHIP_BASE: u32 = 480; // +0..4 — per-position toggle chips
const ZONE_GRADIENT_ALPHA_BASE: u32 = 490; // Intensity slider
const ZONE_GRADIENT_RADIUS: u32 = 491; // Radius slider

// 5 independent glow positions stored in `window_gradient_stops`. Indices
// in the storage array follow `GradientCorner::ALL` (TL, TR, BL, BR, Center).
// Empty string at an index = position disabled.
const GRADIENT_STOP_COUNT: usize = 5;

/// (storage_index, chip_glyph) for the toggle-chip row. Visual order
/// matches the user's layout: TL → BL → Center → TR → BR.
const GRADIENT_CHIPS: [(usize, &str); 5] = [
    (0, "↘"), // Top-Left,    arrow points inward toward center
    (2, "↗"), // Bottom-Left, arrow points inward
    (4, "●"), // Center
    (1, "↙"), // Top-Right,   arrow points inward
    (3, "↖"), // Bottom-Right, arrow points inward
];

// Dropdown menu action IDs (preset selection)
const ACT_ANIM_PRESET: u32 = 370; // +0..3

const PRESET_OPTIONS: &[&str] = &["Cinematic", "Snappy", "Springy", "Linear"];

// Font card (Themes page, right column)
const ZONE_FONT_BTN: u32 = 500;
const ACT_FONT_BASE: u32 = 520; // +0..FONT_OPTIONS.len()

/// DE-wide proportional font choices. Family names must match the faces
/// bundled in `~/.lantern/fonts/`; applied via `[appearance] font_family`
/// as each Lantern app restarts.
const FONT_OPTIONS: &[&str] = &["Inter", "Lexend", "Atkinson Hyperlegible", "IBM Plex Sans"];

/// Stored `font_family` → the family shown/selected in the picker. Empty
/// and the legacy generic `"sans-serif"` mean the theme default.
fn effective_font_family(stored: &str) -> String {
    let s = stored.trim();
    if s.is_empty() || s == "sans-serif" {
        lntrn_theme::FAMILY_PROPORTIONAL.to_string()
    } else {
        s.to_string()
    }
}

// ── Public entry ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_appearance_panel(
    subpanel: crate::wayland::Panel,
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
    use crate::wayland::Panel;

    let row = ROW_H * s;
    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let themes_card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    // Per-card heights, computed once so the scroll math sees the same numbers
    // as the draw calls.
    let themes_card_h =
        crate::appearance_themes::themes_card_height(themes_state, themes_card_inner_w, s);
    let bg_card_h = card_chrome_h + 1.6 * row + row;
    let gradient_card_h = card_chrome_h + 5.0 * row;
    let borders_card_h = card_chrome_h + crate::appearance_layout::BORDER_ROWS * row;
    let effects_card_h = card_chrome_h + crate::appearance_layout::EFFECT_ROWS * row;
    let anim_card_h = card_chrome_h + 4.0 * row + 3.0 * (ROW_H * 0.75) * s;
    let glow_extra_rows = if config.window_manager.focus_glow {
        2.0
    } else {
        0.0
    };
    let focus_card_h = card_chrome_h + (1.0 + glow_extra_rows) * row;
    let font_card_h = card_chrome_h + row;

    // Themes page is a single mega-panel: theme presets full-width, then a
    // two-column body. Left stack = Background Color → Blur & Effects → Borders.
    // Right stack = Window Gradient → Focus Glow → Font.
    let left_stack = bg_card_h + CARD_GAP * s + effects_card_h + CARD_GAP * s + borders_card_h;
    let right_stack = gradient_card_h + CARD_GAP * s + focus_card_h + CARD_GAP * s + font_card_h;
    let two_col_h = left_stack.max(right_stack);

    let content_height = match subpanel {
        Panel::Themes => CARD_OUTER_PAD_V * 2.0 * s + themes_card_h + CARD_GAP * s + two_col_h,
        Panel::Animations => CARD_OUTER_PAD_V * 2.0 * s + anim_card_h,
        _ => 0.0,
    };

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(
            &mut panel_state.wm_scroll,
            scroll_delta * 40.0,
            content_height,
            panel_h,
        );
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut panel_state.wm_scroll);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    match subpanel {
        Panel::Themes => {
            // Top: theme preset grid, full width.
            crate::appearance_themes::draw_themes_card(
                themes_state,
                config,
                painter,
                text,
                ix,
                tex_pass,
                gpu,
                fox,
                tex_draws,
                card_x,
                cy_top,
                card_w,
                themes_card_h,
                s,
                sw,
                sh,
            );
            cy_top += themes_card_h + CARD_GAP * s;

            // Two-column body.
            let col_gap = CARD_GAP * s;
            let col_w = ((card_w - col_gap) / 2.0).floor();
            let left_x = card_x;
            let right_x = card_x + col_w + col_gap;

            // Left column: Background Color → Blur & Effects → Borders
            let mut ly = cy_top;
            draw_background_color_card(
                config, painter, text, ix, fox, left_x, ly, col_w, bg_card_h, s, sw, sh,
            );
            ly += bg_card_h + col_gap;
            crate::appearance_layout::draw_effects_card(
                config,
                painter,
                text,
                ix,
                fox,
                left_x,
                ly,
                col_w,
                effects_card_h,
                s,
                sw,
                sh,
                &ZONE_IDS_LAYOUT,
            );
            ly += effects_card_h + col_gap;
            crate::appearance_layout::draw_borders_card(
                config,
                painter,
                text,
                ix,
                fox,
                left_x,
                ly,
                col_w,
                borders_card_h,
                s,
                sw,
                sh,
                &ZONE_IDS_LAYOUT,
            );

            // Right column: Window Gradient → Focus Glow
            let mut ry = cy_top;
            draw_window_gradient_card(
                config,
                painter,
                text,
                ix,
                fox,
                right_x,
                ry,
                col_w,
                gradient_card_h,
                s,
                sw,
                sh,
            );
            ry += gradient_card_h + col_gap;
            crate::appearance_focus::draw_focus_card(
                config,
                painter,
                text,
                ix,
                fox,
                right_x,
                ry,
                col_w,
                focus_card_h,
                s,
                sw,
                sh,
                &ZONE_IDS_FOCUS,
            );
            ry += focus_card_h + col_gap;
            draw_font_card(
                config,
                panel_state,
                painter,
                text,
                ix,
                fox,
                right_x,
                ry,
                col_w,
                font_card_h,
                s,
                sw,
                sh,
            );
        }
        Panel::Animations => {
            crate::appearance_animations::draw_animations_card(
                config,
                panel_state,
                painter,
                text,
                ix,
                fox,
                card_x,
                cy_top,
                card_w,
                anim_card_h,
                s,
                sw,
                sh,
                &ZONE_IDS_ANIM,
            );
        }
        _ => {}
    }

    let _ = cy_top; // silence "unused-after-last-card" lint in arms with one card

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
            } else if id >= ACT_FONT_BASE && id < ACT_FONT_BASE + FONT_OPTIONS.len() as u32 {
                config.appearance.font_family =
                    FONT_OPTIONS[(id - ACT_FONT_BASE) as usize].to_string();
            } else {
                crate::appearance_themes::dispatch_theme_menu_action(themes_state, config, id);
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
// Two swatch rows: the window's background color (10 swatches incl. Black +
// Brown) and the accent (10 swatches, the unified GLOW_COLORS palette). The
// bg picker writes `appearance.background_color`; chrome.rs reads it via
// `lntrn_theme::active_background_color()`.

#[allow(clippy::too_many_arguments)]
fn draw_background_color_card(
    config: &mut LanternConfig,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    card_x: f32,
    card_y: f32,
    card_w: f32,
    card_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    use crate::panels::BG_COLORS;

    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + 130.0 * s;
    let end_x = card_x + card_w - CARD_INNER_PAD_H * s;

    let mut cy = draw_section_card(
        painter,
        text,
        fox,
        "Background Color",
        card_x,
        card_y,
        card_w,
        card_h,
        s,
        sw,
        sh,
    );

    // Background swatch row (custom because BG_COLORS is a different palette
    // than draw_color_swatch_row's hardcoded GLOW_COLORS).
    draw_bg_swatch_row(
        painter,
        text,
        ix,
        fox,
        "Background",
        ZONE_BG_COLOR_BASE,
        BG_COLORS,
        &config.appearance.background_color,
        label_x,
        ctrl_x,
        end_x,
        &mut cy,
        row,
        lsz,
        s,
        sw,
        sh,
    );

    // Accent row (existing helper handles GLOW_COLORS).
    draw_color_swatch_row(
        painter,
        text,
        ix,
        fox,
        "Accent",
        ZONE_ACCENT_BASE,
        &config.appearance.accent,
        label_x,
        ctrl_x,
        end_x,
        &mut cy,
        row,
        lsz,
        s,
        sw,
        sh,
    );
}

// ── Font card ───────────────────────────────────────────────────────────────
//
// One select-button row that sets `[appearance] font_family` — the DE-wide
// proportional font every lntrn-render app resolves at startup. Fonts are
// bundled in ~/.lantern/fonts; a change applies as apps restart.

#[allow(clippy::too_many_arguments)]
fn draw_font_card(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    card_x: f32,
    card_y: f32,
    card_w: f32,
    card_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + 130.0 * s;

    let mut cy = draw_section_card(
        painter, text, fox, "Font", card_x, card_y, card_w, card_h, s, sw, sh,
    );

    let btn_w = 250.0 * s;
    let btn_h = 40.0 * s;
    let is_open = panel_state.active_dropdown == Some(ZONE_FONT_BTN);
    let current = effective_font_family(&config.appearance.font_family);
    let menu = &panel_state.dropdown_menu;
    draw_select_button(
        "DE Font",
        &current,
        ZONE_FONT_BTN,
        is_open,
        painter,
        text,
        ix,
        fox,
        label_x,
        130.0 * s,
        ctrl_x,
        btn_w,
        btn_h,
        row,
        lsz,
        s,
        sw,
        sh,
        &mut cy,
        menu,
    );
}

// ── Window Gradient card ───────────────────────────────────────────────────
//
// Four swatch rows (BG_COLORS palette) — one per gradient stop. Stops apply
// top→bottom on every window background that uses `FoxPalette::window_fill`.
// A "Disable Gradient" button at the bottom clears `window_gradient_stops`
// so chrome falls back to the solid `background_color`.

#[allow(clippy::too_many_arguments)]
fn draw_window_gradient_card(
    config: &mut crate::config::LanternConfig,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    card_x: f32,
    card_y: f32,
    card_w: f32,
    card_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    use crate::panels::BG_COLORS;

    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + 130.0 * s;
    let end_x = card_x + card_w - CARD_INNER_PAD_H * s;

    let mut cy = draw_section_card(
        painter,
        text,
        fox,
        "Window Gradient",
        card_x,
        card_y,
        card_w,
        card_h,
        s,
        sw,
        sh,
    );

    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;
    let value_w = VALUE_W * s;
    let avail = (card_inner_w - (ctrl_x - card_inner_x) - value_w - 12.0 * s).max(80.0 * s);
    let alpha_ctrl_w = (SLIDER_W * s).min(avail);
    let alpha_value_x = ctrl_x + alpha_ctrl_w + 8.0 * s;
    let alpha_slider_h = SLIDER_H * s;
    let alpha_vsz = VALUE_SIZE * s;

    // Toggle-chip row: 5 chips for the 5 glow positions. Each chip is a
    // selected/deselected pill — clicking it toggles that position on/off.
    // The chips share the row with the implicit "Position" label so the
    // ctrl column starts at `ctrl_x`.
    draw_gradient_chip_row(
        painter,
        text,
        ix,
        fox,
        &config.appearance.window_gradient_stops,
        label_x,
        ctrl_x,
        &mut cy,
        row,
        lsz,
        s,
        sw,
        sh,
    );

    // Shared color swatch row — picking a swatch repaints every currently
    // enabled position. Falls back to the first non-empty stored hex so
    // the row reflects the live look.
    let shared_hex = current_shared_gradient_color(&config.appearance.window_gradient_stops);
    draw_bg_swatch_row(
        painter,
        text,
        ix,
        fox,
        "Color",
        ZONE_GRADIENT_STOP_BASE,
        BG_COLORS,
        &shared_hex,
        label_x,
        ctrl_x,
        end_x,
        &mut cy,
        row,
        lsz,
        s,
        sw,
        sh,
    );

    // Intensity slider — global, applies to all enabled glows. The
    // slider value is cubed downstream in `palette.rs` so the lower
    // half of the slider produces the "subtle tint" range.
    {
        let zone_id = ZONE_GRADIENT_ALPHA_BASE;
        let label_y = cy + (row - lsz) / 2.0;
        text.queue(
            "Intensity",
            lsz,
            label_x,
            label_y,
            fox.text_secondary,
            ctrl_x - label_x,
            sw,
            sh,
        );
        let rect = Rect::new(
            ctrl_x,
            cy + (row - alpha_slider_h) / 2.0,
            alpha_ctrl_w,
            alpha_slider_h,
        );
        let zone = ix.add_zone(zone_id, rect);

        let mut alpha = config
            .appearance
            .window_gradient_stop_alphas
            .first()
            .copied()
            .unwrap_or(1.0);

        if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
            alpha = (f * 100.0).round() / 100.0;
            config.appearance.window_gradient_stop_alphas = vec![alpha; GRADIENT_STOP_COUNT];
        }

        Slider::new(rect)
            .value(alpha)
            .hovered(zone.is_hovered())
            .active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", alpha * 100.0);
        text.queue(
            &val,
            alpha_vsz,
            alpha_value_x,
            label_y,
            fox.text_secondary,
            value_w,
            sw,
            sh,
        );
        cy += row;
    }

    // Radius slider — global, applies to every glow. Stored as a 0..1
    // fraction of each window's half-diagonal so big windows get big
    // glows automatically.
    {
        let zone_id = ZONE_GRADIENT_RADIUS;
        let label_y = cy + (row - lsz) / 2.0;
        text.queue(
            "Radius",
            lsz,
            label_x,
            label_y,
            fox.text_secondary,
            ctrl_x - label_x,
            sw,
            sh,
        );
        let rect = Rect::new(
            ctrl_x,
            cy + (row - alpha_slider_h) / 2.0,
            alpha_ctrl_w,
            alpha_slider_h,
        );
        let zone = ix.add_zone(zone_id, rect);

        let mut radius = config.appearance.window_gradient_radius;
        if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
            radius = (f * 100.0).round() / 100.0;
            config.appearance.window_gradient_radius = radius;
        }

        Slider::new(rect)
            .value(radius)
            .hovered(zone.is_hovered())
            .active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", radius * 100.0);
        text.queue(
            &val,
            alpha_vsz,
            alpha_value_x,
            label_y,
            fox.text_secondary,
            value_w,
            sw,
            sh,
        );
        cy += row;
    }

    // "Disable Gradient" / "Clear" button row.
    let btn_w = 160.0 * s;
    let btn_h = 32.0 * s;
    let btn_rect = Rect::new(label_x, cy + (row - btn_h) / 2.0, btn_w, btn_h);
    let btn_zone = ix.add_zone(ZONE_GRADIENT_CLEAR, btn_rect);
    let enabled = config
        .appearance
        .window_gradient_stops
        .iter()
        .any(|s| !s.is_empty());
    let bg = if btn_zone.is_hovered() {
        fox.surface_2
    } else {
        fox.surface
    };
    painter.rect_filled(btn_rect, 8.0 * s, bg);
    painter.rect_stroke_sdf(btn_rect, 8.0 * s, 1.0 * s, fox.muted.with_alpha(0.4));
    let btn_label = if enabled {
        "Disable Gradient"
    } else {
        "Gradient Disabled"
    };
    let bsz = 15.0 * s;
    let bw = btn_label.len() as f32 * bsz * 0.55;
    text.queue(
        btn_label,
        bsz,
        btn_rect.x + (btn_rect.w - bw) / 2.0,
        btn_rect.y + (btn_rect.h - bsz) / 2.0,
        if enabled {
            fox.text
        } else {
            fox.text_secondary
        },
        btn_rect.w,
        sw,
        sh,
    );
}

/// Toggle-chip row for the 5 glow positions. Each chip is a pill button
/// styled like the old direction picker — accent-highlighted when its
/// position is enabled, muted otherwise. Click handler in
/// `handle_appearance_click` flips that position between empty and
/// "current shared color".
#[allow(clippy::too_many_arguments)]
fn draw_gradient_chip_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    stops: &[String],
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
    text.queue(
        "Position",
        lsz,
        label_x,
        label_y,
        fox.text,
        ctrl_x - label_x,
        sw,
        sh,
    );

    let chip_w = 44.0 * s;
    let chip_h = 32.0 * s;
    let chip_gap = 8.0 * s;
    let glyph_sz = 22.0 * s;
    let mut sx = ctrl_x;
    for (chip_i, (storage_idx, glyph)) in GRADIENT_CHIPS.iter().enumerate() {
        let zone_id = ZONE_GRADIENT_CHIP_BASE + chip_i as u32;
        let chip_rect = Rect::new(sx, *cy + (row - chip_h) / 2.0, chip_w, chip_h);
        let zone = ix.add_zone(zone_id, chip_rect);

        let enabled = stops
            .get(*storage_idx)
            .map(|s| !s.is_empty())
            .unwrap_or(false);

        let bg = if enabled {
            fox.accent.with_alpha(0.18)
        } else if zone.is_hovered() {
            fox.surface_2
        } else {
            fox.surface
        };
        painter.rect_filled(chip_rect, 8.0 * s, bg);
        let stroke_color = if enabled {
            fox.accent
        } else {
            fox.muted.with_alpha(0.4)
        };
        painter.rect_stroke_sdf(chip_rect, 8.0 * s, 1.0 * s, stroke_color);

        let gw = glyph_sz * 0.55;
        text.queue(
            glyph,
            glyph_sz,
            chip_rect.x + (chip_rect.w - gw) / 2.0,
            chip_rect.y + (chip_rect.h - glyph_sz) / 2.0,
            if enabled { fox.accent } else { fox.text },
            chip_rect.w,
            sw,
            sh,
        );

        sx += chip_w + chip_gap;
    }
    *cy += row;
}

/// Pick the "shared" color for the gradient — first non-empty stored
/// stop, or empty string if every position is disabled. Used both for
/// the swatch row's selection-highlight and as the seed color when
/// enabling a previously-disabled chip.
fn current_shared_gradient_color(stops: &[String]) -> String {
    stops
        .iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default()
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
    end_x: f32,
    cy: &mut f32,
    row: f32,
    lsz: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(
        label,
        lsz,
        label_x,
        label_y,
        fox.text,
        ctrl_x - label_x,
        sw,
        sh,
    );

    let (swatch_size, swatch_gap) = crate::panels::fit_swatches(colors.len(), ctrl_x, end_x, s);
    let mut sx = ctrl_x;
    for (i, (hex, _name)) in colors.iter().enumerate() {
        let color = Color::from_hex(hex).unwrap();
        let zone_id = zone_base + i as u32;
        let swatch_rect = Rect::new(
            sx,
            *cy + (row - swatch_size) / 2.0,
            swatch_size,
            swatch_size,
        );
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

/// Ensure `window_gradient_stops` has exactly `GRADIENT_STOP_COUNT` entries
/// before indexing into it. Newly-added slots are empty strings (= position
/// disabled).
fn grow_gradient_stops(config: &mut LanternConfig) {
    let v = &mut config.appearance.window_gradient_stops;
    while v.len() < GRADIENT_STOP_COUNT {
        v.push(String::new());
    }
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
        // Focus glow toggle (Focus Follows Mouse moved to the Input/Mouse page).
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
                let items =
                    make_menu_items(PRESET_OPTIONS, ACT_ANIM_PRESET, &config.animations.preset);
                panel_state
                    .dropdown_menu
                    .open(cursor_x, cursor_y + 16.0, items);
                panel_state.active_dropdown = Some(ZONE_ANIM_PRESET_BTN);
            }
        }
        // DE font dropdown — open menu under cursor
        ZONE_FONT_BTN => {
            if panel_state.active_dropdown == Some(ZONE_FONT_BTN) {
                panel_state.close_dropdown();
            } else {
                let current = effective_font_family(&config.appearance.font_family);
                let items = make_menu_items(FONT_OPTIONS, ACT_FONT_BASE, &current);
                panel_state
                    .dropdown_menu
                    .open(cursor_x, cursor_y + 16.0, items);
                panel_state.active_dropdown = Some(ZONE_FONT_BTN);
            }
        }
        // Accent swatches
        id if id >= ZONE_ACCENT_BASE && id < ZONE_ACCENT_BASE + GLOW_COLORS.len() as u32 => {
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
        // Window gradient: "Disable All" — clear every per-position color.
        ZONE_GRADIENT_CLEAR => {
            config.appearance.window_gradient_stops = vec![String::new(); GRADIENT_STOP_COUNT];
        }
        // Position toggle chip — flip that storage slot between empty
        // and the currently-shared color. First-time enable seeds from
        // the first BG_COLORS entry so the user gets *something* visible.
        id if id >= ZONE_GRADIENT_CHIP_BASE
            && id < ZONE_GRADIENT_CHIP_BASE + GRADIENT_CHIPS.len() as u32 =>
        {
            let chip_i = (id - ZONE_GRADIENT_CHIP_BASE) as usize;
            let storage_idx = GRADIENT_CHIPS[chip_i].0;
            grow_gradient_stops(config);
            let was_enabled = !config.appearance.window_gradient_stops[storage_idx].is_empty();
            if was_enabled {
                config.appearance.window_gradient_stops[storage_idx] = String::new();
            } else {
                let mut seed =
                    current_shared_gradient_color(&config.appearance.window_gradient_stops);
                if seed.is_empty() {
                    seed = crate::panels::BG_COLORS[0].0.into();
                }
                config.appearance.window_gradient_stops[storage_idx] = seed;
            }
        }
        // Shared color swatch — repaints every currently enabled
        // position to the picked hex. If nothing is enabled yet, the
        // click is a no-op (the user toggles a chip on first).
        id if id >= ZONE_GRADIENT_STOP_BASE
            && id < ZONE_GRADIENT_STOP_BASE + crate::panels::BG_COLORS.len() as u32 =>
        {
            let color_idx = (id - ZONE_GRADIENT_STOP_BASE) as usize;
            let new_hex = crate::panels::BG_COLORS[color_idx].0.to_string();
            grow_gradient_stops(config);
            for slot in config.appearance.window_gradient_stops.iter_mut() {
                if !slot.is_empty() {
                    *slot = new_hex.clone();
                }
            }
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
