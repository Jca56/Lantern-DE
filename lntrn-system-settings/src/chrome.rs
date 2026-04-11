//! Window chrome: background, CSD title bar, window control buttons, border.
//!
//! Supports two styles: Fox (warm solid bg) and Night Sky (indigo gradient + glows).

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::FoxPalette;

use crate::config::WindowMode;

// ── Night Sky palette (base colors — opacity applied from config at runtime) ─
const NS_BG_DEEP: Color       = Color::rgb(0.003, 0.001, 0.014);
const NS_BG_SURFACE: Color    = Color::rgb(0.008, 0.003, 0.028);
const NS_GLOW_PINK: Color     = Color::rgba(0.45, 0.14, 0.32, 0.07);
const NS_GLOW_CYAN: Color     = Color::rgba(0.14, 0.35, 0.52, 0.07);
const NS_TEXT_PRIMARY: Color  = Color::rgb(0.80, 0.76, 0.90);
const NS_TEXT_SECONDARY: Color= Color::rgb(0.45, 0.40, 0.58);
const NS_BORDER_SUBTLE: Color = Color::rgba(0.30, 0.20, 0.50, 0.15);
const NS_CLOSE_BG: Color      = Color::rgb(0.45, 0.02, 0.02);
const NS_CLOSE_HOVER: Color   = Color::rgba(0.45, 0.02, 0.02, 0.35);
const NS_CONTROL_HOVER: Color = Color::rgba(0.50, 0.38, 0.70, 0.25);
const NS_CONTROL_ICON: Color  = Color::rgb(0.55, 0.50, 0.68);

// ── Fox palette (warm dark) ─────────────────────────────────────────────────
// `Color::rgb` stores LINEAR-space values; the comment beside each line is the
// equivalent sRGB 8-bit value so it's easy to compare against design specs.
//
// FOX_BG is the linear equivalent of sRGB(24, 24, 24), matching lntrn-terminal.
const FOX_BG: Color           = Color::rgb(0.01032, 0.01032, 0.01032); // sRGB 24,24,24
const FOX_TEXT_PRIMARY: Color = Color::rgb(0.84, 0.84, 0.84);          // sRGB ~236
const FOX_TEXT_SECONDARY: Color = Color::rgb(0.38, 0.38, 0.38);        // sRGB ~167
const FOX_BORDER_SUBTLE: Color= Color::rgba(1.0, 1.0, 1.0, 0.08);
const FOX_CLOSE_BG: Color     = Color::rgb(0.56, 0.013, 0.013);        // sRGB ~200,30,30
const FOX_CLOSE_HOVER: Color  = Color::rgba(0.56, 0.013, 0.013, 0.45);
const FOX_CONTROL_HOVER: Color= Color::rgba(1.0, 1.0, 1.0, 0.12);
const FOX_CONTROL_ICON: Color = Color::rgb(0.45, 0.45, 0.45);          // sRGB ~180

pub const TITLE_BAR_H: f32 = 40.0;
pub const CORNER_RADIUS: f32 = 16.0;

/// Colors used to draw chrome elements for a given [`WindowMode`].
#[derive(Clone, Copy)]
pub struct ChromePalette {
    pub text_primary: Color,
    pub text_secondary: Color,
    pub border: Color,
    pub control_icon: Color,
    pub control_hover: Color,
    pub close_bg: Color,
    pub close_hover: Color,
}

impl ChromePalette {
    pub fn for_mode(mode: WindowMode) -> Self {
        match mode {
            WindowMode::NightSky => Self {
                text_primary: NS_TEXT_PRIMARY,
                text_secondary: NS_TEXT_SECONDARY,
                border: NS_BORDER_SUBTLE,
                control_icon: NS_CONTROL_ICON,
                control_hover: NS_CONTROL_HOVER,
                close_bg: NS_CLOSE_BG,
                close_hover: NS_CLOSE_HOVER,
            },
            WindowMode::Fox => Self {
                text_primary: FOX_TEXT_PRIMARY,
                text_secondary: FOX_TEXT_SECONDARY,
                border: FOX_BORDER_SUBTLE,
                control_icon: FOX_CONTROL_ICON,
                control_hover: FOX_CONTROL_HOVER,
                close_bg: FOX_CLOSE_BG,
                close_hover: FOX_CLOSE_HOVER,
            },
        }
    }
}

/// Build a [`FoxPalette`] for the content area (sliders, toggles, text) for
/// the given window mode. Uses shared palette definitions from `lntrn_theme`.
pub fn content_palette(mode: WindowMode) -> FoxPalette {
    match mode {
        WindowMode::Fox => FoxPalette::dark(),
        WindowMode::NightSky => FoxPalette::night_sky(),
    }
}

