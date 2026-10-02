//! The nav-bar buttons of one split-view pane.
//!
//! A pane has a left-aligned row (view toggle, Back, Forward, Up), a
//! right-aligned row (Sort, Search, and on the right pane the split toggle)
//! and the path strip between them. Each row used to be placed from its own
//! edge with no regard for the other: in a narrow pane they overlapped, and
//! since the right-aligned zones are registered later they won — a click on
//! Back closed split view, Forward opened the sort menu.
//!
//! Everything is placed here in one go instead, so nothing can overlap: as
//! the pane narrows the path strip goes first, then right-aligned buttons,
//! then left-aligned ones. A button that does not fit gets an empty rect;
//! callers register and draw only rects with a width (see `shown`).

use lntrn_render::Rect;

use super::nav_bar_y;

const BTN: f32 = 36.0;
/// Left-aligned buttons, as offsets from the pane's left edge. Split mode
/// drops the cloud button (it stays a sidebar/single-pane affordance), so
/// the row is tighter than the single-pane layout.
const LEFT: [f32; 4] = [6.0, 48.0, 86.0, 124.0];
/// Width of one right-aligned button with its gap.
const SLOT: f32 = 42.0;
const PATH_X: f32 = 168.0;
/// Narrower than this the path strip shows nothing useful and is dropped.
const PATH_MIN_W: f32 = 60.0;

const NONE: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 0.0,
    h: 0.0,
};

/// A button or strip that did not fit has no width: no zone, no drawing.
pub fn shown(rect: &Rect) -> bool {
    rect.w > 0.0
}

#[derive(Clone, Copy, Debug)]
pub struct PaneNav {
    pub view_toggle: Rect,
    pub back: Rect,
    pub forward: Rect,
    pub up: Rect,
    pub path: Rect,
    /// Split-close toggle — on the right pane only, keeping the button at
    /// the window's top-right where the user opened the split from.
    pub split_toggle: Rect,
    pub sort: Rect,
    pub search: Rect,
}

/// What the path strip has to hold right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Strip {
    /// The breadcrumbs: shown when there is room, dropped when not.
    Path,
    /// The search box or the path field: what is typed has to be seen.
    Typing,
    /// The ROOT badge, this wide (physical px), with or without a path
    /// after it: root mode must not be invisible in any pane.
    Badge(f32),
}

