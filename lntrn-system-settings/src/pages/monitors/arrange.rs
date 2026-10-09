//! Where monitors sit beside each other, as rectangles on the desktop in
//! logical pixels. A setup the compositor can use has every monitor
//! touching another along an edge and none over another, with its
//! top-left corner at the origin (the compositor moves it there anyway,
//! see its `output_layout`). A drag lands on the nearest such place. A
//! monitor that changed size is fitted back in against the edge it sat
//! on before, lined up the way it was; one that has only just come on
//! goes to the nearest place there is.

/// One monitor's place on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A mode's size on the desktop at `scale`, rounded as the compositor
/// rounds it.
pub fn logical(width: i32, height: i32, scale: f64) -> (i32, i32) {
    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    (((f64::from(width) / s).round() as i32).max(1), ((f64::from(height) / s).round() as i32).max(1))
}

impl Tile {
    fn right(self) -> i32 {
        self.x + self.w
    }
    fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// Whether the two cover any of the same desktop. Edges that only
    /// meet don't count.
    pub fn overlaps(self, o: Tile) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    /// Whether the two share a stretch of edge the pointer can cross:
    /// side by side or one over the other, never corner to corner.
    pub fn touches(self, o: Tile) -> bool {
        let shared_y = self.bottom().min(o.bottom()) - self.y.max(o.y);
        let shared_x = self.right().min(o.right()) - self.x.max(o.x);
        ((self.right() == o.x || o.right() == self.x) && shared_y >= shared_edge(self.h, o.h)) || ((self.bottom() == o.y || o.bottom() == self.y) && shared_x >= shared_edge(self.w, o.w))
    }
}

/// The least two monitors `a` and `b` long must share of an edge: a
/// quarter of the shorter one.
fn shared_edge(a: i32, b: i32) -> i32 {
    (a.min(b) / 4).max(1)
}

/// `v` along an edge from `lo` to `hi`, pulled onto one of `stops` when
/// it is within `magnet` of it: where edges and middles line up.
fn along(v: i32, lo: i32, hi: i32, stops: [i32; 3], magnet: i32) -> i32 {
    let v = v.clamp(lo, hi.max(lo));
    stops.into_iter().filter(|s| (lo..=hi).contains(s) && (v - s).abs() <= magnet).min_by_key(|s| (v - s).abs()).unwrap_or(v)
}

/// The four sides of a monitor another can sit against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Right,
    Left,
    Below,
    Above,
}

const SIDES: [Side; 4] = [Side::Right, Side::Left, Side::Below, Side::Above];

/// `m` put against `side` of `o`, as near along that edge to where `m`
/// is as still shares enough of it. Within `magnet` pixels of lining up
/// with `o` (starts, ends, middles) it lines up.
fn beside(m: Tile, o: Tile, side: Side, magnet: i32) -> Tile {
    match side {
        Side::Right | Side::Left => {
            let keep = shared_edge(m.h, o.h);
            let y = along(m.y, o.y - m.h + keep, o.bottom() - keep, [o.y, o.bottom() - m.h, o.y + (o.h - m.h) / 2], magnet);
            Tile { x: if side == Side::Right { o.right() } else { o.x - m.w }, y, ..m }
        }
        Side::Below | Side::Above => {
            let keep = shared_edge(m.w, o.w);
            let x = along(m.x, o.x - m.w + keep, o.right() - keep, [o.x, o.right() - m.w, o.x + (o.w - m.w) / 2], magnet);
            Tile { x, y: if side == Side::Below { o.bottom() } else { o.y - m.h }, ..m }
        }
    }
}

fn apart(a: Tile, b: Tile) -> i64 {
    let (dx, dy) = (i64::from(a.x - b.x), i64::from(a.y - b.y));
    dx * dx + dy * dy
}

/// The nearest place to where `moving` is that touches one of `others`
/// and overlaps none of them. Within `magnet` pixels of lining up with
/// the one it touches (tops, bottoms, middles) it lines up. With no
/// others it stays where it is.
pub fn snap(moving: Tile, others: &[Tile], magnet: i32) -> Tile {
    let places = others.iter().flat_map(|o| SIDES.map(|side| beside(moving, *o, side, magnet)));
    let free = places.filter(|t| !others.iter().any(|o| t.overlaps(*o)));
    match (free.min_by_key(|t| apart(*t, moving)), others.iter().max_by_key(|o| o.right())) {
        (Some(t), _) => t,
        // Every place beside the others is taken: off the right end.
        (None, Some(last)) => Tile { x: last.right(), y: last.y, ..moving },
        (None, None) => moving,
    }
}

