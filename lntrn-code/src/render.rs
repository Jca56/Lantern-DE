use lntrn_render::{Color, FontStyle, FontWeight, Rect, TextRenderer};
use lntrn_ui::gpu::{
    FontSize, FoxPalette, InteractionContext, MenuBar, MenuEvent, MenuItem, TextLabel,
};

use crate::bracket_match;
use crate::editor::{self, Editor};
use crate::find_bar::{draw_find_bar, match_color, FindBar};
use crate::minimap;
use crate::scrollbar;
use crate::term_panel::TermPanel;
use crate::sidebar::{draw_sidebar, Sidebar, SIDEBAR_W};
use crate::syntax::{draw_chunk_with_syntax, tokenize_line};
use crate::tab_strip::{draw_tab_strip, TabDragState, TabLabel, TAB_STRIP_H};
use crate::theme::Theme;
use crate::title_bar::{draw_window_controls, title_content_rect, TITLE_BAR_H};
use crate::{
    Gpu, MENU_NEW, MENU_OPEN, MENU_SAVE, MENU_THEME_DARK, MENU_THEME_NIGHT, MENU_THEME_PAPER,
    MENU_TOGGLE_MINIMAP, MENU_TOGGLE_WRAP,
    ZONE_EDITOR, ZONE_EDITOR_SCROLL_THUMB, ZONE_MINIMAP, ZONE_SIDEBAR_SCROLL_THUMB,
};

pub const STATUS_BAR_H: f32 = 30.0;

/// `top_inset` is the find-bar height (or 0 when hidden). `left_inset` is
/// the sidebar width (or 0 when hidden). Both come from main.rs.
pub fn editor_rect(wf: f32, hf: f32, s: f32, top_inset: f32, left_inset: f32) -> Rect {
    let top = (TITLE_BAR_H + TAB_STRIP_H) * s + top_inset;
    let bottom = STATUS_BAR_H * s;
    Rect::new(
        left_inset,
        top,
        (wf - left_inset).max(0.0),
        (hf - top - bottom).max(0.0),
    )
}

pub fn file_menu_items() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        (
            "File",
            vec![
                MenuItem::action_with(MENU_NEW, "New", "Ctrl+N"),
                MenuItem::action_with(MENU_OPEN, "Open", "Ctrl+O"),
                MenuItem::action_with(MENU_SAVE, "Save", "Ctrl+S"),
            ],
        ),
        (
            "View",
            vec![
                MenuItem::action_with(MENU_TOGGLE_WRAP, "Toggle Word Wrap", "Alt+Z"),
                MenuItem::action_with(MENU_TOGGLE_MINIMAP, "Toggle Minimap", "Alt+M"),
                MenuItem::action_with(MENU_THEME_PAPER, "Theme: Paper", ""),
                MenuItem::action_with(MENU_THEME_NIGHT, "Theme: Night Sky", ""),
                MenuItem::action_with(MENU_THEME_DARK, "Theme: Dark", ""),
            ],
        ),
    ]
}

// Wrap computation and per-span measurement helpers live in `wrap.rs`. We
// re-export them so the rest of the crate can keep `render::measure_to_offset`
// working as before.
pub use crate::wrap::{compute_wraps, measure_range, measure_to_offset, span_rendering};

