//! Painter pass for the Files view.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::render::IconRequest;
use super::{
    crumb_segments, file_kind, format_meta, format_size_or_count, home_dir, list_rect,
    row_height, row_icon_size, sidebar_icon_size, sidebar_rect, sidebar_tile_rect, strip_layout,
    FileEntry, FileKind, FilesState, Location, NavButton, ROW_PAD_X,
};

const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const HOVER_PLATE_ALPHA: f32 = 0.10;

fn text_color(alpha: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(alpha)
}
fn dim_color(alpha: f32) -> Color {
    Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.55)
}
fn accent_color(alpha: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
}

/// Body content: sidebar + list. The Files controls strip is drawn
/// separately in the top-most row by [`draw_controls_strip`].
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    state: &FilesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    draw_sidebar(painter, text, icons, state, panel, top_y, scale, text_size, alpha, surface_w, surface_h);
    draw_list(painter, text, icons, state, panel, top_y, scale, text_size, alpha, surface_w, surface_h);
}

// ── Controls-row strip ─────────────────────────────────────────────────────

/// Draw the Files toolbar inside the controls row: [Back][Home] …
/// [pathbar / search] … [Magnifier][Eye]. The collapse chevron is
/// drawn separately by the controls module.
#[allow(clippy::too_many_arguments)]
pub fn draw_controls_strip(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &FilesState,
    strip: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let layout = strip_layout(strip, scale);

    painter.push_clip(strip);
    text.push_clip([strip.x, strip.y, strip.w, strip.h]);

    // Back button.
    let back_enabled = !state.history.is_empty();
    draw_btn(
        painter,
        layout.back,
        scale,
        alpha,
        state.hover_nav == Some(NavButton::Back) && back_enabled,
        false,
    );
    draw_back_glyph(painter, layout.back, scale, alpha * if back_enabled { 0.95 } else { 0.28 });
    let _ = home_dir;

    // Pathbar (breadcrumb or filter input).
    draw_pathbar(painter, text, state, layout.pathbar, scale, text_size, alpha, surface_w, surface_h);

    // Magnifier toggle.
    draw_btn(
        painter,
        layout.magnifier,
        scale,
        alpha,
        state.hover_nav == Some(NavButton::Magnifier),
        state.filter_active,
    );
    draw_magnifier_glyph(painter, layout.magnifier, scale, alpha, state.filter_active);

    // Sort.
    draw_btn(
        painter,
        layout.sort,
        scale,
        alpha,
        state.hover_nav == Some(NavButton::Sort),
        false,
    );
    draw_sort_glyph(painter, layout.sort, scale, alpha);

    // Eye toggle (show-hidden).
    draw_btn(
        painter,
        layout.eye,
        scale,
        alpha,
        state.hover_nav == Some(NavButton::ToggleHidden),
        state.show_hidden,
    );
    draw_eye_glyph(painter, layout.eye, scale, state.show_hidden, alpha);

    painter.pop_clip();
    text.pop_clip();
}

fn draw_btn(painter: &mut Painter, r: Rect, scale: f32, alpha: f32, hovered: bool, active: bool) {
    let radius = 10.0 * scale;
    let plate_alpha = if hovered { 0.22 } else if active { 0.14 } else { 0.08 };
    painter.rect_filled(r, radius, Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(plate_alpha * alpha));
    if hovered || active {
        painter.rect_stroke_sdf(r, radius, 1.4 * scale, accent_color(if active { 0.75 } else { 0.6 } * alpha));
    }
}

fn draw_back_glyph(painter: &mut Painter, r: Rect, scale: f32, alpha: f32) {
    let color = text_color(alpha);
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let stroke = 2.4 * scale;
    let arm = r.w * 0.22;
    painter.line_round(cx + arm * 0.4, cy - arm, cx - arm * 0.6, cy, stroke, color);
    painter.line_round(cx - arm * 0.6, cy, cx + arm * 0.4, cy + arm, stroke, color);
}

fn draw_sort_glyph(painter: &mut Painter, r: Rect, scale: f32, alpha: f32) {
    // Three horizontal lines, descending in length, with a small arrow
    // on the right of the top line indicating "sort direction".
    let color = text_color(0.92 * alpha);
    let stroke = 2.2 * scale;
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let widths = [r.w * 0.50, r.w * 0.34, r.w * 0.20];
    let gap = r.h * 0.16;
    for (i, w) in widths.iter().enumerate() {
        let y = cy - gap + i as f32 * gap;
        painter.line_round(cx - w / 2.0, y, cx + w / 2.0, y, stroke, color);
    }
}

