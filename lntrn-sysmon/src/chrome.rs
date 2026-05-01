//! Window chrome: background, CSD title bar, window control buttons, border.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

// Fox dark-grey palette — matches lntrn-system-settings (linear-space values
// for sRGB 24,24,24 etc.).
const FOX_BG: Color             = Color::rgb(0.01032, 0.01032, 0.01032); // sRGB 24,24,24
pub const TEXT_PRIMARY: Color   = Color::rgb(0.84, 0.84, 0.84);          // sRGB ~236
pub const TEXT_SECONDARY: Color = Color::rgb(0.38, 0.38, 0.38);          // sRGB ~167
pub const BORDER_SUBTLE: Color  = Color::rgba(1.0, 1.0, 1.0, 0.08);
const CLOSE_BG: Color           = Color::rgb(0.56, 0.013, 0.013);        // sRGB ~200,30,30
const CLOSE_HOVER: Color        = Color::rgba(0.56, 0.013, 0.013, 0.45);
const CONTROL_HOVER: Color      = Color::rgba(1.0, 1.0, 1.0, 0.12);
const CONTROL_ICON: Color       = Color::rgb(0.45, 0.45, 0.45);          // sRGB ~180

pub const TITLE_BAR_H: f32 = 40.0;
pub const CORNER_RADIUS: f32 = 16.0;

/// Draw background — solid Fox dark-grey fill. Reads global opacity from config.
pub fn draw_background(p: &mut Painter, wf: f32, hf: f32, r: f32) {
    let opacity = lntrn_theme::background_opacity();
    p.rect_filled(
        Rect::new(0.0, 0.0, wf, hf), r,
        FOX_BG.with_alpha(opacity),
    );
}

/// Draw CSD title text centered.
pub fn draw_title(
    t: &mut TextRenderer, title: &str, s: f32,
    wf: f32, title_h: f32, sw: u32, sh: u32,
) {
    let sz = 22.0 * s;
    let tw = sz * 0.55 * title.len() as f32;
    t.queue(title, sz, (wf - tw) * 0.5, (title_h - sz) * 0.5, TEXT_SECONDARY, wf, sw, sh);
}

/// Draw window control buttons. Returns nothing — hover states are visual only.
pub fn draw_controls(
    p: &mut Painter, cx: f32, cy: f32, s: f32, wf: f32, title_h: f32,
) {
    let btn_r = 14.0 * s;
    let btn_y = title_h * 0.5;
    let close_cx = wf - 28.0 * s;
    let max_cx = wf - 66.0 * s;
    let min_cx = wf - 104.0 * s;
    let thick = 1.5 * s;
    let x_sz = 5.0 * s;

    let dist = |bx: f32| ((cx - bx).powi(2) + (cy - btn_y).powi(2)).sqrt();

    // Close — X
    let hov = dist(close_cx) < btn_r;
    if hov { p.circle_filled(close_cx, btn_y, btn_r, CLOSE_HOVER); }
    let ic = if hov { CLOSE_BG } else { CONTROL_ICON };
    p.line(close_cx - x_sz, btn_y - x_sz, close_cx + x_sz, btn_y + x_sz, thick, ic);
    p.line(close_cx - x_sz, btn_y + x_sz, close_cx + x_sz, btn_y - x_sz, thick, ic);

    // Maximize — square
    let hov = dist(max_cx) < btn_r;
    if hov { p.circle_filled(max_cx, btn_y, btn_r, CONTROL_HOVER); }
    let ic = if hov { TEXT_PRIMARY } else { CONTROL_ICON };
    p.rect_stroke_sdf(
        Rect::new(max_cx - x_sz, btn_y - x_sz, x_sz * 2.0, x_sz * 2.0),
        1.5 * s, thick, ic,
    );

    // Minimize — line
    let hov = dist(min_cx) < btn_r;
    if hov { p.circle_filled(min_cx, btn_y, btn_r, CONTROL_HOVER); }
    let ic = if hov { TEXT_PRIMARY } else { CONTROL_ICON };
    p.line(min_cx - x_sz, btn_y, min_cx + x_sz, btn_y, thick, ic);
}

/// Draw subtle window border (skip when maximized).
pub fn draw_border(p: &mut Painter, wf: f32, hf: f32, r: f32) {
    p.rect_stroke_sdf(Rect::new(0.0, 0.0, wf, hf), r, 1.0, BORDER_SUBTLE);
}
