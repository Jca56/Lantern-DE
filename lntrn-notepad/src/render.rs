use lntrn_render::{Color, FontStyle, FontWeight, Rect, TextRenderer};
use lntrn_ui::gpu::{ContextMenu, FoxPalette, InteractionContext, MenuBar, MenuEvent};

use crate::editor::{self, Editor};
use crate::find_bar::{draw_find_bar, match_color, FindBar};
use crate::format::{Alignment, FormatSpan};
use crate::ribbon;
use crate::scrollbar;
use crate::tab_strip::{draw_tab_strip, TabLabel, TAB_STRIP_H};
use crate::theme::Theme;
use crate::title_bar::{
    draw_window_controls, file_menu_items, title_content_rect, TITLE_BAR_H,
};
use crate::tokens;
use crate::toolbar::{self, FormatToolbar};
use crate::{Gpu, ZONE_EDITOR, ZONE_EDITOR_SCROLL_THUMB};

pub const TOOLBAR_H: f32 = 40.0;
pub const STATUS_BAR_H: f32 = 30.0;

/// `top_inset` is the find-bar height (or 0 when hidden). The gap below the
/// ribbon lives inside this rect's top edge so clicks/scroll stay consistent.
pub fn editor_rect(wf: f32, hf: f32, s: f32, top_inset: f32) -> Rect {
    let top =
        (TITLE_BAR_H + TAB_STRIP_H + TOOLBAR_H + tokens::RIBBON_PAGE_GAP) * s + top_inset;
    let bottom = STATUS_BAR_H * s;
    Rect::new(0.0, top, wf, (hf - top - bottom).max(0.0))
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
            x += text.measure_width_full(span_text, fs, weight, style, span_family(&span));
        }
        if span.end >= byte_offset {
            break;
        }
    }
    x
}

/// Convert a FormatSpan's attrs into (font_size, FontWeight, FontStyle).
pub(crate) fn span_rendering(span: &FormatSpan, default_font_size: f32) -> (f32, FontWeight, FontStyle) {
    let fs = span.attrs.font_size.unwrap_or(default_font_size);
    let weight = if span.attrs.bold { FontWeight::Bold } else { FontWeight::Normal };
    let style = if span.attrs.italic { FontStyle::Italic } else { FontStyle::Normal };
    (fs, weight, style)
}

/// The font family name a span should render with, or `None` for the default.
pub(crate) fn span_family(span: &FormatSpan) -> Option<&'static str> {
    span.attrs
        .font
        .and_then(|idx| crate::fonts::family_for_index_static(idx as usize))
}

/// Compute the x-offset for a given text alignment.
pub fn alignment_offset(align: Alignment, content_max_w: f32, row_w: f32) -> f32 {
    match align {
        Alignment::Left | Alignment::Justify => 0.0,
        Alignment::Center => (content_max_w - row_w).max(0.0) * 0.5,
        Alignment::Right => (content_max_w - row_w).max(0.0),
    }
}

