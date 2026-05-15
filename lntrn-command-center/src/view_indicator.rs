//! Top-strip icons above the panel.
//!
//! Layout (from left to right, in the margin above the panel):
//!   LEFT  : [Gear (Settings)] [Home] [Grow (Size)] [Desktop]
//!   CENTER: view dots
//!   RIGHT : [Usage (Claude)] [Emoji] [Clipboard] [Notes]
//!
//! Desktop + Usage glyphs themselves are drawn by `desktop_settings.rs`
//! and `usage_button.rs`; this module owns their rects so all top-strip
//! buttons line up against the same horizontal baseline.

use lntrn_render::{Color, Painter, Rect};

use crate::app::PanelView;

/// Dot diameter and gap (logical px).
pub const DOT_DIAMETER: f32 = 10.0;
pub const DOT_GAP: f32 = 16.0;
/// Vertical center of the dot row relative to the panel top edge — the
/// dots sit *above* the panel inside the top margin.
pub const STRIP_OFFSET: f32 = 22.0;

/// Standard glyph size for the four big-button icons on the strip.
pub const BTN_SIZE: f32 = 34.0;
/// Gear is slightly smaller — gear teeth read OK at this size.
pub const GEAR_SIZE: f32 = 27.0;
/// Gap between adjacent buttons on the same side.
pub const BTN_GAP: f32 = 14.0;
/// Outer padding from the panel edge to the first button on each side.
pub const SIDE_PAD: f32 = 24.0;

const ACTIVE_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const INACTIVE_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const INACTIVE_ALPHA: f32 = 0.35;
const HOME_IDLE_ALPHA: f32 = 0.70;

/// Rect for the i-th view dot. Used for both rendering and clickable
/// hit-zones (each dot is a tiny shortcut to that view).
pub fn dot_rect(panel: Rect, scale: f32, idx: usize) -> Rect {
    let d = DOT_DIAMETER * scale;
    let gap = DOT_GAP * scale;
    let n = PanelView::ALL.len() as f32;
    let total = n * d + (n - 1.0) * gap;
    let start_x = panel.x + (panel.w - total) / 2.0;
    let cy = panel.y - STRIP_OFFSET * scale;
    let x = start_x + idx as f32 * (d + gap);
    let y = cy - d / 2.0;
    Rect::new(x, y, d, d)
}

/// Slightly enlarged hit-rect for the dots so they're easier to click
/// than the tiny visible circle suggests.
pub fn dot_hit_rect(panel: Rect, scale: f32, idx: usize) -> Rect {
    let base = dot_rect(panel, scale, idx);
    let pad = 6.0 * scale;
    Rect::new(base.x - pad, base.y - pad, base.w + pad * 2.0, base.h + pad * 2.0)
}

pub fn hit_test_dot(panel: Rect, scale: f32, px: f32, py: f32) -> Option<usize> {
    for i in 0..PanelView::ALL.len() {
        let r = dot_hit_rect(panel, scale, i);
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            return Some(i);
        }
    }
    None
}

/// Compute a rect for the `idx`-th button on the LEFT side, sized to
/// `size` logical px. Slots line up against a shared centerline.
fn left_slot_at(panel: Rect, scale: f32, start_logical_x: f32, size: f32) -> Rect {
    let s = size * scale;
    let x = panel.x + start_logical_x * scale;
    let cy = panel.y - STRIP_OFFSET * scale;
    let y = cy - s / 2.0;
    Rect::new(x, y, s, s)
}

/// Compute a rect for an icon on the RIGHT side whose right edge sits
/// `right_logical_pad` from the panel right edge.
fn right_slot_at(panel: Rect, scale: f32, right_logical_pad: f32, size: f32) -> Rect {
    let s = size * scale;
    let x = panel.x + panel.w - s - right_logical_pad * scale;
    let cy = panel.y - STRIP_OFFSET * scale;
    let y = cy - s / 2.0;
    Rect::new(x, y, s, s)
}

/// Per-slot logical sizes for the LEFT strip in render order.
/// 0 = Gear (Settings), 1 = Home, 2 = Grow (Size), 3 = Desktop.
const LEFT_SLOT_SIZES: [f32; 4] = [GEAR_SIZE, BTN_SIZE, BTN_SIZE, BTN_SIZE];

