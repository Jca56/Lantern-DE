use lntrn_render::{Color, Painter, Rect, TextRenderer};

pub const TAB_NAMES: &[&str] = &["System", "Processes", "Performance"];

const TEXT_PRIMARY: Color   = Color::rgb(0.88, 0.85, 0.95);
const TEXT_MUTED: Color     = Color::rgb(0.50, 0.45, 0.62);
const ACCENT: Color         = Color::rgb(0.25, 0.65, 0.90);
const SURFACE: Color        = Color::rgba(0.08, 0.04, 0.16, 0.40);
const BORDER: Color         = Color::rgba(0.30, 0.20, 0.50, 0.12);

fn in_rect(cx: f32, cy: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    cx >= x && cx <= x + w && cy >= y && cy <= y + h
}

pub fn handle_click(cx: f32, cy: f32, s: f32, y: f32, active: &mut usize) -> bool {
    let tab_h = 36.0 * s;
    let pad = 32.0 * s;
    let mut tx = pad;
    for i in 0..TAB_NAMES.len() {
        let tw = TAB_NAMES[i].len() as f32 * 11.0 * s + 28.0 * s;
        if in_rect(cx, cy, tx, y, tw, tab_h) {
            *active = i;
            return true;
        }
        tx += tw + 8.0 * s;
    }
    false
}

pub fn draw(
    p: &mut Painter, t: &mut TextRenderer,
    cx: f32, cy: f32, s: f32, y: f32, wf: f32,
    active: usize, sw: u32, sh: u32,
) -> f32 {
    let tab_h = 36.0 * s;
    let pad = 32.0 * s;
    let mut tx = pad;

    for i in 0..TAB_NAMES.len() {
        let tw = TAB_NAMES[i].len() as f32 * 11.0 * s + 28.0 * s;
        let is_active = active == i;
        let hov = in_rect(cx, cy, tx, y, tw, tab_h);

        if is_active {
            p.rect_filled(Rect::new(tx, y, tw, tab_h), 8.0 * s, SURFACE);
            p.rect_filled(
                Rect::new(tx + 10.0 * s, y + tab_h - 3.0 * s, tw - 20.0 * s, 3.0 * s),
                1.5 * s, ACCENT,
            );
        } else if hov {
            p.rect_filled(Rect::new(tx, y, tw, tab_h), 8.0 * s,
                Color::rgba(0.06, 0.03, 0.12, 0.25));
        }

        let tc = if is_active { TEXT_PRIMARY } else { TEXT_MUTED };
        t.queue(TAB_NAMES[i], 16.0 * s, tx + 14.0 * s, y + 8.0 * s, tc, wf, sw, sh);
        tx += tw + 8.0 * s;
    }

    // Bottom line
    p.rect_filled(
        Rect::new(pad, y + tab_h, wf - pad * 2.0, 1.0 * s), 0.0, BORDER,
    );

    y + tab_h + 1.0 * s
}
