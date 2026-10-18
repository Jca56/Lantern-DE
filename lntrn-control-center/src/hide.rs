//! Auto hide: the bar out of sight until the pointer touches the bottom
//! edge of its monitor, then sliding up, and away again a moment after the
//! pointer leaves it (never while the launcher is up). A Super tap while
//! it is away only peeks: up as from the edge, held a while for a look at
//! the clock, then away again unless the pointer came to it (a second tap
//! opens the launcher). Nothing of it stays
//! lit, for an OLED's sake: hidden, its surface is a see-through strip
//! along the bottom edge that only waits for the pointer.
//!
//! The bar never sits over a window. As it slides, the space it keeps from
//! windows ([`zone`]) is exactly as much of it as is on the screen, frame
//! by frame, and the compositor fits a maximized window to that on every
//! commit: the window's edge rides up ahead of the plate and back down
//! behind it, floating or docked, however long the slide is and wherever
//! it turns round.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::Metrics;

use crate::morph::{Curve, Glide, Morph};

/// How tall the hidden strip is, logical px: enough to catch a pointer
/// pushed against the screen's edge at any scale.
pub const STRIP: u32 = 2;

/// `[control_center] visibility`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Always,
    AutoHide,
}

impl Visibility {
    /// From its stored word; anything else is the default.
    pub fn from_word(word: &str) -> Visibility {
        match word {
            "auto_hide" => Visibility::AutoHide,
            _ => Visibility::Always,
        }
    }
}

/// `[control_center]`'s `visibility` and `auto_hide_*` keys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HideOptions {
    pub visibility: Visibility,
    /// How long after the pointer leaves the bar it sets off down, s.
    pub delay: f64,
    /// How long a slide takes, s.
    pub slide: f64,
    /// How long a peek holds the bar up once it is all the way up, s.
    pub peek: f64,
}

impl HideOptions {
    /// `auto_hide_delay_ms` and `auto_hide_slide_ms`, as System Settings'
    /// sliders offer them.
    pub const DELAYS_MS: (i64, i64) = (0, 3000);
    pub const SLIDES_MS: (i64, i64) = (0, 1000);
    pub const PEEKS_MS: (i64, i64) = (500, 5000);

    pub fn auto(self) -> bool {
        self.visibility == Visibility::AutoHide
    }

    /// The slide, eased; none at all under Reduce Motion.
    pub fn glide(self, reduce_motion: bool) -> Glide {
        if reduce_motion {
            return Glide::NONE;
        }
        Glide { seconds: self.slide, curve: Curve::EaseOutCubic }
    }
}

impl Default for HideOptions {
    fn default() -> Self {
        HideOptions { visibility: Visibility::Always, delay: 0.5, slide: 0.25, peek: 2.0 }
    }
}

/// How far the bar is with being shown, and the surface that takes.
pub struct Reveal {
    /// 0 hidden to 1 shown.
    morph: Morph,
    /// Where it is heading.
    shown: bool,
    /// When to set off down, the pointer having left.
    hide_at: Option<f64>,
    /// A peek on its way up, and how long to hold it there once up.
    peek: Option<f64>,
    /// Whether the surface should be the bar's full height, else the strip.
    full: bool,
}

impl Reveal {
    /// Shown, on its full-height surface, as the bar starts: auto hide
    /// lets it go a moment later, so a fresh bar says it is there.
    pub fn new() -> Reveal {
        Reveal { morph: Morph::at(true), shown: true, hide_at: None, peek: None, full: true }
    }

    /// Whether the bar is out of sight or on its way there.
    pub fn away(&self) -> bool {
        !self.shown
    }

    /// Come up as if the pointer had, and once all the way up, stay
    /// `hold` s before going as after the pointer leaves.
    pub fn peek(&mut self, hold: f64) {
        self.peek = Some(hold.max(0.0));
    }

