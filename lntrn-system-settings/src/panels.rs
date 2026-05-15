use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Button, ButtonVariant, ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext,
    MenuEvent, MenuItem, ScrollArea, Scrollbar, Slider, Toggle,
};

use crate::config::LanternConfig;

pub const ZONE_SAVE: u32 = 900;
pub const ZONE_CANCEL: u32 = 901;

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
const ZONE_WM_GLOW_COLOR_BASE: u32 = 311; // 311..319 for up to 9 color swatches
const ZONE_WM_GLOW_INTENSITY: u32 = 320;
const ZONE_WM_TEST_SPAWN: u32 = 321;
const ZONE_WM_ANIM_ENABLE: u32 = 322;
const ZONE_WM_ANIM_SPEED: u32 = 323;
const ZONE_WM_BORDER_COLOR_BASE: u32 = 330; // 330..339 for swatches
const ZONE_WM_TINT_COLOR_BASE: u32 = 340; // 340..349 for swatches

// Appearance panel zone IDs (350..369).
const ZONE_APPEARANCE_THEME_FOX: u32 = 350;
const ZONE_APPEARANCE_THEME_LANTERN: u32 = 351;
const ZONE_APPEARANCE_ACCENT_BASE: u32 = 360; // 360..368 swatches

// Power panel zone IDs (400–499) and action IDs (500–599) live in
// `power_panel.rs` now.

const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const VALUE_SIZE: f32 = 16.0;
const SLIDER_H: f32 = 36.0;
const SLIDER_W: f32 = 320.0;
const TOGGLE_H: f32 = 36.0;
const PAD_RIGHT: f32 = 32.0;
const LABEL_W: f32 = 200.0;
const VALUE_W: f32 = 60.0;

const GLOW_COLORS: &[(&str, &str)] = &[
    ("#4A9EFF", "Blue"),
    ("#A855F7", "Purple"),
    ("#EC4899", "Pink"),
    ("#22D3EE", "Cyan"),
    ("#22C55E", "Green"),
    ("#F97316", "Orange"),
    ("#EF4444", "Red"),
    ("#EAB308", "Gold"),
    ("#FFFFFF", "White"),
];

// ── Panel state ─────────────────────────────────────────────────────────────

pub struct PanelState {
    pub dropdown_menu: ContextMenu,
    pub active_dropdown: Option<u32>,
    /// Scroll offset for the Power panel.
    pub scroll_offset: f32,
    /// Scroll offset for the Window Manager panel.
    pub wm_scroll: f32,
}

impl PanelState {
    pub fn new(fox: &FoxPalette) -> Self {
        Self {
            dropdown_menu: ContextMenu::new(ContextMenuStyle::from_palette(fox)),
            active_dropdown: None,
            scroll_offset: 0.0,
            wm_scroll: 0.0,
        }
    }