/// Measure the pixel width of a byte range within a line.
pub fn measure_range(
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

pub fn render_frame(
    gpu: &mut Gpu,
    editor: &mut Editor,
    tab_labels: &[TabLabel],
    active_tab: usize,
    find_bar: &FindBar,
    input: &mut InteractionContext,
    menu_bar: &mut MenuBar,
    context_menu: &mut ContextMenu,
    fmt_toolbar: &mut FormatToolbar,
    palette: &FoxPalette,
    theme: Theme,
    scale: f32,
    page_width_frac: f32,
    cursor_visible: bool,
) -> (Option<MenuEvent>, Option<MenuEvent>) {
    let Gpu { ctx, painter, text } = gpu;

    let w = ctx.width();
    let h = ctx.height();
    let wf = w as f32;
    let hf = h as f32;
    let s = scale;
    let pal = palette;

    painter.clear();
    input.begin_frame();

    // ── Window background (the desk colour fills the whole window) ───
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), tokens::RADIUS_WINDOW * s, pal.bg);

    // ── Ribbon panel (title + tabs + toolbar cohesion) ───────────────
    // One top-lit gradient with a tight drop shadow + 1px sheen, painted
    // before any chrome content so controls/labels sit on top of it.
    ribbon::draw_ribbon_bg(painter, pal, wf, s);

    // ── Inline title bar ──────────────────────────────────────────────
    // Window controls + menu render directly on the ribbon panel.
    draw_window_controls(painter, input, pal, theme, wf, s);

    // ── Menu bar (inside title bar content area) ─────────────────────
    let menus = file_menu_items();
    let content = title_content_rect(wf, s);
    menu_bar.update(input, &menus, content, s);
    let labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
    menu_bar.draw_with_labels(painter, text, pal, &labels, w, h, s);

    // ── Tab strip ────────────────────────────────────────────────────
    draw_tab_strip(
        painter, text, input, tab_labels, active_tab, pal, theme, wf, s, w, h,
    );

    // ── Ribbon↔desk seam hairline ────────────────────────────────────
    // Drawn after the tabs so the active tab covered its own seam slice.
    let ribbon_h = ribbon::ribbon_height(s);
    painter.rect_filled(
        Rect::new(0.0, ribbon_h - 1.0 * s, wf, 1.0 * s),
        0.0,
        theme.hairline(),
    );

    // ── Formatting toolbar ────────────────────────────────────────────
    let fmt_state = editor.selection_format_state();
    let para_state = editor.current_para();
    toolbar::draw_toolbar(fmt_toolbar, &fmt_state, &para_state, painter, text, input, pal, wf, s, w, h);

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
            theme,
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

    let font_size = editor::FONT_SIZE * s;
    let pad = editor::PAD * s;

    // Document mode: render the editor body as a centered "page". Its width
    // is user-controlled via the draggable margins (`page_width_frac`), so
    // prose can be a comfortable column or fill the screen — their call.
    // Geometry is computed first so the drop shadow can sit under the sheet.
    let (page_x, page_w) = crate::page::geometry(er, page_width_frac, s);
    let content_x = page_x + pad;
    let content_max_w = (page_w - pad * 2.0).max(10.0);
    let text_y_start = er.y + pad * 1.5 - editor.scroll_offset;
    let page_rect = Rect::new(page_x, er.y, page_w, er.h);
    let r = tokens::RADIUS_PAGE * s;

    // ── Floating page on a warm desk (the signature Word look) ───────
    // Z-order is load-bearing: desk → shadow → sheet → edge → sheen.

    // Desk: a subtle vertical gradient so the gutter has depth, not flatness.
    // Extends up over the ribbon↔page gap so the breathing room reads as desk
    // (the page's top drop shadow gets to actually show there).
    let gap = tokens::RIBBON_PAGE_GAP * s;
    let desk_r = Rect::new(er.x, er.y - gap, er.w, er.h + gap);
    let (desk_top, desk_bot) = tokens::desk_gradient(pal.surface_2);
    painter.rect_gradient_linear(desk_r, 0.0, tokens::GRAD_ANGLE, desk_top, desk_bot);

    // Soft drop shadow under the sheet (before the sheet paints over it).
    let (sh_sigma, sh_color, sh_dx, sh_dy) = tokens::page_shadow(theme);
    painter.shadow(page_rect, r, sh_sigma * s, sh_color, sh_dx * s, sh_dy * s);

    // The page sheet — top corners rounded, bottom square (continuous column).
    painter.rect_4corner(page_rect, [r, r, 0.0, 0.0], pal.bg);

    // Crisp hairline around the sheet, then a faint inner top sheen for paper feel.
    painter.rect_border(page_rect, r, 1.0 * s, tokens::page_edge(theme));
    painter.inner_shadow(page_rect, r, 6.0 * s, tokens::page_sheen(theme), 0.0, -3.0 * s);

    // Draggable margin handles, registered after ZONE_EDITOR so they win the
    // hit-test where they overlap the editor body.
    crate::page::draw_handles(input, painter, pal, er, page_x, page_w, s);

    // Toolbar dropdown option zones MUST be registered AFTER ZONE_EDITOR (and
    // the page handles) — hit-testing is last-registered-wins, and the open
    // dropdown panels float over the editor body. Registering them here makes
    // clicks on font/size options win over the editor underneath.
    toolbar::register_font_option_zones(fmt_toolbar, input, s);
    toolbar::register_size_option_zones(fmt_toolbar, &fmt_state, input, s);

    // ── Compute word wraps ────────────────────────────────────────────
    crate::wrap::compute(text, editor, content_max_w, s, font_size);

    // Build per-line and per-row y-start positions. Each wrap row's height is
    // driven by the largest font size on that row, so big text gets a taller
    // row (no overlap) — this is what makes font-size changes actually work.
    let mut line_y_starts: Vec<f32> = Vec::with_capacity(editor.lines.len());
    let mut row_y_starts: Vec<Vec<f32>> = Vec::with_capacity(editor.lines.len());
    let mut y_accum = text_y_start;
    for (i, wraps) in editor.wrap_rows.iter().enumerate() {
        let para = editor.formats.get(i).para;
        let line_len = editor.lines[i].len();
        y_accum += para.space_before * s;
        line_y_starts.push(y_accum);
        let mut rows: Vec<f32> = Vec::with_capacity(wraps.len());
        for (row_idx, &row_start) in wraps.iter().enumerate() {
            let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_len);
            rows.push(y_accum);
            y_accum += editor.row_height(i, row_start, row_end, s);
        }
        row_y_starts.push(rows);
        y_accum += para.space_after * s;
    }

    // Determine visible doc lines via y-coordinate comparison. A line's bottom
    // edge is the last wrap row's y plus that row's (font-size-driven) height.
    let first_doc = line_y_starts
        .iter()
        .enumerate()
        .position(|(i, _)| {
            let rows = &row_y_starts[i];
            let last_idx = rows.len().saturating_sub(1);
            let last_start = *editor.wrap_rows[i].last().unwrap_or(&0);
            let line_len = editor.lines[i].len();
            let last_h = editor.row_height(i, last_start, line_len, s);
            rows.get(last_idx).map_or(false, |&ly| ly + last_h >= er.y)
        })
        .unwrap_or(0);
    let last_doc = line_y_starts
        .iter()
        .position(|&y| y > er.y + er.h)
        .unwrap_or(editor.lines.len())
        .min(editor.lines.len());

    /// Per-row x-offset for alignment + first-line indent + bullet hanging
    /// indent (which applies to every row of a bullet paragraph).
    #[inline]
    fn row_x_offset(
        text: &mut TextRenderer,
        editor: &Editor,
        line: usize,
        row_idx: usize,
        row_start: usize,
        row_end: usize,
        content_max_w: f32,
        font_size: f32,
        s: f32,
    ) -> f32 {
        let para = editor.formats.get(line).para;
        let bullet = if para.bullet { editor::BULLET_INDENT * s } else { 0.0 };
        let avail = (content_max_w - bullet).max(10.0);
        let row_w = measure_range(text, editor, line, row_start, row_end, font_size);
        let align = alignment_offset(para.alignment, avail, row_w);
        let indent = if row_idx == 0 { para.first_indent * s } else { 0.0 };
        bullet + align + indent
    }

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
                let y = row_y_starts[i][row_idx];
                let row_h = editor.row_height(i, row_start, row_end, s);
                if y + row_h < er.y || y > er.y + er.h {
                    continue;
                }
                let x_off = row_x_offset(
                    text, editor, i, row_idx, row_start, row_end,
                    content_max_w, font_size, s,
                );

                let hl_start = sel_begin.max(row_start);
                let hl_end = sel_finish.min(row_end);
                if hl_start >= hl_end {
                    if i != sel_end.line && row_idx == wraps.len() - 1 && sel_finish >= row_end {
                        let x_end = content_x + x_off
                            + measure_range(text, editor, i, row_start, row_end, font_size);
                        painter.rect_filled(
                            Rect::new(x_end, y, font_size * 0.4, row_h),
                            0.0,
                            sel_color,
                        );
                    }
                    continue;
                }

                let x1 = content_x + x_off
                    + measure_range(text, editor, i, row_start, hl_start, font_size);
                let x2 = content_x + x_off
                    + measure_range(text, editor, i, row_start, hl_end, font_size);
                let extra =
                    if i != sel_end.line && row_idx == wraps.len() - 1 && hl_end == line_len {
                        font_size * 0.4
                    } else {
                        0.0
                    };
                if x2 > x1 || extra > 0.0 {
                    painter.rect_filled(
                        Rect::new(x1, y, (x2 - x1) + extra, row_h),
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
                let y = row_y_starts[m.line][row_idx];
                let row_h = editor.row_height(m.line, row_start, row_end, s);
                if y + row_h < er.y || y > er.y + er.h {
                    continue;
                }
                let x_off = row_x_offset(
                    text, editor, m.line, row_idx, row_start, row_end,
                    content_max_w, font_size, s,
                );
                let hl_start = m.start.max(row_start);
                let hl_end = m.end.min(row_end);
                let x1 = content_x + x_off
                    + measure_range(text, editor, m.line, row_start, hl_start, font_size);
                let x2 = content_x + x_off
                    + measure_range(text, editor, m.line, row_start, hl_end, font_size);
                if x2 > x1 {
                    painter.rect_filled(
                        Rect::new(x1, y, x2 - x1, row_h),
                        2.0 * s,
                        match_color(m_idx == find_bar.current, theme),
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

        let para = editor.formats.get(i).para;
        for (row_idx, &row_start) in wraps.iter().enumerate() {
            let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_str.len());
            let y = row_y_starts[i][row_idx];
            let row_h = editor.row_height(i, row_start, row_end, s);
            if y + row_h < er.y || y > er.y + er.h {
                continue;
            }

            // Bullet glyph on the first row of a bullet paragraph (even when the
            // line has no text yet). The dot is centered on the text's visual
            // midline (~0.58 of the font size below the row top, where glyph ink
            // actually sits given the 1.2 line-height), not the row-box middle.
            if para.bullet && row_idx == 0 {
                let row_fs = editor.row_font_size(i, row_start, row_end) * s;
                let dot_r = (row_fs * 0.16).max(4.0 * s);
                let dot_x = content_x + editor::BULLET_INDENT * 0.5 * s;
                let dot_y = y + row_fs * 0.58;
                painter.circle_filled(dot_x, dot_y, dot_r, pal.text);
            }

            if row_start >= row_end {
                continue;
            }

            let x_off = row_x_offset(
                text, editor, i, row_idx, row_start, row_end,
                content_max_w, font_size, s,
            );

            // Draw format spans clipped to this wrap row.
            let spans = editor.formats.get(i).iter_spans(line_str.len());
            let mut x = content_x + x_off;
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
                let family = span_family(&span);
                let span_color = match span.attrs.color {
                    Some(rgb) => Color::from_rgb8(
                        ((rgb >> 16) & 0xFF) as u8,
                        ((rgb >> 8) & 0xFF) as u8,
                        (rgb & 0xFF) as u8,
                    ),
                    None => pal.text,
                };

                text.queue_full(
                    span_text, fs, x, y, span_color, content_max_w, weight, style, family, w, h,
                );
                let span_w = text.measure_width_full(span_text, fs, weight, style, family);

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
    crate::status_bar::draw_status_bar(editor, painter, text, pal, theme, wf, hf, s, w, h);

    // ── Overlay layer — menus and toolbar dropdowns above editor text ──
    painter.set_layer(1);
    text.set_layer(1);

    // Toolbar dropdown panels (font size, line spacing)
    toolbar::draw_toolbar_overlays(
        fmt_toolbar, &fmt_state, &para_state, painter, text, pal, wf, s, w, h,
    );

    menu_bar.context_menu.update(0.016);
    // Redraw menu bar labels in overlay layer so they aren't covered by the dropdown
    menu_bar.draw_with_labels(painter, text, pal, &labels, w, h, s);
    let menu_event = menu_bar.context_menu.draw(painter, text, input, w, h);

    // Right-click context menu (Copy/Cut/Paste/Select All) — drawn last so it
    // sits above the menu-bar dropdown. Same shared widget + style as the
    // Terminal and File Manager menus.
    context_menu.update(0.016);
    let ctx_event = context_menu.draw(painter, text, input, w, h);

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
            if cursor_visible && !menu_bar.context_menu.is_open() && !context_menu.is_open() {
                let c_line = editor.cursor_line;
                let c_wraps = &editor.wrap_rows[c_line];
                let c_row_idx = c_wraps
                    .partition_point(|&s| s <= editor.cursor_col)
                    .saturating_sub(1);
                let c_row_start = c_wraps[c_row_idx];
                let c_row_end = c_wraps.get(c_row_idx + 1).copied().unwrap_or(editor.lines[c_line].len());
                let c_row_h = editor.row_height(c_line, c_row_start, c_row_end, s);
                let cursor_y = row_y_starts[c_line][c_row_idx];
                // Caret height tracks the row's font size so it's tall on big text.
                let caret_fs = editor.row_font_size(c_line, c_row_start, c_row_end) * s;
                let c_x_off = row_x_offset(
                    text, editor, c_line, c_row_idx, c_row_start, c_row_end,
                    content_max_w, font_size, s,
                );
                let cursor_x = content_x + c_x_off
                    + measure_range(
                        text,
                        editor,
                        c_line,
                        c_row_start,
                        editor.cursor_col,
                        font_size,
                    );

                if cursor_y + c_row_h > er.y && cursor_y < er.y + er.h {
                    painter.clear();
                    painter.rect_filled(
                        Rect::new(cursor_x, cursor_y, 2.5 * s, caret_fs + 2.0),
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

    (menu_event, ctx_event)
}