fn draw_magnifier_glyph(painter: &mut Painter, r: Rect, scale: f32, alpha: f32, active: bool) {
    let color = if active { accent_color(alpha) } else { text_color(0.92 * alpha) };
    let cx = r.x + r.w * 0.43;
    let cy = r.y + r.h * 0.43;
    let radius = r.w * 0.18;
    let stroke = 2.2 * scale;
    // Circle outline (approximated by a polyline).
    let steps = 18;
    let mut prev: Option<(f32, f32)> = None;
    for i in 0..=steps {
        let theta = (i as f32 / steps as f32) * std::f32::consts::TAU;
        let x = cx + theta.cos() * radius;
        let y = cy + theta.sin() * radius;
        if let Some((px, py)) = prev {
            painter.line_round(px, py, x, y, stroke, color);
        }
        prev = Some((x, y));
    }
    // Handle.
    let hx = cx + radius * 0.78;
    let hy = cy + radius * 0.78;
    painter.line_round(hx, hy, hx + radius * 0.95, hy + radius * 0.95, stroke * 1.1, color);
}

fn draw_eye_glyph(painter: &mut Painter, r: Rect, scale: f32, on: bool, alpha: f32) {
    let color = if on { accent_color(alpha) } else { text_color(0.9 * alpha) };
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let stroke = 2.2 * scale;
    let rx = r.w * 0.30;
    let ry = r.w * 0.20;
    let steps = 18;
    for side in [-1.0_f32, 1.0_f32] {
        let mut prev: Option<(f32, f32)> = None;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let theta = -std::f32::consts::FRAC_PI_2 + t * std::f32::consts::PI;
            let x = cx + theta.cos() * rx;
            let y = cy + side * theta.sin().abs() * ry;
            if let Some((px, py)) = prev {
                painter.line_round(px, py, x, y, stroke, color);
            }
            prev = Some((x, y));
        }
    }
    painter.circle_filled(cx, cy, r.w * 0.08, color);
    if !on {
        painter.line_round(
            r.x + r.w * 0.22,
            r.y + r.h * 0.74,
            r.x + r.w * 0.78,
            r.y + r.h * 0.26,
            stroke,
            color,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_pathbar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &FilesState,
    r: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let radius = 10.0 * scale;
    let bg_alpha = if state.filter_active { 0.10 } else { 0.06 };
    painter.rect_filled(r, radius, Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(bg_alpha * alpha));
    painter.rect_stroke_sdf(
        r,
        radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff)
            .with_alpha(if state.filter_active { 0.22 } else { 0.14 } * alpha),
    );

    text.push_clip([r.x, r.y, r.w, r.h]);
    let pad = 14.0 * scale;
    let font = text_size * scale;
    let baseline_y = r.y + (r.h - font) / 2.0;

    if state.filter_active {
        let q = state.filter.query();
        let (s, color) = if q.is_empty() {
            ("Search…", dim_color(alpha))
        } else {
            (q, text_color(0.96 * alpha))
        };
        text.queue(s, font, r.x + pad, baseline_y, color, r.w - pad * 2.0, surface_w, surface_h);
        // Result count badge on the right side.
        if !q.is_empty() {
            let count = format!("{}/{}", state.visible.len(), state.entries.len());
            let cf = (text_size * 0.78) * scale;
            let cw = text.measure_width(&count, cf);
            text.queue(
                &count,
                cf,
                r.x + r.w - pad - cw,
                r.y + (r.h - cf) / 2.0,
                dim_color(alpha),
                cw + 4.0 * scale,
                surface_w,
                surface_h,
            );
        }
    } else {
        // Breadcrumb segments separated by " › ".
        let segs = crumb_segments(&state.cwd);
        let sep = " › ";
        let sep_w = text.measure_width(sep, font);
        let mut x = r.x + pad;
        let max_x = r.x + r.w - pad;
        for (i, seg) in segs.iter().enumerate() {
            if x >= max_x { break; }
            let w = text.measure_width(seg, font);
            let hovered = state.hover_crumb == Some(i);
            if hovered {
                painter.rect_filled(
                    Rect::new(x - 4.0 * scale, r.y + 5.0 * scale, w + 8.0 * scale, r.h - 10.0 * scale),
                    6.0 * scale,
                    Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.18 * alpha),
                );
            }
            let color = if hovered { accent_color(alpha) } else { text_color(0.95 * alpha) };
            text.queue(seg, font, x, baseline_y, color, w + 4.0 * scale, surface_w, surface_h);
            x += w;
            if i + 1 < segs.len() {
                text.queue(sep, font, x, baseline_y, dim_color(alpha), sep_w + 4.0 * scale, surface_w, surface_h);
                x += sep_w;
            }
        }
    }
    text.pop_clip();
}

