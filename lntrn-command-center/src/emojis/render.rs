//! Render the emojis overlay page.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{
    cell_rect_at, cells_per_row, category_strip_rect, category_tab_rect, data, filter_bar_rect,
    grid_rect, png_path_for, Category, EmojisState, CELL_SIZE, GRID_PAD,
};
use crate::render::IconRequest;

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const FLASH_MS: u128 = 280;

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    state: &EmojisState,
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
    draw_category_strip(painter, icons, state, panel, top_y, scale, alpha);
    draw_grid(
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
}

#[allow(clippy::too_many_arguments)]
fn draw_filter_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &EmojisState,
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
    painter.rect_filled(
        bar,
        radius,
        Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(0.06 * alpha),
    );
    if state.filtered() {
        painter.rect_stroke_sdf(
            bar,
            radius,
            1.4 * scale,
            accent(0.55 * alpha),
        );
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
    let h_x = cx + glyph_r * 0.55;
    let h_y = cy + glyph_r * 0.55;
    let t_x = cx + glyph_r * 1.2;
    let t_y = cy + glyph_r * 1.2;
    painter.line_round(h_x, h_y, t_x, t_y, stroke * 1.4, glyph_color);

    // Text input.
    let pad_left = glyph_pad + glyph_r * 2.0 + 14.0 * scale;
    let font = (text_size * scale).max(14.0);
    // `text.queue` treats `y` as the TOP of the glyph row, not the baseline.
    let text_top = bar.y + (bar.h - font) / 2.0;

    let q = state.filter.query();
    let (display, is_placeholder) = if q.is_empty() {
        ("Search emoji…".to_string(), true)
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
    text.queue(&display, font, text_x, text_top, color, text_max_w, surface_w, surface_h);

    // Count badge (filtered total) on the right.
    let visible_n = state.visible_indices().len();
    let count = format!("{}", visible_n);
    let cf = font * 0.78;
    let cw = text.measure_width(&count, cf);
    let cx_pad = 16.0 * scale;
    let cx_x = bar.x + bar.w - cx_pad - cw;
    let cy = bar.y + (bar.h - cf) / 2.0;
    text.queue(&count, cf, cx_x, cy, white(0.55 * alpha), cw + 4.0 * scale, surface_w, surface_h);

    // Caret blink for the input. Only blink when there's actual text or
    // we're considered "focused" — the overlay is always the active
    // surface when open, so always blink.
    if !is_placeholder && state.filter.cursor_visible() {
        let caret_x = text_x + text.measure_width(q, font) + 2.0 * scale;
        let caret_y = bar.y + bar.h * 0.20;
        let caret_h = bar.h * 0.60;
        painter.rect_filled(
            Rect::new(caret_x, caret_y, 2.0 * scale, caret_h),
            1.0 * scale,
            accent(alpha),
        );
    } else if is_placeholder && state.filter.cursor_visible() {
        // Show a caret at the leading edge when empty.
        let caret_x = text_x;
        let caret_y = bar.y + bar.h * 0.20;
        let caret_h = bar.h * 0.60;
        painter.rect_filled(
            Rect::new(caret_x, caret_y, 2.0 * scale, caret_h),
            1.0 * scale,
            accent(0.55 * alpha),
        );
    }
}

fn draw_category_strip(
    painter: &mut Painter,
    icons: &mut Vec<IconRequest>,
    state: &EmojisState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
) {
    let _strip = category_strip_rect(panel, top_y, scale);
    // Dim out when a filter is active — categories don't apply.
    let strip_alpha = if state.filtered() { 0.35 * alpha } else { alpha };

    for (i, cat) in Category::ALL.iter().enumerate() {
        let r = category_tab_rect(panel, top_y, scale, i);
        let active = !state.filtered() && state.active_category == *cat;
        let hovered = state.hover_category == Some(*cat);

        let radius = 12.0 * scale;
        let plate_alpha = if hovered {
            0.18
        } else if active {
            0.22
        } else {
            0.06
        };
        painter.rect_filled(r, radius, white(plate_alpha * strip_alpha));
        if active {
            painter.rect_stroke_sdf(r, radius, 1.6 * scale, accent(0.85 * strip_alpha));
        }

        // Icon centered in the tab.
        let icon_size = r.h * 0.72;
        let ix = r.x + (r.w - icon_size) / 2.0;
        let iy = r.y + (r.h - icon_size) / 2.0;
        let unicode = cat.icon_unicode();
        let path = png_path_for(unicode);
        let cache_key = format!("emoji:{}", unicode);
        icons.push(IconRequest {
            app_id: cache_key,
            icon_name: Some(path.to_string_lossy().into_owned()),
            x: ix,
            y: iy,
            size: icon_size,
            opacity: strip_alpha,
            clip: None,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_grid(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    state: &EmojisState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    _text_size: f32,
    alpha: f32,
    panel_bottom: f32,
    _surface_w: u32,
    _surface_h: u32,
) {
    let grid = grid_rect(panel, top_y, scale, panel_bottom);
    if grid.h <= 0.0 {
        return;
    }
    painter.push_clip(grid);

    let visible = state.visible_indices();
    let per_row = cells_per_row(grid, scale);
    let cell = CELL_SIZE * scale;
    let scroll_px = state.scroll * scale;

    let icon_size = cell * 0.84;
    let icon_inset = (cell - icon_size) / 2.0;

    // Only push icons for cells whose y intersects the grid rect.
    let row_stride = cell + super::CELL_GAP * scale;
    let total_rows = ((visible.len() + per_row - 1) / per_row.max(1)) as i32;
    let first_visible_row = (scroll_px / row_stride).floor().max(0.0) as i32;
    let last_visible_row =
        (((scroll_px + grid.h) / row_stride).ceil() as i32 + 1).min(total_rows);

    for row in first_visible_row..last_visible_row {
        for col in 0..per_row {
            let visible_idx = (row as usize) * per_row + col;
            if visible_idx >= visible.len() {
                break;
            }
            let mut r = cell_rect_at(grid, scale, visible_idx, per_row);
            // Apply scroll.
            r.y -= scroll_px;
            if r.y + r.h < grid.y || r.y > grid.y + grid.h {
                continue;
            }
            let entry_idx = visible[visible_idx];
            let entry = &data::EMOJIS[entry_idx];

            // Hover plate.
            let hovered = state.hover_entry == Some(visible_idx);
            let flash_amount = flash_factor(state.recent_copy, visible_idx);
            let radius = 12.0 * scale;
            let plate_alpha = if flash_amount > 0.0 {
                0.30 + 0.50 * flash_amount
            } else if hovered {
                0.18
            } else {
                0.0
            };
            if plate_alpha > 0.001 {
                let color = if flash_amount > 0.0 {
                    accent(plate_alpha * alpha)
                } else {
                    white(plate_alpha * alpha)
                };
                painter.rect_filled(r, radius, color);
            }

            // Icon PNG.
            let path = png_path_for(entry.unicode);
            let cache_key = format!("emoji:{}", entry.unicode);
            icons.push(IconRequest {
                app_id: cache_key,
                icon_name: Some(path.to_string_lossy().into_owned()),
                x: r.x + icon_inset,
                y: r.y + icon_inset,
                size: icon_size,
                opacity: alpha,
                clip: Some([grid.x, grid.y, grid.w, grid.h]),
            });
        }
    }

    painter.pop_clip();

    // Scrollbar — thin track on the right.
    let max = super::max_scroll(visible.len(), per_row, grid.h, scale);
    if max > 0.0 {
        let track_w = 4.0 * scale;
        let track_x = panel.x + panel.w - GRID_PAD * scale - track_w;
        let track_y = grid.y;
        let track_h = grid.h;
        painter.rect_filled(
            Rect::new(track_x, track_y, track_w, track_h),
            track_w / 2.0,
            white(0.06 * alpha),
        );
        let thumb_h = (track_h * track_h / (track_h + max * scale)).max(24.0 * scale);
        let thumb_y = track_y + (track_h - thumb_h) * (scroll_px / (max * scale)).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::new(track_x, thumb_y, track_w, thumb_h),
            track_w / 2.0,
            white(0.30 * alpha),
        );
    }
}

fn flash_factor(recent: Option<(usize, std::time::Instant)>, idx: usize) -> f32 {
    let Some((rec_idx, t)) = recent else {
        return 0.0;
    };
    if rec_idx != idx {
        return 0.0;
    }
    let ms = t.elapsed().as_millis();
    if ms >= FLASH_MS {
        return 0.0;
    }
    let p = 1.0 - (ms as f32 / FLASH_MS as f32);
    p.clamp(0.0, 1.0)
}

fn accent(a: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(a)
}
fn white(a: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(a)
}
