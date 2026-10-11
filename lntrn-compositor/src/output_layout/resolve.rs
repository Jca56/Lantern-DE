//! Turning intended output positions into a layout that can be shown.
//!
//! A position in `[[monitors]]` (or in a wlr-output-management batch) is an
//! absolute logical coordinate, and it was right for the logical sizes the
//! outputs had when it was written. A logical size changes whenever a scale
//! or a mode does (4K at 1.4 is 2743 wide, at 1.3 it is 2954, in Gaming Mode
//! 3840), and nothing rewrites the neighbours' coordinates when that
//! happens. Taken verbatim, a stale position leaves two outputs showing the
//! same strip of the desktop (2026-10-10: HDMI-A-1 at x=2746 inside DP-1's
//! 0..2954 — every window near the seam drawn on both monitors, the pointer
//! on both, clicks going to whichever output was asked first) or a dead gap
//! between them.
//!
//! So positions are read as *intent* — "to the right of that one, tops
//! level" — and resolved against the sizes the outputs have now:
//!
//! 1. Outputs are placed in order of their intended x (then y). The first
//!    one anchors the layout.
//! 2. One that would overlap an output already placed is pushed out of it
//!    along the axis its origin is displaced on: an output whose origin sits
//!    about one width to the right of its neighbour's was meant to be on its
//!    right, however much the two overlap now. The other coordinate — the
//!    alignment the user chose — is kept.
//! 3. One left touching nothing is slid back until it shares an edge with
//!    the nearest output, so the pointer can always cross.
//! 4. The whole layout is translated so its top-left corner is (0, 0) (see
//!    the module docs for why XWayland needs that).
//!
//! The result never has two outputs overlapping. Resolving a layout that is
//! already sound changes nothing, so this is safe to run after every change.

/// One output as the resolver sees it: where it is meant to sit and how big
/// it is right now, in logical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Slot {
    fn right(&self) -> i32 {
        self.x + self.w
    }

    fn bottom(&self) -> i32 {
        self.y + self.h
    }

    fn overlaps(&self, other: &Slot) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// The two share a stretch of edge (a corner alone doesn't count: the
    /// pointer can't cross through a point).
    fn touches(&self, other: &Slot) -> bool {
        let rows_meet = self.y < other.bottom() && other.y < self.bottom();
        let cols_meet = self.x < other.right() && other.x < self.right();
        (rows_meet && (self.x == other.right() || self.right() == other.x))
            || (cols_meet && (self.y == other.bottom() || self.bottom() == other.y))
    }
}

/// Resolve `slots` in place: afterwards no two overlap, each shares an edge
/// with another wherever its intent allows, and the layout's top-left corner
/// is (0, 0). The slice keeps its order; only positions change.
pub(crate) fn resolve(slots: &mut [Slot]) {
    for s in slots.iter_mut() {
        s.w = s.w.max(1);
        s.h = s.h.max(1);
    }

    let mut order: Vec<usize> = (0..slots.len()).collect();
    order.sort_by(|&a, &b| {
        (slots[a].x, slots[a].y, &slots[a].name).cmp(&(slots[b].x, slots[b].y, &slots[b].name))
    });

    let mut placed: Vec<usize> = Vec::with_capacity(order.len());
    for &i in &order {
        separate(slots, i, &placed);
        attach(slots, i, &placed);
        // Sliding up to a neighbour can't run into a third output when the
        // slide was the shortest one on offer, but the diagonal case moves
        // on both axes: make sure, since "never overlapping" is the promise.
        separate(slots, i, &placed);
        placed.push(i);
    }

    let (ox, oy) = super::layout_origin(slots.iter().map(|s| (s.x, s.y)));
    for s in slots.iter_mut() {
        s.x -= ox;
        s.y -= oy;
    }
}

