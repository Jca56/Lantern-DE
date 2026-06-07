use std::f32::consts::FRAC_PI_2;
use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::app::FrameCtx;
use super::{draw_theme_background, Scene};

// ── Tunables ─────────────────────────────────────────────────────────────────
const CELL_PX: f32 = 36.0; // target grid cell size, logical px (scaled at runtime)
const STEP_SECS: f32 = 0.07; // how often a pipe advances one cell
const TURN_PCT: u64 = 28; // % chance a pipe turns at each step
const PIPE_FRAC: f32 = 0.42; // tube thickness as a fraction of a cell
const BALL_FRAC: f32 = 0.62; // ball-joint radius as a fraction of the tube width
const FILL_FRAC: f32 = 0.52; // wipe the screen once this fraction of cells is full
const CATCHUP_MAX: u32 = 8; // cap sim steps per frame (avoids huge catch-ups)

const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// One pipe: a path through the grid that grows a cell at a time and stops
/// (but stays drawn) when it walks off the edge.
struct Pipe {
    cells: Vec<(i32, i32)>,
    dir: (i32, i32),
    color: Color,
    alive: bool,
}

/// Classic "3D pipes" screensaver: themed tubes with cylindrical shading and
/// glossy chrome ball-joints meander across a grid, then the screen wipes and
/// they start over. Faked in pure 2D — no 3D renderer required.
pub struct Pipes {
    pipes: Vec<Pipe>,
    rng: u64,
    next_step: f32,
    last_step: f32,
    cols: i32,
    rows: i32,
    color_idx: usize,
    ready: bool,
}

impl Default for Pipes {
    fn default() -> Self { Self::new() }
}

impl Pipes {
    pub fn new() -> Self {
        Self {
            pipes: Vec::new(),
            rng: seed(),
            next_step: 0.0,
            last_step: 0.0,
            cols: 0,
            rows: 0,
            color_idx: 0,
            ready: false,
        }
    }

    /// How many pipes grow at once — scales with the grid so big screens feel
    /// busy and small ones don't get cluttered.
    fn target_pipes(&self) -> usize {
        let cells = (self.cols * self.rows).max(1) as usize;
        (cells / 220).clamp(3, 7)
    }

    /// Drop a fresh pipe at a random cell heading a random way.
    fn spawn(&mut self) {
        let cx = (rng_next(&mut self.rng) % self.cols.max(1) as u64) as i32;
        let cy = (rng_next(&mut self.rng) % self.rows.max(1) as u64) as i32;
        let dir = DIRS[(rng_next(&mut self.rng) % 4) as usize];
        let strip = lntrn_theme::GRADIENT_STRIP;
        let c = strip[self.color_idx % strip.len()];
        self.color_idx = self.color_idx.wrapping_add(1);
        self.pipes.push(Pipe {
            cells: vec![(cx, cy)],
            dir,
            // Punch the theme colors well past pastel into neon territory.
            color: Color::from_rgba8(c.r, c.g, c.b, 255).saturate(1.5),
            alive: true,
        });
    }

    /// Reset grid + pipes (first frame or after a resize changes the grid).
    fn reset(&mut self, elapsed: f32) {
        self.pipes.clear();
        self.color_idx = 0;
        self.next_step = elapsed;
        self.last_step = elapsed;
        for _ in 0..self.target_pipes() {
            self.spawn();
        }
    }

