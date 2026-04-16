//! Night Sky window chrome: gradient background, custom title bar, window controls.

use lntrn_render::{Color, Painter, Rect};

// ── Night sky palette ───────────────────────────────────────────────────────
const BG_DEEP: Color = Color::rgb(0.008, 0.003, 0.020);
const BG_SURFACE: Color = Color::rgb(0.020, 0.007, 0.045);
const GLOW_PINK: Color = Color::rgba(0.45, 0.14, 0.32, 0.04);
const GLOW_CYAN: Color = Color::rgba(0.14, 0.35, 0.52, 0.04);
const BORDER_SUBTLE: Color = Color::rgba(0.30, 0.20, 0.50, 0.15);
const CLOSE_BG: Color = Color::rgb(0.45, 0.02, 0.02);
const CLOSE_HOVER: Color = Color::rgba(0.45, 0.02, 0.02, 0.35);
const CONTROL_HOVER: Color = Color::rgba(0.50, 0.38, 0.70, 0.25);
const CONTROL_ICON: Color = Color::rgb(0.55, 0.50, 0.68);
const TEXT_PRIMARY: Color = Color::rgb(0.80, 0.76, 0.90);

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
    // Cyan glow — top-left
    p.rect_gradient_radial(
        Rect::new(-w * 0.35, -h * 0.25, w * 0.8, h * 0.7),
        0.0,
        GLOW_CYAN,
        Color::TRANSPARENT,
    );
    // Pink glow — bottom-right
    p.rect_gradient_radial(
        Rect::new(w * 0.5, h * 0.5, w * 0.8, h * 0.8),
        0.0,
        GLOW_PINK,
        Color::TRANSPARENT,
    );
}

// ── Window controls ─────────────────────────────────────────────────────────

const BTN_RADIUS: f32 = 14.0;
const BTN_THICK: f32 = 1.5;
const ICON_SIZE: f32 = 5.0;

fn btn_positions(w: f32) -> (f32, f32, f32, f32) {
    let y = crate::ui_chrome::TITLE_BAR_HEIGHT * 0.5;
    // Right margin slightly enlarged to pull the controls in from the edge.
    (w - 40.0, w - 78.0, w - 116.0, y)
}

fn dist(cx: f32, cy: f32, bx: f32, by: f32) -> f32 {
    ((cx - bx).powi(2) + (cy - by).powi(2)).sqrt()
}

/// Per-mode colors for window control buttons.
pub struct ControlPalette {
    pub icon: Color,
    pub icon_hover: Color,
    pub hover_bg: Color,
    pub close_bg: Color,
    pub close_icon: Color,
}

impl ControlPalette {
    pub fn default_palette() -> Self {
        Self {
            icon: CONTROL_ICON,
            icon_hover: TEXT_PRIMARY,
            hover_bg: CONTROL_HOVER,
            close_bg: CLOSE_HOVER,
            close_icon: CLOSE_BG,
        }
    }

    pub fn lantern() -> Self {
        Self {
            icon: Color::from_rgba8(140, 120, 90, 255),
            icon_hover: Color::from_rgba8(212, 160, 32, 255),
            hover_bg: Color::from_rgba8(212, 160, 32, 60),
            close_bg: CLOSE_HOVER,
            close_icon: CLOSE_BG,
        }
    }
}

/// Draw window control buttons (close, maximize, minimize).
pub fn draw_controls(p: &mut Painter, cursor: Option<(f32, f32)>, w: f32, pal: &ControlPalette) {
    let (close_x, max_x, min_x, btn_y) = btn_positions(w);
    let (cx, cy) = cursor.unwrap_or((-1.0, -1.0));

    // Close — X
    let hov = dist(cx, cy, close_x, btn_y) < BTN_RADIUS;
    if hov {
        p.circle_filled(close_x, btn_y, BTN_RADIUS, pal.close_bg);
    }
    let ic = if hov { pal.close_icon } else { pal.icon };
    p.line(close_x - ICON_SIZE, btn_y - ICON_SIZE, close_x + ICON_SIZE, btn_y + ICON_SIZE, BTN_THICK, ic);
    p.line(close_x - ICON_SIZE, btn_y + ICON_SIZE, close_x + ICON_SIZE, btn_y - ICON_SIZE, BTN_THICK, ic);

    // Maximize — square
    let hov = dist(cx, cy, max_x, btn_y) < BTN_RADIUS;
    if hov {
        p.circle_filled(max_x, btn_y, BTN_RADIUS, pal.hover_bg);
    }
    let ic = if hov { pal.icon_hover } else { pal.icon };
    p.rect_stroke_sdf(
        Rect::new(max_x - ICON_SIZE, btn_y - ICON_SIZE, ICON_SIZE * 2.0, ICON_SIZE * 2.0),
        1.5, BTN_THICK, ic,
    );

    // Minimize — line
    let hov = dist(cx, cy, min_x, btn_y) < BTN_RADIUS;
    if hov {
        p.circle_filled(min_x, btn_y, BTN_RADIUS, pal.hover_bg);
    }
    let ic = if hov { pal.icon_hover } else { pal.icon };
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
