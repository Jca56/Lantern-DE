use lntrn_render::{Color, FontStyle, FontWeight, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, MenuBar, MenuEvent, MenuItem};

use crate::editor::{self, Editor};
use crate::find_bar::{draw_find_bar, match_color, FindBar};
use crate::format::FormatSpan;
use crate::scrollbar;
use crate::tab_strip::{draw_tab_strip, TabLabel, TAB_STRIP_H};
use crate::theme::Theme;
use crate::title_bar::{draw_window_controls, title_content_rect, TITLE_BAR_H};
use crate::toolbar::{self, FormatToolbar};
use crate::{
    Gpu, MENU_NEW, MENU_OPEN, MENU_SAVE, MENU_SAVE_DOCX, MENU_THEME_DARK, MENU_THEME_NIGHT,
    MENU_THEME_PAPER, ZONE_EDITOR, ZONE_EDITOR_SCROLL_THUMB,
};

pub const TOOLBAR_H: f32 = 40.0;
pub const STATUS_BAR_H: f32 = 30.0;

/// `top_inset` is the find-bar height (or 0 when hidden).
pub fn editor_rect(wf: f32, hf: f32, s: f32, top_inset: f32) -> Rect {
    let top = (TITLE_BAR_H + TAB_STRIP_H + TOOLBAR_H) * s + top_inset;
    let bottom = STATUS_BAR_H * s;
    Rect::new(0.0, top, wf, (hf - top - bottom).max(0.0))
}

pub fn file_menu_items() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        (
            "File",
            vec![
                MenuItem::action_with(MENU_NEW, "New", "Ctrl+N"),
                MenuItem::action_with(MENU_OPEN, "Open", "Ctrl+O"),
                MenuItem::action_with(MENU_SAVE, "Save", "Ctrl+S"),
                MenuItem::action_with(MENU_SAVE_DOCX, "Export .docx", ""),
            ],
        ),
        (
            "View",
            vec![
                MenuItem::action_with(MENU_THEME_PAPER, "Theme: Paper", ""),
                MenuItem::action_with(MENU_THEME_NIGHT, "Theme: Night Sky", ""),
                MenuItem::action_with(MENU_THEME_DARK, "Theme: Dark", ""),
            ],
        ),
    ]
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

/// Measure the pixel width of a byte range within a line.
fn measure_range(
    text: &mut TextRenderer,
    editor: &Editor,
    line: usize,
    from: usize,
    to: usize,
    default_font_size: f32,
) -> f32 {
    if from >= to {
        return 0.0;
    }
    measure_to_offset(text, editor, line, to, default_font_size)
        - measure_to_offset(text, editor, line, from, default_font_size)
}

/// Compute word-wrap break points for a single document line.
/// Returns byte offsets where each visual row starts (first is always 0).
fn compute_line_wraps(
    text: &mut TextRenderer,
    editor: &Editor,
    line_idx: usize,
    max_width: f32,
    default_font_size: f32,
) -> Vec<usize> {
    let line_str = &editor.lines[line_idx];
    if line_str.is_empty() || max_width <= 0.0 {
        return vec![0];
    }

    let spans = editor.formats.get(line_idx).iter_spans(line_str.len());
    let mut row_starts: Vec<usize> = vec![0];
    let mut row_x: f32 = 0.0;
    let mut last_space: Option<(usize, f32)> = None; // (byte_after_space, row_x_at_that_point)

    for span in &spans {
        let (fs, weight, style) = span_rendering(span, default_font_size);
        for (rel_i, ch) in line_str[span.start..span.end].char_indices() {
            let byte_pos = span.start + rel_i;
            let ch_w = text.measure_width_styled(
                &line_str[byte_pos..byte_pos + ch.len_utf8()],
                fs,
                weight,
                style,
            );

            if row_x + ch_w > max_width && byte_pos > *row_starts.last().unwrap() {
                if let Some((sp_byte, sp_x)) = last_space {
                    if sp_byte > *row_starts.last().unwrap() {
                        row_starts.push(sp_byte);
                        row_x -= sp_x;
                    } else {
                        row_starts.push(byte_pos);
                        row_x = 0.0;
                    }
                } else {
                    row_starts.push(byte_pos);
                    row_x = 0.0;
                }
                last_space = None;
            }

            row_x += ch_w;

            if ch == ' ' {
                last_space = Some((byte_pos + 1, row_x));
            }
        }
    }

    row_starts
}

