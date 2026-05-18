// Radix-2 Cooley-Tukey FFT + windowed spectrum analyzer.
// Replaces gst-plugins-good's `spectrum` element so we have zero plugin deps
// beyond playbin/audioconvert/autoaudiosink.

use std::f32::consts::PI;

const DB_FLOOR: f32 = -80.0;

pub struct SpectrumAnalyzer {
    fft_size: usize,
    bands: usize,
    window: Vec<f32>,
    ring: Vec<f32>,
    write_pos: usize,
    samples_since_update: usize,
    update_interval: usize,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(fft_size: usize, bands: usize, sample_rate: u32, fps: u32) -> Self {
        assert!(fft_size.is_power_of_two(), "fft_size must be power of two");
        let mut window = vec![0.0; fft_size];
        let denom = (fft_size - 1) as f32;
        for (i, w) in window.iter_mut().enumerate() {
            // Hann window
            *w = 0.5 * (1.0 - (2.0 * PI * i as f32 / denom).cos());
        }
        let update_interval = (sample_rate / fps.max(1)).max(1) as usize;
        Self {
            fft_size,
            bands,
            window,
            ring: vec![0.0; fft_size],
            write_pos: 0,
            samples_since_update: 0,
            update_interval,
            re: vec![0.0; fft_size],
            im: vec![0.0; fft_size],
        }
    }

    /// Push mono f32 samples. Returns Some(bands) when a fresh spectrum
    /// is ready (roughly every `update_interval` samples).
    pub fn push_samples(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        for &s in samples {
            self.ring[self.write_pos] = s;
            self.write_pos = (self.write_pos + 1) % self.fft_size;
            self.samples_since_update += 1;
        }
        if self.samples_since_update >= self.update_interval {
            self.samples_since_update = 0;
            Some(self.compute())
        } else {
            None
        }
    }

    fn compute(&mut self) -> Vec<f32> {
        let n = self.fft_size;
        // Copy ring (oldest sample first) and apply Hann window.
        for i in 0..n {
            let idx = (self.write_pos + i) % n;
            self.re[i] = self.ring[idx] * self.window[i];
            self.im[i] = 0.0;
        }
        fft_inplace(&mut self.re, &mut self.im);

        let half = n / 2;
        let norm = 2.0 / n as f32;
        let mut mags = vec![DB_FLOOR; half];
        for i in 0..half {
            let m = (self.re[i] * self.re[i] + self.im[i] * self.im[i]).sqrt() * norm;
            mags[i] = if m > 1e-10 {
                (20.0 * m.log10()).max(DB_FLOOR)
            } else {
                DB_FLOOR
            };
        }

        // Linear bin into `bands` to mimic the spectrum element's layout
        // (downstream `log_group_spectrum` will log-group from there).
        let mut out = vec![DB_FLOOR; self.bands];
        let bins_per_band = half as f32 / self.bands as f32;
        for b in 0..self.bands {
            let start = (b as f32 * bins_per_band).floor() as usize;
            let end = (((b + 1) as f32 * bins_per_band).ceil() as usize).min(half);
            let end = end.max(start + 1);
            let mut max = DB_FLOOR;
            for &m in &mags[start..end.min(half)] {
                if m > max {
                    max = m;
                }
            }
            out[b] = max;
        }
        out
    }
}

// Iterative in-place radix-2 Cooley-Tukey FFT.
fn fft_inplace(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert_eq!(im.len(), n);
    debug_assert!(n.is_power_of_two());

    // Bit-reversal permutation
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies
    let mut len = 2usize;
    while len <= n {
        let half = len / 2;
        let theta = -2.0 * PI / len as f32;
        let wlen_re = theta.cos();
        let wlen_im = theta.sin();
        let mut i = 0;
        while i < n {
            let mut wr = 1.0f32;
            let mut wi = 0.0f32;
            for k in 0..half {
                let a = i + k;
                let b = a + half;
                let tr = wr * re[b] - wi * im[b];
                let ti = wr * im[b] + wi * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nwr = wr * wlen_re - wi * wlen_im;
                wi = wr * wlen_im + wi * wlen_re;
                wr = nwr;
            }
            i += len;
        }
        len <<= 1;
    }
}
