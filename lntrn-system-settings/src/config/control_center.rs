//! `[control_center]`: the Control Center, the bar along the bottom of
//! the screen, which reads these keys as they change: its shape (it
//! floats this far from the screen's edges, and docks full width and
//! square while a window is maximized or full screen on its monitor),
//! whether it hides out of sight between uses, and what its clock shows.

use lntrn_props::props;

/// Each key's range: what the page's sliders cover and what the bar holds
/// a hand-edited value to (`lntrn-control-center`'s `Shape`).
pub const CC_HEIGHTS: (i64, i64) = (40, 120);
pub const CC_RADII: (i64, i64) = (0, 32);
pub const CC_GAPS_BELOW: (i64, i64) = (0, 40);
pub const CC_GAPS_SIDES: (i64, i64) = (0, 120);
/// Auto hide's timings, in milliseconds (the bar's `HideOptions`).
pub const CC_HIDE_DELAYS: (i64, i64) = (0, 3000);
pub const CC_SLIDES: (i64, i64) = (0, 1000);
/// Whether it hides: the word stored, and how the page shows it.
pub const CC_VISIBILITIES: [(&str, &str); 2] = [("always", "Always shown"), ("auto_hide", "Auto hide")];
/// The clock's date styles: the word stored, and how the page shows it
/// (by example, as the bar writes it).
pub const CC_DATE_STYLES: [(&str, &str); 4] = [("short", "Thu, Oct 16"), ("numeric", "10/16/2026"), ("long", "Thursday, October 16"), ("with_year", "Oct 16, 2026")];

props! {
    /// The bar's size and where it floats, in logical pixels, whether it
    /// hides, and what its clock shows.
    pub struct ControlCenter {
        pub height: i64 = 64 => { id: 1, hard: 40..=120 },
        /// While it floats; docked, it is square.
        pub corner_radius: i64 = 16 => { id: 2, hard: 0..=32 },
        /// Between the floating bar and the bottom of the screen.
        pub gap_below: i64 = 8 => { id: 3, hard: 0..=40 },
        /// Between the floating bar and the left and right edges.
        pub gap_sides: i64 = 16 => { id: 4, hard: 0..=120 },
        /// 18:52 instead of 6:52 PM.
        pub clock_24h: bool = false => { id: 5 },
        pub clock_seconds: bool = false => { id: 6 },
        /// The date under the time (beside it on a bar too short for two
        /// lines).
        pub clock_date: bool = true => { id: 7 },
        /// One of [`CC_DATE_STYLES`]' words.
        pub clock_date_style: String = "short".to_owned() => { id: 8 },
        /// 6:52 PM, or just 6:52, on the 12-hour clock.
        pub clock_ampm: bool = true => { id: 9 },
        /// One of [`CC_VISIBILITIES`]' words.
        pub visibility: String = "always".to_owned() => { id: 10 },
        /// How long after the pointer leaves the bar it slides away.
        pub auto_hide_delay_ms: i64 = 500 => { id: 11, hard: 0..=3000 },
        /// How long its slide out of sight, or back, takes.
        pub auto_hide_slide_ms: i64 = 250 => { id: 12, hard: 0..=1000 },
    }
}

impl ControlCenter {
    pub fn clamp(&mut self) {
        let hold = |v: i64, (lo, hi): (i64, i64)| v.clamp(lo, hi);
        self.height = hold(self.height, CC_HEIGHTS);
        self.corner_radius = hold(self.corner_radius, CC_RADII);
        self.gap_below = hold(self.gap_below, CC_GAPS_BELOW);
        self.gap_sides = hold(self.gap_sides, CC_GAPS_SIDES);
        self.auto_hide_delay_ms = hold(self.auto_hide_delay_ms, CC_HIDE_DELAYS);
        self.auto_hide_slide_ms = hold(self.auto_hide_slide_ms, CC_SLIDES);
        if !CC_VISIBILITIES.iter().any(|(word, _)| *word == self.visibility) {
            self.visibility = "always".to_owned();
        }
        if !CC_DATE_STYLES.iter().any(|(word, _)| *word == self.clock_date_style) {
            self.clock_date_style = "short".to_owned();
        }
    }
}
