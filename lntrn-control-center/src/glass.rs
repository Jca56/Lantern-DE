//! Glass: where the bar's popups are painted see-through this frame (the
//! windows' opacity), so the compositor can blur what's behind them
//! (`blur:` on its socket). Each panel says where it went as it's
//! painted; the frame takes the lot and tells the compositor.

use std::cell::RefCell;

use lntrn_math::{Rect, Vec2};

thread_local! {
    static PANES: RefCell<Vec<(Rect, f64)>> = const { RefCell::new(Vec::new()) };
}

/// A panel painted see-through over `rect` (window pixels), its corners
/// rounded by `radius`.
pub fn pane(rect: Rect, radius: f64) {
    PANES.with_borrow_mut(|panes| panes.push((rect, radius)));
}

/// The panels painted since the last call.
pub fn take() -> Vec<(Rect, f64)> {
    PANES.with_borrow_mut(std::mem::take)
}

/// The `blur:` line for `panes` (window pixels, `per` of them a logical
/// one across and down: a buffer rounds each its own way) on the surface
/// called `namespace`; none clears it.
pub fn line(namespace: &str, panes: &[(Rect, f64)], per: Vec2) -> String {
    let (across, down) = (per.x.max(f64::EPSILON), per.y.max(f64::EPSILON));
    let mut line = format!("blur:{namespace}");
    for (r, radius) in panes {
        let n = [r.min.x / across, r.min.y / down, r.width() / across, r.height() / down, radius / down].map(|v| v.round() as i64);
        line.push_str(&format!("\t{} {} {} {} {}", n[0], n[1], n[2], n[3], n[4]));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_go_to_the_compositor_in_logical_pixels() {
        let plate = Rect::new(Vec2::new(26.0, 13.0), Vec2::new(2470.0, 96.0));
        assert_eq!(line("lntrn-control-center", &[(plate, 20.8)], Vec2::splat(1.3)), "blur:lntrn-control-center\t20 10 1880 64 16");
        assert_eq!(line("lntrn-control-center", &[], Vec2::splat(1.3)), "blur:lntrn-control-center", "none clears it");
        pane(plate, 4.0);
        assert_eq!(take().len(), 1);
        assert!(take().is_empty(), "taken once");
    }
}
