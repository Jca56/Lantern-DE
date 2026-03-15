use lntrn_render::Rect;
use lntrn_ui::gpu::{
    Button, ButtonVariant, FontSize, FoxPalette, InteractionContext, TextLabel, TitleBar,
};

use crate::editor::{self, Editor};
use crate::{Gpu, ZONE_CLOSE, ZONE_EDITOR, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_NEW, ZONE_OPEN, ZONE_SAVE};

pub const TITLE_BAR_H: f32 = 52.0;
const TOOLBAR_H: f32 = 48.0;
const BTN_W: f32 = 80.0;
const BTN_H: f32 = 36.0;
const BTN_GAP: f32 = 8.0;

pub fn editor_rect(wf: f32, hf: f32, s: f32) -> Rect {
    let top = (TITLE_BAR_H + TOOLBAR_H) * s;
    Rect::new(0.0, top, wf, hf - top)
}

pub fn render_frame(
    gpu: &mut Gpu,
    editor: &mut Editor,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    scale: f32,
) {
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
    let tb = TitleBar::new(title_rect);
    let close_state = input.add_zone(ZONE_CLOSE, tb.close_button_rect());
    let max_state = input.add_zone(ZONE_MAXIMIZE, tb.maximize_button_rect());
    let min_state = input.add_zone(ZONE_MINIMIZE, tb.minimize_button_rect());

    TitleBar::new(title_rect)
        .scale(s)
        .close_hovered(close_state.is_hovered())
        .maximize_hovered(max_state.is_hovered())
        .minimize_hovered(min_state.is_hovered())
        .draw(painter, pal);

    // ── Toolbar ───────────────────────────────────────────────────────
    let toolbar_y = TITLE_BAR_H * s;
    let toolbar_rect = Rect::new(0.0, toolbar_y, wf, TOOLBAR_H * s);
    painter.rect_filled(toolbar_rect, 0.0, pal.surface);

    // Separator line
    painter.rect_filled(
        Rect::new(0.0, toolbar_y + TOOLBAR_H * s - 1.0, wf, 1.0),
        0.0,
        pal.muted.with_alpha(0.2),
    );

    let btn_y = toolbar_y + (TOOLBAR_H * s - BTN_H * s) * 0.5;
    let btn_x_start = 12.0 * s;

    // New button
    let new_rect = Rect::new(btn_x_start, btn_y, BTN_W * s, BTN_H * s);
    let new_state = input.add_zone(ZONE_NEW, new_rect);
    Button::new(new_rect, "New")
        .variant(ButtonVariant::Ghost)
        .hovered(new_state.is_hovered())
        .pressed(new_state.is_active())
        .draw(painter, text, pal, w, h);

    // Open button
    let open_rect = Rect::new(
        btn_x_start + (BTN_W + BTN_GAP) * s,
        btn_y,
        BTN_W * s,
        BTN_H * s,
    );
    let open_state = input.add_zone(ZONE_OPEN, open_rect);
    Button::new(open_rect, "Open")
        .variant(ButtonVariant::Ghost)
        .hovered(open_state.is_hovered())
        .pressed(open_state.is_active())
        .draw(painter, text, pal, w, h);

    // Save button
    let save_rect = Rect::new(
        btn_x_start + 2.0 * (BTN_W + BTN_GAP) * s,
        btn_y,
        BTN_W * s,
        BTN_H * s,
    );
    let save_state = input.add_zone(ZONE_SAVE, save_rect);
    Button::new(save_rect, "Save")
        .variant(ButtonVariant::Primary)
        .hovered(save_state.is_hovered())
        .pressed(save_state.is_active())
        .draw(painter, text, pal, w, h);

    // ── Editor area ───────────────────────────────────────────────────
    let er = editor_rect(wf, hf, s);
    input.add_zone(ZONE_EDITOR, er);

    // Editor background (slightly different from window bg)
    painter.rect_filled(er, 0.0, pal.surface);

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

    let line_num_w = 50.0 * s; // Width for line numbers
    let line_num_font = FontSize::Custom(font_size * 0.85);
    let content_x = text_x + line_num_w;
    let content_max_w = (max_text_w - line_num_w).max(10.0);

    for i in first_visible..last_visible {
        let y = text_y_start + i as f32 * line_h;

        // Skip lines fully outside the editor rect
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

    // ── Submit frame ──────────────────────────────────────────────────
    match ctx.begin_frame("lntrn-notepad") {
        Ok(mut frame) => {
            // Pass 1: background, UI chrome
            painter.render_into(ctx, &mut frame, pal.bg);

            // Pass 2: text
            let view = frame.view().clone();
            text.render_queued(ctx, frame.encoder_mut(), &view);

            // Pass 3: cursor overlay (on top of text)
            let cursor_y = text_y_start + editor.cursor_line as f32 * line_h;
            let char_count = editor.lines[editor.cursor_line][..editor.cursor_col]
                .chars()
                .count();
            let char_w = font_size * 0.52;
            let cursor_x = content_x + char_count as f32 * char_w;

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

            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[lntrn-notepad] render error: {e}"),
    }
}
