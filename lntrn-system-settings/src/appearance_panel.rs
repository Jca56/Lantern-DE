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
    FoxPalette, InteractionContext, MenuEvent, ScrollArea, Scrollbar, Slider,
};

use crate::appearance_themes::ThemesPanelState;
use crate::config::LanternConfig;
use crate::panels::{
    draw_color_swatch_row, draw_section_card, make_menu_items, slider_value_from_cursor,
    PanelState, CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V, GLOW_COLORS, LABEL_SIZE, ROW_H, SLIDER_H,
    SLIDER_W, VALUE_SIZE, VALUE_W,
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
const ZONE_BORDER_COLOR_BASE: u32 = 330; // +0..9 swatches (10 entries)
const ZONE_TINT_COLOR_BASE:   u32 = 340; // +0..9 swatches (10 entries)

// Focus & Glow
const ZONE_FOCUS:            u32 = 304;
const ZONE_GLOW:             u32 = 310;
const ZONE_GLOW_COLOR_BASE:  u32 = 311; // +0..9 swatches (10 entries, 311..320)
const ZONE_GLOW_INTENSITY:   u32 = 321;

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
const ZONE_ACCENT_BASE:      u32 = 360; // +0..9 swatches (GLOW_COLORS, 10 entries)
const ZONE_BG_COLOR_BASE:    u32 = 380; // +0..9 swatches (BG_COLORS, 10 entries)

// Window Gradient card — 4 stop rows × N swatches (BG_COLORS) plus a
// direction-picker row (4 presets) and a "Clear / Disable" button.
// BG_COLORS now has 11 entries; stop block reserves 60 IDs (4 stops × 15)
// to leave headroom for future swatch additions.
const ZONE_GRADIENT_STOP_BASE: u32 = 410; // +0..(BG_COLORS.len()-1) — shared color swatch row
const ZONE_GRADIENT_CLEAR:     u32 = 470; // "Disable All" button
const ZONE_GRADIENT_CHIP_BASE: u32 = 480; // +0..4 — per-position toggle chips
const ZONE_GRADIENT_ALPHA_BASE: u32 = 490; // Intensity slider
const ZONE_GRADIENT_RADIUS:    u32 = 491; // Radius slider

// 5 independent glow positions stored in `window_gradient_stops`. Indices
// in the storage array follow `GradientCorner::ALL` (TL, TR, BL, BR, Center).
// Empty string at an index = position disabled.
const GRADIENT_STOP_COUNT: usize = 5;

/// (storage_index, chip_glyph) for the toggle-chip row. Visual order
/// matches the user's layout: TL → BL → Center → TR → BR.
const GRADIENT_CHIPS: [(usize, &str); 5] = [
    (0, "↘"),  // Top-Left,    arrow points inward toward center
    (2, "↗"),  // Bottom-Left, arrow points inward
    (4, "●"),  // Center
    (1, "↙"),  // Top-Right,   arrow points inward
    (3, "↖"),  // Bottom-Right, arrow points inward
];

// Dropdown menu action IDs (preset selection)
const ACT_ANIM_PRESET: u32 = 370; // +0..3

// Window Sizes card — 4 picker buttons + their menu action bases.
// IDs live well above the theme zones (400..520) to avoid the click-router's
// shared zone namespace handing window-size hits to ZONE_THEME_TILE_BASE.
const ZONE_WSIZE_DEFAULT_BTN: u32 = 700;
const ZONE_WSIZE_SMALL_BTN:   u32 = 701;
const ZONE_WSIZE_MEDIUM_BTN:  u32 = 702;
const ZONE_WSIZE_LARGE_BTN:   u32 = 703;
const ACT_WSIZE_DEFAULT: u32 = 710; // +0..N
const ACT_WSIZE_SMALL:   u32 = 720; // +0..N
const ACT_WSIZE_MEDIUM:  u32 = 730; // +0..N
const ACT_WSIZE_LARGE:   u32 = 740; // +0..N

const PRESET_OPTIONS: &[&str] = &["Cinematic", "Snappy", "Springy", "Linear"];

const WSIZE_ZONES: crate::appearance_window_sizes::WindowSizeZones =
    crate::appearance_window_sizes::WindowSizeZones {
        default_btn: ZONE_WSIZE_DEFAULT_BTN,
        small_btn:   ZONE_WSIZE_SMALL_BTN,
        medium_btn:  ZONE_WSIZE_MEDIUM_BTN,
        large_btn:   ZONE_WSIZE_LARGE_BTN,
    };

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
    // Window Gradient: 1 chip row + 1 color swatch row + Intensity slider
    // + Radius slider + Disable-All button row = 5 rows.
    let gradient_card_h = card_chrome_h + 5.0 * row;
    // Layout & Visual Effects: 2 sliders (Border Width, Corner Radius) +
    // Border Color swatch + 2 sliders (Blur Intensity, Blur Tint) + Tint
    // Color swatch + 2 sliders (Blur Darken, Background Opacity) = 7 rows.
    let layout_card_h = card_chrome_h + 7.0 * row;
    let anim_card_h = card_chrome_h + 4.0 * row + 3.0 * (ROW_H * 0.75) * s;
    let focus_base_rows = 2.0;
    let glow_extra_rows = if config.window_manager.focus_glow { 2.0 } else { 0.0 };
    let focus_card_h = card_chrome_h + (focus_base_rows + glow_extra_rows) * row;
    let wsize_card_h = card_chrome_h + crate::appearance_window_sizes::ROWS * row;

    let content_height = CARD_OUTER_PAD_V * s
        + themes_card_h + CARD_GAP * s
        + theme_card_h + CARD_GAP * s
        + gradient_card_h + CARD_GAP * s
        + layout_card_h + CARD_GAP * s
        + anim_card_h + CARD_GAP * s
        + focus_card_h + CARD_GAP * s
        + wsize_card_h + CARD_OUTER_PAD_V * 2.0 * s;

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

    // ── Card 1.5: Window Gradient ──────────────────────────────────
    draw_window_gradient_card(
        config, painter, text, ix, fox,
        card_x, cy_top, card_w, gradient_card_h, s, sw, sh,
    );
    cy_top += gradient_card_h + CARD_GAP * s;

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
    cy_top += focus_card_h + CARD_GAP * s;

    // ── Card 5: Window Sizes ───────────────────────────────────────
    crate::appearance_window_sizes::draw_window_sizes_card(
        config, panel_state, painter, text, ix, fox,
        card_x, cy_top, card_w, wsize_card_h, s, sw, sh,
        &WSIZE_ZONES,
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
            } else if let Some((target, idx)) = wsize_action_target(id) {
                let opts = crate::appearance_window_sizes::SIZE_PCT_OPTIONS;
                if let Some(label) = opts.get(idx) {
                    let pct = crate::appearance_window_sizes::parse_pct(label);
                    if pct > 0 {
                        match target {
                            WSizeTarget::Default => config.windows.default_size_pct = pct,
                            WSizeTarget::Small   => config.windows.size_small_pct   = pct,
                            WSizeTarget::Medium  => config.windows.size_medium_pct  = pct,
                            WSizeTarget::Large   => config.windows.size_large_pct   = pct,
                        }
                    }
                }
            } else {
                crate::appearance_themes::dispatch_theme_menu_action(
                    themes_state, config, id,
                );
            }
            panel_state.close_dropdown();
        }
    }
}

