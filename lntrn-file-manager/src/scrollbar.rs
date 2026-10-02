//! Scrollbars sized with the display scale, and the two that are not the
//! file list's: the sidebar's and the Properties dialog's.
//!
//! lntrn-ui's `Scrollbar` is built from fixed physical pixels (a 14px bar,
//! a 30px minimum thumb, a 10px grab margin) and takes no scale, while
//! everything around it in Fox is multiplied by the display scale: at 1.4
//! the bar and its grab band were a third smaller than at 1.0. The geometry
//! is worked out here instead, scaled, and handed back as the same
//! `Scrollbar` (its fields are public), so its drag maths is still the
//! widget's own.

use lntrn_render::{Painter, Rect};
use lntrn_ui::gpu::{FoxPalette, InteractionState, Scrollbar};

use crate::app::App;

/// Logical px. Generous on purpose: the bar is something to grab.
const WIDTH: f32 = 16.0;
const PAD: f32 = 4.0;
const MIN_THUMB_H: f32 = 48.0;
/// How far past the bar, on both sides, a press still takes hold of it.
const GRAB_MARGIN: f32 = 10.0;

/// Width a scrollbar takes at the right of its viewport.
pub fn gutter(s: f32) -> f32 {
    (WIDTH + 2.0 * PAD) * s
}

