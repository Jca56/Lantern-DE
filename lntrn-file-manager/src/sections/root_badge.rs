//! The ROOT badge: what makes root mode impossible to overlook.
//!
//! It sits at the head of the path strip of the pane that is in root mode,
//! in the nav bar, which is on screen in every view and layout. The path
//! gives up the room, so nothing is covered. A frame in the same colour
//! runs around that pane's file list. Clicking the badge leaves root mode.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

const LABEL: &str = "ROOT";
/// Same size as the breadcrumbs it stands next to.
const FONT: f32 = 22.0;
const PAD: f32 = 14.0;
/// Room for the "leave" cross after the word.
const CROSS: f32 = 26.0;

fn label_width(text: &mut TextRenderer, s: f32) -> f32 {
    text.measure_width_styled(
        LABEL,
        FONT * s,
        lntrn_render::FontWeight::Bold,
        lntrn_render::FontStyle::Normal,
    )
}

/// Narrowest the badge gets: the word, without the cross. A split pane
/// keeps at least this much strip while it is in root mode, at the cost of
/// its buttons if need be (layout/pane_nav.rs).
pub fn min_width(text: &mut TextRenderer, s: f32) -> f32 {
    PAD * s * 2.0 + label_width(text, s)
}

/// Gap between the badge and what is left of the strip.
pub const GAP: f32 = 10.0;

/// Split the path strip: the badge's rect at its head and what is left for
/// the path. Without root mode the strip is returned whole.
pub fn carve(path: Rect, root: bool, text: &mut TextRenderer, s: f32) -> (Rect, Option<Rect>) {
    if !root {
        return (path, None);
    }
    let gap = GAP * s;
    let compact = min_width(text, s);
    let full = compact + CROSS * s;
    // A narrow split pane has no room for the cross; the word stays, at
    // full size, even if the path is squeezed out entirely.
    let w = if path.w >= full + gap + 60.0 * s {
        full
    } else {
        compact
    };
    let badge = Rect::new(path.x, path.y, w, path.h);
    let rest = Rect::new(
        path.x + w + gap,
        path.y,
        (path.w - w - gap).max(0.0),
        path.h,
    );
    (rest, Some(badge))
}

/// Draw the badge and, when `zone` is given, make it the button that
/// leaves root mode.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    pal: &FoxPalette,
    badge: Rect,
    zone: Option<u32>,
    screen: (u32, u32),
    s: f32,
) {
    let hovered = zone.is_some_and(|id| input.add_zone(id, badge).is_hovered());
    let fill = if hovered {
        Color::rgba(
            (pal.danger.r + 0.1).min(1.0),
            (pal.danger.g + 0.1).min(1.0),
            (pal.danger.b + 0.1).min(1.0),
            1.0,
        )
    } else {
        pal.danger.with_alpha(1.0)
    };
    painter.rect_filled(badge, 8.0 * s, fill);

    let font = FONT * s;
    let pad = PAD * s;
    let label_w = label_width(text, s);
    TextLabel::new(LABEL, badge.x + pad, badge.y + (badge.h - font) * 0.5)
        .size(FontSize::Custom(font))
        .color(Color::WHITE)
        .bold()
        // Slack so the measured last glyph isn't clipped by the bound.
        .max_width(label_w + 8.0 * s)
        .draw(text, screen.0, screen.1);

    // The cross that says "click to leave", when `carve` left room for it.
    if zone.is_none() || badge.w < pad * 2.0 + label_w + CROSS * s - 0.5 {
        return;
    }
    let arm = 6.0 * s;
    let cx = badge.x + badge.w - pad - arm;
    let cy = badge.y + badge.h * 0.5;
    let white = Color::WHITE.with_alpha(if hovered { 1.0 } else { 0.85 });
    painter.line(cx - arm, cy - arm, cx + arm, cy + arm, 2.5 * s, white);
    painter.line(cx - arm, cy + arm, cx + arm, cy - arm, 2.5 * s, white);
}

/// The frame around the file list of the pane in root mode.
pub fn draw_frame(painter: &mut Painter, pal: &FoxPalette, content: Rect, s: f32) {
    // Four bars along the inside of the rect's edges.
    painter.rect_stroke(content, 0.0, 3.0 * s, pal.danger.with_alpha(0.9));
}
