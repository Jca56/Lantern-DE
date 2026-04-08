//! Night Sky window chrome: gradient background, custom title bar, window controls.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

// ── Night sky palette ───────────────────────────────────────────────────────
const BG_DEEP: Color = Color::rgb(0.003, 0.001, 0.014);
const BG_SURFACE: Color = Color::rgb(0.008, 0.003, 0.028);
const GLOW_PINK: Color = Color::rgba(0.45, 0.14, 0.32, 0.04);
const GLOW_CYAN: Color = Color::rgba(0.14, 0.35, 0.52, 0.04);
pub const TEXT_SECONDARY: Color = Color::rgb(0.45, 0.40, 0.58);
const BORDER_SUBTLE: Color = Color::rgba(0.30, 0.20, 0.50, 0.15);
const CLOSE_BG: Color = Color::rgb(0.45, 0.02, 0.02);
const CLOSE_HOVER: Color = Color::rgba(0.45, 0.02, 0.02, 0.35);
const CONTROL_HOVER: Color = Color::rgba(0.50, 0.38, 0.70, 0.25);
const CONTROL_ICON: Color = Color::rgb(0.55, 0.50, 0.68);
const TEXT_PRIMARY: Color = Color::rgb(0.80, 0.76, 0.90);

/// Tab bar fill color for Night Sky mode.
pub const TAB_BAR_BG: Color = Color::rgba(0.005, 0.002, 0.020, 0.85);

pub const TITLE_BAR_HEIGHT: f32 = 40.0;
pub const CORNER_RADIUS: f32 = 16.0;

// ── Background ──────────────────────────────────────────────────────────────

/// Draw gradient background + radial glows.
pub fn draw_background(p: &mut Painter, w: f32, h: f32, maximized: bool) {
    let r = if maximized { 0.0 } else { CORNER_RADIUS };
    let opacity = lntrn_theme::background_opacity();
    p.rect_gradient_linear(
        Rect::new(0.0, 0.0, w, h),
        r,
        std::f32::consts::FRAC_PI_2,
        BG_DEEP.with_alpha(opacity),
        BG_SURFACE.with_alpha(opacity),
    );
    p.rect_gradient_radial(
        Rect::new(-w * 0.35, h * 0.5, w * 0.8, h * 0.8),
        0.0,
        GLOW_PINK,
        Color::TRANSPARENT,
    );
    p.rect_gradient_radial(
        Rect::new(w * 0.5, -h * 0.25, w * 0.8, h * 0.7),
        0.0,
        GLOW_CYAN,
        Color::TRANSPARENT,
    );
}

// ── Window controls ─────────────────────────────────────────────────────────

const BTN_RADIUS: f32 = 14.0;
const BTN_THICK: f32 = 1.5;
const ICON_SIZE: f32 = 5.0;

fn btn_positions(w: f32) -> (f32, f32, f32, f32) {
    let y = TITLE_BAR_HEIGHT * 0.5;
    (w - 28.0, w - 66.0, w - 104.0, y)
}

fn dist(cx: f32, cy: f32, bx: f32, by: f32) -> f32 {
    ((cx - bx).powi(2) + (cy - by).powi(2)).sqrt()
}

/// Draw window control buttons (close, maximize, minimize).
pub fn draw_controls(p: &mut Painter, cursor: Option<(f32, f32)>, w: f32) {
    let (close_x, max_x, min_x, btn_y) = btn_positions(w);
    let (cx, cy) = cursor.unwrap_or((-1.0, -1.0));

    // Close — X
    let hov = dist(cx, cy, close_x, btn_y) < BTN_RADIUS;
    if hov {
        p.circle_filled(close_x, btn_y, BTN_RADIUS, CLOSE_HOVER);
    }
    let ic = if hov { CLOSE_BG } else { CONTROL_ICON };
    p.line(close_x - ICON_SIZE, btn_y - ICON_SIZE, close_x + ICON_SIZE, btn_y + ICON_SIZE, BTN_THICK, ic);
    p.line(close_x - ICON_SIZE, btn_y + ICON_SIZE, close_x + ICON_SIZE, btn_y - ICON_SIZE, BTN_THICK, ic);

    // Maximize — square
    let hov = dist(cx, cy, max_x, btn_y) < BTN_RADIUS;
    if hov {
        p.circle_filled(max_x, btn_y, BTN_RADIUS, CONTROL_HOVER);
    }
    let ic = if hov { TEXT_PRIMARY } else { CONTROL_ICON };
    p.rect_stroke_sdf(
        Rect::new(max_x - ICON_SIZE, btn_y - ICON_SIZE, ICON_SIZE * 2.0, ICON_SIZE * 2.0),
        1.5, BTN_THICK, ic,
    );

    // Minimize — line
    let hov = dist(cx, cy, min_x, btn_y) < BTN_RADIUS;
    if hov {
        p.circle_filled(min_x, btn_y, BTN_RADIUS, CONTROL_HOVER);
    }
    let ic = if hov { TEXT_PRIMARY } else { CONTROL_ICON };
    p.line(min_x - ICON_SIZE, btn_y, min_x + ICON_SIZE, btn_y, BTN_THICK, ic);
}

/// Hit-test window controls. Returns Some(zone_id) matching ui_chrome zone constants.
pub fn hit_test_controls(cursor: (f32, f32), w: f32) -> Option<u32> {
    let (close_x, max_x, min_x, btn_y) = btn_positions(w);
    let (cx, cy) = cursor;
    if dist(cx, cy, close_x, btn_y) < BTN_RADIUS {
        Some(10) // ZONE_CLOSE
    } else if dist(cx, cy, max_x, btn_y) < BTN_RADIUS {
        Some(11) // ZONE_MAXIMIZE
    } else if dist(cx, cy, min_x, btn_y) < BTN_RADIUS {
        Some(12) // ZONE_MINIMIZE
    } else {
        None
    }
}

// ── Border ──────────────────────────────────────────────────────────────────

/// Draw subtle window border (skip when maximized).
pub fn draw_border(p: &mut Painter, w: f32, h: f32, maximized: bool) {
    if !maximized {
        p.rect_stroke_sdf(Rect::new(0.0, 0.0, w, h), CORNER_RADIUS, 1.0, BORDER_SUBTLE);
    }
}
