//! Small marks drawn over an entry's icon.
//!
//! Icon textures are drawn after everything the painter put on the base
//! layer, so a mark painted there with the row would end up underneath a
//! thumbnail. The views therefore only say where a mark goes (`mark_link`)
//! and the frame draws them all once the overlay layer is up
//! (`draw_marked`), the way the play badges of video thumbnails are.

use std::cell::RefCell;

use lntrn_render::{Painter, Rect};
use lntrn_ui::gpu::FoxPalette;

struct Mark {
    cx: f32,
    cy: f32,
    r: f32,
    alpha: f32,
    /// The content area of the pane the entry is in: a row scrolled half
    /// out of it must not paint its mark over the chrome.
    clip: Rect,
}

thread_local! {
    /// The marks of the frame being built. Drawing is single-threaded;
    /// this saves handing a collector through every view function.
    static MARKS: RefCell<Vec<Mark>> = const { RefCell::new(Vec::new()) };
}

/// A new frame: whatever an abandoned one left behind goes.
pub fn start_frame() {
    MARKS.with(|marks| marks.borrow_mut().clear());
}

/// The entry whose icon has its bottom-left corner at `(left, bottom)` is
/// a symbolic link. `r` is the mark's radius.
pub fn mark_link(left: f32, bottom: f32, r: f32, alpha: f32, clip: Rect) {
    MARKS.with(|marks| {
        marks.borrow_mut().push(Mark {
            cx: left + r * 0.7,
            cy: bottom - r * 0.7,
            r,
            alpha,
            clip,
        })
    });
}

/// Draw the marks collected since `start_frame`. Call with the painter on
/// the overlay layer.
pub fn draw_marked(painter: &mut Painter, palette: &FoxPalette) {
    let marks = MARKS.with(|marks| std::mem::take(&mut *marks.borrow_mut()));
    for mark in marks {
        painter.push_clip(mark.clip);
        draw_link_emblem(painter, palette, mark.cx, mark.cy, mark.r, mark.alpha);
        painter.pop_clip();
    }
}

/// The mark of a symbolic link: an arrow in a disc. It sits at the icon's
/// bottom-left corner (the git dot has the bottom-right one), so a link to
/// a folder can be told from the folder at a glance: deleting it removes
/// the link, not what it shows.
fn draw_link_emblem(
    painter: &mut Painter,
    palette: &FoxPalette,
    cx: f32,
    cy: f32,
    r: f32,
    alpha: f32,
) {
    let ring = r * 0.2;
    painter.circle_filled(cx, cy, r + ring, palette.surface.with_alpha(alpha));
    painter.circle_filled(cx, cy, r, palette.text.with_alpha(alpha));
    // An arrow pointing up and to the right, in the disc's inverse colour.
    let ink = palette.surface.with_alpha(alpha);
    let a = r * 0.42;
    let w = (r * 0.28).max(1.0);
    painter.line(cx - a, cy + a, cx + a, cy - a, w, ink);
    painter.line(cx - a * 0.4, cy - a, cx + a, cy - a, w, ink);
    painter.line(cx + a, cy - a, cx + a, cy + a * 0.4, w, ink);
}
