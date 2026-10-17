//! Levels (a volume, a brightness) as fractions, and stepping them the
//! way the desktop's keys do.

/// `steps` 5% steps from `value` (1.0 is 100%), landing on a multiple of
/// 5%: up from 1.04 is 1.05, down from it 1.00, so the figures shown stay
/// round, as the volume keys step. Not held to any range.
pub fn step5(value: f64, steps: i32) -> f64 {
    let k = value / 0.05;
    let base = match steps.signum() {
        1 => (k + 1e-6).floor(),
        -1 => (k - 1e-6).ceil(),
        _ => return value,
    };
    ((base + f64::from(steps)) * 0.05 * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_land_on_round_figures_as_the_keys_do() {
        assert_eq!(step5(1.04, 1), 1.05);
        assert_eq!(step5(1.04, -1), 1.0);
        assert_eq!(step5(1.0, 1), 1.05);
        assert_eq!(step5(1.0, -1), 0.95);
        // 0.65 is 12.999… steps in floating point.
        assert_eq!(step5(0.65, 1), 0.7);
        assert_eq!(step5(0.65, -1), 0.6);
        assert_eq!(step5(0.3, 3), 0.45);
        assert_eq!(step5(0.42, 0), 0.42);
    }
}
