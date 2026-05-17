//! Window chrome: background, CSD title bar, window control buttons, border.
//!
//! Two styles, matching the unified `[appearance].theme` setting:
//!   * Fox (Fox Dark) — neutral dark gray bg
//!   * Lantern        — warm brown bg

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::FoxPalette;

use crate::config::WindowMode;

// ── Fox palette (neutral dark) ──────────────────────────────────────────────
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

// ── Lantern palette (warm brown) ────────────────────────────────────────────
// LN_BG matches lntrn-terminal's Theme::lantern bg so all three apps land on
// the same warm brown when the user picks Lantern. Use from_rgba8 so the
// sRGB→linear conversion is exact (no hand-computed linear constants).
fn ln_bg() -> Color           { Color::from_rgba8(30, 25, 20, 255) }
const LN_TEXT_PRIMARY: Color  = Color::rgb(0.82, 0.79, 0.73);          // warm cream
const LN_TEXT_SECONDARY: Color= Color::rgb(0.40, 0.36, 0.30);          // muted tan
const LN_BORDER_SUBTLE: Color = Color::rgba(1.0, 0.85, 0.65, 0.10);
const LN_CLOSE_BG: Color      = Color::rgb(0.56, 0.013, 0.013);
const LN_CLOSE_HOVER: Color   = Color::rgba(0.56, 0.013, 0.013, 0.45);
const LN_CONTROL_HOVER: Color = Color::rgba(1.0, 0.95, 0.80, 0.12);
const LN_CONTROL_ICON: Color  = Color::rgb(0.42, 0.36, 0.28);

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
            WindowMode::Fox => Self {
                text_primary: FOX_TEXT_PRIMARY,
                text_secondary: FOX_TEXT_SECONDARY,
                border: FOX_BORDER_SUBTLE,
                control_icon: FOX_CONTROL_ICON,
                control_hover: FOX_CONTROL_HOVER,
                close_bg: FOX_CLOSE_BG,
                close_hover: FOX_CLOSE_HOVER,
            },
            WindowMode::Lantern => Self {
                text_primary: LN_TEXT_PRIMARY,
                text_secondary: LN_TEXT_SECONDARY,
                border: LN_BORDER_SUBTLE,
                control_icon: LN_CONTROL_ICON,
                control_hover: LN_CONTROL_HOVER,
                close_bg: LN_CLOSE_BG,
                close_hover: LN_CLOSE_HOVER,
            },
        }
    }
}

/// Build a [`FoxPalette`] for the content area (sliders, toggles, text) for
/// the given window mode. The accent comes from the user's
/// `[appearance].accent` override automatically via `FoxPalette::current()`.
pub fn content_palette(_mode: WindowMode) -> FoxPalette {
    FoxPalette::current()
}

/// Draw the window background — solid color, opacity from
/// `[windows].background_opacity`. Honors `[appearance].background_color`
/// when set (the Background Color swatch picker); otherwise falls back to
/// the variant default.
pub fn draw_background(p: &mut Painter, mode: WindowMode, wf: f32, hf: f32, r: f32) {
    let opacity = lntrn_theme::background_opacity();
    let bg = lntrn_theme::active_background_color()
        .map(|c| Color::from_rgba8(c.r, c.g, c.b, c.a))
        .unwrap_or_else(|| match mode {
            WindowMode::Fox => FOX_BG,
            WindowMode::Lantern => ln_bg(),
        });
    p.rect_filled(Rect::new(0.0, 0.0, wf, hf), r, bg.with_alpha(opacity));
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
