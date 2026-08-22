//! Audio visualizer — 32 bars across the bottom of the screen, driven
//! by a magnitude FFT of the most recent audio samples.
//!
//! Uses the real-input trick: pack N real samples as N/2 complex points,
//! run an N/2-point Cooley-Tukey FFT with a precomputed twiddle table,
//! then unpack to get the spectrum of the original N-point real input.
//! Roughly halves the FFT work vs a naive N-point complex FFT.

use lntrn_render::{Color, Painter, Rect};

use crate::audio::{AudioCapture, FFT_SIZE};

pub const BAR_COUNT: usize = 32;
const HALF: usize = FFT_SIZE / 2;

const GAP_FRAC: f32 = 0.22;
const BOTTOM_MARGIN: f32 = 8.0;
const MAX_HEIGHT_FRAC: f32 = 0.72;
const DECAY: f32 = 0.18;
const ATTACK: f32 = 0.65;

/// RMS² threshold below which we treat the input as silent and skip the
/// FFT entirely. ~ -60 dBFS.
const SILENCE_RMS2: f32 = 1.0e-6;

const BAR_TOP: Color = Color {
    r: 232.0 / 255.0,
    g: 220.0 / 255.0,
    b: 200.0 / 255.0,
    a: 0.85,
};

pub struct Visualizer {
    bars: [f32; BAR_COUNT],
    samples: [f32; FFT_SIZE],
    window: [f32; FFT_SIZE],
    /// Working buffers — used as N/2 complex points for the packed FFT.
    re: [f32; HALF],
    im: [f32; HALF],
    /// Magnitude-squared output after real-input unpack. mag2[k] = |X[k]|²
    /// where X is the DFT of the windowed real signal.
    mag2: [f32; HALF],
    /// W_FFT_SIZE^k = exp(-j·2π·k/FFT_SIZE) for k in 0..FFT_SIZE/2.
    /// Same table serves both the N/2-point FFT (indexed with stride 2)
    /// and the unpack step (indexed directly).
    twiddle_re: [f32; HALF],
    twiddle_im: [f32; HALF],
    /// (lo, hi) FFT bin range for each bar, rebuilt when sample_rate changes.
    bin_ranges: [(u16, u16); BAR_COUNT],
    cached_sample_rate: u32,
}

impl Default for Visualizer {
    fn default() -> Self {
        let mut window = [0.0f32; FFT_SIZE];
        for (i, w) in window.iter_mut().enumerate() {
            let t = (i as f32 * std::f32::consts::TAU) / (FFT_SIZE - 1) as f32;
            *w = 0.5 * (1.0 - t.cos());
        }
        let mut twiddle_re = [0.0f32; HALF];
        let mut twiddle_im = [0.0f32; HALF];
        for k in 0..HALF {
            let theta = -std::f32::consts::TAU * k as f32 / FFT_SIZE as f32;
            let (s, c) = theta.sin_cos();
            twiddle_re[k] = c;
            twiddle_im[k] = s;
        }
        Self {
            bars: [0.0; BAR_COUNT],
            samples: [0.0; FFT_SIZE],
            window,
            re: [0.0; HALF],
            im: [0.0; HALF],
            mag2: [0.0; HALF],
            twiddle_re,
            twiddle_im,
            bin_ranges: [(1, 2); BAR_COUNT],
            cached_sample_rate: 0,
        }
    }
}

