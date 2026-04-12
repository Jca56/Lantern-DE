use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{Dropdown, FoxPalette, InteractionContext};

use crate::format::{Alignment, ParagraphAttrs, TextAttrs};
use crate::render::TOOLBAR_H;
use crate::title_bar::TITLE_BAR_H;

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub const ZONE_FMT_BOLD: u32 = 20;
pub const ZONE_FMT_ITALIC: u32 = 21;
pub const ZONE_FMT_UNDERLINE: u32 = 22;
pub const ZONE_FMT_STRIKE: u32 = 23;
pub const ZONE_FMT_SIZE_BTN: u32 = 24;
pub const ZONE_FMT_SIZE_OPT_BASE: u32 = 30;

pub const ZONE_FMT_ALIGN_LEFT: u32 = 40;
pub const ZONE_FMT_ALIGN_CENTER: u32 = 41;
pub const ZONE_FMT_ALIGN_RIGHT: u32 = 42;
pub const ZONE_FMT_SPACING_BTN: u32 = 44;
pub const ZONE_FMT_SPACING_OPT_BASE: u32 = 50;

pub const FONT_SIZES: &[f32] = &[18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 64.0];
pub const LINE_SPACINGS: &[f32] = &[1.0, 1.15, 1.5, 2.0];

pub fn font_size_index(size: f32) -> usize {
    FONT_SIZES.iter().position(|&s| (s - size).abs() < 0.5).unwrap_or(2)
}

/// Formatting toolbar state.
pub struct FormatToolbar {
    pub size_dropdown_open: bool,
    pub hovered_size_option: Option<usize>,
    pub spacing_dropdown_open: bool,
    pub hovered_spacing_option: Option<usize>,
}

impl FormatToolbar {
    pub fn new() -> Self {
        Self {
            size_dropdown_open: false,
            hovered_size_option: None,
            spacing_dropdown_open: false,
            hovered_spacing_option: None,
        }
    }
}

pub fn spacing_index(spacing: f32) -> usize {
    LINE_SPACINGS.iter().position(|&v| (v - spacing).abs() < 0.01).unwrap_or(2)
}

