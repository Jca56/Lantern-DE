//! The formatting toolbar: B/I/U/S, bullet list toggle, a font-family picker,
//! an editable font-size box with +/− steppers and a preset dropdown, and the
//! alignment buttons. Click + key handling lives in `mouse.rs` / `keys.rs`,
//! wired through the zone IDs below; this module owns the layout + drawing.

use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{Dropdown, FoxPalette, InteractionContext};

use crate::fonts;
use crate::format::{Alignment, ParagraphAttrs, TextAttrs};
use crate::render::TOOLBAR_H;
use crate::title_bar::TITLE_BAR_H;
use crate::tokens;

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub const ZONE_FMT_BOLD: u32 = 20;
pub const ZONE_FMT_ITALIC: u32 = 21;
pub const ZONE_FMT_UNDERLINE: u32 = 22;
pub const ZONE_FMT_STRIKE: u32 = 23;
pub const ZONE_FMT_BULLET: u32 = 25;

pub const ZONE_FMT_SIZE_BOX: u32 = 26; // the editable size field (click = focus)
pub const ZONE_FMT_SIZE_MINUS: u32 = 27;
pub const ZONE_FMT_SIZE_PLUS: u32 = 28;
pub const ZONE_FMT_SIZE_CARET: u32 = 29; // the ▾ that opens the preset list
pub const ZONE_FMT_SIZE_OPT_BASE: u32 = 30; // .. +FONT_SIZES.len()

pub const ZONE_FMT_ALIGN_LEFT: u32 = 40;
pub const ZONE_FMT_ALIGN_CENTER: u32 = 41;
pub const ZONE_FMT_ALIGN_RIGHT: u32 = 42;

pub const ZONE_FMT_FONT_BTN: u32 = 60;
pub const ZONE_FMT_FONT_OPT_BASE: u32 = 70; // .. +fonts::FONTS.len()

pub const FONT_SIZES: &[f32] = &[11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 72.0];
/// Min/max for the editable size field.
pub const SIZE_MIN: f32 = 6.0;
pub const SIZE_MAX: f32 = 300.0;

pub fn font_size_index(size: f32) -> usize {
    FONT_SIZES.iter().position(|&s| (s - size).abs() < 0.5).unwrap_or(4)
}

/// Formatting toolbar state.
pub struct FormatToolbar {
    pub size_dropdown_open: bool,
    pub hovered_size_option: Option<usize>,
    pub font_dropdown_open: bool,
    pub hovered_font_option: Option<usize>,
    /// True while the size box is focused for keyboard entry.
    pub size_editing: bool,
    /// In-progress text in the size box while editing.
    pub size_buffer: String,
}

impl FormatToolbar {
    pub fn new() -> Self {
        Self {
            size_dropdown_open: false,
            hovered_size_option: None,
            font_dropdown_open: false,
            hovered_font_option: None,
            size_editing: false,
            size_buffer: String::new(),
        }
    }

    /// Close every open dropdown / editing state.
    pub fn close_all(&mut self) {
        self.size_dropdown_open = false;
        self.font_dropdown_open = false;
        self.size_editing = false;
    }
}

// ── Layout geometry (shared by draw + overlay + mouse, so they agree) ─────────

/// Resolved x-positions of each toolbar control, in physical pixels.
pub struct ToolbarLayout {
    pub btn_y: f32,
    pub btn_size: f32,
    pub bullet_x: f32,
    pub font_rect: Rect,
    pub size_box: Rect,
    pub size_minus: Rect,
    pub size_plus: Rect,
    pub size_caret: Rect,
    pub align_start_x: f32,
}

const FONT_BTN_W: f32 = 150.0;
const SIZE_BOX_W: f32 = 52.0;
const STEP_W: f32 = 26.0;
const CARET_W: f32 = 22.0;

