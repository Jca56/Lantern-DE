//! Window chrome: background, CSD title bar, window control buttons, border.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

// Night sky palette (base colors — opacity applied from config at runtime)
const BG_DEEP: Color       = Color::rgb(0.003, 0.001, 0.014);
const BG_SURFACE: Color    = Color::rgb(0.008, 0.003, 0.028);
const GLOW_PINK: Color     = Color::rgba(0.45, 0.14, 0.32, 0.07);
const GLOW_CYAN: Color     = Color::rgba(0.14, 0.35, 0.52, 0.07);
pub const TEXT_PRIMARY: Color   = Color::rgb(0.80, 0.76, 0.90);
pub const TEXT_SECONDARY: Color = Color::rgb(0.45, 0.40, 0.58);
pub const BORDER_SUBTLE: Color  = Color::rgba(0.30, 0.20, 0.50, 0.15);
const CLOSE_BG: Color       = Color::rgb(0.45, 0.02, 0.02);
const CLOSE_HOVER: Color    = Color::rgba(0.45, 0.02, 0.02, 0.35);
const CONTROL_HOVER: Color  = Color::rgba(0.50, 0.38, 0.70, 0.25);
const CONTROL_ICON: Color   = Color::rgb(0.55, 0.50, 0.68);

pub const TITLE_BAR_H: f32 = 40.0;
pub const CORNER_RADIUS: f32 = 16.0;

/// Draw background gradient + radial glows. Reads global opacity from config.
pub fn draw_background(p: &mut Painter, wf: f32, hf: f32, r: f32) {
    let opacity = lntrn_theme::background_opacity();
    p.rect_gradient_linear(
        Rect::new(0.0, 0.0, wf, hf), r,
        std::f32::consts::FRAC_PI_2,
        BG_DEEP.with_alpha(opacity),
        BG_SURFACE.with_alpha(opacity),
    );
    p.rect_gradient_radial(
        Rect::new(-wf * 0.35, hf * 0.5, wf * 0.8, hf * 0.8), 0.0,
        GLOW_PINK, Color::TRANSPARENT,
    );
    p.rect_gradient_radial(
        Rect::new(wf * 0.5, -hf * 0.25, wf * 0.8, hf * 0.7), 0.0,
        GLOW_CYAN, Color::TRANSPARENT,
    );
}

/// Draw CSD title text centered.
pub fn draw_title(
    t: &mut TextRenderer, title: &str, s: f32,
    wf: f32, title_h: f32, sw: u32, sh: u32,
) {
    let sz = 20.0 * s;
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
