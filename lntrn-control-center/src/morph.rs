//! The plate's glides, picked up from wherever it is when it turns round:
//! between floating (0) and docked (1), so a window maximized and let go
//! at once never makes it jump, and (auto hide) between hidden (0) and
//! shown (1), so the pointer coming back mid-slide sends it straight back
//! up. Docking keeps step with the window filling
//! the screen: the compositor's maximize glide, its time and curve
//! (`[animations]`, as lntrn-compositor's `animations.rs` reads it), so the
//! two land together. Floating again is the bar's own quick ease-out.

/// How a glide eases, as the compositor's `rect_anim::Curve` does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Linear,
    EaseOutCubic,
    /// Slow ends, quick middle: the `cinematic` preset.
    EaseInOutQuint,
    /// Overshoots and settles: the `springy` preset. The plate can't go
    /// past docked, so it holds there through the overshoot.
    EaseOutBack,
}

impl Curve {
    fn eval(self, p: f64) -> f64 {
        let p = p.clamp(0.0, 1.0);
        match self {
            Curve::Linear => p,
            Curve::EaseOutCubic => 1.0 - (1.0 - p).powi(3),
            Curve::EaseInOutQuint if p < 0.5 => 16.0 * p.powi(5),
            Curve::EaseInOutQuint => 1.0 - (2.0 - 2.0 * p).powi(5) / 2.0,
            Curve::EaseOutBack => {
                const OVERSHOOT: f64 = 1.70158;
                let inv = p - 1.0;
                1.0 + (OVERSHOOT + 1.0) * inv.powi(3) + OVERSHOOT * inv * inv
            }
        }
    }
}

/// How long a whole glide takes, and how it eases.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    pub seconds: f64,
    pub curve: Curve,
}

impl Glide {
    /// Back to floating: quick to leave, gentle to land.
    pub const UNDOCK: Glide = Glide { seconds: 0.2, curve: Curve::EaseOutCubic };
    /// No glide at all: Reduce Motion, or window animations switched off.
    pub const NONE: Glide = Glide { seconds: 0.0, curve: Curve::Linear };
    /// The compositor's base time for a window-state change, before speed.
    const WINDOW_STATE: f64 = 1.0;

    /// The compositor's maximize glide: off when `on` is false (`enabled`
    /// or `state`), its base time over `speed` (held to 0.25..4, as it
    /// holds it), eased the way `preset` eases a window.
    pub fn windows(on: bool, speed: f64, preset: &str) -> Glide {
        if !on {
            return Glide::NONE;
        }
        let speed = if speed.is_finite() { speed.clamp(0.25, 4.0) } else { 1.0 };
        let curve = match preset {
            "snappy" => Curve::EaseOutCubic,
            "springy" => Curve::EaseOutBack,
            "linear" => Curve::Linear,
            _ => Curve::EaseInOutQuint,
        };
        Glide { seconds: Glide::WINDOW_STATE / speed, curve }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Morph {
    from: f64,
    to: f64,
    /// When it set off, on the UI's clock.
    start: f64,
    /// How long this leg takes.
    span: f64,
    curve: Curve,
}

impl Morph {
    /// At rest, at 1 (`on`) or 0.
    pub fn at(on: bool) -> Morph {
        let t = if on { 1.0 } else { 0.0 };
        Morph { from: t, to: t, start: 0.0, span: 0.0, curve: Curve::Linear }
    }

    /// Head for 1 (`on`) or 0 from where it is at `now`. A part of the
    /// way takes its part of `glide`'s time.
    pub fn aim(&mut self, on: bool, now: f64, glide: Glide) {
        let to = if on { 1.0 } else { 0.0 };
        let from = self.value(now);
        let span = glide.seconds.max(0.0) * (to - from).abs();
        *self = Morph { from, to, start: now, span, curve: glide.curve };
    }

    /// Where it is at `now`, 0 to 1.
    pub fn value(&self, now: f64) -> f64 {
        // Landed is exactly there, not a rounding error short of it.
        if !self.moving(now) {
            return self.to;
        }
        let eased = self.curve.eval((now - self.start) / self.span).min(1.0);
        self.from + (self.to - self.from) * eased
    }

    /// Whether it is still on its way at `now`.
    pub fn moving(&self, now: f64) -> bool {
        self.span > 0.0 && now < self.start + self.span
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUICK: Glide = Glide { seconds: 0.2, curve: Curve::EaseOutCubic };

    #[test]
    fn it_glides_and_lands() {
        let mut m = Morph::at(false);
        assert_eq!((m.value(5.0), m.moving(5.0)), (0.0, false));
        m.aim(true, 10.0, QUICK);
        assert_eq!(m.value(10.0), 0.0);
        let half = m.value(10.1);
        assert!(half > 0.5 && half < 1.0, "ease-out: past halfway at half time, {half}");
        assert!(m.moving(10.1));
        assert_eq!((m.value(10.2), m.moving(10.2)), (1.0, false));
    }

    #[test]
    fn turning_round_starts_where_it_is_and_takes_its_part_of_the_time() {
        let mut m = Morph::at(false);
        m.aim(true, 0.0, QUICK);
        let mid = m.value(0.05);
        m.aim(false, 0.05, QUICK);
        assert_eq!(m.value(0.05), mid, "no jump");
        assert!(m.moving(0.05 + 0.2 * mid - 0.001));
        assert!(!m.moving(0.05 + 0.2 * mid + 0.001));
    }

    #[test]
    fn no_glide_lands_at_once_and_aiming_where_it_is_stays_put() {
        let mut m = Morph::at(false);
        m.aim(true, 1.0, Glide::NONE);
        assert_eq!((m.value(1.0), m.moving(1.0)), (1.0, false));
        m.aim(true, 2.0, QUICK);
        assert!(!m.moving(2.0));
        assert_eq!(Morph::at(true).value(0.0), 1.0);
    }

    #[test]
    fn docking_keeps_the_windows_time_and_curve() {
        let w = Glide::windows(true, 1.0, "cinematic");
        assert_eq!((w.seconds, w.curve), (1.0, Curve::EaseInOutQuint));
        assert_eq!(Glide::windows(true, 2.0, "springy"), Glide { seconds: 0.5, curve: Curve::EaseOutBack });
        assert_eq!(Glide::windows(true, 100.0, "snappy").seconds, 0.25, "speed held to 4");
        assert_eq!(Glide::windows(false, 1.0, "linear"), Glide::NONE);
        // The springy overshoot holds at docked instead of passing it.
        let mut m = Morph::at(false);
        m.aim(true, 0.0, Glide::windows(true, 1.0, "springy"));
        assert!((0..=100).all(|i| m.value(f64::from(i) / 100.0) <= 1.0));
        assert_eq!(m.value(1.0), 1.0);
    }
}