/// Which side of `o` the tile `m` sits against, and how far it is from
/// sitting right against it: the side its nearest place beside `o` is on.
fn side_of(m: Tile, o: Tile) -> (Side, i64) {
    SIDES.map(|side| (side, apart(beside(m, o, side, 0), m))).into_iter().min_by_key(|(_, far)| *far).unwrap_or((Side::Right, 0))
}

/// Where along an edge a tile `len` long goes against one from `o` for
/// `o_len`, to sit as it sat when it was `was_len` long from `was`
/// against one from `was_o` for `was_o_len`: starts, ends or middles
/// lined up if they were, else as far along as it was.
fn carried(was: i32, was_len: i32, was_o: i32, was_o_len: i32, len: i32, o: i32, o_len: i32) -> i32 {
    if was == was_o {
        o
    } else if was + was_len == was_o + was_o_len {
        o + o_len - len
    } else if (2 * was + was_len - 2 * was_o - was_o_len).abs() <= 1 {
        o + (o_len - len) / 2
    } else {
        o + (was - was_o)
    }
}

/// Whether `tiles` is a setup the compositor can use as it is: none over
/// another, and all of them joined edge to edge. What [`snap`] and
/// [`tidy`] are held to.
#[cfg(test)]
pub fn sound(tiles: &[Tile]) -> bool {
    if tiles.iter().enumerate().any(|(i, a)| tiles[i + 1..].iter().any(|b| a.overlaps(*b))) {
        return false;
    }
    // Spread from the first along shared edges; all must be reached.
    let mut reached = vec![false; tiles.len()];
    let mut todo: Vec<usize> = tiles.first().map(|_| 0).into_iter().collect();
    while let Some(i) = todo.pop() {
        if std::mem::replace(&mut reached[i], true) {
            continue;
        }
        todo.extend((0..tiles.len()).filter(|&j| !reached[j] && tiles[i].touches(tiles[j])));
    }
    reached.into_iter().all(|r| r)
}

/// Fit the tiles back together after some changed size or joined.
/// `was` is each as it was before (`None` for one that wasn't there).
/// The first stays put, and each of the rest that still sits right
/// against the ones before it stays too. One that doesn't goes back
/// against the tile and the edge it sat on before, lined up as it was;
/// with no before, or that place taken, to the nearest place there is.
/// Then the whole setup is moved so its corner is at the origin.
pub fn tidy(tiles: &mut [Tile], was: &[Option<Tile>]) {
    let before = |i: usize| was.get(i).copied().flatten();
    let mut placed: Vec<usize> = Vec::with_capacity(tiles.len());
    let mut left: Vec<usize> = (0..tiles.len()).collect();
    while !left.is_empty() {
        let fits = |i: usize| placed.is_empty() || (placed.iter().any(|&p| tiles[i].touches(tiles[p])) && !placed.iter().any(|&p| tiles[i].overlaps(tiles[p])));
        let next = match left.iter().position(|&i| fits(i)) {
            Some(at) => left.remove(at),
            None => {
                let i = left.remove(0);
                let t = tiles[i];
                // The placed tile it sat closest against, and on which side.
                let against = before(i).and_then(|t0| placed.iter().filter_map(|&p| before(p).map(|p0| (side_of(t0, p0), p, t0, p0))).min_by_key(|((_, far), ..)| *far));
                let back = against.map(|((side, _), p, t0, p0)| {
                    let o = tiles[p];
                    let aimed = match side {
                        Side::Right | Side::Left => Tile { y: carried(t0.y, t0.h, p0.y, p0.h, t.h, o.y, o.h), ..t },
                        Side::Below | Side::Above => Tile { x: carried(t0.x, t0.w, p0.x, p0.w, t.w, o.x, o.w), ..t },
                    };
                    beside(aimed, o, side, 0)
                });
                let others: Vec<Tile> = placed.iter().map(|&p| tiles[p]).collect();
                tiles[i] = back.filter(|b| !others.iter().any(|o| b.overlaps(*o))).unwrap_or_else(|| snap(t, &others, 0));
                i
            }
        };
        placed.push(next);
    }
    normalize(tiles);
}

/// Move the whole setup so its top-left corner is at the origin.
pub fn normalize(tiles: &mut [Tile]) {
    let (Some(x), Some(y)) = (tiles.iter().map(|t| t.x).min(), tiles.iter().map(|t| t.y).min()) else { return };
    for t in tiles {
        t.x -= x;
        t.y -= y;
    }
}

