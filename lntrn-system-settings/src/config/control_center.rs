//! `[control_center]`: the Control Center, the bar along the bottom of
//! the screen, which reads these keys as they change. It floats this far
//! from the screen's edges and docks full width and square while a window
//! is maximized or full screen on its monitor.

use lntrn_props::props;

/// Each key's range: what the page's sliders cover and what the bar holds
/// a hand-edited value to (`lntrn-control-center`'s `Shape`).
pub const CC_HEIGHTS: (i64, i64) = (40, 120);
pub const CC_RADII: (i64, i64) = (0, 32);
pub const CC_GAPS_BELOW: (i64, i64) = (0, 40);
pub const CC_GAPS_SIDES: (i64, i64) = (0, 120);

props! {
    /// The bar's size and where it floats, in logical pixels.
    pub struct ControlCenter {
        pub height: i64 = 64 => { id: 1, hard: 40..=120 },
        /// While it floats; docked, it is square.
        pub corner_radius: i64 = 16 => { id: 2, hard: 0..=32 },
        /// Between the floating bar and the bottom of the screen.
        pub gap_below: i64 = 8 => { id: 3, hard: 0..=40 },
        /// Between the floating bar and the left and right edges.
        pub gap_sides: i64 = 16 => { id: 4, hard: 0..=120 },
    }
}

impl ControlCenter {
    pub fn clamp(&mut self) {
        let hold = |v: i64, (lo, hi): (i64, i64)| v.clamp(lo, hi);
        self.height = hold(self.height, CC_HEIGHTS);
        self.corner_radius = hold(self.corner_radius, CC_RADII);
        self.gap_below = hold(self.gap_below, CC_GAPS_BELOW);
        self.gap_sides = hold(self.gap_sides, CC_GAPS_SIDES);
    }
}