pub fn layout(s: f32) -> ToolbarLayout {
    let tb_y = TITLE_BAR_H * s;
    let tb_h = TOOLBAR_H * s;
    let btn_size = 34.0 * s;
    let btn_gap = 6.0 * s;
    let btn_y = tb_y + (tb_h - btn_size) * 0.5;
    let start_x = 12.0 * s;

    // B I U S
    let after_bius = start_x + 4.0 * (btn_size + btn_gap);
    let sep1 = after_bius + 4.0 * s;
    // bullet
    let bullet_x = sep1 + 12.0 * s;
    let after_bullet = bullet_x + btn_size;
    let sep2 = after_bullet + 4.0 * s + 8.0 * s;
    // font picker
    let font_x = sep2 + 12.0 * s;
    let font_rect = Rect::new(font_x, btn_y, FONT_BTN_W * s, btn_size);
    let sep3 = font_x + FONT_BTN_W * s + 12.0 * s;
    // size group: [−][ box ][+][▾]
    let minus_x = sep3 + 12.0 * s;
    let size_minus = Rect::new(minus_x, btn_y, STEP_W * s, btn_size);
    let size_box = Rect::new(minus_x + STEP_W * s, btn_y, SIZE_BOX_W * s, btn_size);
    let size_plus = Rect::new(size_box.x + size_box.w, btn_y, STEP_W * s, btn_size);
    let size_caret = Rect::new(size_plus.x + size_plus.w, btn_y, CARET_W * s, btn_size);
    let sep4 = size_caret.x + size_caret.w + 12.0 * s;
    // alignment
    let align_start_x = sep4 + 12.0 * s;

    ToolbarLayout {
        btn_y,
        btn_size,
        bullet_x,
        font_rect,
        size_box,
        size_minus,
        size_plus,
        size_caret,
        align_start_x,
    }
}

/// The selection's font size (or the editor default) — what the size box shows
/// when not being edited, and what the steppers/presets start from.
pub fn displayed_size(fmt_state: &TextAttrs) -> f32 {
    fmt_state.font_size.unwrap_or(crate::editor::FONT_SIZE)
}

