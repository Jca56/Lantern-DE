//! Item-to-item snapping for move/resize drags, plus the guide lines drawn
//! while a snap is active. All math is in canvas units; callers convert the
//! screen-space threshold with `SNAP_PX / zoom`.

use super::doc::CanvasItem;

/// Snap distance in logical screen px.
pub const SNAP_PX: f32 = 10.0;
/// Two coordinates closer than this count as "on the same line".
const EPS: f32 = 0.01;

/// A guide line in canvas units: a vertical guide sits at x = `pos` and spans
/// `a..b` in y; a horizontal one sits at y = `pos` and spans `a..b` in x.
#[derive(Clone, Copy, Debug)]
pub struct Guide {
    pub pos: f32,
    pub a: f32,
    pub b: f32,
}

#[derive(Default, Clone, Debug)]
pub struct SnapGuides {
    pub vertical: Vec<Guide>,
    pub horizontal: Vec<Guide>,
}

impl SnapGuides {
    pub fn clear(&mut self) {
        self.vertical.clear();
        self.horizontal.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.vertical.is_empty() && self.horizontal.is_empty()
    }
}

/// Alignment lines gathered from every item except the one being edited.
pub struct SnapTargets {
    /// Vertical lines: (x, y0, y1) — the source item's vertical extent.
    pub xs: Vec<(f32, f32, f32)>,
    /// Horizontal lines: (y, x0, x1).
    pub ys: Vec<(f32, f32, f32)>,
}

impl SnapTargets {
    /// Edges (and optionally centers) of every item except `skip`.
    pub fn gather(items: &[CanvasItem], skip: usize, centers: bool) -> Self {
        let mut xs = Vec::with_capacity(items.len() * 3);
        let mut ys = Vec::with_capacity(items.len() * 3);
        for (i, it) in items.iter().enumerate() {
            if i == skip {
                continue;
            }
            let (x0, x1, y0, y1) = (it.x, it.x + it.w, it.y, it.y + it.h);
            xs.push((x0, y0, y1));
            xs.push((x1, y0, y1));
            ys.push((y0, x0, x1));
            ys.push((y1, x0, x1));
            if centers {
                xs.push((it.x + it.w * 0.5, y0, y1));
                ys.push((it.y + it.h * 0.5, x0, x1));
            }
        }
        Self { xs, ys }
    }

    /// Smallest x-delta (target − probe) that lands any probe on a target,
    /// or `None` if nothing is within `threshold`.
    pub fn nearest_x(&self, probes: &[f32], threshold: f32) -> Option<f32> {
        best_delta(self.xs.iter().map(|t| t.0), probes, threshold)
    }

    pub fn nearest_y(&self, probes: &[f32], threshold: f32) -> Option<f32> {
        best_delta(self.ys.iter().map(|t| t.0), probes, threshold)
    }
}

fn best_delta(targets: impl Iterator<Item = f32>, probes: &[f32], threshold: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in targets {
        for &p in probes {
            let d = t - p;
            if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.abs()) {
                best = Some(d);
            }
        }
    }
    best
}

/// Snap a moving rect's edges and centers. Returns the snapped origin and
/// the guides for every line it now sits on.
pub fn snap_move(
    targets: &SnapTargets,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    threshold: f32,
) -> (f32, f32, SnapGuides) {
    let mut nx = x;
    let mut ny = y;
    if let Some(d) = targets.nearest_x(&[x, x + w * 0.5, x + w], threshold) {
        nx += d;
    }
    if let Some(d) = targets.nearest_y(&[y, y + h * 0.5, y + h], threshold) {
        ny += d;
    }
    let guides = guides_for(
        targets,
        nx,
        ny,
        w,
        h,
        &[nx, nx + w * 0.5, nx + w],
        &[ny, ny + h * 0.5, ny + h],
    );
    (nx, ny, guides)
}

/// Guides for every target line one of the probes sits exactly on. Each guide
/// spans the union of the edited rect and the target's source item so the
/// line visibly connects the two.
pub fn guides_for(
    targets: &SnapTargets,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    probes_x: &[f32],
    probes_y: &[f32],
) -> SnapGuides {
    let mut g = SnapGuides::default();
    for &(tx, y0, y1) in &targets.xs {
        if probes_x.iter().any(|&p| (p - tx).abs() < EPS) {
            g.vertical.push(Guide {
                pos: tx,
                a: y.min(y0),
                b: (y + h).max(y1),
            });
        }
    }
    for &(ty, x0, x1) in &targets.ys {
        if probes_y.iter().any(|&p| (p - ty).abs() < EPS) {
            g.horizontal.push(Guide {
                pos: ty,
                a: x.min(x0),
                b: (x + w).max(x1),
            });
        }
    }
    g
}