/// Draw the formatting toolbar. Click handling is done in main.rs via zone IDs.
pub fn draw_toolbar(
    toolbar: &mut FormatToolbar,
    fmt_state: &TextAttrs,
    para_state: &ParagraphAttrs,
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    wf: f32,
    scale: f32,
    screen_w: u32,
    screen_h: u32,
) {
    let s = scale;
    let tb_y = TITLE_BAR_H * s;
    let tb_h = TOOLBAR_H * s;

    // Background — same paper color as the rest of the window so the toolbar
    // feels integrated. The tab strip below handles the visual separation
    // from the editor body via its own darker plate + hairline.
    painter.rect_filled(Rect::new(0.0, tb_y, wf, tb_h), 0.0, palette.surface);

    let btn_size = 34.0 * s;
    let btn_gap = 6.0 * s;
    let btn_y = tb_y + (tb_h - btn_size) * 0.5;
    let start_x = 12.0 * s;

    // ── Format toggle buttons ─────────────────────────────────────────
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

        let bg = if *active {
            palette.accent.with_alpha(0.25)
        } else if hovered {
            palette.surface_2
        } else {
            Color::TRANSPARENT
        };
        painter.rect_filled(rect, 6.0 * s, bg);

        if *active {
            painter.rect_stroke(rect, 6.0 * s, 1.0 * s, palette.accent.with_alpha(0.5));
        }

        let font_sz = 20.0 * s;
        let label_w = text.measure_width_styled(label, font_sz, *weight, *style);
        let label_x = x + (btn_size - label_w) * 0.5;
        let label_y = btn_y + (btn_size - font_sz) * 0.5;
        let label_color = if *active { palette.accent } else { palette.text };
        text.queue_styled(
            label, font_sz, label_x, label_y, label_color,
            btn_size, *weight, *style, screen_w, screen_h,
        );

        // U button: draw underline decoration on label
        if *zone_id == ZONE_FMT_UNDERLINE {
            let ul_y = label_y + font_sz + 1.0;
            painter.line(label_x, ul_y, label_x + label_w, ul_y, 1.5 * s, label_color);
        }
        // S button: draw strikethrough decoration on label
        if *zone_id == ZONE_FMT_STRIKE {
            let st_y = label_y + font_sz * 0.55;
            painter.line(label_x, st_y, label_x + label_w, st_y, 1.5 * s, label_color);
        }
    }

    // ── Separator ─────────────────────────────────────────────────────
    let sep_x = start_x + 4.0 * (btn_size + btn_gap) + 4.0 * s;
    painter.line(
        sep_x, btn_y + 4.0 * s,
        sep_x, btn_y + btn_size - 4.0 * s,
        1.0 * s, palette.muted.with_alpha(0.3),
    );

    // ── Font size dropdown ────────────────────────────────────────────
    let dd_x = sep_x + 12.0 * s;
    let dd_w = 90.0 * s;
    let dd_rect = Rect::new(dd_x, btn_y, dd_w, btn_size);

    let current_size = fmt_state.font_size.unwrap_or(crate::editor::FONT_SIZE);
    let selected_idx = font_size_index(current_size);
    let size_labels: Vec<String> = FONT_SIZES.iter().map(|sz| format!("{}", *sz as u32)).collect();
    let size_refs: Vec<&str> = size_labels.iter().map(|s| s.as_str()).collect();

    let dd_state = input.add_zone(ZONE_FMT_SIZE_BTN, dd_rect);

    // Register option zones when open
    if toolbar.size_dropdown_open {
        let dd_tmp = Dropdown::new(dd_rect, &size_refs, selected_idx).scale(s).open(true);
        toolbar.hovered_size_option = None;
        for i in 0..FONT_SIZES.len() {
            let opt_rect = dd_tmp.option_rect(i);
            let opt_state = input.add_zone(ZONE_FMT_SIZE_OPT_BASE + i as u32, opt_rect);
            if opt_state.is_hovered() {
                toolbar.hovered_size_option = Some(i);
            }
        }
    }

    Dropdown::new(dd_rect, &size_refs, selected_idx)
        .scale(s)
        .open(false) // Button only — overlay drawn on layer 1
        .button_hovered(dd_state.is_hovered())
        .draw(painter, text, palette, screen_w, screen_h);

    // ── Separator 2 ──────────────────────────────────────────────────
    let sep2_x = dd_x + dd_w + 12.0 * s;
    painter.line(
        sep2_x, btn_y + 4.0 * s,
        sep2_x, btn_y + btn_size - 4.0 * s,
        1.0 * s, palette.muted.with_alpha(0.3),
    );

    // ── Alignment buttons (radio-style, one active) ──────────────────
    let align_start_x = sep2_x + 12.0 * s;
    let align_zones = [
        (ZONE_FMT_ALIGN_LEFT, Alignment::Left),
        (ZONE_FMT_ALIGN_CENTER, Alignment::Center),
        (ZONE_FMT_ALIGN_RIGHT, Alignment::Right),
    ];

    for (i, (zone_id, alignment)) in align_zones.iter().enumerate() {
        let x = align_start_x + i as f32 * (btn_size + btn_gap);
        let rect = Rect::new(x, btn_y, btn_size, btn_size);
        let state = input.add_zone(*zone_id, rect);
        let hovered = state.is_hovered();
        let active = para_state.alignment == *alignment;

        let bg = if active {
            palette.accent.with_alpha(0.25)
        } else if hovered {
            palette.surface_2
        } else {
            Color::TRANSPARENT
        };
        painter.rect_filled(rect, 6.0 * s, bg);
        if active {
            painter.rect_stroke(rect, 6.0 * s, 1.0 * s, palette.accent.with_alpha(0.5));
        }

        // Draw alignment icon — horizontal lines of varying widths
        let icon_color = if active { palette.accent } else { palette.text };
        let line_h = 2.0 * s;
        let gap = 5.0 * s;
        let ix = x + 7.0 * s;
        let iw = btn_size - 14.0 * s;
        let iy = btn_y + (btn_size - 4.0 * gap) * 0.5;

        match alignment {
            Alignment::Left => {
                painter.rect_filled(Rect::new(ix, iy, iw, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(ix, iy + gap, iw * 0.6, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(ix, iy + gap * 2.0, iw * 0.8, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(ix, iy + gap * 3.0, iw * 0.5, line_h), 0.0, icon_color);
            }
            Alignment::Center => {
                let cx = ix + iw * 0.5;
                painter.rect_filled(Rect::new(cx - iw * 0.5, iy, iw, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(cx - iw * 0.3, iy + gap, iw * 0.6, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(cx - iw * 0.4, iy + gap * 2.0, iw * 0.8, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(cx - iw * 0.25, iy + gap * 3.0, iw * 0.5, line_h), 0.0, icon_color);
            }
            Alignment::Right => {
                let rx = ix + iw;
                painter.rect_filled(Rect::new(rx - iw, iy, iw, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(rx - iw * 0.6, iy + gap, iw * 0.6, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(rx - iw * 0.8, iy + gap * 2.0, iw * 0.8, line_h), 0.0, icon_color);
                painter.rect_filled(Rect::new(rx - iw * 0.5, iy + gap * 3.0, iw * 0.5, line_h), 0.0, icon_color);
            }
            _ => {}
        }
    }

    // ── Separator 3 ──────────────────────────────────────────────────
    let sep3_x = align_start_x + 3.0 * (btn_size + btn_gap) + 4.0 * s;
    painter.line(
        sep3_x, btn_y + 4.0 * s,
        sep3_x, btn_y + btn_size - 4.0 * s,
        1.0 * s, palette.muted.with_alpha(0.3),
    );

    // ── Line spacing dropdown ────────────────────────────────────────
    let sp_x = sep3_x + 12.0 * s;
    let sp_w = 80.0 * s;
    let sp_rect = Rect::new(sp_x, btn_y, sp_w, btn_size);

    let sp_idx = spacing_index(para_state.line_spacing);
    let sp_labels: Vec<String> = LINE_SPACINGS.iter().map(|v| format!("{:.2}×", v)).collect();
    let sp_refs: Vec<&str> = sp_labels.iter().map(|s| s.as_str()).collect();

    let sp_state = input.add_zone(ZONE_FMT_SPACING_BTN, sp_rect);

    if toolbar.spacing_dropdown_open {
        let dd_tmp = Dropdown::new(sp_rect, &sp_refs, sp_idx).scale(s).open(true);
        toolbar.hovered_spacing_option = None;
        for i in 0..LINE_SPACINGS.len() {
            let opt_rect = dd_tmp.option_rect(i);
            let opt_state = input.add_zone(ZONE_FMT_SPACING_OPT_BASE + i as u32, opt_rect);
            if opt_state.is_hovered() {
                toolbar.hovered_spacing_option = Some(i);
            }
        }
    }

    Dropdown::new(sp_rect, &sp_refs, sp_idx)
        .scale(s)
        .open(false) // Button only — overlay drawn on layer 1
        .button_hovered(sp_state.is_hovered())
        .draw(painter, text, palette, screen_w, screen_h);
}

/// Draw open dropdown overlays (call on layer 1 so they render above editor text).
pub fn draw_toolbar_overlays(
    toolbar: &FormatToolbar,
    fmt_state: &TextAttrs,
    para_state: &ParagraphAttrs,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    wf: f32,
    scale: f32,
    screen_w: u32,
    screen_h: u32,
) {
    if !toolbar.size_dropdown_open && !toolbar.spacing_dropdown_open {
        return;
    }

    let s = scale;
    let tb_y = TITLE_BAR_H * s;
    let tb_h = TOOLBAR_H * s;
    let btn_size = 34.0 * s;
    let btn_gap = 6.0 * s;
    let btn_y = tb_y + (tb_h - btn_size) * 0.5;
    let start_x = 12.0 * s;

    let sep_x = start_x + 4.0 * (btn_size + btn_gap) + 4.0 * s;
    let dd_x = sep_x + 12.0 * s;
    let dd_w = 90.0 * s;
    let dd_rect = Rect::new(dd_x, btn_y, dd_w, btn_size);

    // Font size dropdown overlay
    if toolbar.size_dropdown_open {
        let current_size = fmt_state.font_size.unwrap_or(crate::editor::FONT_SIZE);
        let selected_idx = font_size_index(current_size);
        let size_labels: Vec<String> = FONT_SIZES.iter().map(|sz| format!("{}", *sz as u32)).collect();
        let size_refs: Vec<&str> = size_labels.iter().map(|s| s.as_str()).collect();

        Dropdown::new(dd_rect, &size_refs, selected_idx)
            .scale(s)
            .open(true)
            .hovered_option(toolbar.hovered_size_option)
            .draw(painter, text, palette, screen_w, screen_h);
    }

    // Line spacing dropdown overlay
    if toolbar.spacing_dropdown_open {
        let sep2_x = dd_x + dd_w + 12.0 * s;
        let align_start_x = sep2_x + 12.0 * s;
        let sep3_x = align_start_x + 3.0 * (btn_size + btn_gap) + 4.0 * s;
        let sp_x = sep3_x + 12.0 * s;
        let sp_w = 80.0 * s;
        let sp_rect = Rect::new(sp_x, btn_y, sp_w, btn_size);

        let sp_idx = spacing_index(para_state.line_spacing);
        let sp_labels: Vec<String> = LINE_SPACINGS.iter().map(|v| format!("{:.2}×", v)).collect();
        let sp_refs: Vec<&str> = sp_labels.iter().map(|s| s.as_str()).collect();

        Dropdown::new(sp_rect, &sp_refs, sp_idx)
            .scale(s)
            .open(true)
            .hovered_option(toolbar.hovered_spacing_option)
            .draw(painter, text, palette, screen_w, screen_h);
    }
}
