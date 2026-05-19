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
const ZONE_GRADIENT_STOP_BASE: u32 = 410; // +0..(4*BG_COLORS.len()-1)
const ZONE_GRADIENT_CLEAR:     u32 = 470;
const ZONE_GRADIENT_DIR_BASE:  u32 = 480; // +0..3 (4 direction presets)
const ZONE_GRADIENT_ALPHA_BASE: u32 = 490; // +0..(GRADIENT_STOP_COUNT-1) per-stop alpha sliders

const GRADIENT_STOP_COUNT: usize = 4;
const GRADIENT_DEFAULT_STOPS: [&str; GRADIENT_STOP_COUNT] = [
    "#0E0E0E", // Black
    "#2C0F5C", // Purple
    "#0F1F5C", // Blue
    "#5C1230", // Pink
];

/// (config-key, label) for each direction preset. Label uses arrows so the
/// visual meaning is obvious at a glance.
const GRADIENT_DIRECTIONS: &[(&str, &str)] = &[
    ("diagonal",         "↘"),
    ("diagonal-reverse", "↙"),
    ("vertical",         "↓"),
    ("horizontal",       "→"),
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
    // Window Gradient: 1 direction row + 4 × (swatch row + alpha-slider row)
    // + 1 disable-button row.
    let gradient_card_h =
        card_chrome_h + (GRADIENT_STOP_COUNT as f32 * 2.0 + 2.0) * row;
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

    // Direction picker row — 4 chip buttons (↘ ↙ ↓ →). The selected one is
    // outlined; click swaps the saved preset.
    draw_direction_row(
        painter, text, ix, fox,
        &config.appearance.window_gradient_direction,
        label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
    );

    // Render one swatch row + one alpha slider row per stop. Each swatch
    // writes index `i` of window_gradient_stops; clicking a swatch fills
    // earlier missing slots with the default for that index. The alpha
    // slider (0..1, default 1.0) dials that stop's transparency — 0 lets
    // the wallpaper show through at that band of the gradient.
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;
    let value_w = VALUE_W * s;
    let avail = (card_inner_w - (ctrl_x - card_inner_x) - value_w - 12.0 * s).max(80.0 * s);
    let alpha_ctrl_w = (SLIDER_W * s).min(avail);
    let alpha_value_x = ctrl_x + alpha_ctrl_w + 8.0 * s;
    let alpha_slider_h = SLIDER_H * s;
    let alpha_vsz = VALUE_SIZE * s;

    for i in 0..GRADIENT_STOP_COUNT {
        let stop_hex = config
            .appearance
            .window_gradient_stops
            .get(i)
            .cloned()
            .unwrap_or_default();
        let label = match i {
            0 => "Stop 1",
            1 => "Stop 2",
            2 => "Stop 3",
            _ => "Stop 4",
        };
        draw_bg_swatch_row(
            painter, text, ix, fox,
            label, ZONE_GRADIENT_STOP_BASE + (i as u32) * BG_COLORS.len() as u32, BG_COLORS,
            &stop_hex,
            label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
        );

        // Alpha slider row, indented under the swatch row.
        let zone_id = ZONE_GRADIENT_ALPHA_BASE + i as u32;
        let label_y = cy + (row - lsz) / 2.0;
        text.queue(
            "Alpha", lsz,
            label_x + 16.0 * s, label_y,
            fox.text_secondary, ctrl_x - label_x, sw, sh,
        );
        let rect = Rect::new(
            ctrl_x,
            cy + (row - alpha_slider_h) / 2.0,
            alpha_ctrl_w, alpha_slider_h,
        );
        let zone = ix.add_zone(zone_id, rect);

        // Current value (default 1.0 if slot missing).
        let mut alpha = config
            .appearance
            .window_gradient_stop_alphas
            .get(i)
            .copied()
            .unwrap_or(1.0);

        if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
            alpha = (f * 100.0).round() / 100.0;
            // Grow with 1.0s so earlier stops aren't silently zeroed when
            // the user first drags slider 3 from an empty alpha list.
            while config.appearance.window_gradient_stop_alphas.len() <= i {
                config.appearance.window_gradient_stop_alphas.push(1.0);
            }
            config.appearance.window_gradient_stop_alphas[i] = alpha;
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

    // "Disable Gradient" / "Clear" button row.
    let btn_w = 160.0 * s;
    let btn_h = 32.0 * s;
    let btn_rect = Rect::new(
        label_x,
        cy + (row - btn_h) / 2.0,
        btn_w, btn_h,
    );
    let btn_zone = ix.add_zone(ZONE_GRADIENT_CLEAR, btn_rect);
    let enabled = !config.appearance.window_gradient_stops.is_empty();
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

#[allow(clippy::too_many_arguments)]
fn draw_direction_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    current_dir: &str,
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
    text.queue("Direction", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

    let chip_w = 44.0 * s;
    let chip_h = 32.0 * s;
    let chip_gap = 8.0 * s;
    let arrow_sz = 22.0 * s;
    let mut sx = ctrl_x;
    for (i, (key, label)) in GRADIENT_DIRECTIONS.iter().enumerate() {
        let zone_id = ZONE_GRADIENT_DIR_BASE + i as u32;
        let chip_rect = Rect::new(sx, *cy + (row - chip_h) / 2.0, chip_w, chip_h);
        let zone = ix.add_zone(zone_id, chip_rect);

        let is_selected = current_dir.eq_ignore_ascii_case(key);
        let bg = if is_selected {
            fox.accent.with_alpha(0.18)
        } else if zone.is_hovered() {
            fox.surface_2
        } else {
            fox.surface
        };
        painter.rect_filled(chip_rect, 8.0 * s, bg);
        let stroke_color = if is_selected { fox.accent } else { fox.muted.with_alpha(0.4) };
        painter.rect_stroke_sdf(chip_rect, 8.0 * s, 1.0 * s, stroke_color);

        let lw = arrow_sz * 0.55;
        text.queue(
            label, arrow_sz,
            chip_rect.x + (chip_rect.w - lw) / 2.0,
            chip_rect.y + (chip_rect.h - arrow_sz) / 2.0,
            if is_selected { fox.accent } else { fox.text },
            chip_rect.w, sw, sh,
        );

        sx += chip_w + chip_gap;
    }
    *cy += row;
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
        // Window gradient: 4 stop rows × BG_COLORS swatches
        ZONE_GRADIENT_CLEAR => {
            config.appearance.window_gradient_stops.clear();
        }
        // Direction picker chips (↘ ↙ ↓ →)
        id if id >= ZONE_GRADIENT_DIR_BASE
            && id < ZONE_GRADIENT_DIR_BASE + GRADIENT_DIRECTIONS.len() as u32 =>
        {
            let idx = (id - ZONE_GRADIENT_DIR_BASE) as usize;
            config.appearance.window_gradient_direction =
                GRADIENT_DIRECTIONS[idx].0.into();
        }
        id if id >= ZONE_GRADIENT_STOP_BASE
            && id < ZONE_GRADIENT_STOP_BASE
                + (GRADIENT_STOP_COUNT * crate::panels::BG_COLORS.len()) as u32 =>
        {
            let rel = (id - ZONE_GRADIENT_STOP_BASE) as usize;
            let stop_idx = rel / crate::panels::BG_COLORS.len();
            let color_idx = rel % crate::panels::BG_COLORS.len();
            let new_hex = crate::panels::BG_COLORS[color_idx].0.to_string();
            // Grow with defaults so earlier stops aren't blank when the user
            // first interacts with stop 2/3/4 from a disabled state.
            while config.appearance.window_gradient_stops.len() <= stop_idx {
                let i = config.appearance.window_gradient_stops.len();
                config
                    .appearance
                    .window_gradient_stops
                    .push(GRADIENT_DEFAULT_STOPS[i].to_string());
            }
            config.appearance.window_gradient_stops[stop_idx] = new_hex;
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

