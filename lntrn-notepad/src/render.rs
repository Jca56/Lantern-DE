use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{
    FontSize, FoxPalette, InteractionContext, MenuBar, MenuEvent, MenuItem, TextLabel, TitleBar,
};

use crate::editor::{self, Editor};
use crate::{Gpu, MENU_NEW, MENU_OPEN, MENU_SAVE, ZONE_CLOSE, ZONE_EDITOR, ZONE_MAXIMIZE, ZONE_MINIMIZE};

pub const TITLE_BAR_H: f32 = 52.0;

pub fn editor_rect(wf: f32, hf: f32, s: f32) -> Rect {
    let top = TITLE_BAR_H * s;
    Rect::new(0.0, top, wf, hf - top)
}

pub fn file_menu_items() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        ("File", vec![
            MenuItem::action_with(MENU_NEW, "New", "Ctrl+N"),
            MenuItem::action_with(MENU_OPEN, "Open", "Ctrl+O"),
            MenuItem::action_with(MENU_SAVE, "Save", "Ctrl+S"),
        ]),
    ]
}

pub fn render_frame(
    gpu: &mut Gpu,
    editor: &mut Editor,
    input: &mut InteractionContext,
    menu_bar: &mut MenuBar,
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

    // ── Editor area ───────────────────────────────────────────────────
    let er = editor_rect(wf, hf, s);
    input.add_zone(ZONE_EDITOR, er);

    // Editor background (lightened for readability)
    let editor_bg = Color::from_rgb8(70, 70, 74);
    painter.rect_filled(er, 0.0, editor_bg);

    let font_size = editor::FONT_SIZE * s;
    let line_h = editor::FONT_SIZE * editor::LINE_HEIGHT * s;
    let pad = editor::PAD * s;
    let text_x = er.x + pad;
    let text_y_start = er.y + pad - editor.scroll_offset;
    let max_text_w = (er.w - pad * 2.0).max(10.0);

    // Draw line numbers and text lines
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
            if y + line_h < er.y || y > er.y + er.h { continue; }

            // Determine selection span on this line
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

            let x1 = content_x + text.measure_width(
                &editor.lines[i][..sel_begin], font_size,
            );
            let x2 = content_x + text.measure_width(
                &editor.lines[i][..sel_finish], font_size,
            );
            // For lines that are fully selected, extend highlight to show newline
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

        // Line content
        if !editor.lines[i].is_empty() {
            text.queue(
                &editor.lines[i],
                font_size,
                content_x,
                y,
                pal.text,
                content_max_w,
                w,
                h,
            );
        }
    }

    // ── Status bar ────────────────────────────────────────────────────
    let status_h = 28.0 * s;
    let status_y = hf - status_h;
    painter.rect_filled(
        Rect::new(0.0, status_y, wf, status_h),
        0.0,
        pal.surface_2,
    );

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
            // Pass 1: background, UI chrome, context menu shapes
            painter.render_into(ctx, &mut frame, pal.bg);

            // Pass 2: text (labels + editor + context menu text)
            let view = frame.view().clone();
            text.render_queued(ctx, frame.encoder_mut(), &view);

            // Pass 3: cursor overlay (on top of text)
            if cursor_visible {
                let cursor_y = text_y_start + editor.cursor_line as f32 * line_h;
                let before_cursor = &editor.lines[editor.cursor_line][..editor.cursor_col];
                let cursor_x = content_x + text.measure_width(before_cursor, font_size);

                if cursor_y + line_h > er.y && cursor_y < er.y + er.h {
                    painter.clear();
                    painter.rect_filled(
                        Rect::new(cursor_x, cursor_y, 2.5 * s, font_size + 2.0),
                        0.0,
                        pal.accent,
                    );
                    painter.render_pass_overlay(
                        ctx,
                        frame.encoder_mut(),
                        &view,
                    );
                }
            }

            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[lntrn-notepad] render error: {e}"),
    }

    menu_event
}