pub fn render_frame(
    gpu: &mut Gpu,
    editor: &mut Editor,
    tab_labels: &[TabLabel],
    active_tab: usize,
    tab_drag: &Option<TabDragState>,
    minimap_visible: bool,
    term_panel: &mut Option<TermPanel>,
    find_bar: &FindBar,
    sidebar: &mut Sidebar,
    input: &mut InteractionContext,
    menu_bar: &mut MenuBar,
    palette: &FoxPalette,
    theme: Theme,
    scale: f32,
    cursor_visible: bool,
) -> (Option<MenuEvent>, Vec<f32>) {
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
    let tab_edges = draw_tab_strip(
        painter, text, input, tab_labels, active_tab, tab_drag, pal, wf, s, w, h,
    );

    // ── Sidebar (left strip when visible) ─────────────────────────────
    let sidebar_w = if sidebar.visible { SIDEBAR_W * s } else { 0.0 };
    if sidebar.visible {
        let sidebar_top = (TITLE_BAR_H + TAB_STRIP_H) * s;
        let sidebar_bot = hf - STATUS_BAR_H * s;
        let sidebar_rect = Rect::new(0.0, sidebar_top, sidebar_w, sidebar_bot - sidebar_top);
        draw_sidebar(sidebar, painter, text, input, pal, sidebar_rect, s, w, h);
    }

    // ── Find bar overlay (shrinks the editor area when visible) ──────
    let find_bar_top = (TITLE_BAR_H + TAB_STRIP_H) * s;
    let find_bar_h = find_bar.height(s);
    if find_bar_h > 0.0 {
        draw_find_bar(
            find_bar,
            painter,
            text,
            input,
            pal,
            find_bar_top,
            sidebar_w,
            wf - sidebar_w,
            s,
            w,
            h,
        );
    }

    // ── Editor + terminal layout ────────────────────────────────────
    let full_er = editor_rect(wf, hf, s, find_bar_h, sidebar_w);
    // Split vertically when the terminal panel is visible.
    let term_visible = term_panel.as_ref().map_or(false, |p| p.visible);
    let (editor_full, term_rect) = if term_visible {
        let above = TermPanel::editor_rect_above(full_er, s);
        let below = TermPanel::panel_rect(full_er, s);
        (above, Some(below))
    } else {
        (full_er, None)
    };
    let minimap_w = if minimap_visible { minimap::MINIMAP_W * s } else { 0.0 };
    let er = Rect::new(editor_full.x, editor_full.y, (editor_full.w - minimap_w).max(0.0), editor_full.h);
    let minimap_rect = Rect::new(er.x + er.w, er.y, minimap_w, er.h);
    input.add_zone(ZONE_EDITOR, er);

    let font_size = editor::FONT_SIZE * s;
    let line_h = editor::FONT_SIZE * editor::LINE_HEIGHT * s;
    let pad = editor::PAD * s;
    let text_x = er.x + pad;
    let text_y_start = er.y + pad - editor.scroll_offset;
    let max_text_w = (er.w - pad * 2.0).max(10.0);

    let line_num_w = 50.0 * s;
    let line_num_font = FontSize::Custom(font_size * 0.85);
    let content_x = text_x + line_num_w;
    let content_max_w = (max_text_w - line_num_w).max(10.0);

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

    // ── Bracket-match positions ───────────────────────────────────────
    let bracket_pair = bracket_match::find_matching(editor);

    // ── Clip everything inside the editor body so larger fonts (e.g.
    //    h1 headings in the markdown preview) can't bleed into the
    //    toolbar / tab strip above or status bar below.
    painter.push_clip(er);
    text.push_clip([er.x, er.y, er.w, er.h]);

    // ── Draw text lines with formatting spans ─────────────────────────
    for i in first_doc..last_doc {
        let line_str = &editor.lines[i];
        let wraps = &editor.wrap_rows[i];
        // Tokenize the line ONCE per line (not per wrap row).
        let line_tokens = tokenize_line(line_str, editor.language);
        // Byte offsets on this line where a bracket-match char lives.
        let mut bracket_cols: Vec<usize> = Vec::new();
        if let Some((a, b)) = &bracket_pair {
            if a.line == i {
                bracket_cols.push(a.col);
            }
            if b.line == i {
                bracket_cols.push(b.col);
            }
        }

        for (row_idx, &row_start) in wraps.iter().enumerate() {
            let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_str.len());
            let vis_row = vis_offsets[i] + row_idx;
            let y = text_y_start + vis_row as f32 * line_h;
            if y + line_h < er.y || y > er.y + er.h {
                continue;
            }

            // Line number on first row only
            if row_idx == 0 {
                let num_str = format!("{}", i + 1);
                TextLabel::new(&num_str, text_x, y)
                    .size(line_num_font)
                    .color(pal.muted)
                    .draw(text, w, h);

                // Indent guides — colored vertical lines per nesting level.
                let indent_bytes = line_str.len()
                    - line_str
                        .trim_start_matches(|c: char| c == ' ' || c == '\t')
                        .len();
                let indent_levels = indent_bytes / 4;
                if indent_levels > 0 {
                    let tab_w = text.measure_width("    ", font_size);
                    let guide_h = wraps.len() as f32 * line_h;
                    for lvl in 0..indent_levels {
                        let gx = content_x + lvl as f32 * tab_w;
                        painter.line(gx, y, gx, y + guide_h, 1.0 * s, theme.indent_guide_color(lvl));
                    }
                }
            }

            if row_start >= row_end {
                continue;
            }

            // Draw format spans clipped to this wrap row
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
                let default_color = match span.attrs.color {
                    Some(rgb) => Color::from_rgb8(
                        ((rgb >> 16) & 0xFF) as u8,
                        ((rgb >> 8) & 0xFF) as u8,
                        (rgb & 0xFF) as u8,
                    ),
                    None => pal.text,
                };

                // Background "chip" — drawn before the text so it sits behind.
                // Wraps the glyphs with even padding above/below so the chip
                // doesn't visually float relative to the surrounding text.
                if let Some(bg_rgb) = span.attrs.bg_color {
                    let chunk_text = &line_str[clip_start..clip_end];
                    let chip_w = text.measure_width_styled(chunk_text, fs, weight, style);
                    let pad_y = fs * 0.18;
                    let pad_x = 4.0 * s;
                    let chip_h = fs + pad_y * 2.0;
                    let chip_y = y - pad_y;
                    let bg_color = Color::from_rgb8(
                        ((bg_rgb >> 16) & 0xFF) as u8,
                        ((bg_rgb >> 8) & 0xFF) as u8,
                        (bg_rgb & 0xFF) as u8,
                    );
                    painter.rect_filled(
                        Rect::new(x - pad_x, chip_y, chip_w + pad_x * 2.0, chip_h),
                        6.0 * s,
                        bg_color,
                    );
                }

                // Split this clipped span at bracket-match positions so the
                // matched chars render in bold.
                let span_start_x = x;
                let segments =
                    bracket_match::split_at_bracket_cols(clip_start, clip_end, &bracket_cols);
                for (seg_start, seg_end, is_bracket) in segments {
                    let seg_weight = if is_bracket { FontWeight::Bold } else { weight };
                    let seg_w = draw_chunk_with_syntax(
                        text, line_str, seg_start, seg_end, &line_tokens, default_color, theme,
                        fs, x, y, seg_weight, style, content_max_w, w, h,
                    );
                    x += seg_w;
                }
                let span_w = x - span_start_x;

                if span.attrs.underline {
                    let ul_y = y + fs + 2.0;
                    painter.line(span_start_x, ul_y, x, ul_y, 1.5 * s, pal.text);
                }
                if span.attrs.strikethrough {
                    let st_y = y + fs * 0.55;
                    painter.line(span_start_x, st_y, x, st_y, 1.5 * s, pal.text);
                }

                let _ = span_w;
            }
        }
    }

    // Done with editor body — release the clip so chrome can paint freely.
    painter.pop_clip();
    text.pop_clip();

    // ── Editor scrollbar ──────────────────────────────────────────────
    // Minimap replaces the scrollbar when visible.
    if minimap_visible {
        minimap::draw_minimap(painter, editor, input, minimap_rect, theme, pal, s, ZONE_MINIMAP);
    } else {
        scrollbar::draw_editor_scrollbar(editor, painter, input, er, s, ZONE_EDITOR_SCROLL_THUMB);
    }

    // ── Sidebar scrollbar ─────────────────────────────────────────────
    if sidebar.visible {
        let sidebar_top = (TITLE_BAR_H + TAB_STRIP_H) * s + 24.0 * s;
        let sidebar_bot = hf - STATUS_BAR_H * s;
        let sidebar_viewport = Rect::new(
            0.0,
            sidebar_top,
            SIDEBAR_W * s,
            (sidebar_bot - sidebar_top).max(0.0),
        );
        scrollbar::draw_sidebar_scrollbar(
            sidebar,
            painter,
            input,
            sidebar_viewport,
            s,
            ZONE_SIDEBAR_SCROLL_THUMB,
        );
    }

    // ── Terminal panel ────────────────────────────────────────────────
    if let (Some(rect), Some(panel)) = (term_rect, term_panel.as_mut()) {
        panel.update_size(rect, s);
        panel.draw(painter, text, rect, pal, s, w, h);
    }

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
            if cursor_visible && !menu_bar.context_menu.is_open() && !editor.is_preview() {
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

    (menu_event, tab_edges)
}

