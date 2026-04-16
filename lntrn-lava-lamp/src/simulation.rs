use lntrn_render::Rect;

// ── Simple xorshift64 RNG ──────────────────────────────────────────────

pub(crate) struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42);
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / ((1u64 << 24) as f32)
    }

    pub fn range(&mut self, min: f32, max: f32) -> f32 {
        min + self.f32() * (max - min)
    }
}

// ── Blob ───────────────────────────────────────────────────────────────

pub struct Blob {
    pub x: f32,
    pub y: f32,
    /// Radius as a fraction of body width (0.10 = 10% of body width)
    pub base_radius_frac: f32,
    pub radius: f32,
    pub heat: f32,
    pub color_index: usize,
    pub vx: f32,
    pub vy: f32,
    pub wobble_phase: f32,
    wobble_speed: f32,
}

// ── Physics tuning ─────────────────────────────────────────────────────
// High forces + heavy drag → viscous but clearly visible rise/fall.
// Terminal velocity: cold blobs sink ~95 px/s, hot blobs rise ~160 px/s.

const GRAVITY: f32 = 300.0;
const BUOYANCY_MAX: f32 = 800.0;
const HEAT_RATE: f32 = 0.6;
const COOL_RATE: f32 = 0.15;
const WOBBLE_STR: f32 = 60.0;
const DRAG: f32 = 0.95;

// ── Simulation ─────────────────────────────────────────────────────────

pub struct LavaSimulation {
    pub blobs: Vec<Blob>,
    pub time: f32,
    rng: Rng,
    initialized: bool,
}

impl LavaSimulation {
    pub fn new(count: usize) -> Self {
        let mut rng = Rng::new();
        let blobs = (0..count)
            .map(|i| {
                let frac = rng.range(0.15, 0.28);
                Blob {
                    x: 0.0,
                    y: 0.0,
                    base_radius_frac: frac,
                    radius: 0.0,
                    // Stagger heat so blobs start at different cycle phases
                    heat: rng.range(0.0, 1.0),
                    color_index: i,
                    vx: 0.0,
                    vy: 0.0,
                    wobble_phase: rng.range(0.0, std::f32::consts::TAU),
                    wobble_speed: rng.range(0.8, 2.0),
                }
            })
            .collect();

        Self { blobs, time: 0.0, rng, initialized: false }
    }

    pub fn update(&mut self, dt: f32, bounds: Rect) {
        // Place blobs scattered through the body on first frame
        if !self.initialized {
            self.initialized = true;
            let count = self.blobs.len();
            for i in 0..count {
                let r = self.blobs[i].base_radius_frac * bounds.w;
                self.blobs[i].radius = r;
                self.blobs[i].x =
                    self.rng.range(bounds.x + r + 5.0, bounds.x + bounds.w - r - 5.0);
                // Spread blobs across the full height
                self.blobs[i].y =
                    self.rng.range(bounds.y + r + 5.0, bounds.y + bounds.h - r - 5.0);
                // Hot blobs start rising, cool ones start sinking
                if self.blobs[i].heat > 0.5 {
                    self.blobs[i].vy = self.rng.range(-120.0, -40.0);
                } else {
                    self.blobs[i].vy = self.rng.range(20.0, 60.0);
                }
            }
        }

        self.time += dt;

        for blob in &mut self.blobs {
            // Resolve pixel radius from fraction of body width
            blob.radius = blob.base_radius_frac * bounds.w;

            // Heat: warm up near the bottom "bulb", cool as they rise
            let y_frac = ((blob.y - bounds.y) / bounds.h).clamp(0.0, 1.0);
            if y_frac > 0.7 {
                let intensity = (y_frac - 0.7) / 0.3;
                blob.heat = (blob.heat + HEAT_RATE * intensity * dt).min(1.0);
            } else {
                let cool = COOL_RATE * (1.0 - y_frac * 0.5) * dt;
                blob.heat = (blob.heat - cool).max(0.0);
            }

            // Buoyancy vs gravity — slow, viscous motion
            let buoyancy = blob.heat * BUOYANCY_MAX;
            blob.vy += (GRAVITY - buoyancy) * dt;

            // Horizontal wobble
            blob.wobble_phase += blob.wobble_speed * dt;
            blob.vx += blob.wobble_phase.sin() * WOBBLE_STR * dt;

            // Drag (frame-rate independent)
            let d = DRAG.powf(dt * 60.0);
            blob.vx *= d;
            blob.vy *= d;

            // Move
            blob.x += blob.vx * dt;
            blob.y += blob.vy * dt;

            // Bounce off walls
            let left = bounds.x + blob.radius;
            let right = bounds.x + bounds.w - blob.radius;
            let top = bounds.y + blob.radius;
            let bottom = bounds.y + bounds.h - blob.radius;

            if blob.x < left { blob.x = left; blob.vx = blob.vx.abs() * 0.3; }
            if blob.x > right { blob.x = right; blob.vx = -blob.vx.abs() * 0.3; }
            if blob.y < top { blob.y = top; blob.vy = blob.vy.abs() * 0.5; }
            if blob.y > bottom { blob.y = bottom; blob.vy = -blob.vy.abs() * 0.5; }
        }

        // Soft repulsion so blobs don't stack
        self.repel_blobs();
    }

    fn repel_blobs(&mut self) {
        let count = self.blobs.len();
        let mut forces = vec![(0.0f32, 0.0f32); count];
        for i in 0..count {
            for j in (i + 1)..count {
                let dx = self.blobs[j].x - self.blobs[i].x;
                let dy = self.blobs[j].y - self.blobs[i].y;
                let dist = (dx * dx + dy * dy).sqrt().max(0.1);
                let min_d = (self.blobs[i].radius + self.blobs[j].radius) * 1.1;
                if dist < min_d {
                    let f = (min_d - dist) * 0.3;
                    let nx = dx / dist;
                    let ny = dy / dist;
                    forces[i].0 -= nx * f;
                    forces[i].1 -= ny * f;
                    forces[j].0 += nx * f;
                    forces[j].1 += ny * f;
                }
            }
        }
        for (blob, &(fx, fy)) in self.blobs.iter_mut().zip(forces.iter()) {
            blob.vx += fx;
            blob.vy += fy;
        }
    }
}