    /// Advance the simulation by one cell for every living pipe.
    fn step(&mut self) {
        // Wipe and restart once the grid gets too full — the classic look.
        let capacity = (self.cols.max(0) * self.rows.max(0)) as usize;
        let filled: usize = self.pipes.iter().map(|p| p.cells.len()).sum();
        if capacity > 0 && filled as f32 > capacity as f32 * FILL_FRAC {
            self.pipes.clear();
        }

        let mut rng = self.rng; // local copy so we don't double-borrow self
        let (cols, rows) = (self.cols, self.rows);
        for pipe in self.pipes.iter_mut().filter(|p| p.alive) {
            if rng_next(&mut rng) % 100 < TURN_PCT {
                let (dx, dy) = pipe.dir;
                // turn 90° left or right — never reverse
                pipe.dir = if rng_next(&mut rng) & 1 == 0 { (-dy, dx) } else { (dy, -dx) };
            }
            let (hx, hy) = *pipe.cells.last().unwrap();
            let (nx, ny) = (hx + pipe.dir.0, hy + pipe.dir.1);
            if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                pipe.alive = false; // hit a wall — stop growing, stay drawn
            } else {
                pipe.cells.push((nx, ny));
            }
        }
        self.rng = rng;

        // Top the population back up so something is always growing.
        let alive = self.pipes.iter().filter(|p| p.alive).count();
        for _ in alive..self.target_pipes() {
            self.spawn();
        }
    }
}

impl Scene for Pipes {
    fn draw(&mut self, painter: &mut Painter, _text: &mut TextRenderer, ctx: &FrameCtx) {
        draw_theme_background(painter, ctx);

        // Grid derived from the live window size (re-derived every frame).
        let cell = (CELL_PX * ctx.scale).max(10.0);
        let cols = ((ctx.wf / cell).floor() as i32).max(1);
        let rows = ((ctx.hf / cell).floor() as i32).max(1);
        if !self.ready || cols != self.cols || rows != self.rows {
            self.cols = cols;
            self.rows = rows;
            self.ready = true;
            self.reset(ctx.elapsed_secs);
        }

        // Step on a fixed cadence; catch up a little if we fell behind.
        let mut steps = 0;
        while ctx.elapsed_secs >= self.next_step && steps < CATCHUP_MAX {
            self.last_step = self.next_step;
            self.step();
            self.next_step += STEP_SECS;
            steps += 1;
        }
        // Way behind (window was hidden) — resync instead of catching up forever.
        if ctx.elapsed_secs > self.next_step {
            self.last_step = ctx.elapsed_secs;
            self.next_step = ctx.elapsed_secs + STEP_SECS;
        }

        // 0..1 progress toward the next step — smooths out each pipe's head.
        let frac = ((ctx.elapsed_secs - self.last_step) / STEP_SECS).clamp(0.0, 1.0);

        // Center the grid in the window.
        let gx = (ctx.wf - cols as f32 * cell) * 0.5;
        let gy = (ctx.hf - rows as f32 * cell) * 0.5;
        let center = |c: (i32, i32)| -> (f32, f32) {
            (gx + (c.0 as f32 + 0.5) * cell, gy + (c.1 as f32 + 0.5) * cell)
        };

        // Tube/ball/shadow geometry. Light comes from the upper-left.
        let w = cell * PIPE_FRAC;
        let r = w * 0.5;
        let ball_r = w * BALL_FRAC;
        let sigma = w * 0.30;
        let off = w * 0.42;
        let shadow = Color::BLACK.with_alpha(0.33);

        for pipe in &self.pipes {
            let n = pipe.cells.len();
            if n == 0 {
                continue;
            }

            // Pixel points; a living pipe's head grows toward its next cell.
            let mut pts: Vec<(f32, f32)> = pipe.cells.iter().map(|&c| center(c)).collect();
            if pipe.alive && n >= 2 && frac < 1.0 {
                pts[n - 1] = lerp2(pts[n - 2], pts[n - 1], frac);
            }

            // Single cell — just a ball.
            if n == 1 {
                ball_shadow(painter, pts[0], ball_r, sigma, off, shadow);
                ball(painter, pts[0], ball_r, pipe.color, pipe.alive);
                continue;
            }

            // Group collinear cells into straight runs (one cylinder each).
            let mut runs: Vec<(usize, usize)> = Vec::new();
            let mut s = 0usize;
            for i in 1..n {
                let din = step_dir(pipe.cells[i - 1], pipe.cells[i]);
                let turns = if i + 1 < n {
                    step_dir(pipe.cells[i], pipe.cells[i + 1]) != din
                } else {
                    true
                };
                if turns {
                    runs.push((s, i));
                    s = i;
                }
            }

            // Ball joints: the start, every corner, and the head.
            let mut joints: Vec<usize> = vec![0];
            for run in runs.iter().skip(1) {
                joints.push(run.0);
            }

            // 4-stop cylinder ramp: bright edge → base → dark edge.
            let ramp = [
                pipe.color.lighten(0.5),  // glossy highlight streak
                pipe.color.lighten(0.1),  // upper body — stays saturated
                pipe.color,               // vivid base
                pipe.color.darken(0.5),   // shaded rim
            ];

            // ── Shadow sub-pass (all behind the bodies of this pipe) ──
            for &(a, b) in &runs {
                let (rect, _) = tube_rect(pts[a], pts[b], w);
                painter.shadow(rect, r, sigma, shadow, off, off);
            }
            for &j in &joints {
                ball_shadow(painter, pts[j], ball_r, sigma, off, shadow);
            }
            ball_shadow(painter, pts[n - 1], ball_r, sigma, off, shadow);

            // ── Body sub-pass ──
            for &(a, b) in &runs {
                let (rect, angle) = tube_rect(pts[a], pts[b], w);
                painter.rect_gradient_multi_direct(rect, r, angle, &ramp);
            }
            for &j in &joints {
                ball(painter, pts[j], ball_r, pipe.color, false);
            }
            // Head ball — brighter on a living pipe so the leading tip glows.
            ball(painter, pts[n - 1], ball_r, pipe.color, pipe.alive);
        }
    }
}