/// Push slot `i` out of every placed slot it overlaps.
fn separate(slots: &mut [Slot], i: usize, placed: &[usize]) {
    // Each push clears one overlap and may create another further along; a
    // handful of rounds settles any real arrangement.
    for _ in 0..placed.len() * 4 + 4 {
        let Some(&p) = placed.iter().find(|&&p| slots[i].overlaps(&slots[p])) else {
            return;
        };
        let (p, r) = (slots[p].clone(), &mut slots[i]);
        let (dx, dy) = (r.x - p.x, r.y - p.y);
        // How far the origin is displaced on each axis, in sizes of
        // whichever of the two comes first on it.
        let fx = if dx >= 0 { dx as f64 / p.w as f64 } else { -dx as f64 / r.w as f64 };
        let fy = if dy >= 0 { dy as f64 / p.h as f64 } else { -dy as f64 / r.h as f64 };
        if fx >= fy {
            r.x = if dx >= 0 { p.right() } else { p.x - r.w };
        } else {
            r.y = if dy >= 0 { p.bottom() } else { p.y - r.h };
        }
    }
    // Still overlapping something (outputs wedged between each other):
    // give up on the arrangement, not on the promise.
    if placed.iter().any(|&p| slots[i].overlaps(&slots[p])) {
        slots[i].x = placed.iter().map(|&p| slots[p].right()).max().unwrap_or(0);
    }
}