/// Place a pane's nav buttons. `is_right`: the right pane, which also hosts
/// the split toggle.
pub fn pane_nav(pane_x: f32, pane_w: f32, is_right: bool, strip: Strip, s: f32) -> PaneNav {
    let y = nav_bar_y(s);
    // Worked out in logical px, placed in physical.
    let w = (pane_w / s).max(0.0);
    let button = |offset: f32| Rect::new(pane_x + offset * s, y + 6.0 * s, BTN * s, BTN * s);

    // Right-aligned buttons, in the order they sit left to right, each with
    // its rank when there is not room for all (lower stays longer). The
    // split toggle is the way out of split view: it is the last to go.
    let right: &[(usize, u8)] = if is_right {
        &[(0, 0), (1, 2), (2, 1)] // split, sort, search
    } else {
        &[(1, 1), (2, 0)] // sort, search
    };
    let left_full = LEFT[3] + BTN + 6.0;
    let fit = ((w - left_full) / SLOT).floor().max(0.0) as usize;
    // The right pane keeps its split toggle even when the left row has to
    // give way for it.
    let keep = if is_right && w >= SLOT { 1 } else { 0 };
    let n_usual = fit.clamp(keep, right.len());

    // A whole layout with `n_right` right-aligned buttons and the left row
    // either shown (as much of it as fits) or given up, and the width
    // (logical) of the strip that leaves: from the end of the left row, or
    // from the pane's edge without one.
    let build = |n_right: usize, with_left: bool| -> (PaneNav, f32) {
        let right_w = n_right as f32 * SLOT;
        let mut right_rects = [NONE; 3];
        let kept = right.iter().filter(|(_, rank)| (*rank as usize) < n_right);
        for (slot, (which, _)) in kept.enumerate() {
            // Packed against the right edge, in their usual order.
            let from_right = (n_right - slot) as f32 * SLOT;
            right_rects[*which] = button(w - from_right);
        }
        let left = LEFT.map(|offset| {
            if with_left && offset + BTN <= w - right_w {
                button(offset)
            } else {
                NONE
            }
        });
        let from = if with_left { PATH_X } else { LEFT[0] };
        let strip_w = if with_left && !shown(&left[3]) {
            // Between a cut-off left row and the right row there is no
            // strip to speak of.
            0.0
        } else {
            (w - right_w - 8.0 - from).max(0.0)
        };
        let nav = PaneNav {
            view_toggle: left[0],
            back: left[1],
            forward: left[2],
            up: left[3],
            path: Rect::new(pane_x + from * s, y + 5.0 * s, strip_w * s, 38.0 * s),
            split_toggle: right_rects[0],
            sort: right_rects[1],
            search: right_rects[2],
        };
        (nav, strip_w)
    };

    let (mut usual, usual_w) = build(n_usual, true);
    let need = match strip {
        Strip::Path => {
            // Breadcrumbs: only between two complete rows, and only when
            // wide enough to say something.
            if n_usual < right.len() || usual_w < PATH_MIN_W {
                usual.path = NONE;
            }
            return usual;
        }
        Strip::Typing => PATH_MIN_W,
        Strip::Badge(px) => px / s,
    };
    if usual_w >= need {
        return usual;
    }
    // The strip is needed and the usual layout has no room for it: buttons
    // make room. The badge is there for as long as root mode is, so Sort
    // and Search go before Back/Forward/Up do; a box being typed into is
    // there for a moment, and takes the left row's place first.
    let fallbacks: &[(usize, bool)] = match strip {
        Strip::Badge(_) => &[(keep, true), (n_usual, false), (keep, false)],
        _ => &[(n_usual, false), (keep, false)],
    };
    let mut last = usual;
    for (n_right, with_left) in fallbacks {
        let (nav, strip_w) = build(*n_right, *with_left);
        if strip_w >= need {
            return nav;
        }
        last = nav;
    }
    // Not even alone: it gets what there is (and is cut off by the pane).
    if !shown(&last.path) {
        last.path = NONE;
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(n: &PaneNav) -> Vec<(&'static str, Rect)> {
        [
            ("view", n.view_toggle),
            ("back", n.back),
            ("forward", n.forward),
            ("up", n.up),
            ("path", n.path),
            ("split", n.split_toggle),
            ("sort", n.sort),
            ("search", n.search),
        ]
        .into_iter()
        .filter(|(_, r)| shown(r))
        .collect()
    }

    #[test]
    fn nothing_overlaps_and_nothing_leaves_the_pane_at_any_width() {
        for s in [1.0_f32, 1.25, 1.4] {
            for is_right in [false, true] {
                for px in 0..2700 {
                    // Each width with breadcrumbs, a search box and a badge.
                    let (pane_x, pane_w) = (300.0, (px / 3) as f32);
                    let strip = [Strip::Path, Strip::Typing, Strip::Badge(90.0 * s)][px % 3];
                    let nav = pane_nav(pane_x, pane_w, is_right, strip, s);
                    let shown = rects(&nav);
                    for (name, r) in &shown {
                        assert!(
                            r.x >= pane_x - 0.01 && r.x + r.w <= pane_x + pane_w + 0.01,
                            "{name} leaves a {pane_w}px pane ({strip:?}, right: {is_right}, scale {s})"
                        );
                    }
                    for (i, (a_name, a)) in shown.iter().enumerate() {
                        for (b_name, b) in &shown[i + 1..] {
                            let apart = a.x + a.w <= b.x + 0.01 || b.x + b.w <= a.x + 0.01;
                            assert!(
                                apart,
                                "{a_name} and {b_name} overlap in a {pane_w}px pane \
                                 ({strip:?}, right: {is_right}, scale {s})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_pane_at_the_minimum_width_has_every_button_and_a_path_strip() {
        for is_right in [false, true] {
            let nav = pane_nav(
                240.0,
                crate::layout::SPLIT_PANE_MIN_W,
                is_right,
                Strip::Path,
                1.0,
            );
            assert_eq!(rects(&nav).len(), if is_right { 8 } else { 7 });
            assert!(nav.path.w >= PATH_MIN_W);
        }
        // The roomy layout is the one there always was.
        let nav = pane_nav(240.0, 600.0, true, Strip::Path, 1.0);
        assert_eq!(nav.back.x, 240.0 + 48.0);
        assert_eq!(nav.split_toggle.x, 240.0 + 600.0 - 126.0);
        assert_eq!(nav.sort.x, 240.0 + 600.0 - 84.0);
        assert_eq!(nav.search.x, 240.0 + 600.0 - 42.0);
        assert_eq!(nav.path.x, 240.0 + 168.0);
        assert_eq!(nav.path.w, 600.0 - 134.0 - 168.0);
    }

    #[test]
    fn a_narrow_pane_drops_the_path_then_sort_then_search() {
        // Left pane, too narrow for the path strip but not for the buttons.
        let nav = pane_nav(0.0, 260.0, false, Strip::Path, 1.0);
        assert!(!shown(&nav.path));
        assert!(shown(&nav.sort) && shown(&nav.search) && shown(&nav.up));
        // One right-aligned button fits: Search stays, at the edge.
        let nav = pane_nav(0.0, 210.0, false, Strip::Path, 1.0);
        assert!(!shown(&nav.sort));
        assert_eq!(nav.search.x, 210.0 - 42.0);
        assert!(shown(&nav.up));
        // None fits: the left row is whole and alone.
        let nav = pane_nav(0.0, 170.0, false, Strip::Path, 1.0);
        assert!(!shown(&nav.sort) && !shown(&nav.search));
        assert!(shown(&nav.up));
    }

    #[test]
    fn a_search_box_gets_a_place_even_where_a_path_strip_does_not() {
        // 260px: no path strip. Searching, the box replaces the left row.
        let nav = pane_nav(100.0, 260.0, false, Strip::Typing, 1.0);
        assert!(shown(&nav.path));
        assert_eq!(nav.path.x, 106.0);
        assert_eq!(nav.path.x + nav.path.w, 100.0 + 260.0 - 84.0 - 8.0);
        assert!(!shown(&nav.back) && !shown(&nav.view_toggle));
        // The Search button (which closes the box) is still there.
        assert!(shown(&nav.search));
        // With room for a strip, typing changes nothing.
        let roomy = pane_nav(100.0, 500.0, false, Strip::Typing, 1.0);
        assert!(shown(&roomy.back));
        assert_eq!(roomy.path.x, 100.0 + 168.0);
    }

    #[test]
    fn the_root_badge_always_has_its_place() {
        let badge = 90.0;
        for is_right in [false, true] {
            // From the narrowest pane that could hold it at all.
            for px in 150..700 {
                let nav = pane_nav(0.0, px as f32, is_right, Strip::Badge(badge), 1.0);
                assert!(
                    nav.path.w >= badge,
                    "no room for the badge in a {px}px pane (right: {is_right})"
                );
                if is_right {
                    assert!(shown(&nav.split_toggle));
                }
            }
        }
        // Sort and Search make room before Back/Forward/Up do.
        let nav = pane_nav(0.0, 300.0, false, Strip::Badge(badge), 1.0);
        assert!(!shown(&nav.sort) && !shown(&nav.search));
        assert!(shown(&nav.back) && shown(&nav.up));
        assert_eq!(nav.path.x, 168.0);
        // Narrower still, the left row goes too; the badge does not.
        let nav = pane_nav(0.0, 200.0, false, Strip::Badge(badge), 1.0);
        assert!(!shown(&nav.back));
        assert_eq!(nav.path.x, 6.0);
        // A roomy pane is laid out as ever.
        let nav = pane_nav(0.0, 600.0, true, Strip::Badge(badge), 1.0);
        assert_eq!(rects(&nav).len(), 8);
    }

    #[test]
    fn the_right_pane_never_loses_the_way_out_of_split_view() {
        for px in 42..400 {
            let nav = pane_nav(500.0, px as f32, true, Strip::Path, 1.0);
            assert!(shown(&nav.split_toggle), "no split toggle at {px}px");
        }
        // Squeezed, Up and Forward give way to it rather than sit under it.
        let nav = pane_nav(0.0, 130.0, true, Strip::Path, 1.0);
        assert!(shown(&nav.view_toggle) && shown(&nav.back));
        assert!(!shown(&nav.forward) && !shown(&nav.up));
        assert_eq!(nav.split_toggle.x, 130.0 - 42.0);
    }
}