// ── Window-size menu action routing ────────────────────────────────────────

#[derive(Copy, Clone)]
enum WSizeTarget { Default, Small, Medium, Large }

/// Decode a menu action id back into (target rung, preset index). Each rung
/// owns a contiguous 10-id block (ACT_WSIZE_*), so we just bucket on the
/// upper digit.
fn wsize_action_target(id: u32) -> Option<(WSizeTarget, usize)> {
    let len = crate::appearance_window_sizes::SIZE_PCT_OPTIONS.len() as u32;
    let bases = [
        (ACT_WSIZE_DEFAULT, WSizeTarget::Default),
        (ACT_WSIZE_SMALL,   WSizeTarget::Small),
        (ACT_WSIZE_MEDIUM,  WSizeTarget::Medium),
        (ACT_WSIZE_LARGE,   WSizeTarget::Large),
    ];
    for (base, target) in bases {
        if id >= base && id < base + len {
            return Some((target, (id - base) as usize));
        }
    }
    None
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
        painter, text, fox, "Window Gradient",
        card_x, card_y, card_w, card_h, s, sw, sh,
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
        painter, text, ix, fox,
        &config.appearance.window_gradient_stops,
        label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
    );

    // Shared color swatch row — picking a swatch repaints every currently
    // enabled position. Falls back to the first non-empty stored hex so
    // the row reflects the live look.
    let shared_hex = current_shared_gradient_color(&config.appearance.window_gradient_stops);
    draw_bg_swatch_row(
        painter, text, ix, fox,
        "Color", ZONE_GRADIENT_STOP_BASE, BG_COLORS,
        &shared_hex,
        label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
    );

    // Intensity slider — global, applies to all enabled glows. The
    // slider value is cubed downstream in `palette.rs` so the lower
    // half of the slider produces the "subtle tint" range.
    {
        let zone_id = ZONE_GRADIENT_ALPHA_BASE;
        let label_y = cy + (row - lsz) / 2.0;
        text.queue(
            "Intensity", lsz,
            label_x, label_y,
            fox.text_secondary, ctrl_x - label_x, sw, sh,
        );
        let rect = Rect::new(
            ctrl_x,
            cy + (row - alpha_slider_h) / 2.0,
            alpha_ctrl_w, alpha_slider_h,
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
            config.appearance.window_gradient_stop_alphas =
                vec![alpha; GRADIENT_STOP_COUNT];
        }

        Slider::new(rect)
            .value(alpha)
            .hovered(zone.is_hovered())
            .active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.0}%", alpha * 100.0);
        text.queue(
            &val, alpha_vsz,
            alpha_value_x, label_y,
            fox.text_secondary, value_w, sw, sh,
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
            "Radius", lsz,
            label_x, label_y,
            fox.text_secondary, ctrl_x - label_x, sw, sh,
        );
        let rect = Rect::new(
            ctrl_x,
            cy + (row - alpha_slider_h) / 2.0,
            alpha_ctrl_w, alpha_slider_h,
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
            &val, alpha_vsz,
            alpha_value_x, label_y,
            fox.text_secondary, value_w, sw, sh,
        );
        cy += row;
    }

    // "Disable Gradient" / "Clear" button row.
    let btn_w = 160.0 * s;
    let btn_h = 32.0 * s;
    let btn_rect = Rect::new(
        label_x,
        cy + (row - btn_h) / 2.0,
        btn_w, btn_h,
    );
    let btn_zone = ix.add_zone(ZONE_GRADIENT_CLEAR, btn_rect);
    let enabled = config.appearance.window_gradient_stops.iter().any(|s| !s.is_empty());
    let bg = if btn_zone.is_hovered() {
        fox.surface_2
    } else {
        fox.surface
    };
    painter.rect_filled(btn_rect, 8.0 * s, bg);
    painter.rect_stroke_sdf(btn_rect, 8.0 * s, 1.0 * s, fox.muted.with_alpha(0.4));
    let btn_label = if enabled { "Disable Gradient" } else { "Gradient Disabled" };
    let bsz = 15.0 * s;
    let bw = btn_label.len() as f32 * bsz * 0.55;
    text.queue(
        btn_label, bsz,
        btn_rect.x + (btn_rect.w - bw) / 2.0,
        btn_rect.y + (btn_rect.h - bsz) / 2.0,
        if enabled { fox.text } else { fox.text_secondary },
        btn_rect.w, sw, sh,
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
    text.queue("Position", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

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
        let stroke_color = if enabled { fox.accent } else { fox.muted.with_alpha(0.4) };
        painter.rect_stroke_sdf(chip_rect, 8.0 * s, 1.0 * s, stroke_color);

        let gw = glyph_sz * 0.55;
        text.queue(
            glyph, glyph_sz,
            chip_rect.x + (chip_rect.w - gw) / 2.0,
            chip_rect.y + (chip_rect.h - glyph_sz) / 2.0,
            if enabled { fox.accent } else { fox.text },
            chip_rect.w, sw, sh,
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
        // Window-size picker buttons — each opens a dropdown of percentage
        // presets keyed to the matching ACT_WSIZE_* base.
        id @ (ZONE_WSIZE_DEFAULT_BTN | ZONE_WSIZE_SMALL_BTN
              | ZONE_WSIZE_MEDIUM_BTN | ZONE_WSIZE_LARGE_BTN) => {
            if panel_state.active_dropdown == Some(id) {
                panel_state.close_dropdown();
            } else {
                let (base, cur) = match id {
                    ZONE_WSIZE_DEFAULT_BTN => (ACT_WSIZE_DEFAULT, config.windows.default_size_pct),
                    ZONE_WSIZE_SMALL_BTN   => (ACT_WSIZE_SMALL,   config.windows.size_small_pct),
                    ZONE_WSIZE_MEDIUM_BTN  => (ACT_WSIZE_MEDIUM,  config.windows.size_medium_pct),
                    ZONE_WSIZE_LARGE_BTN   => (ACT_WSIZE_LARGE,   config.windows.size_large_pct),
                    _ => unreachable!(),
                };
                let items = crate::appearance_window_sizes::build_size_menu(base, cur);
                panel_state.dropdown_menu.open(cursor_x, cursor_y + 16.0, items);
                panel_state.active_dropdown = Some(id);
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
        // Window gradient: "Disable All" — clear every per-position color.
        ZONE_GRADIENT_CLEAR => {
            config.appearance.window_gradient_stops =
                vec![String::new(); GRADIENT_STOP_COUNT];
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
                let mut seed = current_shared_gradient_color(
                    &config.appearance.window_gradient_stops,
                );
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