/// X offset (logical, from panel left edge) of slot `idx`'s left edge.
/// Walks the slot list cumulatively so a smaller gear at the front
/// doesn't push the later slots off-center.
fn left_slot_x_logical(idx: usize) -> f32 {
    let mut x = SIDE_PAD;
    for i in 0..idx.min(LEFT_SLOT_SIZES.len()) {
        x += LEFT_SLOT_SIZES[i] + BTN_GAP;
    }
    x
}

/// X offset (logical, from panel right edge) of each right-side slot.
/// idx 0 = rightmost (Notes), then Clipboard, Emoji, Usage.
fn right_slot_pad_logical(idx: usize) -> f32 {
    SIDE_PAD + idx as f32 * (BTN_SIZE + BTN_GAP)
}

pub fn gear_rect(panel: Rect, scale: f32) -> Rect {
    left_slot_at(panel, scale, left_slot_x_logical(0), GEAR_SIZE)
}
pub fn home_rect(panel: Rect, scale: f32) -> Rect {
    left_slot_at(panel, scale, left_slot_x_logical(1), BTN_SIZE)
}
pub fn grow_rect(panel: Rect, scale: f32) -> Rect {
    left_slot_at(panel, scale, left_slot_x_logical(2), BTN_SIZE)
}
pub fn desktop_button_rect(panel: Rect, scale: f32) -> Rect {
    left_slot_at(panel, scale, left_slot_x_logical(3), BTN_SIZE)
}

pub fn notes_rect(panel: Rect, scale: f32) -> Rect {
    right_slot_at(panel, scale, right_slot_pad_logical(0), BTN_SIZE)
}
pub fn clipboard_rect(panel: Rect, scale: f32) -> Rect {
    right_slot_at(panel, scale, right_slot_pad_logical(1), BTN_SIZE)
}
pub fn emoji_rect(panel: Rect, scale: f32) -> Rect {
    right_slot_at(panel, scale, right_slot_pad_logical(2), BTN_SIZE)
}
pub fn usage_rect(panel: Rect, scale: f32) -> Rect {
    right_slot_at(panel, scale, right_slot_pad_logical(3), BTN_SIZE)
}

