use lntrn_render::{Color, FontStyle, FontWeight, Rect, TextRenderer};
use lntrn_ui::gpu::{
    FontSize, FoxPalette, GradientStrip, InteractionContext, MenuBar, MenuEvent, MenuItem,
    TextLabel, TitleBar,
};

use crate::editor::{self, Editor};
use crate::format::FormatSpan;
use crate::toolbar::{self, FormatToolbar};
use crate::{
    Gpu, MENU_NEW, MENU_OPEN, MENU_SAVE, MENU_SAVE_DOCX, ZONE_CLOSE, ZONE_EDITOR, ZONE_MAXIMIZE,
    ZONE_MINIMIZE,
};

pub const TITLE_BAR_H: f32 = 52.0;
pub const TOOLBAR_H: f32 = 40.0;

pub fn editor_rect(wf: f32, hf: f32, s: f32) -> Rect {
    let top = (TITLE_BAR_H + TOOLBAR_H) * s;
    Rect::new(0.0, top, wf, hf - top)
}

pub fn file_menu_items() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![(
        "File",
        vec![
            MenuItem::action_with(MENU_NEW, "New", "Ctrl+N"),
            MenuItem::action_with(MENU_OPEN, "Open", "Ctrl+O"),
            MenuItem::action_with(MENU_SAVE, "Save", "Ctrl+S"),
            MenuItem::action_with(MENU_SAVE_DOCX, "Export .docx", ""),
        ],
    )]
}

/// Measure the x-offset from content_x to a byte offset within a line,
/// accounting for per-span font size and weight/style.
pub fn measure_to_offset(
    text: &mut TextRenderer,
    editor: &Editor,
    line: usize,
    byte_offset: usize,
    default_font_size: f32,
) -> f32 {
    if byte_offset == 0 {
        return 0.0;
    }
    let line_str = &editor.lines[line];
    let spans = editor.formats.get(line).iter_spans(line_str.len());
    let mut x = 0.0;
    for span in &spans {
        if span.start >= byte_offset {
            break;
        }
        let end = span.end.min(byte_offset);
        let span_text = &line_str[span.start..end];
        if !span_text.is_empty() {
            let (fs, weight, style) = span_rendering(&span, default_font_size);
            x += text.measure_width_styled(span_text, fs, weight, style);
        }
        if span.end >= byte_offset {
            break;
        }
    }
    x
}

/// Convert a FormatSpan's attrs into (font_size, FontWeight, FontStyle).
fn span_rendering(span: &FormatSpan, default_font_size: f32) -> (f32, FontWeight, FontStyle) {
    let fs = span.attrs.font_size.unwrap_or(default_font_size);
    let weight = if span.attrs.bold { FontWeight::Bold } else { FontWeight::Normal };
    let style = if span.attrs.italic { FontStyle::Italic } else { FontStyle::Normal };
    (fs, weight, style)
}