// ── Body: sidebar ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    state: &FilesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let sb = sidebar_rect(panel, top_y, scale);
    if sb.h <= 0.0 {
        return;
    }
    painter.push_clip(sb);
    text.push_clip([sb.x, sb.y, sb.w, sb.h]);

    for (i, loc) in Location::ALL.iter().enumerate() {
        let r = sidebar_tile_rect(panel, top_y, scale, text_size, i);
        if r.y > sb.y + sb.h { break; }
        let hovered = state.hover_location == Some(*loc);
        let active = (*loc == Location::Home && state.cwd == home_dir())
            || (*loc != Location::Home && state.cwd.starts_with(loc.path()));
        let radius = 10.0 * scale;
        let plate_alpha = if hovered { 0.16 } else if active { 0.10 } else { 0.0 };
        if plate_alpha > 0.001 {
            painter.rect_filled(r, radius, Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(plate_alpha * alpha));
        }
        if active {
            painter.rect_stroke_sdf(r, radius, 1.2 * scale, accent_color(0.55 * alpha));
        }
        let icon_size = sidebar_icon_size(text_size) * scale;
        let ic_x = r.x + 12.0 * scale;
        let ic_y = r.y + (r.h - icon_size) / 2.0;
        icons.push(IconRequest {
            app_id: format!("loc:{}", loc.label()),
            icon_name: Some(loc.icon_name().to_string()),
            x: ic_x,
            y: ic_y,
            size: icon_size,
            opacity: alpha,
            clip: Some([sb.x, sb.y, sb.w, sb.h]),
        });
        let font = (text_size * 0.95) * scale;
        let label_x = ic_x + icon_size + 12.0 * scale;
        let label_y = r.y + (r.h - font) / 2.0;
        let label_color = if active || hovered { accent_color(alpha) } else { text_color(0.92 * alpha) };
        text.queue(
            loc.label(),
            font,
            label_x,
            label_y,
            label_color,
            r.w - (label_x - r.x) - 8.0 * scale,
            surface_w,
            surface_h,
        );
    }

    painter.pop_clip();
    text.pop_clip();
}

// ── Body: list ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_list(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    state: &FilesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let list = list_rect(panel, top_y, scale);
    if list.h <= 0.0 {
        return;
    }
    painter.push_clip(list);
    text.push_clip([list.x, list.y, list.w, list.h]);

    if state.visible.is_empty() {
        let f = (text_size * 1.10) * scale;
        let msg = if state.filter.query().is_empty() { "(empty)" } else { "no matches" };
        text.queue(
            msg,
            f,
            list.x + ROW_PAD_X * scale,
            list.y + 12.0 * scale,
            dim_color(alpha),
            list.w,
            surface_w,
            surface_h,
        );
        painter.pop_clip();
        text.pop_clip();
        return;
    }

    let row_h = row_height(text_size) * scale;
    let first_visible = (state.scroll / row_h).floor().max(0.0) as usize;
    let visible_count = ((list.h / row_h).ceil() as usize) + 2;
    let last_visible = (first_visible + visible_count).min(state.visible.len());

    for i in first_visible..last_visible {
        let Some(entry) = state.entry_for_visible(i) else { continue };
        let row_y = list.y + i as f32 * row_h - state.scroll;
        let row = Rect::new(list.x, row_y, list.w, row_h);

        if state.hover_entry == Some(i) {
            painter.rect_filled(
                row,
                8.0 * scale,
                Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(HOVER_PLATE_ALPHA * alpha),
            );
        }
        draw_row(painter, text, icons, entry, row, scale, text_size, alpha, list, surface_w, surface_h);
    }

    painter.pop_clip();
    text.pop_clip();
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    entry: &FileEntry,
    row: Rect,
    scale: f32,
    text_size: f32,
    alpha: f32,
    clip: Rect,
    surface_w: u32,
    surface_h: u32,
) {
    let icon_size = row_icon_size(text_size) * scale;
    let icon_x = row.x + ROW_PAD_X * scale;
    let icon_y = row.y + (row.h - icon_size) / 2.0;
    let icon_rect = Rect::new(icon_x, icon_y, icon_size, icon_size);
    if entry.is_dir {
        draw_folder_glyph(painter, icon_rect, scale, alpha);
    } else {
        match file_kind(entry) {
            FileKind::Image => {
                draw_thumb_plate(painter, icon_rect, scale, alpha);
                icons.push(IconRequest {
                    app_id: format!("thumb:{}", entry.path.to_string_lossy()),
                    icon_name: Some(entry.path.to_string_lossy().into_owned()),
                    x: icon_rect.x,
                    y: icon_rect.y,
                    size: icon_size,
                    opacity: alpha,
                    clip: Some([clip.x, clip.y, clip.w, clip.h]),
                });
            }
            FileKind::Video => {
                draw_thumb_plate(painter, icon_rect, scale, alpha);
                let thumb_path = crate::launcher::icons::video_thumb_path(&entry.path);
                if thumb_path.exists() {
                    let key = thumb_path.to_string_lossy().into_owned();
                    icons.push(IconRequest {
                        app_id: format!("thumb:{}", key),
                        icon_name: Some(key),
                        x: icon_rect.x,
                        y: icon_rect.y,
                        size: icon_size,
                        opacity: alpha,
                        clip: Some([clip.x, clip.y, clip.w, clip.h]),
                    });
                }
                // Always overlay a small play triangle so video rows are
                // identifiable even while the thumb is still rendering.
                draw_play_overlay(painter, icon_rect, scale, alpha);
            }
            FileKind::Generic => draw_file_glyph(painter, icon_rect, scale, alpha),
        }
    }
    let name_x = icon_x + icon_size + 14.0 * scale;
    let name_font = (text_size * 1.10) * scale;
    let meta_font = (text_size * 0.78) * scale;
    let total_text_h = name_font + 4.0 * scale + meta_font;
    let name_y = row.y + (row.h - total_text_h) / 2.0;
    let meta_y = name_y + name_font + 4.0 * scale;

    let meta = format_meta(entry);
    let size = format_size_or_count(entry);
    let size_w = text.measure_width(&size, meta_font);
    let size_x = row.x + row.w - ROW_PAD_X * scale - size_w;
    let name_max_w = (size_x - name_x - 12.0 * scale).max(20.0);

    text.queue(&entry.name, name_font, name_x, name_y, text_color(0.96 * alpha), name_max_w, surface_w, surface_h);
    text.queue(&meta, meta_font, name_x, meta_y, dim_color(alpha), name_max_w, surface_w, surface_h);
    if !size.is_empty() {
        text.queue(&size, meta_font, size_x, meta_y, dim_color(alpha), size_w + 4.0 * scale, surface_w, surface_h);
    }
}