fn point_in(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

pub fn hit_home(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    point_in(home_rect(panel, scale), px, py)
}
pub fn hit_grow(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    point_in(grow_rect(panel, scale), px, py)
}
pub fn hit_gear(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    point_in(gear_rect(panel, scale), px, py)
}
pub fn hit_emoji(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    point_in(emoji_rect(panel, scale), px, py)
}
pub fn hit_clipboard(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    point_in(clipboard_rect(panel, scale), px, py)
}
pub fn hit_notes(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    point_in(notes_rect(panel, scale), px, py)
}

fn glyph_color(hovered: bool, active: bool, alpha: f32) -> Color {
    if hovered || active {
        Color::from_rgb8(ACTIVE_RGB.0, ACTIVE_RGB.1, ACTIVE_RGB.2).with_alpha(alpha)
    } else {
        Color::from_rgb8(INACTIVE_RGB.0, INACTIVE_RGB.1, INACTIVE_RGB.2)
            .with_alpha(HOME_IDLE_ALPHA * alpha)
    }
}

pub fn draw_gear(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered: bool,
    active: bool,
) {
    let r = gear_rect(panel, scale);
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let color = glyph_color(hovered, active, alpha);
    let body_r = r.w * 0.30;
    let tooth_inner = body_r;
    let tooth_outer = r.w * 0.46;
    let tooth_w = 3.5 * scale;
    let stroke = 2.0 * scale;

    for i in 0..8 {
        let theta = i as f32 * std::f32::consts::FRAC_PI_4;
        let (sin, cos) = theta.sin_cos();
        let x1 = cx + cos * tooth_inner;
        let y1 = cy + sin * tooth_inner;
        let x2 = cx + cos * tooth_outer;
        let y2 = cy + sin * tooth_outer;
        painter.line_round(x1, y1, x2, y2, tooth_w, color);
    }

    let ring_d = body_r * 2.0;
    painter.rect_stroke_sdf(
        Rect::new(cx - body_r, cy - body_r, ring_d, ring_d),
        body_r,
        stroke,
        color,
    );
    painter.circle_filled(cx, cy, r.w * 0.10, color);
}

pub fn draw_dots(painter: &mut Painter, panel: Rect, scale: f32, alpha: f32, current: PanelView) {
    for (i, view) in PanelView::ALL.iter().enumerate() {
        let r = dot_rect(panel, scale, i);
        let cx = r.x + r.w / 2.0;
        let cy = r.y + r.h / 2.0;
        let radius = r.w / 2.0;
        if *view == current {
            painter.circle_filled(
                cx,
                cy,
                radius,
                Color::from_rgb8(ACTIVE_RGB.0, ACTIVE_RGB.1, ACTIVE_RGB.2).with_alpha(alpha),
            );
        } else {
            painter.circle_filled(
                cx,
                cy,
                radius,
                Color::from_rgb8(INACTIVE_RGB.0, INACTIVE_RGB.1, INACTIVE_RGB.2)
                    .with_alpha(INACTIVE_ALPHA * alpha),
            );
        }
    }
}

pub fn draw_grow(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    grown: bool,
    hovered: bool,
) {
    let r = grow_rect(panel, scale);
    let stroke = 2.0 * scale;
    let color = glyph_color(hovered, grown, alpha);

    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let arm = r.w * 0.32;
    let head = arm * 0.45;
    if grown {
        painter.line_round(cx - arm, cy - arm, cx - arm * 0.4, cy - arm * 0.4, stroke, color);
        painter.line_round(cx - arm * 0.4, cy - arm * 0.4, cx - arm * 0.4 + head, cy - arm * 0.4, stroke, color);
        painter.line_round(cx - arm * 0.4, cy - arm * 0.4, cx - arm * 0.4, cy - arm * 0.4 + head, stroke, color);
        painter.line_round(cx + arm, cy + arm, cx + arm * 0.4, cy + arm * 0.4, stroke, color);
        painter.line_round(cx + arm * 0.4, cy + arm * 0.4, cx + arm * 0.4 - head, cy + arm * 0.4, stroke, color);
        painter.line_round(cx + arm * 0.4, cy + arm * 0.4, cx + arm * 0.4, cy + arm * 0.4 - head, stroke, color);
    } else {
        painter.line_round(cx - arm * 0.4, cy - arm * 0.4, cx - arm, cy - arm, stroke, color);
        painter.line_round(cx - arm, cy - arm, cx - arm + head, cy - arm, stroke, color);
        painter.line_round(cx - arm, cy - arm, cx - arm, cy - arm + head, stroke, color);
        painter.line_round(cx + arm * 0.4, cy + arm * 0.4, cx + arm, cy + arm, stroke, color);
        painter.line_round(cx + arm, cy + arm, cx + arm - head, cy + arm, stroke, color);
        painter.line_round(cx + arm, cy + arm, cx + arm, cy + arm - head, stroke, color);
    }
}

pub fn draw_home(painter: &mut Painter, panel: Rect, scale: f32, alpha: f32, hovered: bool) {
    let r = home_rect(panel, scale);
    let stroke = 2.0 * scale;
    let color = glyph_color(hovered, false, alpha);

    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let half = r.w * 0.40;
    let body_top = cy - half * 0.20;
    let body_bot = cy + half * 0.55;
    let body_left = cx - half;
    let body_right = cx + half;
    let roof_apex_y = cy - half * 0.55;

    painter.line_round(body_left - 2.0 * scale, body_top, cx, roof_apex_y, stroke, color);
    painter.line_round(cx, roof_apex_y, body_right + 2.0 * scale, body_top, stroke, color);
    painter.line_round(body_left, body_top, body_left, body_bot, stroke, color);
    painter.line_round(body_right, body_top, body_right, body_bot, stroke, color);
    painter.line_round(body_left, body_bot, body_right, body_bot, stroke, color);
    let door_w = half * 0.35;
    let door_h = half * 0.55;
    painter.rect_filled(
        Rect::new(cx - door_w / 2.0, body_bot - door_h, door_w, door_h),
        1.0 * scale,
        color,
    );
}

/// Smiley-face glyph for the Emojis page.
pub fn draw_emoji(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    active: bool,
    hovered: bool,
) {
    let r = emoji_rect(panel, scale);
    let stroke = 2.0 * scale;
    let color = glyph_color(hovered, active, alpha);

    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let face_r = r.w * 0.42;

    // Outer face circle (drawn as an SDF rounded square with corner =
    // radius → reads as a circle outline).
    let d = face_r * 2.0;
    painter.rect_stroke_sdf(
        Rect::new(cx - face_r, cy - face_r, d, d),
        face_r,
        stroke,
        color,
    );

    // Eyes — two small filled dots, slightly above center.
    let eye_r = r.w * 0.06;
    let eye_dx = face_r * 0.42;
    let eye_dy = -face_r * 0.18;
    painter.circle_filled(cx - eye_dx, cy + eye_dy, eye_r, color);
    painter.circle_filled(cx + eye_dx, cy + eye_dy, eye_r, color);

    // Smile — approximate an arc with three short line segments below
    // center.
    let smile_y = cy + face_r * 0.20;
    let smile_w = face_r * 0.60;
    let dip = face_r * 0.22;
    let x0 = cx - smile_w;
    let x1 = cx - smile_w * 0.4;
    let x2 = cx + smile_w * 0.4;
    let x3 = cx + smile_w;
    let y_corner = smile_y + dip * 0.55;
    let y_bot = smile_y + dip;
    painter.line_round(x0, smile_y, x1, y_corner, stroke, color);
    painter.line_round(x1, y_corner, x2, y_bot, stroke, color);
    painter.line_round(x2, y_bot, x3, smile_y - dip * 0.05, stroke, color);
    // Lift the right end slightly so the arc reads symmetrically.
    painter.line_round(x2, y_bot, x3, y_corner, stroke, color);
}

/// Clipboard glyph for the Clipboard History page.
pub fn draw_clipboard(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    active: bool,
    hovered: bool,
) {
    let r = clipboard_rect(panel, scale);
    let stroke = 2.0 * scale;
    let color = glyph_color(hovered, active, alpha);

    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;

    // Board body — tall rounded rect.
    let body_w = r.w * 0.62;
    let body_h = r.w * 0.72;
    let body_x = cx - body_w / 2.0;
    let body_y = cy - body_h / 2.0 + r.w * 0.04;
    let radius = r.w * 0.10;
    painter.rect_stroke_sdf(
        Rect::new(body_x, body_y, body_w, body_h),
        radius,
        stroke,
        color,
    );

    // Clip tab at the top — small horizontal pill.
    let clip_w = body_w * 0.48;
    let clip_h = r.w * 0.14;
    let clip_x = cx - clip_w / 2.0;
    let clip_y = body_y - clip_h / 2.0;
    painter.rect_stroke_sdf(
        Rect::new(clip_x, clip_y, clip_w, clip_h),
        clip_h / 2.0,
        stroke,
        color,
    );

    // Three short horizontal lines as "content".
    let line_w = body_w * 0.55;
    let line_x = cx - line_w / 2.0;
    let line_step = body_h * 0.20;
    let line_y0 = body_y + body_h * 0.32;
    for i in 0..3 {
        let y = line_y0 + i as f32 * line_step;
        painter.line_round(line_x, y, line_x + line_w, y, stroke * 0.85, color);
    }
}

/// Notepad glyph for the Quick Notes page.
pub fn draw_notes(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    active: bool,
    hovered: bool,
) {
    let r = notes_rect(panel, scale);
    let stroke = 2.0 * scale;
    let color = glyph_color(hovered, active, alpha);

    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;

    // Pad body — rounded rect, slightly wider than the clipboard.
    let body_w = r.w * 0.68;
    let body_h = r.w * 0.74;
    let body_x = cx - body_w / 2.0;
    let body_y = cy - body_h / 2.0 + r.w * 0.06;
    let radius = r.w * 0.10;
    painter.rect_stroke_sdf(
        Rect::new(body_x, body_y, body_w, body_h),
        radius,
        stroke,
        color,
    );

    // Spiral binding — five small dots along the top edge.
    let dot_r = r.w * 0.045;
    let binding_y = body_y - dot_r * 0.4;
    let binding_step = body_w / 6.0;
    for i in 0..5 {
        let dx = body_x + binding_step * (i as f32 + 1.0);
        painter.circle_filled(dx, binding_y, dot_r, color);
    }

    // Folded corner — a small triangle missing from the top-right.
    let fold = r.w * 0.14;
    let fr_x = body_x + body_w - fold;
    let fr_y = body_y;
    painter.line_round(fr_x, fr_y, fr_x + fold, fr_y + fold, stroke, color);
    painter.line_round(fr_x, fr_y, fr_x, fr_y + fold, stroke * 0.7, color);
    painter.line_round(fr_x, fr_y + fold, fr_x + fold, fr_y + fold, stroke * 0.7, color);

    // Three short horizontal text lines.
    let line_w = body_w * 0.55;
    let line_x = body_x + body_w * 0.16;
    let line_step = body_h * 0.20;
    let line_y0 = body_y + body_h * 0.34;
    for i in 0..3 {
        let y = line_y0 + i as f32 * line_step;
        let w = if i == 2 { line_w * 0.65 } else { line_w };
        painter.line_round(line_x, y, line_x + w, y, stroke * 0.85, color);
    }
}

