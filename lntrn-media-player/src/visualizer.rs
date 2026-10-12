//! The audio visualizer: spectrum bars that follow the music, eased over
//! time so they move the same on a 60 Hz laptop panel and a 240 Hz monitor,
//! and that settle to rest when playback pauses or ends.

use lntrn_render::{Color, Painter, Rect};

pub const VIS_BARS: usize = 20;

/// Easing time constants, seconds. A bar jumps toward a louder value fast and
/// falls back slowly (matches the old per-frame 0.7 / 0.25 factors at 60 fps).
const ATTACK: f32 = 0.014;
const RELEASE: f32 = 0.058;
/// Closer than this to its target, a bar counts as arrived.
const SETTLED: f32 = 0.002;

pub struct Visualizer {
    bars: Vec<f32>,
    targets: Vec<f32>,
}

impl Visualizer {
    pub fn new() -> Self {
        Self {
            bars: vec![0.0; VIS_BARS],
            targets: vec![0.0; VIS_BARS],
        }
    }

    pub fn bars(&self) -> &[f32] {
        &self.bars
    }

    /// Aim the bars at a fresh spectrum (dB per band), or at rest with `None`.
    pub fn set_spectrum(&mut self, spectrum: Option<&[f32]>) {
        self.targets = match spectrum {
            Some(raw) => log_group_spectrum(raw, VIS_BARS),
            None => vec![0.0; VIS_BARS],
        };
    }

    /// Ease every bar toward its target by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        for (bar, &target) in self.bars.iter_mut().zip(&self.targets) {
            let tau = if target > *bar { ATTACK } else { RELEASE };
            *bar += (target - *bar) * (1.0 - (-dt / tau).exp());
            if (target - *bar).abs() < SETTLED {
                *bar = target;
            }
        }
    }

    /// Every bar has reached its target: nothing left to animate.
    pub fn is_settled(&self) -> bool {
        self.bars.iter().zip(&self.targets).all(|(b, t)| b == t)
    }

    /// Drop to rest at once (a new track starts from silence).
    pub fn reset(&mut self) {
        self.bars.fill(0.0);
        self.targets.fill(0.0);
    }
}

/// Group linear FFT bins into log-spaced visual bars.
fn log_group_spectrum(raw: &[f32], num_bars: usize) -> Vec<f32> {
    let n = raw.len();
    if n == 0 {
        return vec![0.0; num_bars];
    }
    // The top ~12% of bands sit above ~16kHz — effectively silent for music, so
    // the last bar never moved. Map the bars only across the usable/musical span
    // so every bar (including the far right) gets real energy.
    let usable = ((n as f32) * 0.86).round().max(1.0) as usize;
    let usable = usable.min(n);

    let mut bars = Vec::with_capacity(num_bars);
    for i in 0..num_bars {
        let lo = ((i as f64 / num_bars as f64).powf(2.0) * usable as f64) as usize;
        let hi = (((i + 1) as f64 / num_bars as f64).powf(2.0) * usable as f64) as usize;
        let lo = lo.min(usable.saturating_sub(1));
        let hi = hi.max(lo + 1).min(usable);

        let sum: f32 = raw[lo..hi].iter().sum();
        let avg = sum / (hi - lo) as f32;
        let freq_t = i as f64 / num_bars as f64;
        let treble_boost = (freq_t * 25.0) as f32;
        let normalized = ((avg + treble_boost + 40.0) / 40.0).clamp(0.0, 1.0);
        bars.push(normalized.powf(0.8));
    }
    bars
}

// ── Drawing: Classic Bars (flush to bottom of canvas) ────────────────────

