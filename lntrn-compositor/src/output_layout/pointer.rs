//! Keeping the pointer on the layout, and in its place on it.

use smithay::utils::{Logical, Point};

use crate::Lantern;

/// The pointer's place on an output: which one, and how far across and down
/// it, as a share of its size.
pub(crate) struct PointerAnchor {
    output: String,
    fx: f64,
    fy: f64,
}

impl Lantern {
    /// The point of the layout nearest to `pos`: `pos` itself when an output
    /// shows it. The outputs' bounding box is not the layout — beside a
    /// smaller monitor it has corners no output covers.
    pub(crate) fn nearest_point_on_layout(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        let mut best: Option<(f64, Point<f64, Logical>)> = None;
        for output in self.workspaces.outputs_iter() {
            let Some(geo) = self.workspaces.output_geometry(output) else {
                continue;
            };
            let (left, top) = (geo.loc.x as f64, geo.loc.y as f64);
            let x = pos.x.clamp(left, (left + geo.size.w as f64 - 1.0).max(left));
            let y = pos.y.clamp(top, (top + geo.size.h as f64 - 1.0).max(top));
            let d = (pos.x - x).powi(2) + (pos.y - y).powi(2);
            if best.map_or(true, |(b, _)| d < b) {
                best = Some((d, Point::from((x, y))));
            }
        }
        best.map_or(pos, |(_, p)| p)
    }

    /// Where the pointer is, as a share of the output showing it. Taken
    /// before a layout change and given to [`Self::relayout_from`], it keeps
    /// the cursor on the same monitor at the same spot of the glass: an
    /// output that moves or changes scale would otherwise slide out from
    /// under a pointer that stayed at its old global coordinates. `None`
    /// before the pointer has ever moved (it then stays at the origin).
    pub(crate) fn pointer_anchor(&self) -> Option<PointerAnchor> {
        self.last_pointer_render_location?;
        let pos = self.seat.get_pointer()?.current_location();
        self.workspaces.outputs_iter().find_map(|o| {
            let geo = self.workspaces.output_geometry(o)?.to_f64();
            geo.contains(pos).then(|| PointerAnchor {
                output: o.name(),
                fx: (pos.x - geo.loc.x) / geo.size.w.max(1.0),
                fy: (pos.y - geo.loc.y) / geo.size.h.max(1.0),
            })
        })
    }

    /// Put the pointer back on its anchor, or with the anchor's output gone
    /// (or no anchor) on the nearest point of the layout, and let whatever
    /// is under it now have pointer focus.
    pub(super) fn restore_pointer(&mut self, anchor: Option<PointerAnchor>) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let pos = pointer.current_location();
        let wanted = anchor
            .and_then(|a| {
                let geo = self
                    .workspaces
                    .output_by_name(&a.output)
                    .and_then(|o| self.workspaces.output_geometry(o))?
                    .to_f64();
                Some(Point::from((
                    geo.loc.x + a.fx * geo.size.w,
                    geo.loc.y + a.fy * geo.size.h,
                )))
            })
            .unwrap_or(pos);
        let target = self.nearest_point_on_layout(wanted);
        if (target.x - pos.x).abs() > 0.5 || (target.y - pos.y).abs() > 0.5 {
            self.warp_pointer_to(target);
        }
        // Windows moved even where the pointer didn't.
        self.last_pointer_under = None;
        self.refocus_pointer_at_cursor();
    }
}
