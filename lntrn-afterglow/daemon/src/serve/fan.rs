//! The fan curve, followed once a second: NVIDIA's driver only takes a
//! fixed speed, so the helper reads the card's heat and sets the speed
//! its curve gives.
//!
//! It climbs at once but comes down only once the card is a few degrees
//! cooler than the point it climbed at, and then slowly, so fans don't
//! hunt up and down around a point. Near the heat where the card slows
//! itself it runs the fans flat out whatever the curve says, and if the
//! card stops saying how hot it is the fans go back to the card's own
//! curve.

use lntrn_afterglowd::tune::{Point, curve_at};

/// How much cooler than a point the card must be before the speed it
/// climbed to there comes down, °C.
const HYSTERESIS: f64 = 4.0;
/// The most the speed comes down in one step (a second), percent.
const MAX_DROP: u32 = 2;
/// How far below its slowdown heat the card runs its fans flat out, °C.
const FAILSAFE_MARGIN: u32 = 5;
/// How many readings in a row may go missing before the curve gives up.
const MISSES: u32 = 3;

/// What to do with the fans this step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Hold,
    Set(u32),
    /// Give them back to the card.
    Auto,
}

#[derive(Default)]
pub struct FanLoop {
    curve: Option<Vec<Point>>,
    /// The speed last set.
    last: Option<u32>,
    misses: u32,
    /// Why the curve gave up, for the window to say.
    pub trouble: Option<String>,
}

impl FanLoop {
    pub fn start(&mut self, points: Vec<Point>) {
        *self = FanLoop { curve: Some(points), ..FanLoop::default() };
    }

    pub fn stop(&mut self) {
        self.curve = None;
        self.last = None;
    }

    pub fn running(&self) -> bool {
        self.curve.is_some()
    }

    /// The speed the curve has the fans at.
    pub fn percent(&self) -> Option<u32> {
        self.curve.as_ref().and(self.last)
    }

    /// Give up on the curve, saying why.
    pub fn fail(&mut self, why: String) {
        self.stop();
        self.trouble = Some(why);
    }

    /// One step: the card's heat now (`None` when it wouldn't say), the
    /// speeds its fans may be set to, and the heat it slows itself at.
    pub fn step(&mut self, celsius: Option<u32>, (lo, hi): (u32, u32), slowdown: u32) -> Step {
        let Some(curve) = &self.curve else { return Step::Hold };
        let Some(celsius) = celsius else {
            self.misses += 1;
            if self.misses >= MISSES {
                self.fail("The card stopped saying how hot it is, so its fans went back to their own curve.".to_owned());
                return Step::Auto;
            }
            return Step::Hold;
        };
        self.misses = 0;
        let target = if celsius + FAILSAFE_MARGIN >= slowdown {
            hi
        } else {
            let c = f64::from(celsius);
            let wanted = curve_at(curve, c).round() as u32;
            match self.last {
                Some(last) if wanted < last => {
                    let cooler = curve_at(curve, c + HYSTERESIS).round() as u32;
                    if cooler >= last { last } else { cooler.max(last.saturating_sub(MAX_DROP)) }
                }
                _ => wanted,
            }
        };
        let target = target.clamp(lo, hi);
        if self.last == Some(target) {
            return Step::Hold;
        }
        self.last = Some(target);
        Step::Set(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RANGE: (u32, u32) = (30, 100);

    fn running() -> FanLoop {
        let mut f = FanLoop::default();
        f.start(vec![Point::new(40, 30), Point::new(60, 50), Point::new(80, 100)]);
        f
    }

    #[test]
    fn it_climbs_at_once_and_holds_until_well_cooler() {
        let mut f = running();
        assert_eq!(f.step(Some(50), RANGE, 95), Step::Set(40));
        assert_eq!(f.step(Some(50), RANGE, 95), Step::Hold, "nothing changed");
        assert_eq!(f.step(Some(70), RANGE, 95), Step::Set(75), "up at once");
        // A degree or two cooler: still where it climbed to.
        assert_eq!(f.step(Some(68), RANGE, 95), Step::Hold);
        assert_eq!(f.step(Some(66), RANGE, 95), Step::Hold);
        // Well cooler: down, two points at a time.
        assert_eq!(f.step(Some(50), RANGE, 95), Step::Set(73));
        assert_eq!(f.step(Some(50), RANGE, 95), Step::Set(71));
        assert_eq!(f.percent(), Some(71));
    }

    #[test]
    fn near_the_slowdown_heat_it_runs_flat_out_and_never_below_the_least() {
        let mut f = running();
        assert_eq!(f.step(Some(90), RANGE, 95), Step::Set(100));
        let mut low = FanLoop::default();
        low.start(vec![Point::new(40, 0), Point::new(60, 10)]);
        assert_eq!(low.step(Some(30), RANGE, 95), Step::Set(30), "held to the fans' least");
    }

    #[test]
    fn a_card_that_goes_quiet_gets_its_own_curve_back() {
        let mut f = running();
        assert_eq!(f.step(Some(50), RANGE, 95), Step::Set(40));
        assert_eq!(f.step(None, RANGE, 95), Step::Hold);
        assert_eq!(f.step(None, RANGE, 95), Step::Hold);
        assert_eq!(f.step(None, RANGE, 95), Step::Auto);
        assert!(!f.running() && f.trouble.is_some());
        assert_eq!(f.step(Some(50), RANGE, 95), Step::Hold, "stopped");
        // A new curve clears the trouble.
        f.start(vec![Point::new(40, 30), Point::new(80, 100)]);
        assert!(f.trouble.is_none());
    }
}
