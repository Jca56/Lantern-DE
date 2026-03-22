use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{Dropdown, FoxPalette, InteractionContext};

use crate::format::TextAttrs;
use crate::render::{TITLE_BAR_H, TOOLBAR_H};

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub const ZONE_FMT_BOLD: u32 = 20;
pub const ZONE_FMT_ITALIC: u32 = 21;
pub const ZONE_FMT_UNDERLINE: u32 = 22;
pub const ZONE_FMT_STRIKE: u32 = 23;
pub const ZONE_FMT_SIZE_BTN: u32 = 24;
pub const ZONE_FMT_SIZE_OPT_BASE: u32 = 30;

pub const FONT_SIZES: &[f32] = &[18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 64.0];

pub fn font_size_index(size: f32) -> usize {
    FONT_SIZES.iter().position(|&s| (s - size).abs() < 0.5).unwrap_or(2)
}

/// Formatting toolbar state.
pub struct FormatToolbar {
    pub size_dropdown_open: bool,
    pub hovered_size_option: Option<usize>,
}

impl FormatToolbar {
    pub fn new() -> Self {
        Self {
            size_dropdown_open: false,
            hovered_size_option: None,
        }
    }
}

/// Draw the formatting toolbar. Click handling is done in main.rs via zone IDs.
pub fn draw_toolbar(
    toolbar: &mut FormatToolbar,
    fmt_state: &TextAttrs,
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

    // Background
    painter.rect_filled(Rect::new(0.0, tb_y, wf, tb_h), 0.0, palette.surface);
    // Bottom border
    painter.line(0.0, tb_y + tb_h, wf, tb_y + tb_h, 1.0 * s, palette.muted.with_alpha(0.2));

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
        .open(toolbar.size_dropdown_open)
        .button_hovered(dd_state.is_hovered())
        .hovered_option(toolbar.hovered_size_option)
        .draw(painter, text, palette, screen_w, screen_h);
}