/// Draw the formatting toolbar. Click handling is done in mouse.rs via zone IDs.
#[allow(clippy::too_many_arguments)]
pub fn draw_toolbar(
    toolbar: &mut FormatToolbar,
    fmt_state: &TextAttrs,
    para_state: &ParagraphAttrs,
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    _wf: f32,
    scale: f32,
    screen_w: u32,
    screen_h: u32,
) {
    let s = scale;
    let lay = layout(s);
    let btn_size = lay.btn_size;
    let btn_gap = 6.0 * s;
    let btn_y = lay.btn_y;
    let start_x = 12.0 * s;
    let gold = palette.accent;

    // No background fill — the ribbon gradient covers the toolbar band.

    // ── B / I / U / S toggle buttons ──────────────────────────────────
    let buttons: [(u32, &str, bool, FontWeight, FontStyle); 4] = [
        (ZONE_FMT_BOLD, "B", fmt_state.bold, FontWeight::Bold, FontStyle::Normal),
        (ZONE_FMT_ITALIC, "I", fmt_state.italic, FontWeight::Normal, FontStyle::Italic),
        (ZONE_FMT_UNDERLINE, "U", fmt_state.underline, FontWeight::Normal, FontStyle::Normal),
        (ZONE_FMT_STRIKE, "S", fmt_state.strikethrough, FontWeight::Normal, FontStyle::Normal),
    ];

    for (i, (zone_id, label, active, weight, style)) in buttons.iter().enumerate() {
        let x = start_x + i as f32 * (btn_size + btn_gap);
        let rect = Rect::new(x, btn_y, btn_size, btn_size);
        let state = input.add_zone(*zone_id, rect);
        let hovered = state.is_hovered();
        pill_bg(painter, rect, *active, hovered, gold, s);

        let font_sz = 20.0 * s;
        let label_w = text.measure_width_styled(label, font_sz, *weight, *style);
        let label_x = x + (btn_size - label_w) * 0.5;
        let label_y = btn_y + (btn_size - font_sz) * 0.5;
        let label_color = if *active { gold } else { palette.text };
        text.queue_styled(
            label, font_sz, label_x, label_y, label_color,
            btn_size, *weight, *style, screen_w, screen_h,
        );
        if *zone_id == ZONE_FMT_UNDERLINE {
            let ul_y = label_y + font_sz + 1.0;
            painter.line(label_x, ul_y, label_x + label_w, ul_y, 1.5 * s, label_color);
        }
        if *zone_id == ZONE_FMT_STRIKE {
            let st_y = label_y + font_sz * 0.55;
            painter.line(label_x, st_y, label_x + label_w, st_y, 1.5 * s, label_color);
        }
    }

    separator(painter, palette, start_x + 4.0 * (btn_size + btn_gap) + 4.0 * s, btn_y, btn_size, s);

    // ── Bullet list toggle ────────────────────────────────────────────
    {
        let rect = Rect::new(lay.bullet_x, btn_y, btn_size, btn_size);
        let state = input.add_zone(ZONE_FMT_BULLET, rect);
        let active = para_state.bullet;
        pill_bg(painter, rect, active, state.is_hovered(), gold, s);
        let col = if active { gold } else { palette.text };
        // Draw a simple bullet glyph: a dot + two lines.
        let cx = rect.x + 9.0 * s;
        let cy0 = rect.center_y() - 6.0 * s;
        let gap = 6.0 * s;
        let line_x = cx + 6.0 * s;
        let line_w = btn_size - 20.0 * s;
        for k in 0..2 {
            let yy = cy0 + k as f32 * gap;
            painter.circle_filled(cx, yy + 1.0 * s, 1.8 * s, col);
            painter.rect_filled(Rect::new(line_x, yy, line_w, 2.0 * s), 1.0 * s, col);
        }
    }

    separator(painter, palette, lay.bullet_x + btn_size + 4.0 * s, btn_y, btn_size, s);

    // ── Font-family picker button ─────────────────────────────────────
    {
        let rect = lay.font_rect;
        let state = input.add_zone(ZONE_FMT_FONT_BTN, rect);
        let hovered = state.is_hovered() || toolbar.font_dropdown_open;
        pill_bg(painter, rect, toolbar.font_dropdown_open, hovered, gold, s);

        let fam_idx = fonts::family_index(
            fmt_state.font.and_then(|i| fonts::family_for_index_static(i as usize)),
        );
        let fam_name = fonts::FONTS
            .get(fam_idx)
            .or_else(|| fonts::FONTS.first())
            .map(|(n, _)| *n)
            .unwrap_or("Sans");
        let fam_family = fonts::family_for_index_static(fam_idx);
        let font_sz = 18.0 * s;
        let ty = rect.y + (rect.h - font_sz) * 0.5;
        // Render the label in its own typeface for a live preview.
        text.queue_full(
            fam_name, font_sz, rect.x + 12.0 * s, ty, palette.text,
            rect.w - 30.0 * s, FontWeight::Normal, FontStyle::Normal, fam_family, screen_w, screen_h,
        );
        caret(painter, rect.x + rect.w - 16.0 * s, rect.center_y(), palette.text_secondary, s);
    }

    // ── Size group: [−] [ box ] [+] [▾] ───────────────────────────────
    draw_size_group(toolbar, fmt_state, painter, text, input, palette, &lay, s, screen_w, screen_h);

    // ── Alignment buttons ─────────────────────────────────────────────
    let align_zones = [
        (ZONE_FMT_ALIGN_LEFT, Alignment::Left),
        (ZONE_FMT_ALIGN_CENTER, Alignment::Center),
        (ZONE_FMT_ALIGN_RIGHT, Alignment::Right),
    ];
    for (i, (zone_id, alignment)) in align_zones.iter().enumerate() {
        let x = lay.align_start_x + i as f32 * (btn_size + btn_gap);
        let rect = Rect::new(x, btn_y, btn_size, btn_size);
        let state = input.add_zone(*zone_id, rect);
        let active = para_state.alignment == *alignment;
        pill_bg(painter, rect, active, state.is_hovered(), gold, s);

        let icon_color = if active { gold } else { palette.text };
        let line_h = 2.0 * s;
        let gap = 5.0 * s;
        let ix = x + 7.0 * s;
        let iw = btn_size - 14.0 * s;
        let iy = btn_y + (btn_size - 4.0 * gap) * 0.5;
        let widths = [1.0, 0.6, 0.8, 0.5];
        for (r, frac) in widths.iter().enumerate() {
            let w = iw * frac;
            let lx = match alignment {
                Alignment::Center => ix + (iw - w) * 0.5,
                Alignment::Right => ix + (iw - w),
                _ => ix,
            };
            painter.rect_filled(Rect::new(lx, iy + r as f32 * gap, w, line_h), 0.0, icon_color);
        }
    }
}