/// The box around all of `tiles`.
pub fn bounds(tiles: &[Tile]) -> Tile {
    let x = tiles.iter().map(|t| t.x).min().unwrap_or(0);
    let y = tiles.iter().map(|t| t.y).min().unwrap_or(0);
    let right = tiles.iter().map(|t| t.right()).max().unwrap_or(1);
    let bottom = tiles.iter().map(|t| t.bottom()).max().unwrap_or(1);
    Tile { x, y, w: (right - x).max(1), h: (bottom - y).max(1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(x: i32, y: i32, w: i32, h: i32) -> Tile {
        Tile { x, y, w, h }
    }

    #[test]
    fn a_mode_is_as_big_as_the_compositor_makes_it() {
        assert_eq!(logical(3840, 2160, 1.4), (2743, 1543));
        assert_eq!(logical(1920, 1080, 1.0), (1920, 1080));
        assert_eq!(logical(2560, 1440, 1.25), (2048, 1152));
        // A scale that isn't one is no scale.
        assert_eq!(logical(1920, 1080, 0.0), (1920, 1080));
        assert_eq!(logical(1920, 1080, f64::NAN), (1920, 1080));
    }

    #[test]
    fn touching_is_sharing_an_edge_not_a_corner() {
        let a = t(0, 0, 1000, 600);
        assert!(a.touches(t(1000, 0, 800, 600)) && a.touches(t(1000, 400, 800, 600)));
        assert!(a.touches(t(100, 600, 500, 400)) && a.touches(t(-800, -100, 800, 600)));
        // Corner to corner, a gap, a sliver of edge, and one over the other.
        assert!(!a.touches(t(1000, 600, 800, 600)));
        assert!(!a.touches(t(1001, 0, 800, 600)));
        assert!(!a.touches(t(1000, 590, 800, 600)));
        assert!(!a.touches(t(500, 0, 800, 600)) && a.overlaps(t(500, 0, 800, 600)));
        assert!(!a.overlaps(t(1000, 0, 800, 600)), "edges that meet are not over each other");
    }

    #[test]
    fn a_drag_lands_on_the_nearest_side() {
        let big = t(0, 0, 2743, 1543);
        let small = |x, y| t(x, y, 1920, 1080);
        // Dropped overlapping the right half: it goes to the right edge.
        assert_eq!(snap(small(2000, 300), &[big], 0), small(2743, 300));
        // Far to the left: the left edge, and no lower than still shares enough.
        assert_eq!(snap(small(-5000, 3000), &[big], 0), small(-1920, 1543 - 270));
        // Under it, nearer the bottom than either side.
        assert_eq!(snap(small(400, 1400), &[big], 0), small(400, 1543));
        assert_eq!(snap(small(400, -2000), &[big], 0), small(400, -1080));
        // Alone, it stays where it was put.
        assert_eq!(snap(small(77, 88), &[], 0), small(77, 88));
    }

    #[test]
    fn near_enough_to_lined_up_is_lined_up() {
        let big = t(0, 0, 2743, 1543);
        let small = |x, y| t(x, y, 1920, 1080);
        // Tops, bottoms and middles pull within the magnet's reach.
        assert_eq!(snap(small(2800, 60), &[big], 100), small(2743, 0));
        assert_eq!(snap(small(2800, 400), &[big], 100), small(2743, 463));
        assert_eq!(snap(small(2800, 250), &[big], 100), small(2743, 231));
        assert_eq!(snap(small(2800, 130), &[big], 100), small(2743, 130), "out of reach of all three");
        // Left edges line up under it.
        assert_eq!(snap(small(-40, 1600), &[big], 100), small(0, 1543));
    }

    #[test]
    fn a_drag_never_lands_on_a_third_monitor() {
        let (a, b) = (t(0, 0, 1000, 600), t(1000, 0, 1000, 600));
        // Between them is taken: it goes over or under, wherever is nearest.
        let got = snap(t(600, 100, 800, 600), &[a, b], 0);
        assert!(!got.overlaps(a) && !got.overlaps(b) && (got.touches(a) || got.touches(b)), "{got:?}");
        assert!(sound(&[a, b, got]));
    }

    #[test]
    fn a_setup_is_sound_when_joined_and_clear() {
        assert!(sound(&[t(0, 0, 1000, 600)]) && sound(&[]));
        assert!(sound(&[t(0, 0, 1000, 600), t(1000, 100, 800, 600), t(1800, 0, 500, 500)]), "a chain is joined");
        assert!(!sound(&[t(0, 0, 1000, 600), t(1003, 0, 800, 600)]), "a gap");
        assert!(!sound(&[t(0, 0, 1000, 600), t(900, 0, 800, 600)]), "one over the other");
        assert!(!sound(&[t(0, 0, 1000, 600), t(1000, 0, 800, 600), t(5000, 0, 500, 500)]), "one left out");
    }

    /// `tiles` fitted back together, having been `was` before.
    fn tidied<const N: usize>(mut tiles: [Tile; N], was: [Tile; N]) -> [Tile; N] {
        tidy(&mut tiles, &was.map(Some));
        assert!(sound(&tiles), "{tiles:?}");
        tiles
    }

    #[test]
    fn a_monitor_that_changed_size_keeps_its_neighbours_on_their_sides() {
        let was = [t(0, 0, 2743, 1543), t(2743, 0, 1920, 1080)];
        // The left one shrinks: the right one follows its edge in.
        assert_eq!(tidied([t(0, 0, 1920, 1080), was[1]], was), [t(0, 0, 1920, 1080), t(1920, 0, 1920, 1080)]);
        // It grows over the right one, which is pushed out along the same
        // edge, though the place above it would be nearer.
        assert_eq!(tidied([t(0, 0, 3840, 2160), was[1]], was), [t(0, 0, 3840, 2160), t(3840, 0, 1920, 1080)]);
        // The right one is the one that changes: it stays on its edge.
        assert_eq!(tidied([was[0], t(2743, 0, 1280, 720)], was), [was[0], t(2743, 0, 1280, 720)]);

        // On the far side of the first, it is the one that moves, and the
        // corner goes back to the origin.
        let was = [t(1920, 0, 2743, 1543), t(0, 0, 1920, 1080)];
        assert_eq!(tidied([was[0], t(0, 0, 1000, 600)], was), [t(1000, 0, 2743, 1543), t(0, 0, 1000, 600)]);
        // The compositor's own rounding leaves a hair between them: still
        // the same edge.
        let was = [t(0, 0, 2743, 1543), t(2746, 0, 1920, 1080)];
        assert_eq!(tidied([t(0, 0, 3840, 2160), was[1]], was), [t(0, 0, 3840, 2160), t(3840, 0, 1920, 1080)]);
    }

    #[test]
    fn a_refit_keeps_what_was_lined_up_lined_up() {
        // Bottoms level: they stay level when the big one shrinks.
        let was = [t(0, 0, 2743, 1543), t(2743, 463, 1920, 1080)];
        assert_eq!(tidied([t(0, 0, 2194, 1234), was[1]], was), [t(0, 0, 2194, 1234), t(2194, 154, 1920, 1080)]);
        // Middles level: they stay level.
        let was = [t(0, 0, 2000, 1000), t(2000, 250, 1000, 500)];
        assert_eq!(tidied([t(0, 0, 3000, 1500), was[1]], was), [t(0, 0, 3000, 1500), t(3000, 500, 1000, 500)]);
        // Neither: as far down the edge as it was, while that still
        // shares enough of it.
        let was = [t(0, 0, 2000, 1000), t(2000, 700, 1000, 800)];
        assert_eq!(tidied([t(0, 0, 1000, 500), was[1]], was), [t(0, 0, 1000, 500), t(1000, 375, 1000, 800)]);
        // Under it, left edges level.
        let was = [t(0, 0, 2000, 1000), t(0, 1000, 1000, 600)];
        assert_eq!(tidied([t(0, 0, 3000, 1500), was[1]], was), [t(0, 0, 3000, 1500), t(0, 1500, 1000, 600)]);
    }

    #[test]
    fn what_hangs_together_is_left_alone_and_a_newcomer_finds_a_place() {
        // A chain joined through its middle: nothing moves.
        let chain = [t(0, 0, 1000, 600), t(1800, 0, 500, 500), t(1000, 100, 800, 600)];
        assert_eq!(tidied(chain, chain), chain);
        assert_eq!(bounds(&chain), t(0, 0, 2300, 700));
        // One that wasn't there before goes to the nearest place.
        let mut tiles = [t(0, 0, 2743, 1543), t(2746, 0, 1920, 1080)];
        let was = [Some(tiles[0]), None];
        tidy(&mut tiles, &was);
        assert_eq!(tiles, [t(0, 0, 2743, 1543), t(2743, 0, 1920, 1080)]);
        // Its old place is taken by a third: it goes where there is room.
        let was = [t(0, 0, 1000, 600), t(1000, 0, 500, 600), t(1500, 0, 800, 600)];
        let got = tidied([t(0, 0, 1000, 600), t(1000, 0, 900, 600), was[2]], was);
        assert_eq!((got[0], got[1]), (t(0, 0, 1000, 600), t(1000, 0, 900, 600)));
        assert_eq!(got[2], t(1900, 0, 800, 600), "still to the right of the one it sat against");
    }

    #[test]
    fn the_corner_goes_to_the_origin() {
        let mut tiles = [t(215, 40, 2746, 1543), t(2961, 0, 1920, 1080)];
        normalize(&mut tiles);
        assert_eq!(tiles, [t(0, 40, 2746, 1543), t(2746, 0, 1920, 1080)]);
        normalize(&mut []);
    }
}
