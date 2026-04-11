//! Inline title bar — window controls drawn directly on the paper background
//! so they feel like part of the document rather than chrome sitting on top.

use lntrn_render::{Color, Painter, Rect};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::{ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE};

/// Title bar height in logical pixels (multiplied by `scale` at draw time).
pub const TITLE_BAR_H: f32 = 34.0;

/// Width of each window-control button (close / max / min).
const WIN_BTN_W: f32 = 38.0;

/// Left padding before the menu bar so labels don't hug the window edge.
const CONTENT_LEFT_PAD: f32 = 8.0;

/// Layout rect for the title bar's content area (everything left of the window
/// controls). The File menu is drawn here.
pub fn title_content_rect(wf: f32, s: f32) -> Rect {
    let controls_w = WIN_BTN_W * 3.0 * s;
    let left = CONTENT_LEFT_PAD * s;
    Rect::new(
        left,
        0.0,
        (wf - controls_w - left).max(0.0),
        TITLE_BAR_H * s,
    )
}

/// Draw the inline window controls (minimize / maximize / close) at the right
/// edge of the title bar. Registers hit zones via `input`.
pub fn draw_window_controls(
    painter: &mut Painter,
    input: &mut InteractionContext,
    pal: &FoxPalette,
    wf: f32,
    s: f32,
) {
    let title_h = TITLE_BAR_H * s;
    let btn_w = WIN_BTN_W * s;
    let close_rect = Rect::new(wf - btn_w, 0.0, btn_w, title_h);
    let max_rect = Rect::new(wf - btn_w * 2.0, 0.0, btn_w, title_h);
    let min_rect = Rect::new(wf - btn_w * 3.0, 0.0, btn_w, title_h);

    let close_state = input.add_zone(ZONE_CLOSE, close_rect);
    let max_state = input.add_zone(ZONE_MAXIMIZE, max_rect);
    let min_state = input.add_zone(ZONE_MINIMIZE, min_rect);

    // Subtle warm hover tint that blends with paper.
    let hover_bg = Color::from_rgba8(60, 50, 35, 28);
    let icon_color = pal.text_secondary;
    let icon_sz = 12.0 * s;
    let stroke = 1.5 * s;

    // ── Minimize ────────────────────────────────────────────────
    if min_state.is_hovered() {
        painter.rect_filled(min_rect, 0.0, hover_bg);
    }
    let mcx = min_rect.center_x();
    let mcy = min_rect.center_y();
    painter.rect_filled(
        Rect::new(mcx - icon_sz * 0.5, mcy - stroke * 0.5, icon_sz, stroke),
        0.0,
        icon_color,
    );

    // ── Maximize ────────────────────────────────────────────────
    if max_state.is_hovered() {
        painter.rect_filled(max_rect, 0.0, hover_bg);
    }
    let xcx = max_rect.center_x();
    let xcy = max_rect.center_y();
    painter.rect_stroke(
        Rect::new(xcx - icon_sz * 0.5, xcy - icon_sz * 0.5, icon_sz, icon_sz),
        1.0 * s,
        stroke,
        icon_color,
    );

    // ── Close (warm red bg on hover, rounds top-right window corner) ─
    let close_hovered = close_state.is_hovered();
    if close_hovered {
        let close_bg = Color::from_rgb8(204, 78, 60);
        let r = 10.0 * s;
        painter.rect_filled(close_rect, r, close_bg);
        // Square off the bottom-left, top-left, and bottom-right so only the
        // top-right corner stays rounded (matches the window corner radius).
        painter.rect_filled(
            Rect::new(close_rect.x, close_rect.y, r, r),
            0.0,
            close_bg,
        );
        painter.rect_filled(
            Rect::new(close_rect.x, close_rect.y + close_rect.h - r, r, r),
            0.0,
            close_bg,
        );
        painter.rect_filled(
            Rect::new(
                close_rect.x + close_rect.w - r,
                close_rect.y + close_rect.h - r,
                r,
                r,
            ),
            0.0,
            close_bg,
        );
    }
    let cx = close_rect.center_x();
    let cy = close_rect.center_y();
    let half = icon_sz * 0.5;
    let close_icon_color = if close_hovered {
        Color::WHITE
    } else {
        icon_color
    };
    painter.line(
        cx - half, cy - half, cx + half, cy + half, stroke, close_icon_color,
    );
    painter.line(
        cx + half, cy - half, cx - half, cy + half, stroke, close_icon_color,
    );
}
