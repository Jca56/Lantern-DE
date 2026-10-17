//! The edge barrier. A bar hidden at its output's bottom edge (`edge:` on
//! the workspace socket, by its layer namespace) comes up when the pointer
//! is pushed against that edge; with a monitor below, the pointer would
//! pass straight through onto it instead. So it stops on the edge's last
//! row, where the bar hears it, and only a push of [`PUSH`] more takes it
//! on across.

use smithay::utils::{Logical, Point};

use crate::state::Lantern;

/// How far past the edge the pointer is pushed before it crosses, in
/// logical pixels of motion.
const PUSH: f64 = 40.0;

impl Lantern {
    /// Where the pointer goes from `from` when it would go to `to`: held
    /// on a hidden bar's edge until it's pushed through.
    pub(crate) fn edge_barrier(&mut self, from: Point<f64, Logical>, to: Point<f64, Logical>) -> Point<f64, Logical> {
        let output = self.output_containing(from);
        let barred = output.as_ref().is_some_and(|o| self.workspace_ipc.edges.iter().any(|ns| self.layer_origin(ns).is_some_and(|(out, _)| out.name() == o.name())));
        let Some(geo) = output.filter(|_| barred).and_then(|o| self.workspaces.output_geometry(&o)) else {
            self.edge_push = 0.0;
            return to;
        };
        let edge = f64::from(geo.loc.y + geo.size.h) - 1.0;
        let (at, pushed) = held(from, to, edge, self.edge_push);
        self.edge_push = pushed;
        at
    }
}

/// From `from` toward `to`, against an edge on row `edge` already pushed
/// past by `pushed`: where the pointer goes, and how far it's pushed now.
fn held(from: Point<f64, Logical>, to: Point<f64, Logical>, edge: f64, pushed: f64) -> (Point<f64, Logical>, f64) {
    if to.y < edge {
        // Away from it: the next push starts over.
        return (to, 0.0);
    }
    let pushed = pushed + (to.y - from.y.max(edge));
    if pushed >= PUSH { (to, 0.0) } else { ((to.x, edge).into(), pushed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Point<f64, Logical> {
        (x, y).into()
    }

    #[test]
    fn the_pointer_stops_on_the_edge_until_pushed_through() {
        let edge = 1079.0;
        // Coming down fast: it stops on the edge's row.
        let (p, pushed) = held(at(500.0, 1060.0), at(505.0, 1100.0), edge, 0.0);
        assert_eq!((p, pushed), (at(505.0, edge), 21.0));
        // Pushed on, it crosses once the push adds up.
        let (p, pushed) = held(p, at(505.0, edge + 10.0), edge, pushed);
        assert_eq!((p.y, pushed), (edge, 31.0));
        let (p, pushed) = held(p, at(505.0, edge + 10.0), edge, pushed);
        assert_eq!((p.y, pushed), (edge + 10.0, 0.0));
        // Along the edge, the push is kept; away from it, it starts over.
        assert_eq!(held(at(500.0, edge), at(530.0, edge), edge, 12.0).1, 12.0);
        assert_eq!(held(at(500.0, edge), at(500.0, 1000.0), edge, 12.0), (at(500.0, 1000.0), 0.0));
    }
}
