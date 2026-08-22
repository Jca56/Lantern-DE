use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_render::{Color, Painter, TextRenderer};

use super::{draw_theme_background, Scene};
use crate::app::FrameCtx;

// ── Tunables ─────────────────────────────────────────────────────────────────
const FONT_PX: f32 = 26.0; // glyph size, logical px (scaled at runtime)
const COL_W_FRAC: f32 = 0.62; // column spacing as a fraction of font size (smaller = denser)
const ROW_H_FRAC: f32 = 1.12; // row spacing as a fraction of font size
const MIN_LEN: i32 = 12; // shortest trail (rows)
const MAX_LEN: i32 = 32; // longest trail (rows)
const SPEED_MIN: f32 = 6.0; // fall speed (rows / second)
const SPEED_MAX: f32 = 20.0;
const FLICKER_HZ: f32 = 7.0; // avg glyph mutations per column per second
const TRAIL_LEVELS: usize = 6; // fade steps; keep CHARSET chars × (this+1) < 512 cache cap
                               // Hybrid, most film-accurate: half-width katakana (U+FF66–FF9D) + digits + a few
                               // Latin. Katakana needs a kana font (UDEV Gothic); cosmic-text resolves per-glyph.
                               // 70 chars × 7 colors = 490 cached layouts — under the 512 cap.
const CHARSET: &str = "ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789ZKXA";

/// One vertical stream of glyphs falling down a single column.
struct Column {
    head: f32, // head position in rows (fractional); starts above the top
    speed: f32,
    len: i32,
    last_row: i32,   // last integer row the head occupied
    glyphs: Vec<u8>, // glyph index (into CHARSET) per grid row
}

/// "Matrix" digital rain — columns of glyphs cascade down with a bright leading
/// character and a fading themed trail. Theme green, ASCII glyphs, GPU text.
pub struct Matrix {
    cols: Vec<Column>,
    glyph_strs: Vec<String>, // single-char strings, indexed by glyph id (no per-frame alloc)
    head_color: Color,
    trail: Vec<Color>, // bright → dark, indexed by fade level
    rng: u64,
    n_cols: usize,
    rows: i32,
    last: f32,
    ready: bool,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::new()
    }
}

impl Matrix {
    pub fn new() -> Self {
        let glyph_strs = CHARSET.chars().map(|c| c.to_string()).collect();

        // Matrix green, pulled from the theme and pushed vivid. The head is a
        // near-white tip; the trail fades toward near-black.
        let g = lntrn_theme::colors::GRADIENT_GREEN;
        let base = Color::from_rgba8(g.r, g.g, g.b, 255).saturate(1.3);
        let dark = base.darken(0.9);
        let head_color = base.lighten(0.7);
        let trail = (0..TRAIL_LEVELS)
            .map(|k| base.lerp(dark, k as f32 / (TRAIL_LEVELS - 1) as f32))
            .collect();

        Self {
            cols: Vec::new(),
            glyph_strs,
            head_color,
            trail,
            rng: seed(),
            n_cols: 0,
            rows: 0,
            last: 0.0,
            ready: false,
        }
    }

    /// Park a column above the top with fresh speed/length so streams stagger
    /// instead of marching in lock-step.
    fn respawn(col: &mut Column, rows: i32, rng: &mut u64) {
        col.head = -(rng_range(rng, rows.max(1) as u64) as f32);
        col.speed = SPEED_MIN + rng_unit(rng) * (SPEED_MAX - SPEED_MIN);
        col.len = MIN_LEN + rng_range(rng, (MAX_LEN - MIN_LEN) as u64) as i32;
        col.last_row = col.head.floor() as i32;
    }

    fn rebuild(&mut self, now: f32) {
        let mut rng = self.rng;
        let rows = self.rows;
        let charset = self.glyph_strs.len() as u64;
        self.cols = (0..self.n_cols)
            .map(|_| {
                let mut col = Column {
                    head: 0.0,
                    speed: 0.0,
                    len: 0,
                    last_row: 0,
                    glyphs: (0..rows.max(1))
                        .map(|_| rng_range(&mut rng, charset) as u8)
                        .collect(),
                };
                Self::respawn(&mut col, rows, &mut rng);
                col
            })
            .collect();
        self.rng = rng;
        self.last = now;
    }
}