/// Recompute all word-wrap info and store on the editor.
fn compute_wraps(
    text: &mut TextRenderer,
    editor: &mut Editor,
    max_width: f32,
    default_font_size: f32,
) {
    let mut wraps = Vec::with_capacity(editor.lines.len());
    for i in 0..editor.lines.len() {
        wraps.push(compute_line_wraps(text, editor, i, max_width, default_font_size));
    }
    editor.wrap_rows = wraps;
}

pub fn render_frame(
    gpu: &mut Gpu,
    editor: &mut Editor,
    tab_labels: &[TabLabel],
    active_tab: usize,
    find_bar: &FindBar,
    input: &mut InteractionContext,
    menu_bar: &mut MenuBar,
    fmt_toolbar: &mut FormatToolbar,
    palette: &FoxPalette,
    theme: Theme,
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

    // ── Window background (one continuous paper sheet) ───────────────
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, pal.bg);

    // ── Inline title bar ──────────────────────────────────────────────
    // No background fill — it shares the paper bg from above. We just draw
    // the window controls and let the menu bar render on top.
    draw_window_controls(painter, input, pal, wf, s);

    // ── Menu bar (inside title bar content area) ─────────────────────
    let menus = file_menu_items();
    let content = title_content_rect(wf, s);
    menu_bar.update(input, &menus, content, s);
    let labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
    menu_bar.draw_with_labels(painter, text, pal, &labels, w, h, s);

    // ── Tab strip ────────────────────────────────────────────────────
    draw_tab_strip(
        painter, text, input, tab_labels, active_tab, pal, wf, s, w, h,
    );

    // ── Formatting toolbar ────────────────────────────────────────────
    let fmt_state = editor.selection_format_state();
    toolbar::draw_toolbar(fmt_toolbar, &fmt_state, painter, text, input, pal, wf, s, w, h);

    // ── Find bar overlay (shrinks the editor area when visible) ──────
    let find_bar_top = (TITLE_BAR_H + TAB_STRIP_H + TOOLBAR_H) * s;
    let find_bar_h = find_bar.height(s);
    if find_bar_h > 0.0 {
        draw_find_bar(
            find_bar,
            painter,
            text,
            input,
            pal,
            find_bar_top,
            0.0,
            wf,
            s,
            w,
            h,
        );
    }

    // ── Editor area ───────────────────────────────────────────────────
    let er = editor_rect(wf, hf, s, find_bar_h);
    input.add_zone(ZONE_EDITOR, er);
    // Editor surface shares the paper bg — no separate fill needed. The
    // toolbar already draws a hairline along its bottom edge that visually
    // separates the writing surface from the chrome.

    let font_size = editor::FONT_SIZE * s;
    let line_h = editor::FONT_SIZE * editor::LINE_HEIGHT * s;
    let pad = editor::PAD * s;

    // Document mode: render the editor body as a centered "page" with
    // generous side margins so prose has a fixed comfortable measure
    // instead of stretching to the window edge.
    let max_page_w = 800.0 * s;
    let page_w = er.w.min(max_page_w);
    let page_x = er.x + (er.w - page_w) * 0.5;
    let content_x = page_x + pad;
    let content_max_w = (page_w - pad * 2.0).max(10.0);
    let text_y_start = er.y + pad * 1.5 - editor.scroll_offset;

    // ── Compute word wraps ────────────────────────────────────────────
    compute_wraps(text, editor, content_max_w, font_size);

    let mut vis_offsets: Vec<usize> = Vec::with_capacity(editor.lines.len());
    let mut cum = 0usize;
    for wraps in &editor.wrap_rows {
        vis_offsets.push(cum);
        cum += wraps.len();
    }
    let total_vis_rows = cum;

    let first_vis_row = ((editor.scroll_offset - pad) / line_h).floor().max(0.0) as usize;
    let vis_count = ((er.h + line_h) / line_h).ceil() as usize + 1;
    let last_vis_row = (first_vis_row + vis_count).min(total_vis_rows);

    let first_doc = if vis_offsets.is_empty() {
        0
    } else {
        vis_offsets.partition_point(|&o| o <= first_vis_row).saturating_sub(1)
    };
    let last_doc = if vis_offsets.is_empty() {
        0
    } else {
        vis_offsets
            .partition_point(|&o| o <= last_vis_row)
            .min(editor.lines.len())
    };

    // ── Selection highlight ───────────────────────────────────────────
    let sel_color = theme.selection_color();
    if let Some((sel_start, sel_end)) = editor.selection_range() {
        for i in first_doc..last_doc {
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

            let wraps = &editor.wrap_rows[i];
            for (row_idx, &row_start) in wraps.iter().enumerate() {
                let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_len);
                let vis_row = vis_offsets[i] + row_idx;
                let y = text_y_start + vis_row as f32 * line_h;
                if y + line_h < er.y || y > er.y + er.h {
                    continue;
                }

                let hl_start = sel_begin.max(row_start);
                let hl_end = sel_finish.min(row_end);
                if hl_start >= hl_end {
                    if i != sel_end.line && row_idx == wraps.len() - 1 && sel_finish >= row_end {
                        let x_end =
                            content_x + measure_range(text, editor, i, row_start, row_end, font_size);
                        painter.rect_filled(
                            Rect::new(x_end, y, font_size * 0.4, line_h),
                            0.0,
                            sel_color,
                        );
                    }
                    continue;
                }

                let x1 =
                    content_x + measure_range(text, editor, i, row_start, hl_start, font_size);
                let x2 =
                    content_x + measure_range(text, editor, i, row_start, hl_end, font_size);
                let extra =
                    if i != sel_end.line && row_idx == wraps.len() - 1 && hl_end == line_len {
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
    }

    // ── Find-bar match highlights ─────────────────────────────────────
    if !find_bar.matches.is_empty() {
        for (m_idx, m) in find_bar.matches.iter().enumerate() {
            if m.line < first_doc || m.line >= last_doc {
                continue;
            }
            let wraps = &editor.wrap_rows[m.line];
            for (row_idx, &row_start) in wraps.iter().enumerate() {
                let row_end = wraps
                    .get(row_idx + 1)
                    .copied()
                    .unwrap_or_else(|| editor.lines[m.line].len());
                if m.end <= row_start || m.start >= row_end {
                    continue;
                }
                let vis_row = vis_offsets[m.line] + row_idx;
                let y = text_y_start + vis_row as f32 * line_h;
                if y + line_h < er.y || y > er.y + er.h {
                    continue;
                }
                let hl_start = m.start.max(row_start);
                let hl_end = m.end.min(row_end);
                let x1 = content_x
                    + measure_range(text, editor, m.line, row_start, hl_start, font_size);
                let x2 = content_x
                    + measure_range(text, editor, m.line, row_start, hl_end, font_size);
                if x2 > x1 {
                    painter.rect_filled(
                        Rect::new(x1, y, x2 - x1, line_h),
                        2.0 * s,
                        match_color(m_idx == find_bar.current),
                    );
                }
            }
        }
    }

    // ── Clip the editor body so headings / large fonts can't bleed
    //    into the toolbar / tab strip above or status bar below.
    painter.push_clip(er);
    text.push_clip([er.x, er.y, er.w, er.h]);

    // ── Draw text lines with formatting spans ─────────────────────────
    for i in first_doc..last_doc {
        let line_str = &editor.lines[i];
        let wraps = &editor.wrap_rows[i];

        for (row_idx, &row_start) in wraps.iter().enumerate() {
            let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_str.len());
            let vis_row = vis_offsets[i] + row_idx;
            let y = text_y_start + vis_row as f32 * line_h;
            if y + line_h < er.y || y > er.y + er.h {
                continue;
            }
            if row_start >= row_end {
                continue;
            }

            // Draw format spans clipped to this wrap row.
            let spans = editor.formats.get(i).iter_spans(line_str.len());
            let mut x = content_x;
            for span in &spans {
                if span.end <= row_start || span.start >= row_end {
                    continue;
                }
                let clip_start = span.start.max(row_start);
                let clip_end = span.end.min(row_end);
                let span_text = &line_str[clip_start..clip_end];
                if span_text.is_empty() {
                    continue;
                }

                let (fs, weight, style) = span_rendering(&span, font_size);
                let span_color = match span.attrs.color {
                    Some(rgb) => Color::from_rgb8(
                        ((rgb >> 16) & 0xFF) as u8,
                        ((rgb >> 8) & 0xFF) as u8,
                        (rgb & 0xFF) as u8,
                    ),
                    None => pal.text,
                };

                text.queue_styled(
                    span_text, fs, x, y, span_color, content_max_w, weight, style, w, h,
                );
                let span_w = text.measure_width_styled(span_text, fs, weight, style);

                if span.attrs.underline {
                    let ul_y = y + fs + 2.0;
                    painter.line(x, ul_y, x + span_w, ul_y, 1.5 * s, pal.text);
                }
                if span.attrs.strikethrough {
                    let st_y = y + fs * 0.55;
                    painter.line(x, st_y, x + span_w, st_y, 1.5 * s, pal.text);
                }

                x += span_w;
            }
        }
    }

    // Done with editor body — release the clip so chrome can paint freely.
    painter.pop_clip();
    text.pop_clip();

    // ── Editor scrollbar ──────────────────────────────────────────────
    scrollbar::draw_editor_scrollbar(editor, painter, input, er, s, ZONE_EDITOR_SCROLL_THUMB);

    // ── Status bar ────────────────────────────────────────────────────
    crate::status_bar::draw_status_bar(editor, painter, text, pal, wf, hf, s, w, h);

    // ── Context menu (dropdown from menu bar) — overlay layer ──────────
    painter.set_layer(1);
    text.set_layer(1);
    menu_bar.context_menu.update(0.016);
    // Redraw menu bar labels in overlay layer so they aren't covered by the dropdown
    menu_bar.draw_with_labels(painter, text, pal, &labels, w, h, s);
    let menu_event = menu_bar.context_menu.draw(painter, text, input, w, h);

    // ── Submit frame (layered) ───────────────────────────────────────
    match ctx.begin_frame("lntrn-notepad") {
        Ok(mut frame) => {
            let view = frame.view().clone();

            // Layer 0: base shapes + text
            painter.render_layer(0, ctx, frame.encoder_mut(), &view, Some(Color::rgba(0.0, 0.0, 0.0, 0.0)));
            text.render_layer(0, ctx, frame.encoder_mut(), &view);

            // Flush so glyphon's prepare() for layer 1 doesn't overwrite layer 0 vertices
            frame.flush(ctx);

            // Layer 1: menu overlay shapes + text
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(1, ctx, frame.encoder_mut(), &view);

            // Cursor overlay (on top of text, but not on top of menus).
            // Preview tabs hide the cursor since they're read-only.
            if cursor_visible && !menu_bar.context_menu.is_open() {
                let c_wraps = &editor.wrap_rows[editor.cursor_line];
                let c_row_idx = c_wraps
                    .partition_point(|&s| s <= editor.cursor_col)
                    .saturating_sub(1);
                let c_row_start = c_wraps[c_row_idx];
                let c_vis_row = vis_offsets[editor.cursor_line] + c_row_idx;
                let cursor_y = text_y_start + c_vis_row as f32 * line_h;
                let cursor_x = content_x
                    + measure_range(
                        text,
                        editor,
                        editor.cursor_line,
                        c_row_start,
                        editor.cursor_col,
                        font_size,
                    );

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