/// Draw the editable size box + steppers + preset caret.
#[allow(clippy::too_many_arguments)]
fn draw_size_group(
    toolbar: &FormatToolbar,
    fmt_state: &TextAttrs,
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    lay: &ToolbarLayout,
    s: f32,
    screen_w: u32,
    screen_h: u32,
) {
    let gold = palette.accent;

    // Minus / plus steppers.
    for (rect, zone, glyph_minus) in [
        (lay.size_minus, ZONE_FMT_SIZE_MINUS, true),
        (lay.size_plus, ZONE_FMT_SIZE_PLUS, false),
    ] {
        let st = input.add_zone(zone, rect);
        pill_bg(painter, rect, false, st.is_hovered(), gold, s);
        let cx = rect.center_x();
        let cy = rect.center_y();
        let hw = 6.0 * s;
        painter.rect_filled(Rect::new(cx - hw, cy - 1.0 * s, hw * 2.0, 2.0 * s), 1.0 * s, palette.text);
        if !glyph_minus {
            painter.rect_filled(Rect::new(cx - 1.0 * s, cy - hw, 2.0 * s, hw * 2.0), 1.0 * s, palette.text);
        }
    }

    // The editable box.
    let box_r = lay.size_box;
    let st = input.add_zone(ZONE_FMT_SIZE_BOX, box_r);
    let focused = toolbar.size_editing;
    painter.rect_filled(box_r, tokens::RADIUS_INPUT * s, palette.bg);
    if focused {
        painter.rect_stroke_sdf(box_r, tokens::RADIUS_INPUT * s, 2.0 * s, gold);
    } else {
        let edge = if st.is_hovered() { gold.with_alpha(0.5) } else { palette.muted.with_alpha(0.5) };
        painter.rect_stroke_sdf(box_r, tokens::RADIUS_INPUT * s, 1.0 * s, edge);
    }
    let shown = if focused {
        toolbar.size_buffer.clone()
    } else {
        format!("{}", displayed_size(fmt_state) as i32)
    };
    let font_sz = 18.0 * s;
    let tw = text.measure_width(&shown, font_sz);
    let tx = box_r.x + (box_r.w - tw) * 0.5;
    let ty = box_r.y + (box_r.h - font_sz) * 0.5;
    text.queue(&shown, font_sz, tx, ty, palette.text, box_r.w, screen_w, screen_h);
    if focused {
        painter.rect_filled(Rect::new(tx + tw + 1.0 * s, ty, 2.0 * s, font_sz), 0.0, gold);
    }

    // Preset caret.
    let caret_r = lay.size_caret;
    let cst = input.add_zone(ZONE_FMT_SIZE_CARET, caret_r);
    pill_bg(painter, caret_r, toolbar.size_dropdown_open, cst.is_hovered(), gold, s);
    caret(painter, caret_r.center_x(), caret_r.center_y(), palette.text_secondary, s);
}

/// Draw open dropdown overlays (call on layer 1 so they render above text).
#[allow(clippy::too_many_arguments)]
pub fn draw_toolbar_overlays(
    toolbar: &FormatToolbar,
    fmt_state: &TextAttrs,
    _para_state: &ParagraphAttrs,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    _wf: f32,
    scale: f32,
    screen_w: u32,
    screen_h: u32,
) {
    let s = scale;
    let lay = layout(s);

    // Size preset dropdown — opens under the caret, spanning box+caret width.
    if toolbar.size_dropdown_open {
        let dd_rect = Rect::new(lay.size_box.x, lay.size_box.y, lay.size_box.w + lay.size_caret.w, lay.size_box.h);
        let cur = displayed_size(fmt_state);
        let sel = font_size_index(cur);
        let labels: Vec<String> = FONT_SIZES.iter().map(|sz| format!("{}", *sz as u32)).collect();
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        Dropdown::new(dd_rect, &refs, sel)
            .scale(s)
            .open(true)
            .hovered_option(toolbar.hovered_size_option)
            .draw(painter, text, palette, screen_w, screen_h);
    }

    // Font-family dropdown — each option rendered in its own face.
    if toolbar.font_dropdown_open {
        draw_font_dropdown(toolbar, fmt_state, painter, text, palette, &lay, s, screen_w, screen_h);
    }
}

/// Custom font dropdown so each row previews its own typeface.
#[allow(clippy::too_many_arguments)]
fn draw_font_dropdown(
    toolbar: &FormatToolbar,
    fmt_state: &TextAttrs,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    lay: &ToolbarLayout,
    s: f32,
    screen_w: u32,
    screen_h: u32,
) {
    let r = lay.font_rect;
    let row_h = 30.0 * s;
    let n = fonts::FONTS.len();
    let panel = Rect::new(r.x, r.y + r.h + 2.0 * s, r.w, row_h * n as f32 + 8.0 * s);
    painter.shadow(panel, 8.0 * s, 8.0 * s, Color::from_rgba8(0, 0, 0, 70), 0.0, 4.0 * s);
    painter.rect_filled(panel, 8.0 * s, palette.bg);
    painter.rect_stroke_sdf(panel, 8.0 * s, 1.0 * s, palette.muted.with_alpha(0.4));

    let cur_idx = fonts::family_index(
        fmt_state.font.and_then(|i| fonts::family_for_index_static(i as usize)),
    );
    let font_sz = 18.0 * s;
    for (i, (name, _)) in fonts::FONTS.iter().enumerate() {
        let row = Rect::new(panel.x + 4.0 * s, panel.y + 4.0 * s + i as f32 * row_h, panel.w - 8.0 * s, row_h);
        let hovered = toolbar.hovered_font_option == Some(i);
        if hovered {
            painter.rect_filled(row, 5.0 * s, palette.accent.with_alpha(0.16));
        }
        let fam = fonts::family_for_index_static(i);
        let col = if i == cur_idx { palette.accent } else { palette.text };
        let ty = row.y + (row.h - font_sz) * 0.5;
        text.queue_full(
            name, font_sz, row.x + 10.0 * s, ty, col, row.w - 16.0 * s,
            FontWeight::Normal, FontStyle::Normal, fam, screen_w, screen_h,
        );
    }
}