fn draw_folder_glyph(painter: &mut Painter, r: Rect, scale: f32, alpha: f32) {
    let color = accent_color(0.85 * alpha);
    let radius = 4.0 * scale;
    let tab_w = r.w * 0.45;
    let tab_h = r.h * 0.18;
    painter.rect_filled(Rect::new(r.x, r.y + r.h * 0.18, tab_w, tab_h), radius, color);
    let body_y = r.y + r.h * 0.30;
    let body_h = r.h * 0.60;
    painter.rect_filled(Rect::new(r.x, body_y, r.w, body_h), radius, color);
}

/// Subtle frame behind a thumbnail while it loads. Matches the corner
/// radius the image/video thumbs visually expect.
fn draw_thumb_plate(painter: &mut Painter, r: Rect, scale: f32, alpha: f32) {
    let plate = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.10 * alpha);
    painter.rect_filled(r, 4.0 * scale, plate);
}

/// Small accent play triangle in the lower-right corner of a video
/// thumbnail. Stays visible after the thumb loads so video rows remain
/// distinguishable from image rows at a glance.
fn draw_play_overlay(painter: &mut Painter, r: Rect, scale: f32, alpha: f32) {
    let play = Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(0.95 * alpha);
    let cx = r.x + r.w * 0.50;
    let cy = r.y + r.h * 0.50;
    let half = r.w * 0.15;
    let stroke = 3.0 * scale;
    painter.line_round(cx - half, cy - half, cx + half, cy, stroke, play);
    painter.line_round(cx + half, cy, cx - half, cy + half, stroke, play);
    painter.line_round(cx - half, cy + half, cx - half, cy - half, stroke, play);
}

fn draw_file_glyph(painter: &mut Painter, r: Rect, scale: f32, alpha: f32) {
    let color = text_color(0.78 * alpha);
    let radius = 4.0 * scale;
    let inset = r.w * 0.08;
    let body = Rect::new(r.x + inset, r.y + inset, r.w - inset * 2.0, r.h - inset * 2.0);
    painter.rect_stroke_sdf(body, radius, 2.0 * scale, color);
    let fold = body.w * 0.30;
    let stroke = 2.0 * scale;
    painter.line_round(body.x + body.w - fold, body.y, body.x + body.w, body.y + fold, stroke, color);
    painter.line_round(body.x + body.w - fold, body.y, body.x + body.w - fold, body.y + fold, stroke, color);
    painter.line_round(body.x + body.w - fold, body.y + fold, body.x + body.w, body.y + fold, stroke, color);
}