/// Track and thumb for `viewport` showing `content_height` of content
/// scrolled by `offset`.
pub fn bar(viewport: &Rect, content_height: f32, offset: f32, s: f32) -> Scrollbar {
    let pad = PAD * s;
    let track = Rect::new(
        viewport.x + viewport.w - WIDTH * s - pad,
        viewport.y + pad,
        WIDTH * s,
        (viewport.h - pad * 2.0).max(0.0),
    );
    let visible_ratio = if content_height > 0.0 {
        (viewport.h / content_height).min(1.0)
    } else {
        1.0
    };
    let thumb_h = (track.h * visible_ratio).max(MIN_THUMB_H * s).min(track.h);
    let max_offset = (content_height - viewport.h).max(0.0);
    let fraction = if max_offset > 0.0 {
        (offset / max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb_y = track.y + (track.h - thumb_h) * fraction;
    Scrollbar {
        track,
        thumb: Rect::new(track.x, thumb_y, track.w, thumb_h),
        visible_ratio,
    }
}

/// The band that takes hold of the bar: wider than what is drawn.
pub fn hover_zone(bar: &Scrollbar, s: f32) -> Rect {
    let margin = GRAB_MARGIN * s;
    Rect::new(
        bar.track.x - margin,
        bar.track.y,
        bar.track.w + margin * 2.0,
        bar.track.h,
    )
}

/// Draw track and thumb. The file list's bar hides until the pointer is on
/// it (`always: false`); the sidebar's and the dialog's stay visible, since
/// they are the only sign that there is more to see.
pub fn draw(
    painter: &mut Painter,
    bar: &Scrollbar,
    state: InteractionState,
    palette: &FoxPalette,
    always: bool,
) {
    if bar.visible_ratio >= 1.0 {
        return;
    }
    let idle = matches!(state, InteractionState::Idle);
    if idle && !always {
        return;
    }
    let radius = bar.track.w * 0.5;
    let track_alpha = if idle { 0.15 } else { 0.3 };
    painter.rect_filled(bar.track, radius, palette.bg.with_alpha(track_alpha));
    let thumb = match state {
        InteractionState::Pressed | InteractionState::Dragging => palette.accent,
        InteractionState::Hovered => palette.text_secondary.with_alpha(0.7),
        InteractionState::Idle => palette.text_secondary.with_alpha(0.35),
    };
    painter.rect_filled(bar.thumb, radius, thumb);
}

/// Where a press at `cy` takes hold of `bar`: the distance from the thumb's
/// top to keep under the pointer, and, for a press on the track, the offset
/// that brings the thumb's middle there first.
fn grab(bar: &Scrollbar, cy: f32, content_h: f32, viewport_h: f32) -> (f32, Option<f32>) {
    if cy >= bar.thumb.y && cy <= bar.thumb.y + bar.thumb.h {
        (cy - bar.thumb.y, None)
    } else {
        (
            bar.thumb.h * 0.5,
            Some(bar.offset_for_thumb_y(cy, content_h, viewport_h)),
        )
    }
}

/// The offset that keeps the grabbed point of the thumb under `cy`.
fn follow(bar: &Scrollbar, grab_dy: f32, cy: f32, content_h: f32, viewport_h: f32) -> f32 {
    bar.offset_for_thumb_y(cy - grab_dy + bar.thumb.h * 0.5, content_h, viewport_h)
}

// ── The sidebar's and the Properties dialog's bars ─────────────────────────

/// The sidebar's scrollbar: at the right of its strip, left of the
/// gradient line that separates it from the file list.
pub fn sidebar_bar(layout: &crate::layout::SidebarLayout, s: f32) -> Option<Scrollbar> {
    if !layout.scrollable() {
        return None;
    }
    let v = layout.viewport;
    let strip = Rect::new(v.x, v.y, v.w - 4.0 * s, v.h);
    Some(bar(&strip, layout.content_h, layout.scroll, s))
}

/// Room the sidebar's rows leave for that bar.
pub fn sidebar_gutter(s: f32) -> f32 {
    gutter(s) + 2.0 * s
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Aside {
    Sidebar,
    Properties,
}

/// A scrollbar thumb being dragged, other than the file list's.
#[derive(Clone, Copy, Debug)]
pub struct AsideDrag {
    which: Aside,
    grab_dy: f32,
}

/// The left button went down at `cy` on the sidebar's scrollbar.
pub fn press_sidebar(app: &mut App, cy: f32, hf: f32, s: f32) -> Option<AsideDrag> {
    let layout = app.sidebar_layout(hf, s);
    let bar = sidebar_bar(&layout, s)?;
    let (grab_dy, jump) = grab(&bar, cy, layout.content_h, layout.viewport.h);
    if let Some(offset) = jump {
        app.sidebar_scroll = offset;
    }
    Some(AsideDrag {
        which: Aside::Sidebar,
        grab_dy,
    })
}

/// The left button went down at `cy` on the Properties dialog's scrollbar.
pub fn press_properties(app: &mut App, cy: f32, s: f32) -> Option<AsideDrag> {
    let props = app.properties.as_mut()?;
    let view = props.scroll_view?;
    let bar = bar(&view.viewport, view.content_h, *props.scroll_slot(), s);
    let (grab_dy, jump) = grab(&bar, cy, view.content_h, view.viewport.h);
    if let Some(offset) = jump {
        *props.scroll_slot() = offset;
    }
    Some(AsideDrag {
        which: Aside::Properties,
        grab_dy,
    })
}

impl AsideDrag {
    /// The pointer is at `cy` with the button still down.
    pub fn follow(&self, app: &mut App, cy: f32, hf: f32, s: f32) {
        match self.which {
            Aside::Sidebar => {
                let layout = app.sidebar_layout(hf, s);
                if let Some(bar) = sidebar_bar(&layout, s) {
                    app.sidebar_scroll =
                        follow(&bar, self.grab_dy, cy, layout.content_h, layout.viewport.h);
                }
            }
            Aside::Properties => {
                let Some(props) = app.properties.as_mut() else {
                    return;
                };
                let Some(view) = props.scroll_view else {
                    return;
                };
                let bar = bar(&view.viewport, view.content_h, *props.scroll_slot(), s);
                *props.scroll_slot() =
                    follow(&bar, self.grab_dy, cy, view.content_h, view.viewport.h);
            }
        }
    }
}

/// A wheel turn of `delta` px with the pointer at `(cx, cy)`: taken here
/// when it belongs to the Properties dialog (open: it covers the window, so
/// the wheel is its own) or to the sidebar (pointer over it, and more rows
/// than fit). False when it is the file list's.
pub fn wheel_aside(app: &mut App, delta: f32, cx: f32, cy: f32, hf: f32, s: f32) -> bool {
    // A dialog covers the window: the wheel is its own (the list of a
    // cloud dialog scrolls), and never moves what is behind it.
    if app.op_dialog_open() {
        app.cloud_list_wheel(delta);
        return true;
    }
    if let Some(props) = app.properties.as_mut() {
        // Clamped against the real heights when the dialog is next drawn.
        // (Nothing scrolls on the picker's Custom tab: nothing to move.)
        if props.scroll_view.is_some() {
            *props.scroll_slot() += delta;
        }
        return true;
    }
    // Only a sidebar with something to scroll takes the wheel; over one
    // that fits, it moves the file list as it always did.
    let layout = app.sidebar_layout(hf, s);
    if layout.scrollable() && layout.viewport.contains(cx, cy) {
        app.sidebar_scroll = (layout.scroll + delta).clamp(0.0, layout.max_scroll());
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_grows_with_the_display_scale() {
        let viewport = Rect::new(100.0, 50.0, 800.0, 600.0);
        let at_1 = bar(&viewport, 60_000.0, 0.0, 1.0);
        let at_14 = bar(&viewport, 60_000.0, 0.0, 1.4);
        assert_eq!(at_1.track.w, WIDTH);
        assert!((at_14.track.w - WIDTH * 1.4).abs() < 0.01);
        // A long list: the thumb stops shrinking at a size that can be hit.
        assert_eq!(at_1.thumb.h, MIN_THUMB_H);
        assert!((at_14.thumb.h - MIN_THUMB_H * 1.4).abs() < 0.01);
        assert!((hover_zone(&at_14, 1.4).w - (WIDTH + 2.0 * GRAB_MARGIN) * 1.4).abs() < 0.01);
        // Inside the viewport, against its right edge.
        assert!((at_14.track.x + at_14.track.w - (900.0 - PAD * 1.4)).abs() < 0.01);
        assert!(gutter(1.4) >= at_14.track.w);
    }

    #[test]
    fn the_thumb_tracks_the_offset_and_back() {
        let viewport = Rect::new(0.0, 0.0, 300.0, 400.0);
        let content = 1400.0;
        let top = bar(&viewport, content, 0.0, 1.0);
        assert_eq!(top.thumb.y, top.track.y);
        let end = bar(&viewport, content, 1000.0, 1.0);
        assert!((end.thumb.y + end.thumb.h - (end.track.y + end.track.h)).abs() < 0.01);
        // Past the end is the end, not a thumb outside its track.
        let past = bar(&viewport, content, 5000.0, 1.0);
        assert_eq!(past.thumb.y, end.thumb.y);

        // Dragging: the point that was grabbed stays under the pointer.
        let mid = bar(&viewport, content, 400.0, 1.0);
        let press_y = mid.thumb.y + 10.0;
        let (grab_dy, jump) = grab(&mid, press_y, content, viewport.h);
        assert_eq!((grab_dy, jump), (10.0, None));
        let moved = follow(&mid, grab_dy, press_y + 30.0, content, viewport.h);
        let after = bar(&viewport, content, moved, 1.0);
        assert!((after.thumb.y - (mid.thumb.y + 30.0)).abs() < 0.01);

        // A press on the track brings the thumb's middle there.
        let (grab_dy, jump) = grab(&top, 300.0, content, viewport.h);
        assert_eq!(grab_dy, top.thumb.h * 0.5);
        let jumped = bar(&viewport, content, jump.unwrap(), 1.0);
        assert!((jumped.thumb.y + jumped.thumb.h * 0.5 - 300.0).abs() < 0.01);
    }

    #[test]
    fn the_sidebar_bar_runs_beside_the_rows_not_over_them() {
        use crate::layout::{build_sidebar_layout, SidebarSpec};
        let spec = SidebarSpec {
            places: 9,
            favorites: 12,
            drives: 3,
            ..Default::default()
        };
        for s in [1.0_f32, 1.25, 1.4] {
            let viewport = Rect::new(0.0, 44.0 * s, 240.0 * s, 500.0 * s);
            let layout = build_sidebar_layout(s, &spec, viewport, 0.0, sidebar_gutter(s));
            let bar = sidebar_bar(&layout, s).expect("more rows than fit");
            let band = hover_zone(&bar, s);
            for row in layout.place_items.iter().chain(&layout.drive_items) {
                assert!(row.x + row.w <= band.x + 0.01, "scale {s}");
            }
            let plus = layout.favorites_plus;
            assert!(plus.x + plus.w <= band.x + 0.01, "scale {s}");
            // And inside the sidebar, left of its gradient edge.
            assert!(bar.track.x + bar.track.w <= 236.0 * s + 0.01);
        }
        // Everything fits: no bar, and the wheel is not the sidebar's.
        let roomy = Rect::new(0.0, 44.0, 240.0, 2000.0);
        let layout = build_sidebar_layout(1.0, &spec, roomy, 0.0, sidebar_gutter(1.0));
        assert!(sidebar_bar(&layout, 1.0).is_none());
    }

    #[test]
    fn content_that_fits_has_no_bar_to_draw() {
        let viewport = Rect::new(0.0, 0.0, 300.0, 400.0);
        let fits = bar(&viewport, 380.0, 0.0, 1.25);
        assert!(fits.visible_ratio >= 1.0);
        assert_eq!(fits.thumb.h, fits.track.h);
        // An empty or degenerate viewport does not produce a negative track.
        let flat = bar(&Rect::new(0.0, 0.0, 300.0, 2.0), 1000.0, 0.0, 1.0);
        assert_eq!(flat.track.h, 0.0);
        assert_eq!(flat.thumb.h, 0.0);
    }
}
