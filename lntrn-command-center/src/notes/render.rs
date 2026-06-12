//! Render the Quick Notes overlay page.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{
    action_buttons, body_inner_height, body_line_height, body_max_scroll, body_rect,
    editor_rect, filter_rect, list_max_scroll, list_rect, list_row_rect, new_btn_rect,
    title_field_rect, NoteFocus, NotesState,
};

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const FLASH_MS: u128 = 2000;

fn accent(a: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(a)
}
fn white(a: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(a)
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &NotesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let panel_bottom = panel.y + panel.h;
    draw_toolbar(painter, text, state, panel, top_y, scale, text_size, alpha, surface_w, surface_h);
    draw_action_row(painter, text, state, panel, top_y, scale, text_size, alpha, surface_w, surface_h);
    draw_list(painter, text, state, panel, top_y, scale, text_size, alpha, panel_bottom, surface_w, surface_h);
    draw_editor(painter, text, state, panel, top_y, scale, text_size, alpha, panel_bottom, surface_w, surface_h);
    if state.confirm_delete {
        draw_confirm_delete(painter, text, panel, scale, text_size, alpha, surface_w, surface_h);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_toolbar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &NotesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    // Filter input.
    let bar = filter_rect(panel, top_y, scale);
    let radius = 12.0 * scale;
    painter.rect_filled(bar, radius, white(0.06 * alpha));
    let focused_filter = state.focus == NoteFocus::Filter;
    if focused_filter || state.filtered() {
        painter.rect_stroke_sdf(bar, radius, 1.4 * scale, accent(0.55 * alpha));
    }

    let glyph_pad = 14.0 * scale;
    let glyph_r = (bar.h * 0.18).min(10.0 * scale);
    let cx = bar.x + glyph_pad + glyph_r;
    let cy = bar.y + bar.h / 2.0;
    let stroke = 1.8 * scale;
    let glyph_color = white(0.65 * alpha);
    painter.rect_stroke_sdf(
        Rect::new(cx - glyph_r, cy - glyph_r, glyph_r * 2.0, glyph_r * 2.0),
        glyph_r,
        stroke,
        glyph_color,
    );
    painter.line_round(
        cx + glyph_r * 0.55,
        cy + glyph_r * 0.55,
        cx + glyph_r * 1.2,
        cy + glyph_r * 1.2,
        stroke * 1.4,
        glyph_color,
    );

    let pad_left = glyph_pad + glyph_r * 2.0 + 14.0 * scale;
    let font = (text_size * scale).max(14.0);
    let text_top = bar.y + (bar.h - font) / 2.0;

    let q = state.filter.query();
    let (display, is_placeholder) = if q.is_empty() {
        ("Search notes…".to_string(), true)
    } else {
        (q.to_string(), false)
    };
    let color = if is_placeholder { white(0.40 * alpha) } else { white(0.95 * alpha) };
    let text_x = bar.x + pad_left;
    let text_max_w = (bar.w - pad_left - 14.0 * scale).max(0.0);
    // Selection highlight under the filter text.
    if let Some((sel_s, sel_e)) = state.filter.selection_range() {
        let q = state.filter.query();
        let x0 = text_x + text.measure_width(&q[..sel_s], font);
        let x1 = text_x + text.measure_width(&q[..sel_e], font);
        let sel_h = font + 4.0 * scale;
        let sel_y = bar.y + (bar.h - sel_h) / 2.0;
        painter.rect_filled(
            Rect::new(x0, sel_y, (x1 - x0).max(2.0 * scale), sel_h),
            2.0 * scale,
            accent(0.30 * alpha),
        );
    }
    text.queue(&display, font, text_x, text_top, color, text_max_w, surface_w, surface_h);
    if focused_filter && state.filter.cursor_visible() {
        let caret_x = if is_placeholder { text_x } else { text_x + text.measure_width(q, font) + 2.0 * scale };
        let caret_y = bar.y + bar.h * 0.20;
        let caret_h = bar.h * 0.60;
        painter.rect_filled(
            Rect::new(caret_x, caret_y, 2.0 * scale, caret_h),
            1.0 * scale,
            accent(alpha),
        );
    }

    // + New button.
    let btn = new_btn_rect(panel, top_y, scale);
    painter.rect_filled(btn, radius, accent(0.85 * alpha));
    let label = "+ New";
    let lf = (text_size * scale * 0.88).max(14.0);
    let lw = text.measure_width(label, lf);
    let ly = btn.y + (btn.h - lf) / 2.0;
    text.queue(
        label,
        lf,
        btn.x + (btn.w - lw) / 2.0,
        ly,
        Color::from_rgb8(0, 0, 0).with_alpha(0.85 * alpha),
        btn.w,
        surface_w,
        surface_h,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_action_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &NotesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let (pin, stick, export, delete) = action_buttons(panel, top_y, scale);
    let has_selection = state.selected_id.is_some();
    let is_pinned = state
        .selected_index()
        .map(|i| state.notes[i].pinned)
        .unwrap_or(false);
    let is_sticky = state
        .selected_index()
        .map(|i| state.notes[i].sticky)
        .unwrap_or(false);
    draw_action_btn(
        painter, text, pin, scale, text_size, alpha,
        if is_pinned { "Unpin" } else { "Pin" },
        is_pinned, has_selection, surface_w, surface_h,
    );
    draw_action_btn(
        painter, text, stick, scale, text_size, alpha,
        if is_sticky { "Unstick" } else { "Stick" },
        is_sticky, has_selection, surface_w, surface_h,
    );
    draw_action_btn(
        painter, text, export, scale, text_size, alpha,
        "Export", false, has_selection, surface_w, surface_h,
    );
    draw_action_btn_destructive(
        painter, text, delete, scale, text_size, alpha,
        "Delete", has_selection, surface_w, surface_h,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_list(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &NotesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    panel_bottom: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let list = list_rect(panel, top_y, scale, panel_bottom);
    if list.h <= 0.0 {
        return;
    }
    painter.push_clip(list);
    text.push_clip([list.x, list.y, list.w, list.h]);

    let visible = state.visible_indices();
    if visible.is_empty() {
        let msg = if state.notes.is_empty() {
            "No notes yet — tap “+ New”."
        } else {
            "No matches."
        };
        let font = (text_size * scale * 0.92).max(14.0);
        let ty = list.y + 36.0 * scale;
        text.queue(msg, font, list.x + 12.0 * scale, ty, white(0.55 * alpha), list.w - 24.0 * scale, surface_w, surface_h);
    }

    let scroll_px = state.list_scroll * scale;
    for (vis_idx, &note_idx) in visible.iter().enumerate() {
        let mut row = list_row_rect(list, scale, vis_idx);
        row.y -= scroll_px;
        if row.y + row.h < list.y || row.y > list.y + list.h {
            continue;
        }
        let note = &state.notes[note_idx];
        let selected = state.selected_id == Some(note.id);
        let hovered = state.hover_idx == Some(vis_idx);
        let radius = 12.0 * scale;
        let plate_a = if selected {
            0.18
        } else if hovered {
            0.10
        } else if note.pinned {
            0.06
        } else {
            0.04
        };
        let plate_color = if selected {
            accent(plate_a * alpha)
        } else {
            white(plate_a * alpha)
        };
        painter.rect_filled(row, radius, plate_color);
        if selected {
            painter.rect_stroke_sdf(row, radius, 1.4 * scale, accent(0.6 * alpha));
        }

        let pad = 14.0 * scale;
        let mut text_x = row.x + pad;
        // Pin badge to the left of the title if pinned.
        if note.pinned {
            let star_cx = row.x + pad + 6.0 * scale;
            let star_cy = row.y + row.h * 0.32;
            draw_star(painter, star_cx, star_cy, 5.0 * scale, scale, alpha, true);
            text_x += 18.0 * scale;
        }

        let title_text = if note.title.trim().is_empty() {
            "Untitled".to_string()
        } else {
            note.title.clone()
        };
        let title_color = if note.title.trim().is_empty() {
            white(0.55 * alpha)
        } else {
            white(0.95 * alpha)
        };
        let title_font = (text_size * scale * 0.95).max(15.0);
        let title_y = row.y + row.h * 0.18;
        let title_max_w = (row.x + row.w - pad - text_x).max(0.0);
        text.queue(&title_text, title_font, text_x, title_y, title_color, title_max_w, surface_w, surface_h);

        let preview = first_line(&note.body);
        let preview_font = (text_size * scale * 0.72).max(12.0);
        let preview_y = row.y + row.h * 0.56;
        text.queue(
            &preview,
            preview_font,
            text_x,
            preview_y,
            white(0.50 * alpha),
            title_max_w,
            surface_w,
            surface_h,
        );

        // Created-on badge on the right.
        let ago_text = created_label(note.created_ms);
        let af = (text_size * scale * 0.65).max(11.0);
        let aw = text.measure_width(&ago_text, af);
        let ax = row.x + row.w - pad - aw;
        let ay = row.y + row.h * 0.18;
        text.queue(&ago_text, af, ax, ay, white(0.45 * alpha), aw + 4.0 * scale, surface_w, surface_h);
    }

    painter.pop_clip();
    text.pop_clip();

    // Scrollbar.
    let max = list_max_scroll(visible.len(), list.h, scale);
    if max > 0.0 {
        let track_w = 4.0 * scale;
        let track_x = list.x + list.w - track_w - 2.0 * scale;
        let track_y = list.y;
        let track_h = list.h;
        painter.rect_filled(Rect::new(track_x, track_y, track_w, track_h), track_w / 2.0, white(0.06 * alpha));
        let thumb_h = (track_h * track_h / (track_h + max * scale)).max(20.0 * scale);
        let thumb_y = track_y + (track_h - thumb_h) * (scroll_px / (max * scale)).clamp(0.0, 1.0);
        painter.rect_filled(Rect::new(track_x, thumb_y, track_w, thumb_h), track_w / 2.0, white(0.30 * alpha));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_editor(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &NotesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    panel_bottom: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let editor = editor_rect(panel, top_y, scale, panel_bottom);
    if editor.h <= 0.0 {
        return;
    }

    // No selection → empty state.
    if state.selected_id.is_none() {
        let msg = "Select a note on the left, or create one with “+ New”.";
        let font = (text_size * scale * 0.95).max(15.0);
        let ty = editor.y + editor.h / 2.0 - font / 2.0;
        text.queue(msg, font, editor.x + 12.0 * scale, ty, white(0.55 * alpha), editor.w - 24.0 * scale, surface_w, surface_h);
        return;
    }

    // Title input.
    let title = title_field_rect(editor, scale);
    let title_radius = 10.0 * scale;
    painter.rect_filled(title, title_radius, white(0.05 * alpha));
    let title_focused = state.focus == NoteFocus::Title;
    if title_focused {
        painter.rect_stroke_sdf(title, title_radius, 1.4 * scale, accent(0.65 * alpha));
    }
    let title_pad = 12.0 * scale;
    let title_font = (text_size * scale).max(15.0);
    let title_text_top = title.y + (title.h - title_font) / 2.0;
    let title_q = state.title.query();
    let (title_disp, title_placeholder) = if title_q.is_empty() {
        ("Title…".to_string(), true)
    } else {
        (title_q.to_string(), false)
    };
    let title_color = if title_placeholder { white(0.40 * alpha) } else { white(0.95 * alpha) };
    // Selection highlight under the title text.
    if let Some((sel_s, sel_e)) = state.title.selection_range() {
        let q = state.title.query();
        let x0 = title.x + title_pad + text.measure_width(&q[..sel_s], title_font);
        let x1 = title.x + title_pad + text.measure_width(&q[..sel_e], title_font);
        let sel_h = title_font + 4.0 * scale;
        let sel_y = title.y + (title.h - sel_h) / 2.0;
        painter.rect_filled(
            Rect::new(x0, sel_y, (x1 - x0).max(2.0 * scale), sel_h),
            2.0 * scale,
            accent(0.30 * alpha),
        );
    }
    text.queue(
        &title_disp,
        title_font,
        title.x + title_pad,
        title_text_top,
        title_color,
        title.w - title_pad * 2.0,
        surface_w,
        surface_h,
    );
    if title_focused && state.title.cursor_visible() {
        let caret_x = if title_placeholder {
            title.x + title_pad
        } else {
            title.x + title_pad + text.measure_width(title_q, title_font) + 2.0 * scale
        };
        let caret_y = title.y + title.h * 0.20;
        let caret_h = title.h * 0.60;
        painter.rect_filled(
            Rect::new(caret_x, caret_y, 2.0 * scale, caret_h),
            1.0 * scale,
            accent(alpha),
        );
    }

    // Body editor.
    let body = body_rect(editor, scale);
    let body_radius = 10.0 * scale;
    painter.rect_filled(body, body_radius, white(0.04 * alpha));
    let body_focused = state.focus == NoteFocus::Body;
    if body_focused {
        painter.rect_stroke_sdf(body, body_radius, 1.4 * scale, accent(0.55 * alpha));
    }
    draw_body_text(painter, text, state, body, scale, text_size, alpha, body_focused, surface_w, surface_h);

    // Flash toast (e.g. export confirmation) — drawn over the body area.
    if let Some((msg, t)) = &state.flash_text {
        let elapsed = t.elapsed().as_millis();
        if elapsed < FLASH_MS {
            let fade = 1.0 - (elapsed as f32 / FLASH_MS as f32);
            let font = (text_size * scale * 0.85).max(13.0);
            let tw = text.measure_width(msg, font);
            let pad = 12.0 * scale;
            let tbox = Rect::new(
                body.x + (body.w - tw - pad * 2.0) / 2.0,
                body.y + body.h - font - pad * 3.0,
                tw + pad * 2.0,
                font + pad,
            );
            painter.rect_filled(tbox, 10.0 * scale, accent(0.85 * fade * alpha));
            text.queue(
                msg,
                font,
                tbox.x + pad,
                tbox.y + (tbox.h - font) / 2.0,
                Color::from_rgb8(0, 0, 0).with_alpha(0.92 * fade * alpha),
                tw + 4.0 * scale,
                surface_w,
                surface_h,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_body_text(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &NotesState,
    body: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    focused: bool,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = 12.0 * scale;
    let line_h = body_line_height(text_size, scale);
    // Body gets a ~12% bump over the global text_size so prose feels
    // comfortable to read inside the editor without inflating every
    // other label on the page. Must match `body_line_height`'s formula.
    let font = (text_size * scale * 1.12).max(16.0);
    let inner_w = (body.w - pad * 2.0).max(0.0);
    let inner_h = body_inner_height(body, scale);
    let inner = Rect::new(body.x + pad, body.y + pad, inner_w, inner_h);
    if inner.w <= 0.0 || inner.h <= 0.0 {
        return;
    }
    painter.push_clip(inner);
    text.push_clip([inner.x, inner.y, inner.w, inner.h]);

    let buf = state.body.raw();
    let is_placeholder = buf.is_empty();
    let scroll_px = state.body_scroll;

    if is_placeholder {
        text.queue(
            "Start typing…",
            font,
            inner.x,
            inner.y,
            white(0.35 * alpha),
            inner.w,
            surface_w,
            surface_h,
        );
    } else {
        let first_visible = (scroll_px / line_h).floor() as i32;
        let last_visible = ((scroll_px + inner.h) / line_h).ceil() as i32;

        // Selection highlight rectangles — painted under the text.
        if let Some((sel_s, sel_e)) = state.body.selection_range() {
            let mut line_start = 0usize;
            for (i, line) in buf.split('\n').enumerate() {
                let line_end = line_start + line.len();
                let li = i as i32;
                if li >= first_visible && li <= last_visible {
                    // Intersect [sel_s, sel_e] with [line_start, line_end].
                    let s = sel_s.max(line_start);
                    let e = sel_e.min(line_end);
                    if s < e {
                        let x0 = inner.x
                            + text.measure_width(&buf[line_start..s], font);
                        let x1 = inner.x
                            + text.measure_width(&buf[line_start..e], font);
                        let y = inner.y + li as f32 * line_h - scroll_px;
                        painter.rect_filled(
                            Rect::new(x0, y, (x1 - x0).max(2.0 * scale), line_h),
                            2.0 * scale,
                            accent(0.30 * alpha),
                        );
                    } else if sel_s <= line_end && sel_e > line_end {
                        // Selection wraps past end-of-line — paint a thin
                        // trailing chunk so the user sees the newline is
                        // selected too.
                        let x0 = inner.x
                            + text.measure_width(&buf[line_start..line_end], font);
                        let y = inner.y + li as f32 * line_h - scroll_px;
                        painter.rect_filled(
                            Rect::new(x0, y, 6.0 * scale, line_h),
                            2.0 * scale,
                            accent(0.20 * alpha),
                        );
                    }
                }
                // Next line starts past the '\n' (or at buf end on last line).
                line_start = line_end + 1;
            }
        }

        for (i, line) in buf.split('\n').enumerate() {
            let line_idx = i as i32;
            if line_idx < first_visible {
                continue;
            }
            if line_idx > last_visible {
                break;
            }
            let y = inner.y + line_idx as f32 * line_h - scroll_px;
            text.queue(line, font, inner.x, y, white(0.95 * alpha), inner.w, surface_w, surface_h);
        }
    }

    if focused && state.body.cursor_visible() {
        let cursor = state.body.cursor_byte();
        let pre = &buf[..cursor];
        let line_idx = pre.matches('\n').count();
        let line_start = pre.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col_text = &buf[line_start..cursor];
        let caret_x_in_line = text.measure_width(col_text, font);
        let caret_x = inner.x + caret_x_in_line;
        let caret_y = inner.y + line_idx as f32 * line_h - scroll_px;
        if caret_y >= inner.y - line_h && caret_y <= inner.y + inner.h {
            painter.rect_filled(
                Rect::new(caret_x, caret_y, 2.0 * scale, font),
                1.0 * scale,
                accent(alpha),
            );
        }
    }

    painter.pop_clip();
    text.pop_clip();

    // Body scrollbar.
    let line_count = buf.split('\n').count();
    let max = body_max_scroll(line_count, body, scale, text_size);
    if max > 0.0 {
        let track_w = 4.0 * scale;
        let track_x = body.x + body.w - track_w - 4.0 * scale;
        let track_y = inner.y;
        let track_h = inner.h;
        painter.rect_filled(
            Rect::new(track_x, track_y, track_w, track_h),
            track_w / 2.0,
            white(0.06 * alpha),
        );
        let thumb_h = (track_h * track_h / (track_h + max)).max(20.0 * scale);
        let thumb_y = track_y + (track_h - thumb_h) * (scroll_px / max).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::new(track_x, thumb_y, track_w, thumb_h),
            track_w / 2.0,
            white(0.30 * alpha),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_action_btn(
    painter: &mut Painter,
    text: &mut TextRenderer,
    r: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    label: &str,
    active: bool,
    enabled: bool,
    surface_w: u32,
    surface_h: u32,
) {
    let eff_alpha = if enabled { alpha } else { 0.45 * alpha };
    let radius = 10.0 * scale;
    let plate_a = if active { 0.18 } else { 0.08 };
    let plate_color = if active { accent(plate_a * eff_alpha) } else { white(plate_a * eff_alpha) };
    painter.rect_filled(r, radius, plate_color);
    if active {
        painter.rect_stroke_sdf(r, radius, 1.0 * scale, accent(0.55 * eff_alpha));
    } else {
        painter.rect_stroke_sdf(r, radius, 1.0 * scale, white(0.18 * eff_alpha));
    }
    let font = (text_size * scale * 0.78).max(13.0);
    let lw = text.measure_width(label, font);
    let lc = if active { accent(0.95 * eff_alpha) } else { white(0.85 * eff_alpha) };
    let ty = r.y + (r.h - font) / 2.0;
    text.queue(label, font, r.x + (r.w - lw) / 2.0, ty, lc, r.w, surface_w, surface_h);
}

#[allow(clippy::too_many_arguments)]
fn draw_action_btn_destructive(
    painter: &mut Painter,
    text: &mut TextRenderer,
    r: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    label: &str,
    enabled: bool,
    surface_w: u32,
    surface_h: u32,
) {
    let eff_alpha = if enabled { alpha } else { 0.40 * alpha };
    let radius = 10.0 * scale;
    let red = Color::from_rgb8(0xd0, 0x4a, 0x4a);
    painter.rect_filled(r, radius, red.with_alpha(0.18 * eff_alpha));
    painter.rect_stroke_sdf(r, radius, 1.0 * scale, red.with_alpha(0.65 * eff_alpha));
    let font = (text_size * scale * 0.78).max(13.0);
    let lw = text.measure_width(label, font);
    let ty = r.y + (r.h - font) / 2.0;
    text.queue(
        label,
        font,
        r.x + (r.w - lw) / 2.0,
        ty,
        red.with_alpha(0.95 * eff_alpha),
        r.w,
        surface_w,
        surface_h,
    );
}

fn draw_star(painter: &mut Painter, cx: f32, cy: f32, radius: f32, scale: f32, alpha: f32, active: bool) {
    let stroke = 1.4 * scale;
    let color = if active { accent(0.95 * alpha) } else { white(0.40 * alpha) };
    let mut pts: [(f32, f32); 10] = [(0.0, 0.0); 10];
    for i in 0..10 {
        let theta = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let rr = if i % 2 == 0 { radius } else { radius * 0.42 };
        pts[i] = (cx + theta.cos() * rr, cy + theta.sin() * rr);
    }
    for i in 0..10 {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % 10];
        painter.line_round(x1, y1, x2, y2, stroke, color);
    }
}

fn first_line(s: &str) -> String {
    let line = s.split('\n').next().unwrap_or("").trim();
    if line.is_empty() {
        "(empty)".to_string()
    } else if line.chars().count() > 80 {
        let mut out: String = line.chars().take(80).collect();
        out.push('…');
        out
    } else {
        line.to_string()
    }
}

/// Short human-readable date for the created badge in the list row,
/// e.g. "Today", "Yesterday", "Mar 12", "12/05/24".
fn created_label(timestamp_ms: u128) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if timestamp_ms == 0 {
        return String::new();
    }
    // Convert to NaiveDateTime in local time via chrono.
    let secs = (timestamp_ms / 1000) as i64;
    let created = chrono::DateTime::<chrono::Local>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64),
    );
    let now = chrono::DateTime::<chrono::Local>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(now_ms as u64),
    );
    let created_date = created.date_naive();
    let today = now.date_naive();
    let diff = (today - created_date).num_days();
    if diff == 0 {
        format!("Today {}", created.format("%H:%M"))
    } else if diff == 1 {
        format!("Yesterday {}", created.format("%H:%M"))
    } else if diff < 7 {
        created.format("%a %H:%M").to_string()
    } else if diff < 365 {
        created.format("%b %-d").to_string()
    } else {
        created.format("%Y-%m-%d").to_string()
    }
}

#[allow(dead_code)]
fn ago(timestamp_ms: u128) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dt = now_ms.saturating_sub(timestamp_ms);
    let secs = dt / 1000;
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_confirm_delete(
    painter: &mut Painter,
    text: &mut TextRenderer,
    panel: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    painter.rect_filled(panel, 0.0, Color::from_rgb8(0, 0, 0).with_alpha(0.45 * alpha));
    let w = 380.0 * scale;
    let h = 180.0 * scale;
    let x = panel.x + (panel.w - w) / 2.0;
    let y = panel.y + (panel.h - h) / 2.0;
    let r = Rect::new(x, y, w, h);
    painter.rect_filled(r, 16.0 * scale, Color::from_rgb8(40, 40, 44).with_alpha(0.97 * alpha));
    painter.rect_stroke_sdf(r, 16.0 * scale, 1.2 * scale, white(0.18 * alpha));

    let font_title = (text_size * scale).max(16.0);
    let font_body = (text_size * scale * 0.85).max(13.0);
    let pad = 18.0 * scale;
    let title = "Delete this note?";
    let body = "This permanently removes the note file from disk.";
    let title_y = y + pad;
    text.queue(title, font_title, x + pad, title_y, white(0.95 * alpha), w - pad * 2.0, surface_w, surface_h);
    text.queue(body, font_body, x + pad, title_y + font_title * 1.5, white(0.65 * alpha), w - pad * 2.0, surface_w, surface_h);

    let btn_h = 38.0 * scale;
    let btn_w = (w - pad * 2.0 - 12.0 * scale) / 2.0;
    let by = y + h - pad - btn_h;
    let cancel = Rect::new(x + pad, by, btn_w, btn_h);
    let confirm = Rect::new(x + pad + btn_w + 12.0 * scale, by, btn_w, btn_h);
    painter.rect_filled(cancel, 10.0 * scale, white(0.10 * alpha));
    let red = Color::from_rgb8(0xd0, 0x4a, 0x4a);
    painter.rect_filled(confirm, 10.0 * scale, red.with_alpha(0.85 * alpha));
    let fb = font_body;
    let cl_text = "Esc — Cancel";
    let yes_text = "Enter — Delete";
    let cl_w = text.measure_width(cl_text, fb);
    let yes_w = text.measure_width(yes_text, fb);
    let btn_text_y = by + (btn_h - fb) / 2.0;
    text.queue(cl_text, fb, cancel.x + (cancel.w - cl_w) / 2.0, btn_text_y, white(0.85 * alpha), btn_w, surface_w, surface_h);
    text.queue(yes_text, fb, confirm.x + (confirm.w - yes_w) / 2.0, btn_text_y, white(0.95 * alpha), btn_w, surface_w, surface_h);
}