    pub fn close_dropdown(&mut self) {
        self.dropdown_menu.close();
        self.active_dropdown = None;
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) fn slider_value_from_cursor(
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

/// Render a labeled row of color-swatch picker buttons using the shared
/// `GLOW_COLORS` palette. Zone IDs run `zone_base + 0..GLOW_COLORS.len()`.
/// The `selected_hex` is compared case-insensitively against each swatch.
/// `cy` is advanced by one `row` after the swatches are drawn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_color_swatch_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    label: &str,
    zone_base: u32,
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
    for (i, (hex, _name)) in GLOW_COLORS.iter().enumerate() {
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

/// Returns true if the rect at (text_x, row_y, text_w, row_h) significantly overlaps the menu.
/// Uses a margin to ignore shadow/padding overlap at the edges.
pub(crate) fn hidden_by_menu(text_x: f32, row_y: f32, text_w: f32, row_h: f32, menu: &ContextMenu) -> bool {
    if !menu.is_open() { return false; }
    if let Some(b) = menu.bounds() {
        // Shrink menu bounds by margin to ignore shadow overlap
        let margin = 8.0;
        let mx = b.x + margin;
        let my = b.y + margin;
        let mw = (b.w - margin * 2.0).max(0.0);
        let mh = (b.h - margin * 2.0).max(0.0);
        let overlaps_y = row_y < (my + mh) && (row_y + row_h) > my;
        let overlaps_x = text_x < (mx + mw) && (text_x + text_w) > mx;
        overlaps_x && overlaps_y
    } else {
        false
    }
}

pub(crate) fn draw_select_button(
    label: &str, current: &str,
    zone_id: u32, is_open: bool,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette,
    label_x: f32, label_w: f32, btn_x: f32, btn_w: f32, btn_h: f32,
    row: f32, lsz: f32, s: f32, sw: u32, sh: u32,
    cy: &mut f32, menu: &ContextMenu,
) {
    // Always draw the label on the left
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(label, lsz, label_x, label_y, fox.text, label_w, sw, sh);

    let rect = Rect::new(btn_x, *cy + (row - btn_h) / 2.0, btn_w, btn_h);
    let zone = ix.add_zone(zone_id, rect);

    // Always draw the button shape (painter z-order handles it)
    let bg = if is_open || zone.is_hovered() { fox.surface_2 } else { fox.surface };
    let r = 6.0 * s;
    painter.rect_filled(rect, r, bg);
    painter.rect_stroke_sdf(rect, r, 1.0 * s, fox.muted.with_alpha(0.3));

    let font = 18.0 * s;
    let pad_h = 14.0 * s;

    // Only skip button TEXT if it overlaps the menu
    let skip_text = hidden_by_menu(btn_x, *cy, btn_w, row, menu);
    if !skip_text {
        let text_y = rect.y + (rect.h - font) / 2.0;
        let display: String = current.chars().take(1).flat_map(|c| c.to_uppercase())
            .chain(current.chars().skip(1)).collect();
        text.queue(&display, font, rect.x + pad_h, text_y, fox.text,
            btn_w - pad_h * 2.0 - 12.0 * s, sw, sh);
    }

    // Always draw chevron (painter shape, not text)
    let chev_s = 8.0 * s;
    let chev_x = rect.x + rect.w - pad_h - chev_s;
    let chev_cy = rect.y + rect.h * 0.5;
    let chev_c = fox.text_secondary;
    if is_open {
        painter.line(chev_x, chev_cy + chev_s * 0.35, chev_x + chev_s * 0.5, chev_cy - chev_s * 0.35, 1.5 * s, chev_c);
        painter.line(chev_x + chev_s * 0.5, chev_cy - chev_s * 0.35, chev_x + chev_s, chev_cy + chev_s * 0.35, 1.5 * s, chev_c);
    } else {
        painter.line(chev_x, chev_cy - chev_s * 0.35, chev_x + chev_s * 0.5, chev_cy + chev_s * 0.35, 1.5 * s, chev_c);
        painter.line(chev_x + chev_s * 0.5, chev_cy + chev_s * 0.35, chev_x + chev_s, chev_cy - chev_s * 0.35, 1.5 * s, chev_c);
    }

    *cy += row;
}

pub(crate) fn make_menu_items(options: &[&str], base_id: u32, current: &str) -> Vec<MenuItem> {
    options.iter().enumerate().map(|(i, opt)| {
        let selected = opt.to_lowercase() == current.to_lowercase();
        if selected {
            MenuItem::Action {
                id: base_id + i as u32,
                label: format!("• {}", opt),
                shortcut: None, enabled: true, danger: false,
            }
        } else {
            MenuItem::action(base_id + i as u32, *opt)
        }
    }).collect()
}

// ── Window Manager panel ────────────────────────────────────────────────────

// ── Shared card layout constants (used by WM, Mouse, etc. panels) ──────────
pub(crate) const CARD_OUTER_PAD_H: f32 = 36.0; // gutter on left/right of cards
pub(crate) const CARD_OUTER_PAD_V: f32 = 16.0; // top/bottom padding of scroll content
pub(crate) const CARD_INNER_PAD_H: f32 = 24.0; // horizontal padding inside a card
pub(crate) const CARD_INNER_PAD_V: f32 = 16.0; // vertical padding inside a card
pub(crate) const CARD_HEADER_H: f32 = 36.0;    // section header strip height
pub(crate) const CARD_GAP: f32 = 28.0;         // gap between cards
const CARD_RADIUS: f32 = 12.0;
const SECTION_HEADER_SZ: f32 = 18.0;

/// Draw a section card background + header. Returns the y of the first
/// content row inside the card.
pub(crate) fn draw_section_card(
    painter: &mut Painter, text: &mut TextRenderer,
    fox: &FoxPalette,
    label: &str,
    x: f32, y: f32, w: f32, h: f32,
    s: f32, sw: u32, sh: u32,
) -> f32 {
    let card_rect = Rect::new(x, y, w, h);
    painter.rect_filled(card_rect, CARD_RADIUS * s, fox.surface.with_alpha(0.45));
    painter.rect_stroke_sdf(
        card_rect, CARD_RADIUS * s, 1.0 * s,
        fox.muted.with_alpha(0.18),
    );
    // Header label
    let header_sz = SECTION_HEADER_SZ * s;
    let header_y = y + CARD_INNER_PAD_V * s;
    text.queue(
        label, header_sz,
        x + CARD_INNER_PAD_H * s, header_y,
        fox.accent, w - CARD_INNER_PAD_H * 2.0 * s, sw, sh,
    );
    // Thin gold underline accent
    let underline_w = label.len() as f32 * header_sz * 0.55;
    painter.rect_filled(
        Rect::new(
            x + CARD_INNER_PAD_H * s,
            header_y + header_sz + 4.0 * s,
            underline_w,
            2.0 * s,
        ),
        1.0 * s,
        fox.accent.with_alpha(0.6),
    );
    // First content row begins below the header
    y + CARD_HEADER_H * s + CARD_INNER_PAD_V * s
}

// ── Appearance panel ────────────────────────────────────────────────────────

pub fn draw_appearance_panel(
    config: &mut LanternConfig,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, x: f32, y: f32, w: f32, panel_h: f32,
    s: f32, sw: u32, sh: u32,
) {
    let _ = panel_h;
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    // Card 1: Theme — radio between Fox Dark and Lantern.
    let theme_rows = 2.5;
    let theme_card_h = card_chrome_h + theme_rows * row;
    let mut cy_top = y + CARD_OUTER_PAD_V * s;
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Theme",
            card_x, cy_top, card_w, theme_card_h, s, sw, sh,
        );

        let active = config.appearance.theme.as_str();
        let btn_w = 220.0 * s;
        let btn_h = 48.0 * s;
        let gap = 16.0 * s;

        let fox_dark_rect = Rect::new(card_inner_x, cy, btn_w, btn_h);
        let lantern_rect = Rect::new(card_inner_x + btn_w + gap, cy, btn_w, btn_h);
        let fox_zone = ix.add_zone(ZONE_APPEARANCE_THEME_FOX, fox_dark_rect);
        let lantern_zone = ix.add_zone(ZONE_APPEARANCE_THEME_LANTERN, lantern_rect);

        draw_theme_card(painter, text, fox, fox_dark_rect, "Fox Dark",
            active == "fox-dark" || active == "fox",
            fox_zone.is_hovered(), s, sw, sh,
            Color::from_hex("#181818").unwrap(),
        );
        draw_theme_card(painter, text, fox, lantern_rect, "Lantern",
            active == "lantern",
            lantern_zone.is_hovered(), s, sw, sh,
            Color::from_hex("#221812").unwrap(),
        );
        cy += btn_h + row * 0.5;
        let _ = cy;
    }

    cy_top += theme_card_h + CARD_GAP * s;

    // Card 2: Accent — same 9-color palette every other picker uses.
    let accent_card_h = card_chrome_h + row;
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Accent",
            card_x, cy_top, card_w, accent_card_h, s, sw, sh,
        );
        let label_x = card_inner_x;
        let ctrl_x = card_inner_x + 100.0 * s;
        draw_color_swatch_row(
            painter, text, ix, fox,
            "Color", ZONE_APPEARANCE_ACCENT_BASE,
            &config.appearance.accent,
            label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
        );
    }
}