/// Slide slot `i` up to the nearest placed slot if it touches none.
fn attach(slots: &mut [Slot], i: usize, placed: &[usize]) {
    if placed.is_empty() || placed.iter().any(|&p| slots[i].touches(&slots[p])) {
        return;
    }

    // The shortest slide along one axis that makes it share an edge.
    let mut best: Option<(bool, i32)> = None;
    for &p in placed {
        let (p, r) = (&slots[p], &slots[i]);
        let rows_meet = r.y < p.bottom() && p.y < r.bottom();
        let cols_meet = r.x < p.right() && p.x < r.right();
        let slide = if rows_meet {
            Some((true, if r.x >= p.right() { p.right() - r.x } else { p.x - r.right() }))
        } else if cols_meet {
            Some((false, if r.y >= p.bottom() { p.bottom() - r.y } else { p.y - r.bottom() }))
        } else {
            None
        };
        if let Some(slide) = slide {
            if best.map_or(true, |b| slide.1.abs() < b.1.abs()) {
                best = Some(slide);
            }
        }
    }
    if let Some((along_x, d)) = best {
        if along_x {
            slots[i].x += d;
        } else {
            slots[i].y += d;
        }
        return;
    }

    // Diagonal from everything. Bring it level with the nearest slot (inside
    // its rows, as far as it fits), then up against its side.
    let gap = |r: &Slot, p: &Slot| {
        let gx = (p.x - r.right()).max(r.x - p.right()).max(0);
        let gy = (p.y - r.bottom()).max(r.y - p.bottom()).max(0);
        gx + gy
    };
    let Some(&nearest) = placed.iter().min_by_key(|&&p| gap(&slots[i], &slots[p])) else {
        return;
    };
    let (p, r) = (slots[nearest].clone(), &mut slots[i]);
    r.y = r.y.clamp(p.y, (p.bottom() - r.h).max(p.y));
    r.x = if r.x >= p.right() { p.right() } else { p.x - r.w };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str, x: i32, y: i32, w: i32, h: i32) -> Slot {
        Slot { name: name.into(), x, y, w, h }
    }

    fn at(slots: &[Slot], name: &str) -> (i32, i32) {
        let s = slots.iter().find(|s| s.name == name).unwrap();
        (s.x, s.y)
    }

    fn sound(slots: &[Slot]) -> bool {
        slots
            .iter()
            .enumerate()
            .all(|(i, a)| slots.iter().skip(i + 1).all(|b| !a.overlaps(b)))
    }

    /// The layout this was written for: DP-1 went from scale 1.4 to 1.3 and
    /// HDMI-A-1 kept the x it had for the narrower DP-1.
    #[test]
    fn a_neighbour_that_grew_pushes_the_next_one_along() {
        let mut s = vec![slot("HDMI-A-1", 2746, 0, 1920, 1080), slot("DP-1", 0, 0, 2954, 1662)];
        resolve(&mut s);
        assert_eq!(at(&s, "DP-1"), (0, 0));
        assert_eq!(at(&s, "HDMI-A-1"), (2954, 0));
    }

    /// Gaming Mode: the primary at 1.0 is wider than the overlap is tall, so
    /// the smaller-penetration axis would be the wrong one.
    #[test]
    fn a_deep_overlap_still_separates_along_the_row() {
        let mut s = vec![slot("DP-1", 0, 0, 3840, 2160), slot("HDMI-A-1", 2743, 0, 1920, 1080)];
        resolve(&mut s);
        assert_eq!(at(&s, "HDMI-A-1"), (3840, 0));
    }

    #[test]
    fn a_neighbour_that_shrank_pulls_the_next_one_back() {
        let mut s = vec![slot("DP-1", 0, 0, 2560, 1440), slot("HDMI-A-1", 2954, 200, 1920, 1080)];
        resolve(&mut s);
        // The gap closes; the vertical offset the user chose stays.
        assert_eq!(at(&s, "HDMI-A-1"), (2560, 200));
    }

    #[test]
    fn a_stack_separates_down_the_column() {
        let mut s = vec![slot("DP-1", 0, 0, 2954, 1662), slot("HDMI-A-1", 400, 1543, 1920, 1080)];
        resolve(&mut s);
        assert_eq!(at(&s, "DP-1"), (0, 0));
        assert_eq!(at(&s, "HDMI-A-1"), (400, 1662));
    }

    #[test]
    fn an_output_above_stays_above_and_the_origin_follows() {
        // HDMI on top, now taller than the room it was given.
        let mut s = vec![slot("DP-1", 0, 1080, 2954, 1662), slot("HDMI-A-1", 517, 0, 2560, 1440)];
        resolve(&mut s);
        assert_eq!(at(&s, "HDMI-A-1"), (517, 0));
        assert_eq!(at(&s, "DP-1"), (0, 1440));
    }

    #[test]
    fn a_sound_layout_is_left_alone() {
        let before = vec![
            slot("DP-1", 0, 0, 2954, 1662),
            slot("HDMI-A-1", 2954, 291, 1920, 1080),
            slot("DP-2", 1000, 1662, 1280, 720),
        ];
        let mut s = before.clone();
        resolve(&mut s);
        assert_eq!(s, before);
        // And resolving twice is resolving once.
        resolve(&mut s);
        assert_eq!(s, before);
    }

    #[test]
    fn outputs_with_no_position_line_up_in_a_row() {
        let mut s = vec![slot("B", 0, 0, 1920, 1080), slot("A", 0, 0, 2560, 1440)];
        resolve(&mut s);
        assert_eq!(at(&s, "A"), (0, 0));
        assert_eq!(at(&s, "B"), (2560, 0));
    }

    #[test]
    fn the_origin_is_normalized() {
        let mut s = vec![slot("DP-1", 215, 40, 2743, 1543), slot("HDMI-A-1", 2958, 40, 1920, 1080)];
        resolve(&mut s);
        assert_eq!(at(&s, "DP-1"), (0, 0));
        assert_eq!(at(&s, "HDMI-A-1"), (2743, 0));
    }

    #[test]
    fn a_diagonal_output_is_brought_alongside() {
        let mut s = vec![slot("A", 0, 0, 1000, 1000), slot("B", 1500, 1500, 500, 500)];
        resolve(&mut s);
        assert_eq!(at(&s, "B"), (1000, 500));
    }

    #[test]
    fn three_in_a_row_chain() {
        let mut s = vec![
            slot("L", 0, 0, 1920, 1080),
            slot("M", 1500, 0, 2954, 1662),
            slot("R", 4000, 0, 1920, 1080),
        ];
        resolve(&mut s);
        assert_eq!(at(&s, "M"), (1920, 0));
        assert_eq!(at(&s, "R"), (4874, 0));
        assert!(sound(&s));
    }

    #[test]
    fn nothing_ever_overlaps() {
        // Everything piled on one spot, assorted sizes.
        let mut s = vec![
            slot("a", 10, 10, 800, 600),
            slot("b", 20, 5, 1920, 1080),
            slot("c", 15, 400, 1280, 1024),
            slot("d", 600, 300, 3840, 2160),
            slot("e", 10, 10, 0, 0),
        ];
        resolve(&mut s);
        assert!(sound(&s), "{s:?}");
    }
}