impl Visualizer {
    pub fn tick(&mut self, audio: &AudioCapture) {
        audio.snapshot(&mut self.samples);

        let sample_rate = audio.sample_rate().max(1);
        if sample_rate != self.cached_sample_rate {
            self.rebuild_bin_ranges(sample_rate);
            self.cached_sample_rate = sample_rate;
        }

        // Cheap silence gate: skip the FFT and just decay bars.
        let mut sum_sq = 0.0f32;
        for &s in self.samples.iter() {
            sum_sq += s * s;
        }
        let mean_sq = sum_sq / FFT_SIZE as f32;
        if mean_sq < SILENCE_RMS2 {
            for b in self.bars.iter_mut() {
                *b *= 1.0 - DECAY;
                if *b < 1.0e-4 {
                    *b = 0.0;
                }
            }
            return;
        }

        // Pack windowed real input as N/2 complex points:
        //   z[k] = (r[2k] · w[2k]) + j · (r[2k+1] · w[2k+1])
        for k in 0..HALF {
            self.re[k] = self.samples[2 * k] * self.window[2 * k];
            self.im[k] = self.samples[2 * k + 1] * self.window[2 * k + 1];
        }

        // FFT of size N/2 on packed data.
        fft_radix2(
            &mut self.re,
            &mut self.im,
            &self.twiddle_re,
            &self.twiddle_im,
        );

        // Unpack: derive |X[k]|² for k in 1..HALF from the N/2-point FFT
        // of the packed sequence. See real-input FFT identities.
        //
        // Z[k] = E[k] + j·O[k]  (E, O = N/2-point DFTs of even/odd samples)
        // E[k] = ½(Z[k] + conj(Z[HALF-k]))
        // O[k] = -j·½(Z[k] - conj(Z[HALF-k]))
        // X[k] = E[k] + W_N^k · O[k]
        // X[HALF-k] = conj(E[k]) - conj(W_N^k · O[k])
        for k in 1..=HALF / 2 {
            let km = HALF - k;
            let y_re = self.re[k];
            let y_im = self.im[k];
            let ym_re = self.re[km];
            let ym_im = self.im[km];

            let e_re = 0.5 * (y_re + ym_re);
            let e_im = 0.5 * (y_im - ym_im);
            let o_re = 0.5 * (y_im + ym_im);
            let o_im = 0.5 * (ym_re - y_re);

            let cos_k = self.twiddle_re[k];
            let sin_k = self.twiddle_im[k];
            let wo_re = cos_k * o_re - sin_k * o_im;
            let wo_im = cos_k * o_im + sin_k * o_re;

            let x_re = e_re + wo_re;
            let x_im = e_im + wo_im;
            self.mag2[k] = x_re * x_re + x_im * x_im;

            if km != k {
                let xm_re = e_re - wo_re;
                let xm_im = wo_im - e_im;
                self.mag2[km] = xm_re * xm_re + xm_im * xm_im;
            }
        }

        // Bar binning. bin_ranges max is HALF, mag2 valid up to HALF-1.
        for b in 0..BAR_COUNT {
            let (i0, i1) = self.bin_ranges[b];
            let i0 = i0 as usize;
            let i1 = (i1 as usize).min(HALF);
            let mut peak_sq = 0.0f32;
            for i in i0..i1 {
                let m = self.mag2[i];
                if m > peak_sq {
                    peak_sq = m;
                }
            }
            let n = (peak_sq.sqrt() / FFT_SIZE as f32) * 2.0;
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

    fn rebuild_bin_ranges(&mut self, sample_rate: u32) {
        let bin_hz = sample_rate as f32 / FFT_SIZE as f32;
        let nyquist = sample_rate as f32 * 0.5;
        let f_lo = 30.0f32;
        let f_hi = nyquist.min(16_000.0);
        let log_lo = f_lo.ln();
        let log_hi = f_hi.ln();
        let max_bin = HALF as u16;
        for b in 0..BAR_COUNT {
            let t0 = b as f32 / BAR_COUNT as f32;
            let t1 = (b + 1) as f32 / BAR_COUNT as f32;
            let f0 = (log_lo + (log_hi - log_lo) * t0).exp();
            let f1 = (log_lo + (log_hi - log_lo) * t1).exp();
            let i0 = ((f0 / bin_hz) as u16).max(1);
            let i1 = ((f1 / bin_hz) as u16).max(i0 + 1).min(max_bin);
            self.bin_ranges[b] = (i0, i1);
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

    pub fn has_motion(&self) -> bool {
        self.bars.iter().any(|&v| v > 0.005)
    }
}

// ── FFT ────────────────────────────────────────────────────────────────────

/// In-place radix-2 Cooley-Tukey on N/2 complex points. Twiddle table is
/// the full FFT_SIZE table; this routine indexes it with stride 2 so the
/// effective per-stage twiddles are W_{HALF}^k.
fn fft_radix2(
    re: &mut [f32; HALF],
    im: &mut [f32; HALF],
    tw_re: &[f32; HALF],
    tw_im: &[f32; HALF],
) {
    let n = HALF;
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = ((i as u32).reverse_bits() >> (32 - bits)) as usize;
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut size = 2;
    while size <= n {
        let half = size / 2;
        // Twiddles for an N/2-point FFT at stage `size` are W_size^j =
        // W_{HALF}^{j · (HALF/size)} = W_{FFT_SIZE}^{j · (FFT_SIZE/size)}.
        let stride = FFT_SIZE / size;
        let mut i = 0;
        while i < n {
            for j in 0..half {
                let tw_idx = j * stride;
                let wr = tw_re[tw_idx];
                let wi = tw_im[tw_idx];
                let k = i + j;
                let l = k + half;
                let tr = wr * re[l] - wi * im[l];
                let ti = wr * im[l] + wi * re[l];
                re[l] = re[k] - tr;
                im[l] = im[k] - ti;
                re[k] += tr;
                im[k] += ti;
            }
            i += size;
        }
        size <<= 1;
    }
}