/// Draw the window background. Fox: solid warm-dark fill with rounded corners.
/// Night Sky: gradient + radial glows. Opacity is read from `lntrn_theme`.
pub fn draw_background(p: &mut Painter, mode: WindowMode, wf: f32, hf: f32, r: f32) {
    let opacity = lntrn_theme::background_opacity();
    match mode {
        WindowMode::Fox => {
            p.rect_filled(
                Rect::new(0.0, 0.0, wf, hf),
                r,
                FOX_BG.with_alpha(opacity),
            );
        }
        WindowMode::NightSky => {
            p.rect_gradient_linear(
                Rect::new(0.0, 0.0, wf, hf), r,
                std::f32::consts::FRAC_PI_2,
                NS_BG_DEEP.with_alpha(opacity),
                NS_BG_SURFACE.with_alpha(opacity),
            );
            p.rect_gradient_radial(
                Rect::new(-wf * 0.35, hf * 0.5, wf * 0.8, hf * 0.8), 0.0,
                NS_GLOW_PINK, Color::TRANSPARENT,
            );
            p.rect_gradient_radial(
                Rect::new(wf * 0.5, -hf * 0.25, wf * 0.8, hf * 0.7), 0.0,
                NS_GLOW_CYAN, Color::TRANSPARENT,
            );
        }
    }
}

/// Draw CSD title text centered. Uses the secondary text color for the palette.
pub fn draw_title(
    t: &mut TextRenderer, title: &str, s: f32,
    wf: f32, title_h: f32, pal: &ChromePalette, sw: u32, sh: u32,
) {
    let sz = 16.0 * s;
    let tw = sz * 0.55 * title.len() as f32;
    t.queue(title, sz, (wf - tw) * 0.5, (title_h - sz) * 0.5, pal.text_secondary, wf, sw, sh);
}

/// Draw window control buttons (close / maximize / minimize).
pub fn draw_controls(
    p: &mut Painter, cx: f32, cy: f32, s: f32, wf: f32, title_h: f32, pal: &ChromePalette,
) {
    let btn_r = 12.0 * s;
    let btn_y = title_h * 0.5;
    let close_cx = wf - 24.0 * s;
    let max_cx = wf - 56.0 * s;
    let min_cx = wf - 88.0 * s;
    let thick = 1.5 * s;
    let x_sz = 4.5 * s;

    let dist = |bx: f32| ((cx - bx).powi(2) + (cy - btn_y).powi(2)).sqrt();

    // Close — X
    let hov = dist(close_cx) < btn_r;
    if hov { p.circle_filled(close_cx, btn_y, btn_r, pal.close_hover); }
    let ic = if hov { pal.close_bg } else { pal.control_icon };
    p.line(close_cx - x_sz, btn_y - x_sz, close_cx + x_sz, btn_y + x_sz, thick, ic);
    p.line(close_cx - x_sz, btn_y + x_sz, close_cx + x_sz, btn_y - x_sz, thick, ic);

    // Maximize — square
    let hov = dist(max_cx) < btn_r;
    if hov { p.circle_filled(max_cx, btn_y, btn_r, pal.control_hover); }
    let ic = if hov { pal.text_primary } else { pal.control_icon };
    p.rect_stroke_sdf(
        Rect::new(max_cx - x_sz, btn_y - x_sz, x_sz * 2.0, x_sz * 2.0),
        1.5 * s, thick, ic,
    );

    // Minimize — line
    let hov = dist(min_cx) < btn_r;
    if hov { p.circle_filled(min_cx, btn_y, btn_r, pal.control_hover); }
    let ic = if hov { pal.text_primary } else { pal.control_icon };
    p.line(min_cx - x_sz, btn_y, min_cx + x_sz, btn_y, thick, ic);
}

/// Standard control button hit-test radius.
pub const CONTROL_HIT_R: f32 = 18.0;
/// Horizontal offsets from the right edge of the window to each control center.
pub const CLOSE_OFFSET: f32 = 24.0;
pub const MAX_OFFSET: f32 = 56.0;
pub const MIN_OFFSET: f32 = 88.0;

/// Draw subtle window border (skip when maximized).
pub fn draw_border(p: &mut Painter, wf: f32, hf: f32, r: f32, pal: &ChromePalette) {
    p.rect_stroke_sdf(Rect::new(0.0, 0.0, wf, hf), r, 1.0, pal.border);
}