impl Scene for Matrix {
    fn draw(&mut self, painter: &mut Painter, text: &mut TextRenderer, ctx: &FrameCtx) {
        draw_theme_background(painter, ctx);

        let font = (FONT_PX * ctx.scale).max(14.0);
        let col_w = font * COL_W_FRAC;
        let row_h = font * ROW_H_FRAC;
        let n_cols = ((ctx.wf / col_w).floor() as usize).max(1);
        let rows = ((ctx.hf / row_h).ceil() as i32 + 1).max(1);
        if !self.ready || n_cols != self.n_cols || rows != self.rows {
            self.n_cols = n_cols;
            self.rows = rows;
            self.ready = true;
            self.rebuild(ctx.elapsed_secs);
        }

        let dt = (ctx.elapsed_secs - self.last).clamp(0.0, 0.1);
        self.last = ctx.elapsed_secs;
        let flicker_chance = dt * FLICKER_HZ;

        // ── Advance every column ──
        let mut rng = self.rng;
        let charset = self.glyph_strs.len() as u64;
        for col in &mut self.cols {
            col.head += col.speed * dt;
            let hr = col.head.floor() as i32;
            // Lay down a fresh glyph in each row the head just entered.
            if hr != col.last_row {
                for r in (col.last_row + 1)..=hr {
                    if r >= 0 && (r as usize) < col.glyphs.len() {
                        col.glyphs[r as usize] = rng_range(&mut rng, charset) as u8;
                    }
                }
                col.last_row = hr;
            }
            // Occasionally mutate a visible glyph so the rain shimmers.
            if rng_unit(&mut rng) < flicker_chance {
                let span = col.len.max(1) as u64;
                let r = hr - rng_range(&mut rng, span) as i32;
                if r >= 0 && (r as usize) < col.glyphs.len() {
                    col.glyphs[r as usize] = rng_range(&mut rng, charset) as u8;
                }
            }
            if (col.head - col.len as f32) > rows as f32 {
                Self::respawn(col, rows, &mut rng);
            }
        }
        self.rng = rng;

        // ── Draw ──
        let x0 = (ctx.wf - n_cols as f32 * col_w) * 0.5;
        let sw = ctx.wf.round().max(1.0) as u32;
        let sh = ctx.hf.round().max(1.0) as u32;
        let max_w = font * 2.0;

        for (c, col) in self.cols.iter().enumerate() {
            let head_int = col.head.floor() as i32;
            let lo = (head_int - col.len).max(0);
            let hi = head_int.min(rows - 1);
            let x = x0 + c as f32 * col_w;
            for r in lo..=hi {
                let d = head_int - r; // 0 at the head
                let color = if d == 0 {
                    self.head_color
                } else {
                    let t = (d - 1) as f32 / (col.len - 1).max(1) as f32;
                    self.trail[(t * (TRAIL_LEVELS - 1) as f32).round() as usize % TRAIL_LEVELS]
                };
                let s = &self.glyph_strs[col.glyphs[r as usize] as usize];
                let y = r as f32 * row_h;
                text.queue_family(
                    s,
                    font,
                    x,
                    y,
                    color,
                    max_w,
                    lntrn_theme::FAMILY_MONOSPACE,
                    sw,
                    sh,
                );
            }
        }
    }
}

// ── Tiny self-contained PRNG (xorshift64) — no external crate needed ─────────
fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    nanos | 1
}

fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Uniform integer in `0..n` (n > 0).
fn rng_range(state: &mut u64, n: u64) -> u64 {
    rng_next(state) % n.max(1)
}

/// Uniform float in `0.0..1.0`.
fn rng_unit(state: &mut u64) -> f32 {
    (rng_next(state) >> 40) as f32 / (1u64 << 24) as f32
}
