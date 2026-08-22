//! Glyph outlines: path commands, affine transforms, and bézier flattening.
//!
//! Outlines come out of the `glyf` parser in font units (y up). At raster time
//! they are flattened into line segments in pixel space (y down) through an
//! [`Affine`] that bakes scale, flip, and bitmap translation in one step.
//! Quadratic béziers subdivide adaptively so flattening error stays under a
//! quarter pixel at any size.

/// Row-major 2×3 affine transform: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy, Debug)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn apply(&self, p: (f32, f32)) -> [f32; 2] {
        [
            self.a * p.0 + self.c * p.1 + self.e,
            self.b * p.0 + self.d * p.1 + self.f,
        ]
    }

    /// Compose so that `self` applies first, then `outer`: `outer ∘ self`.
    /// Used by composite glyphs to nest component transforms.
    pub fn then(&self, outer: &Affine) -> Affine {
        Affine {
            a: outer.a * self.a + outer.c * self.b,
            b: outer.b * self.a + outer.d * self.b,
            c: outer.a * self.c + outer.c * self.d,
            d: outer.b * self.c + outer.d * self.d,
            e: outer.a * self.e + outer.c * self.f + outer.e,
            f: outer.b * self.e + outer.d * self.f + outer.f,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PathCmd {
    Move([f32; 2]),
    Line([f32; 2]),
    /// Quadratic bézier: control point, end point (TrueType `glyf`).
    Quad([f32; 2], [f32; 2]),
    /// Cubic bézier: two control points, end point (CFF charstrings).
    Cubic([f32; 2], [f32; 2], [f32; 2]),
}

/// A glyph outline as a flat command list in font units. Composite glyphs are
/// pre-baked: component transforms are applied when commands are emitted.
#[derive(Clone, Debug, Default)]
pub struct Outline {
    pub cmds: Vec<PathCmd>,
}

/// Max chord deviation allowed when flattening curves, in output pixels.
const FLATTEN_TOL: f32 = 0.25;

/// Flatten `cmds` through `t` into line segments fed to `sink(p0, p1)`.
/// Contours are closed automatically (TrueType contours are always closed).
pub fn flatten(cmds: &[PathCmd], t: &Affine, mut sink: impl FnMut([f32; 2], [f32; 2])) {
    let tp = |p: [f32; 2]| t.apply((p[0], p[1]));
    let mut start = [0.0f32; 2];
    let mut cur = [0.0f32; 2];
    let mut open = false;
    for cmd in cmds {
        match *cmd {
            PathCmd::Move(p) => {
                if open && cur != start {
                    sink(cur, start);
                }
                start = tp(p);
                cur = start;
                open = true;
            }
            PathCmd::Line(p) => {
                let q = tp(p);
                sink(cur, q);
                cur = q;
            }
            PathCmd::Quad(c, p) => {
                let (c, q) = (tp(c), tp(p));
                flatten_quad(cur, c, q, &mut sink);
                cur = q;
            }
            PathCmd::Cubic(c1, c2, p) => {
                let (c1, c2, q) = (tp(c1), tp(c2), tp(p));
                flatten_cubic(cur, c1, c2, q, &mut sink);
                cur = q;
            }
        }
    }
    if open && cur != start {
        sink(cur, start);
    }
}

/// Adaptive quadratic flattening. Max deviation of a quad from its chord is
/// `|p0 − 2c + p1| / 4`; splitting into `n` pieces shrinks it by `n²`, so
/// `n = ⌈√(dev / 4·tol)⌉` keeps error under `FLATTEN_TOL`.
fn flatten_quad(
    p0: [f32; 2],
    c: [f32; 2],
    p1: [f32; 2],
    sink: &mut impl FnMut([f32; 2], [f32; 2]),
) {
    let devx = p0[0] - 2.0 * c[0] + p1[0];
    let devy = p0[1] - 2.0 * c[1] + p1[1];
    let dev = (devx * devx + devy * devy).sqrt();
    let n = (dev / (4.0 * FLATTEN_TOL)).sqrt().ceil().clamp(1.0, 64.0) as usize;
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        let q = [
            mt * mt * p0[0] + 2.0 * mt * t * c[0] + t * t * p1[0],
            mt * mt * p0[1] + 2.0 * mt * t * c[1] + t * t * p1[1],
        ];
        sink(prev, q);
        prev = q;
    }
}

/// Adaptive cubic flattening. The chord deviation is bounded by the second
/// differences of the control polygon (×3/4), shrinking by `n²` per split.
fn flatten_cubic(
    p0: [f32; 2],
    c1: [f32; 2],
    c2: [f32; 2],
    p1: [f32; 2],
    sink: &mut impl FnMut([f32; 2], [f32; 2]),
) {
    let d1x = p0[0] - 2.0 * c1[0] + c2[0];
    let d1y = p0[1] - 2.0 * c1[1] + c2[1];
    let d2x = c1[0] - 2.0 * c2[0] + p1[0];
    let d2y = c1[1] - 2.0 * c2[1] + p1[1];
    let dev = (d1x * d1x + d1y * d1y).max(d2x * d2x + d2y * d2y).sqrt() * 0.75;
    let n = (dev / (4.0 * FLATTEN_TOL)).sqrt().ceil().clamp(1.0, 96.0) as usize;
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
        let q = [
            a * p0[0] + b * c1[0] + c * c2[0] + d * p1[0],
            a * p0[1] + b * c1[1] + c * c2[1] + d * p1[1],
        ];
        sink(prev, q);
        prev = q;
    }
}