/// Register font-dropdown option hit zones (called from draw so hover updates).
pub fn register_font_option_zones(
    toolbar: &mut FormatToolbar,
    input: &mut InteractionContext,
    s: f32,
) {
    if !toolbar.font_dropdown_open {
        return;
    }
    let lay = layout(s);
    let r = lay.font_rect;
    let row_h = 30.0 * s;
    toolbar.hovered_font_option = None;
    for i in 0..fonts::FONTS.len() {
        let row = Rect::new(
            r.x + 4.0 * s,
            r.y + r.h + 2.0 * s + 4.0 * s + i as f32 * row_h,
            r.w - 8.0 * s,
            row_h,
        );
        let st = input.add_zone(ZONE_FMT_FONT_OPT_BASE + i as u32, row);
        if st.is_hovered() {
            toolbar.hovered_font_option = Some(i);
        }
    }
}

/// Register size-preset option zones + update hover.
pub fn register_size_option_zones(
    toolbar: &mut FormatToolbar,
    fmt_state: &TextAttrs,
    input: &mut InteractionContext,
    s: f32,
) {
    if !toolbar.size_dropdown_open {
        return;
    }
    let lay = layout(s);
    let dd_rect = Rect::new(lay.size_box.x, lay.size_box.y, lay.size_box.w + lay.size_caret.w, lay.size_box.h);
    let cur = displayed_size(fmt_state);
    let sel = font_size_index(cur);
    let labels: Vec<String> = FONT_SIZES.iter().map(|sz| format!("{}", *sz as u32)).collect();
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let dd = Dropdown::new(dd_rect, &refs, sel).scale(s).open(true);
    toolbar.hovered_size_option = None;
    for i in 0..FONT_SIZES.len() {
        let opt = dd.option_rect(i);
        let st = input.add_zone(ZONE_FMT_SIZE_OPT_BASE + i as u32, opt);
        if st.is_hovered() {
            toolbar.hovered_size_option = Some(i);
        }
    }
}

// ── Small drawing helpers ─────────────────────────────────────────────────────

/// Pill background for a toolbar button (gold active / hover lift / transparent).
fn pill_bg(painter: &mut Painter, rect: Rect, active: bool, hovered: bool, gold: Color, s: f32) {
    let r = tokens::RADIUS_PILL * s;
    if hovered && !active {
        painter.shadow(rect, r, 2.0 * s, tokens::hover_lift_color(), 0.0, 1.0 * s);
    }
    let bg = if active {
        gold.with_alpha(tokens::GOLD_ACTIVE_BG_ALPHA)
    } else if hovered {
        gold.with_alpha(tokens::GOLD_HOVER_ALPHA)
    } else {
        Color::TRANSPARENT
    };
    painter.rect_filled(rect, r, bg);
    if active {
        painter.rect_stroke_sdf(rect, r, 1.0 * s, gold.with_alpha(tokens::GOLD_BORDER_ALPHA));
    }
}

/// Vertical group separator.
fn separator(painter: &mut Painter, palette: &FoxPalette, x: f32, btn_y: f32, btn_size: f32, s: f32) {
    painter.line(
        x, btn_y + 4.0 * s, x, btn_y + btn_size - 4.0 * s,
        1.0 * s, palette.muted.with_alpha(0.3),
    );
}

/// A small downward chevron (▾).
fn caret(painter: &mut Painter, cx: f32, cy: f32, color: Color, s: f32) {
    let w = 4.0 * s;
    painter.triangle(cx - w, cy - w * 0.5, cx + w, cy - w * 0.5, cx, cy + w * 0.7, color);
}