// ── Drawing helpers ──────────────────────────────────────────────────────────

/// Axis-aligned capsule covering the run, plus the shading angle. The gradient
/// runs *across* the tube (top→bottom for horizontal, left→right for vertical)
/// so the lengthwise highlight reads as a rounded cylinder.
fn tube_rect(a: (f32, f32), b: (f32, f32), w: f32) -> (Rect, f32) {
    let r = w * 0.5;
    if (a.0 - b.0).abs() >= (a.1 - b.1).abs() {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let y = (a.1 + b.1) * 0.5;
        (Rect::new(x0 - r, y - r, (x1 - x0) + w, w), FRAC_PI_2)
    } else {
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        let x = (a.0 + b.0) * 0.5;
        (Rect::new(x - r, y0 - r, w, (y1 - y0) + w), 0.0)
    }
}

/// A glossy sphere: radial body (lit center → dark rim) + an offset upper-left
/// specular glint. `bright` lifts a living pipe's head.
fn ball(p: &mut Painter, c: (f32, f32), r: f32, base: Color, bright: bool) {
    let rect = Rect::new(c.0 - r, c.1 - r, r * 2.0, r * 2.0);
    let lift = if bright { 0.55 } else { 0.30 };
    p.rect_gradient_radial(rect, r, base.lighten(lift), base.darken(0.55));
    let glint = if bright { 0.9 } else { 0.6 };
    p.rect_radial_glow(rect, r, (0.32, 0.30), r * 0.95, Color::WHITE.with_alpha(glint));
}

fn ball_shadow(p: &mut Painter, c: (f32, f32), r: f32, sigma: f32, off: f32, color: Color) {
    let rect = Rect::new(c.0 - r, c.1 - r, r * 2.0, r * 2.0);
    p.shadow(rect, r, sigma, color, off, off);
}

fn lerp2(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn step_dir(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    ((b.0 - a.0).signum(), (b.1 - a.1).signum())
}

// ── Tiny self-contained PRNG (xorshift64) — no external crate needed ─────────
fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    nanos | 1 // xorshift needs a non-zero state
}

fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}
