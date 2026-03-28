use lntrn_render::{Color, Painter, Rect};

const BLOB_COUNT: usize = 14;

struct Blob {
    /// Normalized X position (0.0–1.0 across bar width).
    x: f32,
    /// Normalized Y position (0.0–1.0 across bar height).
    y: f32,
    /// Base radius as fraction of bar height.
    base_r: f32,
    /// Speed multipliers for the sine oscillations.
    freq_x: f32,
    freq_y: f32,
    freq_r: f32,
    /// Phase offsets so blobs don't move in sync.
    phase_x: f32,
    phase_y: f32,
    phase_r: f32,
    /// Which color slot this blob uses (index into VAPORWAVE palette).
    color_idx: usize,
}

/// Vaporwave palette — purples, pinks, cyans.
const VAPORWAVE: [[u8; 3]; 6] = [
    [180, 60, 220],  // purple
    [220, 50, 160],  // hot pink
    [100, 200, 240], // cyan
    [140, 80, 250],  // violet
    [240, 100, 200], // magenta-pink
    [60, 180, 220],  // teal-cyan
];

pub struct LavaLamp {
    blobs: Vec<Blob>,
    time: f32,
    pub enabled: bool,
}

impl LavaLamp {
    pub fn new() -> Self {
        let blobs: Vec<Blob> = (0..BLOB_COUNT)
            .map(|i| {
                let fi = i as f32;
                let seed = fi * 7.31;
                Blob {
                    x: frac(seed * 0.37 + 0.1),
                    y: frac(seed * 0.53 + 0.2),
                    base_r: 0.35 + frac(seed * 0.71) * 0.45,
                    freq_x: 0.15 + frac(seed * 0.43) * 0.25,
                    freq_y: 0.10 + frac(seed * 0.61) * 0.20,
                    freq_r: 0.20 + frac(seed * 0.29) * 0.15,
                    phase_x: fi * 1.7,
                    phase_y: fi * 2.3,
                    phase_r: fi * 0.9,
                    color_idx: i % VAPORWAVE.len(),
                }
            })
            .collect();

        Self {
            blobs,
            time: 0.0,
            enabled: false,
        }
    }

    pub fn update(&mut self, dt: f32) {
        if self.enabled {
            self.time += dt;
        }
    }

    /// Draw the pulsing gradient background that replaces the normal bar bg.
    pub fn draw_background(
        &self,
        painter: &mut Painter,
        bar_x: f32,
        bar_y: f32,
        bar_w: f32,
        bar_h: f32,
        corner_radius: f32,
        opacity: f32,
    ) {
        if !self.enabled {
            return;
        }

        let t = self.time;

        // Slowly cycling gradient angle
        let angle = t * 0.15;
        // Pulse between deep purple-black and dark teal-black
        let pulse = ((t * 0.3).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        let c1 = Color::from_rgb8(30, 10, 50).lerp(Color::from_rgb8(10, 30, 50), pulse);
        let c2 = Color::from_rgb8(50, 10, 40).lerp(Color::from_rgb8(15, 40, 60), pulse);

        let bar_rect = Rect::new(bar_x, bar_y, bar_w, bar_h);
        painter.rect_gradient_linear(
            bar_rect,
            corner_radius,
            angle,
            c1.with_alpha(opacity),
            c2.with_alpha(opacity),
        );
    }

    /// Draw the lava blobs on top of the background.
    pub fn draw_blobs(
        &self,
        painter: &mut Painter,
        bar_x: f32,
        bar_y: f32,
        bar_w: f32,
        bar_h: f32,
        opacity: f32,
    ) {
        if !self.enabled {
            return;
        }

        let t = self.time;

        for blob in &self.blobs {
            // Animate position with slow sine waves
            let ax = blob.x + 0.12 * (t * blob.freq_x + blob.phase_x).sin()
                + 0.06 * (t * blob.freq_x * 1.7 + blob.phase_y).cos();
            let ay = blob.y + 0.15 * (t * blob.freq_y + blob.phase_y).sin()
                + 0.08 * (t * blob.freq_y * 1.3 + blob.phase_x).cos();

            // Pulsing radius
            let r_scale = 1.0 + 0.3 * (t * blob.freq_r + blob.phase_r).sin();
            let r = blob.base_r * r_scale * bar_h;

            let cx = bar_x + ax * bar_w;
            let cy = bar_y + ay * bar_h;

            let [cr, cg, cb] = VAPORWAVE[blob.color_idx];
            let alpha = (opacity * 100.0) as u8;
            let center = Color::from_rgba8(cr, cg, cb, alpha);
            let edge = Color::from_rgba8(cr, cg, cb, 0);

            // Square rect with corner_radius = r makes the radial gradient circular
            let size = r * 2.0;
            let blob_rect = Rect::new(cx - r, cy - r, size, size);
            painter.rect_gradient_radial(blob_rect, r, center, edge);
        }
    }
}

/// Fractional part of a float, always positive.
fn frac(x: f32) -> f32 {
    x - x.floor()
}