/// Draw a theme preview card — a swatch in the variant's bg color with the
/// name centered below. Selected gets the accent ring, hovered gets a muted
/// ring.
fn draw_theme_card(
    painter: &mut Painter,
    text: &mut TextRenderer,
    fox: &FoxPalette,
    rect: Rect,
    label: &str,
    selected: bool,
    hovered: bool,
    s: f32,
    sw: u32,
    sh: u32,
    swatch_color: Color,
) {
    let r = 8.0 * s;
    painter.rect_filled(rect, r, swatch_color);
    let ring = if selected {
        fox.accent
    } else if hovered {
        fox.text_secondary.with_alpha(0.5)
    } else {
        fox.muted.with_alpha(0.2)
    };
    let thick = if selected { 2.5 * s } else { 1.0 * s };
    painter.rect_stroke_sdf(rect, r, thick, ring);
    let label_sz = 18.0 * s;
    let tw = label_sz * 0.55 * label.len() as f32;
    text.queue(
        label, label_sz,
        rect.x + (rect.w - tw) * 0.5,
        rect.y + (rect.h - label_sz) * 0.5,
        if selected { fox.accent } else { fox.text },
        rect.w, sw, sh,
    );
}

pub fn handle_appearance_click(config: &mut LanternConfig, zone_id: u32) {
    match zone_id {
        ZONE_APPEARANCE_THEME_FOX => {
            config.appearance.theme = "fox-dark".into();
        }
        ZONE_APPEARANCE_THEME_LANTERN => {
            config.appearance.theme = "lantern".into();
        }
        id if id >= ZONE_APPEARANCE_ACCENT_BASE
            && id < ZONE_APPEARANCE_ACCENT_BASE + GLOW_COLORS.len() as u32 =>
        {
            let idx = (id - ZONE_APPEARANCE_ACCENT_BASE) as usize;
            config.appearance.accent = GLOW_COLORS[idx].0.into();
        }
        _ => {}
    }
}