    /// Take a frame at `now`: the bar is `want`ed (the pointer is on it,
    /// the launcher is up, or it never hides), on a surface `grown` to its
    /// full height or not yet. Returns how long until it needs a frame of
    /// its own, while it waits out the delay.
    pub fn step(&mut self, now: f64, want: bool, grown: bool, glide: Glide, delay: f64) -> Option<f64> {
        if let Some(hold) = self.peek
            && self.shown
            && !self.morph.moving(now)
        {
            // A peek all the way up: held its own while, then as ever.
            self.peek = None;
            if !want {
                self.hide_at = Some(now + hold);
            }
        }
        let want = want || self.peek.is_some();
        let mut wait = None;
        if want {
            self.hide_at = None;
            self.full = true;
            // Up once there is room to come up in.
            if !self.shown && grown {
                self.shown = true;
                self.morph.aim(true, now, glide);
            }
        } else if self.shown {
            let at = *self.hide_at.get_or_insert(now + delay.max(0.0));
            if now < at {
                wait = Some(at - now);
            } else {
                self.hide_at = None;
                self.shown = false;
                self.morph.aim(false, now, glide);
            }
        }
        if !want && !self.shown && !self.morph.moving(now) {
            // Out of sight: down to the strip.
            self.full = false;
        }
        wait
    }

    /// 0 hidden to 1 shown, at `now`.
    pub fn value(&self, now: f64) -> f64 {
        self.morph.value(now)
    }

    pub fn moving(&self, now: f64) -> bool {
        self.morph.moving(now)
    }

    /// Whether the surface should be the bar's full height.
    pub fn full(&self) -> bool {
        self.full
    }
}

/// Whether `surface` (window px) is the bar's full height yet rather than
/// the strip: the compositor answers a new size a frame or so after it is
/// asked for.
pub fn grown(surface: Rect, m: Metrics) -> bool {
    surface.height() > m.px(f64::from(STRIP)) * 2.0
}

/// The strip along the bottom of `surface` the pointer brings the bar up
/// from. Kept in the shown bar's input region too, so a pointer resting
/// on the screen's edge keeps it up rather than hiding it under itself.
pub fn strip(surface: Rect, m: Metrics) -> Rect {
    Rect::new(Vec2::new(surface.min.x, surface.max.y - m.px(f64::from(STRIP))), surface.max)
}