pub fn render_frame(
    gpu: &mut Gpu,
    editor: &mut Editor,
    input: &mut InteractionContext,
    menu_bar: &mut MenuBar,
    fmt_toolbar: &mut FormatToolbar,
    palette: &FoxPalette,
    scale: f32,
    cursor_visible: bool,
) -> Option<MenuEvent> {
    let Gpu { ctx, painter, text } = gpu;

    let w = ctx.width();
    let h = ctx.height();
    let wf = w as f32;
    let hf = h as f32;
    let s = scale;
    let pal = palette;

    painter.clear();
    input.begin_frame();

    // ── Window background ─────────────────────────────────────────────
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, pal.bg);

    // ── Title bar ─────────────────────────────────────────────────────
    let title_rect = Rect::new(0.0, 0.0, wf, TITLE_BAR_H * s);
    let tb = TitleBar::new(title_rect).scale(s);
    let close_state = input.add_zone(ZONE_CLOSE, tb.close_button_rect());
    let max_state = input.add_zone(ZONE_MAXIMIZE, tb.maximize_button_rect());
    let min_state = input.add_zone(ZONE_MINIMIZE, tb.minimize_button_rect());
    let content = tb.content_rect();

    tb.close_hovered(close_state.is_hovered())
        .maximize_hovered(max_state.is_hovered())
        .minimize_hovered(min_state.is_hovered())
        .draw(painter, pal);

    // ── Menu bar (inside title bar content area) ─────────────────────
    let menus = file_menu_items();
    menu_bar.update(input, &menus, content, s);
    let labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
    menu_bar.draw_with_labels(painter, text, pal, &labels, w, h, s);

    // ── Formatting toolbar ────────────────────────────────────────────
    let fmt_state = editor.selection_format_state();
    toolbar::draw_toolbar(fmt_toolbar, &fmt_state, painter, text, input, pal, wf, s, w, h);

    // ── Gradient strip below title bar (on top of toolbar) ───────────
    let strip_y = TITLE_BAR_H * s;
    let mut strip = GradientStrip::new(0.0, strip_y, wf);
    strip.height = 4.0 * s;
    strip.draw(painter);

    // ── Editor area ───────────────────────────────────────────────────
    let er = editor_rect(wf, hf, s);
    input.add_zone(ZONE_EDITOR, er);

    let editor_bg = Color::from_rgb8(90, 90, 96);
    painter.rect_filled(er, 0.0, editor_bg);

    let font_size = editor::FONT_SIZE * s;
    let line_h = editor::FONT_SIZE * editor::LINE_HEIGHT * s;
    let pad = editor::PAD * s;
    let text_x = er.x + pad;
    let text_y_start = er.y + pad - editor.scroll_offset;
    let max_text_w = (er.w - pad * 2.0).max(10.0);

    let first_visible = ((editor.scroll_offset - pad) / line_h).floor().max(0.0) as usize;
    let visible_lines = ((er.h + line_h) / line_h).ceil() as usize + 1;
    let last_visible = (first_visible + visible_lines).min(editor.lines.len());

    let line_num_w = 50.0 * s;
    let line_num_font = FontSize::Custom(font_size * 0.85);
    let content_x = text_x + line_num_w;
    let content_max_w = (max_text_w - line_num_w).max(10.0);

    // ── Selection highlight ───────────────────────────────────────────
    let sel_color = pal.accent.with_alpha(0.3);
    if let Some((sel_start, sel_end)) = editor.selection_range() {
        for i in first_visible..last_visible {
            let y = text_y_start + i as f32 * line_h;
            if y + line_h < er.y || y > er.y + er.h {
                continue;
            }

            let line_len = editor.lines[i].len();
            let (sel_begin, sel_finish) = if i < sel_start.line || i > sel_end.line {
                continue;
            } else if i == sel_start.line && i == sel_end.line {
                (sel_start.col, sel_end.col)
            } else if i == sel_start.line {
                (sel_start.col, line_len)
            } else if i == sel_end.line {
                (0, sel_end.col)
            } else {
                (0, line_len)
            };

            let x1 = content_x + measure_to_offset(text, editor, i, sel_begin, font_size);
            let x2 = content_x + measure_to_offset(text, editor, i, sel_finish, font_size);
            let extra = if i != sel_end.line && sel_finish == line_len {
                font_size * 0.4
            } else {
                0.0
            };
            if x2 > x1 || extra > 0.0 {
                painter.rect_filled(
                    Rect::new(x1, y, (x2 - x1) + extra, line_h),
                    0.0,
                    sel_color,
                );
            }
        }
    }

    // ── Draw text lines with formatting spans ─────────────────────────
    for i in first_visible..last_visible {
        let y = text_y_start + i as f32 * line_h;
        if y + line_h < er.y || y > er.y + er.h {
            continue;
        }

        // Line number
        let num_str = format!("{}", i + 1);
        TextLabel::new(&num_str, text_x, y)
            .size(line_num_font)
            .color(pal.muted)
            .draw(text, w, h);

        // Line content — iterate format spans
        let line_str = &editor.lines[i];
        if line_str.is_empty() {
            continue;
        }
        let spans = editor.formats.get(i).iter_spans(line_str.len());
        let mut x = content_x;
        for span in &spans {
            let span_text = &line_str[span.start..span.end];
            if span_text.is_empty() {
                continue;
            }
            let (fs, weight, style) = span_rendering(span, font_size);
            text.queue_styled(span_text, fs, x, y, pal.text, content_max_w, weight, style, w, h);
            let span_w = text.measure_width_styled(span_text, fs, weight, style);

            // Underline
            if span.attrs.underline {
                let ul_y = y + fs + 2.0;
                painter.line(x, ul_y, x + span_w, ul_y, 1.5 * s, pal.text);
            }
            // Strikethrough
            if span.attrs.strikethrough {
                let st_y = y + fs * 0.55;
                painter.line(x, st_y, x + span_w, st_y, 1.5 * s, pal.text);
            }

            x += span_w;
        }
    }

    // ── Status bar ────────────────────────────────────────────────────
    let status_h = 28.0 * s;
    let status_y = hf - status_h;
    painter.rect_filled(Rect::new(0.0, status_y, wf, status_h), 0.0, pal.surface_2);

    let status_text = format!(
        "Ln {}, Col {}  |  {} lines",
        editor.cursor_line + 1,
        editor.cursor_col + 1,
        editor.lines.len(),
    );
    let status_font = FontSize::Custom(18.0 * s);
    TextLabel::new(&status_text, 12.0 * s, status_y + 4.0 * s)
        .size(status_font)
        .color(pal.text_secondary)
        .draw(text, w, h);

    // ── Context menu (dropdown from menu bar) ──────────────────────────
    menu_bar.context_menu.update(0.016);
    let menu_event = menu_bar.context_menu.draw(painter, text, input, w, h);

    // ── Submit frame ──────────────────────────────────────────────────
    match ctx.begin_frame("lntrn-notepad") {
        Ok(mut frame) => {
            painter.render_into(ctx, &mut frame, pal.bg);

            let view = frame.view().clone();
            text.render_queued(ctx, frame.encoder_mut(), &view);

            // Cursor overlay (on top of text)
            if cursor_visible {
                let cursor_y = text_y_start + editor.cursor_line as f32 * line_h;
                let cursor_x = content_x
                    + measure_to_offset(text, editor, editor.cursor_line, editor.cursor_col, font_size);

                if cursor_y + line_h > er.y && cursor_y < er.y + er.h {
                    painter.clear();
                    painter.rect_filled(
                        Rect::new(cursor_x, cursor_y, 2.5 * s, font_size + 2.0),
                        0.0,
                        pal.accent,
                    );
                    painter.render_pass_overlay(ctx, frame.encoder_mut(), &view);
                }
            }

            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[lntrn-notepad] render error: {e}"),
    }

    menu_event
}
