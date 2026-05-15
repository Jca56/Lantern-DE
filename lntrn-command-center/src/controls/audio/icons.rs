//! Speaker + mic glyphs drawn from primitives. Both support a muted
//! state (diagonal red slash).

use lntrn_render::{Color, Painter, Rect};

/// Mute slash color — red so it's unambiguous.
pub(super) const MUTE_SLASH_RGB: (u8, u8, u8) = (0xe0, 0x40, 0x40);

/// Original speaker draw used by the expanded view — wraps the
/// colored variant with the default white fill.
pub(super) fn draw_speaker(
    painter: &mut Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    muted: bool,
    alpha: f32,
) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    draw_speaker_colored(painter, x, y, w, h, muted, color);
}

/// Variant of [`draw_speaker`] that accepts an explicit fill color —
/// used by the inline tile so it can recolor the icon gold when the
/// tile is hovered or its view is active.
pub(super) fn draw_speaker_colored(
    painter: &mut Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    muted: bool,
    color: Color,
) {
    let pt = |fx: f32, fy: f32| (x + fx * w, y + fy * h);

    // Speaker silhouette: small box on the left + flared cone on the right.
    //
    //         ╱│
    //   ┌──┐ ╱ │
    //   │  │   │
    //   │  │   │
    //   └──┘ ╲ │
    //         ╲│
    //
    // Decompose into rectangle (the box part) + two triangles (cone).
    // Box: 0.0..0.35 horiz, 0.30..0.70 vert.
    let (bx0, by0) = pt(0.0, 0.30);
    let (bx1, by1) = pt(0.35, 0.70);
    painter.rect_filled(
        Rect::new(bx0, by0, bx1 - bx0, by1 - by0),
        0.0,
        color,
    );
    // Cone — two triangles forming a pentagon. Top half: top-left of box,
    // top-right of cone, midline-right.
    let cone_top_left = pt(0.35, 0.30);
    let cone_top_right = pt(0.95, 0.0);
    let cone_mid_right = pt(0.95, 0.5);
    let cone_bot_left = pt(0.35, 0.70);
    let cone_bot_right = pt(0.95, 1.0);
    painter.triangle(
        cone_top_left.0, cone_top_left.1,
        cone_top_right.0, cone_top_right.1,
        cone_mid_right.0, cone_mid_right.1,
        color,
    );
    painter.triangle(
        cone_top_left.0, cone_top_left.1,
        cone_mid_right.0, cone_mid_right.1,
        cone_bot_left.0, cone_bot_left.1,
        color,
    );
    painter.triangle(
        cone_bot_left.0, cone_bot_left.1,
        cone_mid_right.0, cone_mid_right.1,
        cone_bot_right.0, cone_bot_right.1,
        color,
    );

    if muted {
        // Diagonal red slash — bottom-left to top-right corner.
        let red = Color::from_rgb8(MUTE_SLASH_RGB.0, MUTE_SLASH_RGB.1, MUTE_SLASH_RGB.2)
            .with_alpha(color.a);
        let p1 = pt(0.0, 1.0);
        let p2 = pt(1.0, 0.0);
        painter.line(p1.0, p1.1, p2.0, p2.1, w * 0.12, red);
    }
}

/// Draw a stylised microphone — rounded "head" capsule + thin neck +
/// wide base. When muted, the same red diagonal slash as the speaker.
pub(super) fn draw_mic(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, muted: bool, alpha: f32) {
    let pt = |fx: f32, fy: f32| (x + fx * w, y + fy * h);
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);

    // Head: rounded vertical capsule centered on x, y in [0.10, 0.65].
    let head_w = w * 0.45;
    let head_x = x + (w - head_w) / 2.0;
    let head_top = y + 0.10 * h;
    let head_h = 0.55 * h;
    painter.rect_filled(
        Rect::new(head_x, head_top, head_w, head_h),
        head_w * 0.5,
        color,
    );

    // Neck — thin vertical strip from head bottom to base top.
    let neck_w = w * 0.12;
    let neck_x = x + (w - neck_w) / 2.0;
    let neck_top = head_top + head_h;
    let neck_h = 0.18 * h;
    painter.rect_filled(
        Rect::new(neck_x, neck_top, neck_w, neck_h),
        0.0,
        color,
    );

    // Base — wider horizontal strip at the bottom.
    let base_w = w * 0.70;
    let base_x = x + (w - base_w) / 2.0;
    let base_top = neck_top + neck_h;
    let base_h = 0.08 * h;
    painter.rect_filled(
        Rect::new(base_x, base_top, base_w, base_h),
        base_h * 0.5,
        color,
    );

    if muted {
        let red = Color::from_rgb8(MUTE_SLASH_RGB.0, MUTE_SLASH_RGB.1, MUTE_SLASH_RGB.2)
            .with_alpha(alpha);
        let p1 = pt(0.05, 0.95);
        let p2 = pt(0.95, 0.05);
        painter.line(p1.0, p1.1, p2.0, p2.1, w * 0.12, red);
    }
}
