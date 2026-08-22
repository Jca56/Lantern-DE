//! Render the Clipboard History overlay page.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{
    clear_btn_rect, filter_bar_rect, image_row_actions, list_rect, max_scroll, row_rect_at,
    text_row_actions, ClipboardState, Entry, PAD,
};
use crate::render::IconRequest;

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const FLASH_MS: u128 = 320;

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
    icons: &mut Vec<IconRequest>,
    state: &ClipboardState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let panel_bottom = panel.y + panel.h;
    draw_filter_bar(
        painter, text, state, panel, top_y, scale, text_size, alpha, surface_w, surface_h,
    );
    draw_clear_btn(
        painter, text, state, panel, top_y, scale, text_size, alpha, surface_w, surface_h,
    );
    draw_list(
        painter,
        text,
        icons,
        state,
        panel,
        top_y,
        scale,
        text_size,
        alpha,
        panel_bottom,
        surface_w,
        surface_h,
    );
    if state.confirm_clear {
        draw_confirm_clear(
            painter, text, panel, scale, text_size, alpha, surface_w, surface_h,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_filter_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &ClipboardState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let bar = filter_bar_rect(panel, top_y, scale);
    let radius = 12.0 * scale;
    painter.rect_filled(bar, radius, white(0.06 * alpha));
    if state.filtered() {
        painter.rect_stroke_sdf(bar, radius, 1.4 * scale, accent(0.55 * alpha));
    }

    // Magnifier glyph on the left.
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
    // `text.queue` treats `y` as the TOP of the glyph row, not the baseline.
    let text_top = bar.y + (bar.h - font) / 2.0;

    let q = state.filter.query();
    let (display, is_placeholder) = if q.is_empty() {
        ("Search clipboard…".to_string(), true)
    } else {
        (q.to_string(), false)
    };
    let color = if is_placeholder {
        white(0.40 * alpha)
    } else {
        white(0.95 * alpha)
    };
    let text_x = bar.x + pad_left;
    let text_max_w = (bar.w - pad_left - 14.0 * scale).max(0.0);
    text.queue(
        &display, font, text_x, text_top, color, text_max_w, surface_w, surface_h,
    );

    if state.filter.cursor_visible() {
        let caret_x = if is_placeholder {
            text_x
        } else {
            text_x + text.measure_width(q, font) + 2.0 * scale
        };
        let caret_y = bar.y + bar.h * 0.20;
        let caret_h = bar.h * 0.60;
        let a = if is_placeholder { 0.55 } else { 1.0 };
        painter.rect_filled(
            Rect::new(caret_x, caret_y, 2.0 * scale, caret_h),
            1.0 * scale,
            accent(a * alpha),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_clear_btn(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &ClipboardState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let r = clear_btn_rect(panel, top_y, scale);
    let radius = 12.0 * scale;
    let enabled = !state.entries.is_empty();
    let plate_a = if enabled { 0.10 } else { 0.04 };
    painter.rect_filled(r, radius, white(plate_a * alpha));
    if enabled {
        painter.rect_stroke_sdf(r, radius, 1.0 * scale, white(0.20 * alpha));
    }
    let label = "Clear All";
    let font = (text_size * scale * 0.78).max(13.0);
    let lw = text.measure_width(label, font);
    let tx = r.x + (r.w - lw) / 2.0;
    let ty = r.y + (r.h - font) / 2.0;
    let lc = if enabled {
        white(0.85 * alpha)
    } else {
        white(0.35 * alpha)
    };
    text.queue(label, font, tx, ty, lc, r.w, surface_w, surface_h);
}

#[allow(clippy::too_many_arguments)]
fn draw_list(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    state: &ClipboardState,
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
        let msg = if state.entries.is_empty() {
            "Clipboard history is empty — copy something to get started."
        } else {
            "No matches."
        };
        let font = (text_size * scale * 0.92).max(15.0);
        let baseline = list.y + 40.0 * scale + font;
        text.queue(
            msg,
            font,
            list.x + PAD * scale,
            baseline,
            white(0.55 * alpha),
            list.w - PAD * 2.0 * scale,
            surface_w,
            surface_h,
        );
    }

    let scroll_px = state.scroll * scale;

    for (vis_idx, &entry_idx) in visible.iter().enumerate() {
        let mut row = row_rect_at(list, state, scale, vis_idx);
        row.y -= scroll_px;
        if row.y + row.h < list.y || row.y > list.y + list.h {
            continue;
        }
        let entry = &state.entries[entry_idx];
        let hovered = state.hover_idx == Some(vis_idx);
        let flash = flash_factor(state.recent_copy, entry.id);
        if entry.is_image() {
            draw_image_row(
                painter, text, icons, entry, row, scale, text_size, alpha, hovered, flash,
                surface_w, surface_h,
            );
        } else {
            draw_text_row(
                painter, text, entry, row, scale, text_size, alpha, hovered, flash, surface_w,
                surface_h,
            );
        }
    }

    painter.pop_clip();
    text.pop_clip();

    // Scrollbar.
    let max = max_scroll(state, list.h, scale);
    if max > 0.0 {
        let track_w = 4.0 * scale;
        let track_x = panel.x + panel.w - PAD * scale - track_w;
        let track_y = list.y;
        let track_h = list.h;
        painter.rect_filled(
            Rect::new(track_x, track_y, track_w, track_h),
            track_w / 2.0,
            white(0.06 * alpha),
        );
        let thumb_h = (track_h * track_h / (track_h + max)).max(24.0 * scale);
        let thumb_y = track_y + (track_h - thumb_h) * (scroll_px / max).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::new(track_x, thumb_y, track_w, thumb_h),
            track_w / 2.0,
            white(0.30 * alpha),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    entry: &Entry,
    row: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    hovered: bool,
    flash: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let radius = 14.0 * scale;
    let plate_a = if flash > 0.0 {
        0.28 + 0.50 * flash
    } else if hovered {
        0.14
    } else if entry.pinned {
        0.08
    } else {
        0.05
    };
    let plate_color = if flash > 0.0 {
        accent(plate_a * alpha)
    } else {
        white(plate_a * alpha)
    };
    painter.rect_filled(row, radius, plate_color);
    if entry.pinned {
        painter.rect_stroke_sdf(row, radius, 1.0 * scale, accent(0.45 * alpha));
    }

    let pad = 16.0 * scale;
    let thumb_size = (row.h - 16.0 * scale).max(40.0 * scale);
    let thumb_x = row.x + pad;
    let thumb_y = row.y + (row.h - thumb_size) / 2.0;

    // Text glyph plate — small rounded rect with three faux text lines.
    let rect = Rect::new(thumb_x, thumb_y, thumb_size, thumb_size);
    painter.rect_filled(rect, 10.0 * scale, white(0.08 * alpha));
    let inner_pad = thumb_size * 0.18;
    let line_w = thumb_size - inner_pad * 2.0;
    let line_h = 2.0 * scale;
    for i in 0..3 {
        let y = thumb_y + inner_pad + i as f32 * (thumb_size * 0.20);
        let w = if i == 2 { line_w * 0.60 } else { line_w };
        painter.rect_filled(
            Rect::new(thumb_x + inner_pad, y, w, line_h),
            line_h / 2.0,
            white(0.45 * alpha),
        );
    }

    let (pin_rect, del_rect) = text_row_actions(row, scale);
    let preview_x = thumb_x + thumb_size + 14.0 * scale;
    let preview_w = (pin_rect.x - preview_x - 8.0 * scale).max(0.0);
    let font_preview = (text_size * scale * 0.95).max(15.0);
    let font_meta = (text_size * scale * 0.72).max(12.0);

    let preview_text = entry
        .preview
        .as_deref()
        .map(one_line)
        .unwrap_or_else(|| "(no preview)".to_string());
    let preview_y = row.y + row.h * 0.36;
    text.queue(
        &preview_text,
        font_preview,
        preview_x,
        preview_y,
        white(0.95 * alpha),
        preview_w,
        surface_w,
        surface_h,
    );

    let meta = format!("#{}  ·  {}", entry.id, ago(entry.timestamp_ms));
    let meta_y = row.y + row.h * 0.72;
    text.queue(
        &meta,
        font_meta,
        preview_x,
        meta_y,
        white(0.50 * alpha),
        preview_w,
        surface_w,
        surface_h,
    );

    draw_star(painter, pin_rect, scale, alpha, entry.pinned, hovered);
    if hovered {
        draw_x(painter, del_rect, scale, alpha);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_image_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    entry: &Entry,
    row: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    hovered: bool,
    flash: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let radius = 14.0 * scale;
    // The row plate sits *behind* the image as a checkerboard / dark
    // background. When the image is aspect-fit smaller than the row, the
    // letterboxing reads as intentional.
    let dark_plate = Color::from_rgb8(20, 20, 24).with_alpha(0.55 * alpha);
    painter.rect_filled(row, radius, dark_plate);

    let pad = 10.0 * scale;
    let inset_x = row.x + pad;
    let inset_y = row.y + pad;
    let inset_w = (row.w - pad * 2.0).max(0.0);
    let inset_h = (row.h - pad * 2.0).max(0.0);

    // Aspect-fit thumbnail. We don't know the image's pixel aspect ratio
    // from the IPC payload, so we ask the IconCache to scale to fit
    // the maximum dimension. That yields a square crop today; once we
    // pass aspect info in the IPC we can use real letterboxing.
    let max_side = inset_h.min(inset_w);
    let thumb_x = inset_x + (inset_w - max_side) / 2.0;
    let thumb_y = inset_y;
    if let Some(p) = &entry.image_path {
        icons.push(IconRequest {
            app_id: format!("clip:{}", entry.id),
            icon_name: Some(p.clone()),
            x: thumb_x,
            y: thumb_y,
            size: max_side,
            opacity: alpha,
            clip: Some([row.x, row.y, row.w, row.h]),
        });
    }

    // Hover / flash / pinned overlays — semi-transparent so the image
    // still reads through.
    if flash > 0.0 {
        painter.rect_filled(row, radius, accent(0.28 * flash * alpha));
    } else if hovered {
        painter.rect_filled(row, radius, white(0.06 * alpha));
    }
    if entry.pinned {
        painter.rect_stroke_sdf(row, radius, 1.6 * scale, accent(0.85 * alpha));
    }

    // Action chips top-right with a dark plate so they stay legible
    // against any image content.
    let (pin_rect, del_rect) = image_row_actions(row, scale);
    let chip_plate = Color::from_rgb8(0, 0, 0).with_alpha(0.45 * alpha);
    painter.rect_filled(pin_rect, pin_rect.w / 2.0, chip_plate);
    draw_star(painter, pin_rect, scale, alpha, entry.pinned, hovered);
    if hovered {
        painter.rect_filled(del_rect, del_rect.w / 2.0, chip_plate);
        draw_x(painter, del_rect, scale, alpha);
    }

    // Tiny timestamp chip bottom-right.
    let meta = format!("#{}  ·  {}", entry.id, ago(entry.timestamp_ms));
    let font_meta = (text_size * scale * 0.72).max(12.0);
    let mw = text.measure_width(&meta, font_meta);
    let chip_pad = 6.0 * scale;
    let chip_h = font_meta + chip_pad * 2.0;
    let chip_w = mw + chip_pad * 2.0;
    let chip_x = row.x + row.w - 10.0 * scale - chip_w;
    let chip_y = row.y + row.h - 10.0 * scale - chip_h;
    painter.rect_filled(
        Rect::new(chip_x, chip_y, chip_w, chip_h),
        chip_h / 2.0,
        chip_plate,
    );
    text.queue(
        &meta,
        font_meta,
        chip_x + chip_pad,
        chip_y + chip_pad,
        white(0.85 * alpha),
        chip_w,
        surface_w,
        surface_h,
    );
}

fn draw_star(
    painter: &mut Painter,
    r: Rect,
    scale: f32,
    alpha: f32,
    active: bool,
    row_hovered: bool,
) {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let radius = r.w * 0.42;
    let stroke = 1.6 * scale;
    let color = if active {
        accent(0.95 * alpha)
    } else if row_hovered {
        white(0.55 * alpha)
    } else {
        white(0.25 * alpha)
    };
    // Approximate 5-point star with line segments.
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

fn draw_x(painter: &mut Painter, r: Rect, scale: f32, alpha: f32) {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let arm = r.w * 0.32;
    let stroke = 1.8 * scale;
    let color = white(0.75 * alpha);
    painter.line_round(cx - arm, cy - arm, cx + arm, cy + arm, stroke, color);
    painter.line_round(cx + arm, cy - arm, cx - arm, cy + arm, stroke, color);
}

fn flash_factor(recent: Option<(u64, std::time::Instant)>, id: u64) -> f32 {
    let Some((rec_id, t)) = recent else {
        return 0.0;
    };
    if rec_id != id {
        return 0.0;
    }
    let ms = t.elapsed().as_millis();
    if ms >= FLASH_MS {
        return 0.0;
    }
    (1.0 - ms as f32 / FLASH_MS as f32).clamp(0.0, 1.0)
}

fn one_line(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch == '\n' || ch == '\r' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
        if out.len() > 200 {
            break;
        }
    }
    out
}

fn ago(timestamp_ms: u128) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dt = now_ms.saturating_sub(timestamp_ms);
    let secs = dt / 1000;
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_confirm_clear(
    painter: &mut Painter,
    text: &mut TextRenderer,
    panel: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    // Scrim over the page body.
    painter.rect_filled(
        panel,
        0.0,
        Color::from_rgb8(0, 0, 0).with_alpha(0.45 * alpha),
    );
    let w = 360.0 * scale;
    let h = 170.0 * scale;
    let x = panel.x + (panel.w - w) / 2.0;
    let y = panel.y + (panel.h - h) / 2.0;
    let r = Rect::new(x, y, w, h);
    painter.rect_filled(
        r,
        16.0 * scale,
        Color::from_rgb8(40, 40, 44).with_alpha(0.97 * alpha),
    );
    painter.rect_stroke_sdf(r, 16.0 * scale, 1.2 * scale, white(0.18 * alpha));

    let font_title = (text_size * scale).max(16.0);
    let font_body = (text_size * scale * 0.85).max(13.0);
    let title = "Clear all history?";
    let body = "This removes every clipboard entry. Pinned items are kept.";
    let pad = 18.0 * scale;
    let title_y = y + pad;
    text.queue(
        title,
        font_title,
        x + pad,
        title_y,
        white(0.95 * alpha),
        w - pad * 2.0,
        surface_w,
        surface_h,
    );
    text.queue(
        body,
        font_body,
        x + pad,
        title_y + font_title * 1.5,
        white(0.65 * alpha),
        w - pad * 2.0,
        surface_w,
        surface_h,
    );

    // Buttons (rendered without their own hit-tests for v1 — Esc cancels, Enter confirms).
    let btn_h = 36.0 * scale;
    let btn_w = (w - pad * 2.0 - 12.0 * scale) / 2.0;
    let by = y + h - pad - btn_h;
    let cancel = Rect::new(x + pad, by, btn_w, btn_h);
    let confirm = Rect::new(x + pad + btn_w + 12.0 * scale, by, btn_w, btn_h);
    painter.rect_filled(cancel, 10.0 * scale, white(0.10 * alpha));
    painter.rect_filled(confirm, 10.0 * scale, accent(0.85 * alpha));
    let fb = font_body;
    let cl_text = "Esc — Cancel";
    let yes_text = "Enter — Clear";
    let cl_w = text.measure_width(cl_text, fb);
    let yes_w = text.measure_width(yes_text, fb);
    let btn_text_y = by + (btn_h - fb) / 2.0;
    text.queue(
        cl_text,
        fb,
        cancel.x + (cancel.w - cl_w) / 2.0,
        btn_text_y,
        white(0.85 * alpha),
        btn_w,
        surface_w,
        surface_h,
    );
    text.queue(
        yes_text,
        fb,
        confirm.x + (confirm.w - yes_w) / 2.0,
        btn_text_y,
        Color::from_rgb8(0, 0, 0).with_alpha(0.85 * alpha),
        btn_w,
        surface_w,
        surface_h,
    );
}