pub fn draw_wm_panel(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, x: f32, y: f32, w: f32, panel_h: f32,
    s: f32, sw: u32, sh: u32, scroll_delta: f32,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    // Card geometry
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    // Inner control layout — labels, fixed-width slider, value column inside the card
    let label_w = LABEL_W * s;
    let value_w = VALUE_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;
    let avail = (card_inner_w - label_w - value_w - 12.0 * s).max(80.0 * s);
    let ctrl_w = (SLIDER_W * s).min(avail);
    let value_x = ctrl_x + ctrl_w + 8.0 * s;

    // Row counts per card
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

    // Handle scroll
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

    // ─────────────────────────────────────────────────────────────────
    // Card 1: Layout
    // ─────────────────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Layout",
            card_x, cy_top, card_w, layout_card_h, s, sw, sh,
        );

        // Slider closure scoped to an inner block so its mutable borrows on
        // painter/text/ix release before we draw the swatch row that
        // follows. `cy` stays alive — it was declared one scope up.
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

        // Border Color swatches — same accent palette the glow uses.
        draw_color_swatch_row(
            painter, text, ix, fox,
            "Border Color", ZONE_WM_BORDER_COLOR_BASE,
            &config.window_manager.border_color,
            label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
        );
    }

    cy_top += layout_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 2: Focus & Glow
    // ─────────────────────────────────────────────────────────────────
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
        // Glow Color swatches row
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

        // Glow Intensity slider (0–60% maps to alpha 0.0–0.6)
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
        cy += row;
    }

    cy_top += focus_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 3: Visual Effects
    // ─────────────────────────────────────────────────────────────────
    let mut cy = draw_section_card(
        painter, text, fox, "Visual Effects",
        card_x, cy_top, card_w, effects_card_h, s, sw, sh,
    );

    // Blur Intensity
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

    // Blur Tint
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

    // Blur Tint Color swatches — same palette as the focus glow + border.
    draw_color_swatch_row(
        painter, text, ix, fox,
        "Tint Color", ZONE_WM_TINT_COLOR_BASE,
        &config.windows.blur_tint_color,
        label_x, ctrl_x, &mut cy, row, lsz, s, sw, sh,
    );

    // Blur Darken
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

    // Background Opacity
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

    // ─────────────────────────────────────────────────────────────────
    // Card 4: Animations — master toggle + speed multiplier
    // ─────────────────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Animations",
            card_x, cy_top, card_w, animations_card_h, s, sw, sh,
        );

        // Enabled toggle
        {
            let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
            let toggle = Toggle::new(rect, config.animations.enabled)
                .label("Enable Animations").scale(s);
            let track = toggle.track_rect();
            let zone = ix.add_zone(ZONE_WM_ANIM_ENABLE, track);
            toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
            cy += row;
        }

        // Speed slider (0.25x – 3x). Snap to 0.05 to keep the value tidy.
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

    // ─────────────────────────────────────────────────────────────────
    // Card 5: Testing — spawn a blank 500x500 SSD window so the user
    // can preview titlebar / corner-radius / border-width changes.
    // ─────────────────────────────────────────────────────────────────
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

    // Draw scrollbar outside the clip region
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

// ── Save / Cancel bar ───────────────────────────────────────────────────────

pub fn draw_save_cancel_bar(
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, content_x: f32, w: f32, bottom_y: f32,
    s: f32, sw: u32, sh: u32,
) {
    let bar_h = 56.0 * s;
    let bar_y = bottom_y - bar_h;
    let pad = PAD_RIGHT * s;
    let btn_h = 38.0 * s;
    let btn_w = 100.0 * s;
    let gap = 12.0 * s;

    // Subtle separator above
    painter.rect_filled(
        Rect::new(content_x + 16.0 * s, bar_y, w - 32.0 * s, 1.0 * s),
        0.0,
        fox.muted.with_alpha(0.2),
    );

    // Save button (right-aligned, primary)
    let save_x = content_x + w - pad - btn_w;
    let save_y = bar_y + (bar_h - btn_h) / 2.0;
    let save_rect = Rect::new(save_x, save_y, btn_w, btn_h);
    let save_zone = ix.add_zone(ZONE_SAVE, save_rect);
    Button::new(save_rect, "Save")
        .variant(ButtonVariant::Primary)
        .hovered(save_zone.is_hovered())
        .pressed(save_zone.is_active())
        .scale(s)
        .draw(painter, text, fox, sw, sh);

    // Cancel button (left of save)
    let cancel_x = save_x - gap - btn_w;
    let cancel_rect = Rect::new(cancel_x, save_y, btn_w, btn_h);
    let cancel_zone = ix.add_zone(ZONE_CANCEL, cancel_rect);
    Button::new(cancel_rect, "Cancel")
        .variant(ButtonVariant::Ghost)
        .hovered(cancel_zone.is_hovered())
        .pressed(cancel_zone.is_active())
        .scale(s)
        .draw(painter, text, fox, sw, sh);
}
