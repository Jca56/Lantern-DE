//! Audio visualizer — 64 bars across the bottom of the screen, driven
//! by a magnitude FFT of the most recent audio samples.
//!
//! FFT is a hand-rolled radix-2 Cooley-Tukey; no external crate.

use lntrn_render::{Color, Painter, Rect};

use crate::audio::{AudioCapture, FFT_SIZE};

/// Number of bars drawn.
pub const BAR_COUNT: usize = 32;

/// Bar gap as a fraction of bar slot width.
const GAP_FRAC: f32 = 0.22;

/// Bottom margin (logical px) between the bars and the screen edge.
const BOTTOM_MARGIN: f32 = 8.0;

/// Max bar height as a fraction of screen height.
const MAX_HEIGHT_FRAC: f32 = 0.72;

/// Per-frame decay applied to the smoothed bar values when the new
/// reading is lower. Smaller = floppier, larger = snappier.
const DECAY: f32 = 0.18;
/// Attack rate when the new reading is higher.
const ATTACK: f32 = 0.65;

/// Tan that matches the clock text (#e8dcc8).
const BAR_TOP: Color = Color {
    r: 232.0 / 255.0,
    g: 220.0 / 255.0,
    b: 200.0 / 255.0,
    a: 0.85,
};

pub struct Visualizer {
    /// Smoothed normalized bar magnitudes (0..1).
    bars: [f32; BAR_COUNT],
    /// Sample buffer reused each frame.
    samples: [f32; FFT_SIZE],
    /// Pre-computed Hann window.
    window: [f32; FFT_SIZE],
}

impl Default for Visualizer {
    fn default() -> Self {
        let mut window = [0.0f32; FFT_SIZE];
        for (i, w) in window.iter_mut().enumerate() {
            // Hann: 0.5 * (1 - cos(2π i / (N-1)))
            let t = (i as f32 * std::f32::consts::TAU) / (FFT_SIZE - 1) as f32;
            *w = 0.5 * (1.0 - t.cos());
        }
        Self {
            bars: [0.0; BAR_COUNT],
            samples: [0.0; FFT_SIZE],
            window,
        }
    }
}

impl Visualizer {
    pub fn tick(&mut self, audio: &AudioCapture) {
        audio.snapshot(&mut self.samples);
        let sample_rate = audio.sample_rate().max(1) as f32;

        // Apply Hann window.
        let mut re = [0.0f32; FFT_SIZE];
        let mut im = [0.0f32; FFT_SIZE];
        for i in 0..FFT_SIZE {
            re[i] = self.samples[i] * self.window[i];
        }

        fft_radix2(&mut re, &mut im);

        // Bin → bar mapping: log-spaced from 30 Hz to ~16 kHz.
        let nyquist = sample_rate * 0.5;
        let bin_hz = sample_rate / FFT_SIZE as f32;
        let f_lo = 30.0f32;
        let f_hi = nyquist.min(16_000.0);
        let log_lo = f_lo.ln();
        let log_hi = f_hi.ln();

        for b in 0..BAR_COUNT {
            let t0 = b as f32 / BAR_COUNT as f32;
            let t1 = (b + 1) as f32 / BAR_COUNT as f32;
            let f0 = (log_lo + (log_hi - log_lo) * t0).exp();
            let f1 = (log_lo + (log_hi - log_lo) * t1).exp();
            let i0 = ((f0 / bin_hz) as usize).max(1);
            let i1 = ((f1 / bin_hz) as usize).max(i0 + 1).min(FFT_SIZE / 2);
            let mut peak = 0.0f32;
            for i in i0..i1 {
                let mag = (re[i] * re[i] + im[i] * im[i]).sqrt();
                if mag > peak {
                    peak = mag;
                }
            }
            // Normalize: divide by N, take dB-ish curve so quiet
            // detail shows. log10(1 + 10x) is a cheap soft compander.
            let n = (peak / FFT_SIZE as f32) * 2.0;
            let shaped = (1.0 + 80.0 * n).log10() / 1.8;
            let target = shaped.clamp(0.0, 1.0);

            let cur = self.bars[b];
            let next = if target > cur {
                cur + (target - cur) * ATTACK
            } else {
                cur + (target - cur) * DECAY
            };
            self.bars[b] = next.clamp(0.0, 1.0);
        }
    }

    pub fn draw(&self, painter: &mut Painter, surface_w: u32, surface_h: u32, scale: f32) {
        let sw = surface_w as f32;
        let sh = surface_h as f32;
        let bottom = sh - BOTTOM_MARGIN * scale;
        let max_h = sh * MAX_HEIGHT_FRAC;

        let slot_w = sw / BAR_COUNT as f32;
        let gap = slot_w * GAP_FRAC;
        let bar_w = slot_w - gap;
        let corner = (bar_w * 0.20).min(6.0 * scale);

        for (i, &v) in self.bars.iter().enumerate() {
            if v <= 0.002 {
                continue;
            }
            let h = (v * max_h).max(2.0 * scale);
            let x = i as f32 * slot_w + gap * 0.5;
            let y = bottom - h;
            painter.rect_filled(Rect::new(x, y, bar_w, h), corner, BAR_TOP);
        }
    }

    /// Are any bars still glowing? Used to decide if we keep repainting
    /// once audio goes silent.
    pub fn has_motion(&self) -> bool {
        self.bars.iter().any(|&v| v > 0.005)
    }
}

// ── FFT ────────────────────────────────────────────────────────────────────

/// In-place radix-2 Cooley-Tukey. `re.len()` and `im.len()` must equal
/// `FFT_SIZE`, which must be a power of two.
fn fft_radix2(re: &mut [f32; FFT_SIZE], im: &mut [f32; FFT_SIZE]) {
    let n = FFT_SIZE;
    // Bit reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = (i as u32).reverse_bits() >> (32 - bits);
        let j = j as usize;
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterflies.
    let mut size = 2;
    while size <= n {
        let half = size / 2;
        let theta = -std::f32::consts::TAU / size as f32;
        let (sin, cos) = theta.sin_cos();
        let mut i = 0;
        while i < n {
            let mut wr = 1.0f32;
            let mut wi = 0.0f32;
            for j in 0..half {
                let k = i + j;
                let l = k + half;
                let tr = wr * re[l] - wi * im[l];
                let ti = wr * im[l] + wi * re[l];
                re[l] = re[k] - tr;
                im[l] = im[k] - ti;
                re[k] += tr;
                im[k] += ti;
                let new_wr = wr * cos - wi * sin;
                wi = wr * sin + wi * cos;
                wr = new_wr;
            }
            i += size;
        }
        size <<= 1;
    }
}
