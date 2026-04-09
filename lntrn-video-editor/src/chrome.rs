//! Window chrome: background, CSD title bar, window control buttons, border.
//! Colors and patterns match lantern-studio's warm brown/gold theme.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

// ── Lantern Studio palette (sRGB hex → linear via from_rgb8) ───────────────
// Dark colors: x^2 ≈ x^2.2, accurate enough for const
const fn c(r: u8, g: u8, b: u8) -> Color {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    Color { r: rf * rf, g: gf * gf, b: bf * bf, a: 1.0 }
}

pub const BG: Color          = c(0x12, 0x10, 0x0e);
pub const PANEL: Color       = c(0x19, 0x12, 0x00);
pub const PANEL_DARK: Color  = c(0x10, 0x0c, 0x00);
pub const BUTTON: Color      = c(0x2a, 0x22, 0x18);
pub const BUTTON_HOVER: Color = c(0x3d, 0x32, 0x25);
pub const ACTIVE: Color      = c(0x4a, 0x40, 0x38);
pub const BORDER: Color      = c(0x30, 0x28, 0x20);
pub const INPUT_BG: Color    = c(0x15, 0x12, 0x10);

// Bright colors need proper sRGB→linear (x^2 is too inaccurate for high values)
pub fn accent() -> Color { Color::from_rgb8(0xff, 0xc8, 0x00) }
pub fn text() -> Color { Color::from_rgb8(0xe8, 0xdc, 0xc8) }
pub fn text_dim() -> Color { Color::from_rgb8(0x8a, 0x7d, 0x6a) }
pub fn close_red() -> Color { Color::from_rgb8(0xe8, 0x1d, 0x23) }

// Rainbow accent gradient colors
pub fn rainbow() -> [Color; 6] {
    [
        Color::from_rgb8(0xe9, 0x45, 0x60), // rose
        Color::from_rgb8(0xf0, 0xc0, 0x40), // gold
        Color::from_rgb8(0xa8, 0xe7, 0x2e), // lime
        Color::from_rgb8(0x29, 0xad, 0xff), // sky-blue
        Color::from_rgb8(0x83, 0x76, 0x9c), // purple
        Color::from_rgb8(0xff, 0x77, 0xa8), // pink
    ]
}

pub const TITLE_BAR_H: f32 = 44.0;
pub const CORNER_RADIUS: f32 = 0.0;

/// Draw the app background (solid dark brown).
pub fn draw_background(p: &mut Painter, wf: f32, hf: f32) {
    p.rect_filled(Rect::new(0.0, 0.0, wf, hf), 0.0, BG);
}

/// Draw title bar background + logo text + bottom border.
pub fn draw_title_bar(
    p: &mut Painter, t: &mut TextRenderer, s: f32, wf: f32, sw: u32, sh: u32,
) {
    let h = TITLE_BAR_H * s;

    // Title bar background
    p.rect_filled(Rect::new(0.0, 0.0, wf, h), 0.0, PANEL);

    // Bottom border
    p.rect_filled(Rect::new(0.0, h - 1.0 * s, wf, 1.0 * s), 0.0, BORDER);

    // Logo text — right-aligned like lantern-studio
    let title = "L A N T E R N   E D I T";
    let sz = 22.0 * s;
    let tw = sz * 0.52 * title.len() as f32;
    let logo_x = wf - tw - 140.0 * s; // before window controls
    t.queue(title, sz, logo_x, (h - sz) * 0.5, accent(), wf, sw, sh);
}

/// Draw menu bar labels on the left side of the title bar.
pub fn draw_menu_labels(
    t: &mut TextRenderer, s: f32, title_h: f32, sw: u32, sh: u32, wf: f32,
) {
    let labels = ["File", "Edit", "View", "Clip", "Effects"];
    let sz = 16.0 * s;
    let y = (title_h - sz) * 0.5;
    let mut x = 12.0 * s;
    let col = text();
    for label in &labels {
        t.queue(label, sz, x, y, col, wf, sw, sh);
        x += (label.len() as f32 * sz * 0.55) + 16.0 * s;
    }
}

/// Draw window control buttons (minimize, maximize, close).
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
    let td = text_dim();
    let tx = text();

    // Close — X
    let hov = dist(close_cx) < btn_r;
    if hov { p.circle_filled(close_cx, btn_y, btn_r, close_red()); }
    let ic = if hov { Color::WHITE } else { td };
    p.line(close_cx - x_sz, btn_y - x_sz, close_cx + x_sz, btn_y + x_sz, thick, ic);
    p.line(close_cx - x_sz, btn_y + x_sz, close_cx + x_sz, btn_y - x_sz, thick, ic);

    // Maximize — square
    let hov = dist(max_cx) < btn_r;
    if hov { p.circle_filled(max_cx, btn_y, btn_r, Color::rgba(1.0, 1.0, 1.0, 0.15)); }
    let ic = if hov { tx } else { td };
    p.rect_stroke_sdf(
        Rect::new(max_cx - x_sz, btn_y - x_sz, x_sz * 2.0, x_sz * 2.0),
        1.5 * s, thick, ic,
    );

    // Minimize — line
    let hov = dist(min_cx) < btn_r;
    if hov { p.circle_filled(min_cx, btn_y, btn_r, Color::rgba(1.0, 1.0, 1.0, 0.15)); }
    let ic = if hov { tx } else { td };
    p.line(min_cx - x_sz, btn_y, min_cx + x_sz, btn_y, thick, ic);
}

/// Draw window border.
pub fn draw_border(p: &mut Painter, wf: f32, hf: f32) {
    p.rect_stroke_sdf(Rect::new(0.0, 0.0, wf, hf), 0.0, 1.0, BORDER);
}

/// Draw the 4px rainbow gradient strip horizontally (left→right).
pub fn draw_rainbow_h(p: &mut Painter, x: f32, y: f32, w: f32, s: f32) {
    let rb = rainbow();
    let h = 4.0 * s;
    p.rect_gradient_multi(
        Rect::new(x, y, w, h), 0.0,
        0.0, // angle 0 = left→right
        &[
            (0.0, rb[0]),
            (0.2, rb[1]),
            (0.4, rb[2]),
            (0.6, rb[3]),
            (0.8, rb[4]),
            (1.0, rb[5]),
        ],
    );
}