/// The space to keep from windows with the bar `shown` of the way up: as
/// much of it as is on the screen, of `travel` logical px when all of it
/// is, rounded down so a window's edge goes under the plate rather than
/// leaving a gap above it; `full` once it is all the way up.
pub fn zone(shown: f64, travel: f64, full: u32) -> u32 {
    if shown >= 1.0 {
        return full;
    }
    ((shown * travel).floor().max(0.0) as u32).min(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLIDE: Glide = Glide { seconds: 0.2, curve: Curve::Linear };
    const DELAY: f64 = 0.5;

    #[test]
    fn it_waits_out_the_delay_slides_down_and_shrinks_to_the_strip() {
        let mut r = Reveal::new();
        assert_eq!(r.step(0.0, true, true, GLIDE, DELAY), None);
        assert_eq!((r.value(0.0), r.full()), (1.0, true));
        // The pointer leaves at 1: half a second to come back.
        assert_eq!(r.step(1.0, false, true, GLIDE, DELAY), Some(0.5));
        assert!(r.step(1.4, false, true, GLIDE, DELAY).is_some_and(|w| (w - 0.1).abs() < 1e-9));
        assert_eq!(r.value(1.4), 1.0, "still up while it waits");
        assert_eq!(r.step(1.5, false, true, GLIDE, DELAY), None);
        assert!(r.moving(1.6) && r.full(), "the surface stays tall while it slides");
        r.step(1.7, false, true, GLIDE, DELAY);
        assert_eq!((r.value(1.7), r.full()), (0.0, false), "landed: down to the strip");
    }

    #[test]
    fn the_edge_grows_the_surface_first_then_it_slides_up() {
        let mut r = Reveal::new();
        r.step(0.0, false, true, GLIDE, 0.0);
        r.step(0.2, false, true, GLIDE, 0.0);
        assert!(!r.full());
        r.step(5.0, true, false, GLIDE, DELAY);
        assert!(r.full(), "asks for its full height");
        assert!(!r.moving(5.0) && r.value(5.0) == 0.0, "but waits for it");
        r.step(5.05, true, true, GLIDE, DELAY);
        assert!(r.moving(5.1));
        assert_eq!(r.value(5.25), 1.0);
    }

    #[test]
    fn the_pointer_back_mid_slide_turns_it_round_and_the_wait_starts_over() {
        let mut r = Reveal::new();
        r.step(0.0, false, true, GLIDE, DELAY);
        r.step(0.5, false, true, GLIDE, DELAY);
        let mid = r.value(0.6);
        assert!(mid > 0.0 && mid < 1.0);
        r.step(0.6, true, true, GLIDE, DELAY);
        assert_eq!(r.value(0.6), mid, "no jump");
        assert!(r.full());
        assert_eq!(r.value(0.8), 1.0);
        // A fresh leave waits the whole delay again.
        assert_eq!(r.step(2.0, false, true, GLIDE, DELAY), Some(0.5));
    }

    #[test]
    fn a_peek_comes_up_from_the_strip_holds_then_goes() {
        let mut r = Reveal::new();
        r.step(0.0, false, true, GLIDE, 0.0);
        r.step(0.2, false, true, GLIDE, 0.0);
        assert!(r.away() && !r.full());
        r.peek(2.0);
        r.step(1.0, false, false, GLIDE, DELAY);
        assert!(r.full() && r.value(1.0) == 0.0, "asks for its full height, then waits for it");
        r.step(1.05, false, true, GLIDE, DELAY);
        assert!(!r.away() && r.moving(1.1));
        r.step(1.1, false, true, GLIDE, DELAY);
        // Landed at 1.25: the hold starts on the frame after, not the
        // hover delay.
        assert!(r.step(1.3, false, true, GLIDE, DELAY).is_some_and(|w| (w - 2.0).abs() < 1e-9));
        assert!(r.step(3.0, false, true, GLIDE, DELAY).is_some_and(|w| (w - 0.3).abs() < 1e-9));
        assert_eq!(r.value(3.0), 1.0, "still up for its look");
        r.step(3.3, false, true, GLIDE, DELAY);
        assert!(r.away() && r.moving(3.4));
        r.step(3.6, false, true, GLIDE, DELAY);
        assert_eq!((r.value(3.6), r.full()), (0.0, false), "away again, down to the strip");
    }

    #[test]
    fn the_pointer_coming_to_a_peek_keeps_it_up_like_a_hover() {
        let mut r = Reveal::new();
        r.step(0.0, false, true, GLIDE, 0.0);
        r.step(0.2, false, true, GLIDE, 0.0);
        r.peek(2.0);
        r.step(1.0, false, true, GLIDE, DELAY);
        r.step(1.3, false, true, GLIDE, DELAY);
        // The pointer comes and stays past the hold.
        assert_eq!(r.step(1.5, true, true, GLIDE, DELAY), None);
        assert_eq!(r.step(9.0, true, true, GLIDE, DELAY), None);
        assert_eq!(r.value(9.0), 1.0);
        // It leaves: the hover delay, not the peek's hold.
        assert_eq!(r.step(10.0, false, true, GLIDE, DELAY), Some(DELAY));
    }

    #[test]
    fn a_peek_turns_a_bar_on_its_way_down_round() {
        let mut r = Reveal::new();
        r.step(0.0, false, true, GLIDE, 0.0);
        r.step(0.1, false, true, GLIDE, 0.0);
        let mid = r.value(0.1);
        assert!(r.away() && mid > 0.0 && mid < 1.0);
        r.peek(1.0);
        r.step(0.1, false, true, GLIDE, DELAY);
        assert!(!r.away());
        assert_eq!(r.value(0.1), mid, "no jump");
    }

    #[test]
    fn the_zone_is_as_much_of_the_bar_as_shows() {
        assert_eq!(zone(1.0, 72.3, 72), 72, "all the way up: exactly the bar's own");
        assert_eq!(zone(0.0, 72.0, 72), 0);
        assert_eq!(zone(0.5, 72.0, 72), 36);
        assert_eq!(zone(0.51, 64.6, 64), 32, "rounded down");
        assert_eq!(zone(0.999, 80.0, 64), 64, "never past the bar's own");
    }

    #[test]
    fn visibility_reads_its_word() {
        assert_eq!(Visibility::from_word("auto_hide"), Visibility::AutoHide);
        assert_eq!(Visibility::from_word("always"), Visibility::Always);
        assert_eq!(Visibility::from_word("sometimes"), Visibility::Always);
        assert_eq!(HideOptions::default().glide(true), Glide::NONE);
    }
}