/// Color a bar by its frequency position (`t`: 0.0 = bass .. 1.0 = highs) using
/// the active theme's 5-stop gradient, brightening slightly with loudness so
/// peaks pop.
fn bar_color(stops: &[(u8, u8, u8); 5], t: f32, magnitude: f32) -> Color {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let idx = (t as usize).min(stops.len() - 2);
    let next = idx + 1;
    let frac = t - t.floor();
    let (r0, g0, b0) = stops[idx];
    let (r1, g1, b1) = stops[next];
    let r = r0 as f32 + (r1 as f32 - r0 as f32) * frac;
    let g = g0 as f32 + (g1 as f32 - g0 as f32) * frac;
    let b = b0 as f32 + (b1 as f32 - b0 as f32) * frac;
    let bright = 1.0 + magnitude.clamp(0.0, 1.0) * 0.25;
    Color::from_rgba8(
        (r * bright).min(255.0) as u8,
        (g * bright).min(255.0) as u8,
        (b * bright).min(255.0) as u8,
        255,
    )
}

pub fn draw_bars(painter: &mut Painter, bars: &[f32], canvas: Rect, s: f32, stops: &[(u8, u8, u8); 5]) {
    let num_bars = bars.len();
    if num_bars == 0 {
        return;
    }
    let gap = 3.0 * s;
    let total_gap = gap * (num_bars - 1) as f32;
    let side_margin = 24.0 * s;
    let available_w = canvas.w - side_margin * 2.0;
    let bar_w = ((available_w - total_gap) / num_bars as f32).max(4.0 * s);
    // Bars sit flush against the bottom of the canvas (= bottom of window geometry).
    let base_y = canvas.y + canvas.h;
    let max_h = canvas.h * 0.95;
    let border = 3.0 * s;

    for (i, &magnitude) in bars.iter().enumerate() {
        let t = i as f32 / (num_bars.saturating_sub(1).max(1)) as f32;

        let bar_h = (magnitude * max_h).max(3.0 * s);
        let x = canvas.x + side_margin + i as f32 * (bar_w + gap);
        let y = base_y - bar_h;

        let color = bar_color(stops, t, magnitude);

        painter.rect_filled(
            Rect::new(
                x - border,
                y - border,
                bar_w + border * 2.0,
                bar_h + border * 2.0,
            ),
            0.0,
            Color::BLACK,
        );
        painter.rect_filled(Rect::new(x, y, bar_w, bar_h), 0.0, color);

        if magnitude > 0.5 {
            let glow_a = (magnitude - 0.5) * 0.3;
            painter.rect_filled(
                Rect::new(x - 2.0 * s, y - 2.0 * s, bar_w + 4.0 * s, bar_h + 4.0 * s),
                0.0,
                color.with_alpha(glow_a),
            );
        }

        let cap_h = 3.0 * s;
        painter.rect_filled(
            Rect::new(x, y, bar_w, cap_h),
            0.0,
            color.lighten(0.3).with_alpha(0.9),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_by_time_not_by_frame_count() {
        // One 16 ms step and four 4 ms steps must land in the same place.
        let loud = vec![0.0f32; 64];
        let mut a = Visualizer::new();
        let mut b = Visualizer::new();
        a.set_spectrum(Some(&loud));
        b.set_spectrum(Some(&loud));
        a.step(0.016);
        for _ in 0..4 {
            b.step(0.004);
        }
        for (x, y) in a.bars().iter().zip(b.bars()) {
            assert!((x - y).abs() < 1e-4, "{x} vs {y}");
        }
        assert!(!a.is_settled());
    }

    #[test]
    fn settles_to_rest_when_the_music_stops() {
        let mut v = Visualizer::new();
        v.set_spectrum(Some(&vec![0.0f32; 64]));
        v.step(1.0);
        assert!(v.is_settled() && v.bars().iter().all(|&b| b > 0.9));
        v.set_spectrum(None);
        assert!(!v.is_settled());
        v.step(2.0);
        assert!(v.is_settled() && v.bars().iter().all(|&b| b == 0.0));
    }

    #[test]
    fn silence_is_flat() {
        let mut v = Visualizer::new();
        v.set_spectrum(Some(&vec![-80.0f32; 64]));
        v.step(1.0);
        assert!(v.bars().iter().all(|&b| b == 0.0));
    }
}
