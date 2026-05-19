//! Click ripple animation: three concentric expanding/fading rings
//! spawned at the click point.
//!
//! Each ring is rasterized at its *actual* render diameter (cached by
//! diameter so we don't re-import the same texture each frame). That
//! keeps the stroke a constant ~1.5px in physical pixels regardless of
//! how big the ring grows — a single fixed-size buffer would scale its
//! stroke with its diameter and end up looking like a thick donut at
//! large sizes (which is what the user kept seeing).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    utils::{Logical, Physical, Point, Transform},
};

const RING_COUNT: usize = 3;
const RING_DURATION_MS: u64 = 500;
/// Soft fade-in window at the start, in ms. Prevents the rings from
/// "popping" on at full opacity, which read as a flicker on top of the
/// equally-fast click reaction time.
const RING_FADE_IN_MS: u64 = 80;
/// Relative concentric offset between adjacent rings (fraction of
/// cursor diameter). Ring N spawns at `start_frac + N * RING_GAP`.
const RING_GAP: f32 = 0.18;
/// Baseline ring diameter as a fraction of the cursor's pixel size,
/// tuned so the rings stay tight around the tip rather than engulfing
/// the whole cursor. Matches the reference mockup in
/// `~/-lntrn-cursor-click.1.svg`.
const RING_START_FRAC: f32 = 0.3;
const RING_END_FRAC: f32 = 0.9;
/// Constant physical-pixel ring stroke width. Independent of diameter
/// so rings stay thin no matter how large the ripple grows.
const RING_STROKE_PX: f32 = 1.5;
/// Hard cap on how big a single ring buffer can get. A 256-px ring on a
/// 1.0 scale display takes ~262KB — small. The cap stops a runaway
/// `click_anim_size = 3.0` on a `cursor_size = 64` from exploding.
const RING_DIAMETER_CAP: u32 = 256;

pub struct ClickAnimState {
    /// Pixel-diameter → pre-rasterized ring. Built lazily as new sizes
    /// appear during an animation; cleared when the color changes.
    buffers: HashMap<u32, MemoryRenderBuffer>,
    color: (u8, u8, u8),
    clicks: Vec<Click>,
}

struct Click {
    start: Instant,
    origin: Point<f64, Logical>,
}

struct Settings {
    enabled: bool,
    size: f32,
    color: (u8, u8, u8),
}

fn read_settings() -> Settings {
    let enabled = crate::input::read_input_setting("click_anim_enabled", "true") != "false";
    let size = (crate::input::read_input_setting_f64("click_anim_size", 1.0) as f32)
        .clamp(0.0, 3.0);
    let raw_color = crate::input::read_input_setting("click_anim_color", "");
    let color_str = if raw_color.is_empty() {
        // "Inherit" mode samples the cursor's body-light stop so the
        // ripple matches the bright top of the arrow gradient.
        crate::input::read_input_setting("cursor_body_light", "#ffffff")
    } else {
        raw_color
    };
    Settings {
        enabled,
        size,
        color: crate::cursor::parse_hex_color(&color_str),
    }
}

impl ClickAnimState {
    pub fn new(_color: (u8, u8, u8)) -> Self {
        // Color is sampled from config on first render; we don't keep
        // the constructor argument because the user can change it
        // before the first click anyway.
        Self {
            buffers: HashMap::new(),
            color: (0, 0, 0),
            clicks: Vec::new(),
        }
    }

    fn sync_color(&mut self, color: (u8, u8, u8)) {
        if color == self.color && !self.buffers.is_empty() {
            return;
        }
        self.buffers.clear();
        self.color = color;
    }

    pub fn trigger(&mut self, origin: Point<f64, Logical>) {
        let s = read_settings();
        if !s.enabled || s.size <= 0.0 {
            self.clicks.clear();
            return;
        }
        self.sync_color(s.color);
        self.clicks.clear();
        self.clicks.push(Click { start: Instant::now(), origin });
    }

    pub fn tick(&mut self) -> bool {
        let s = read_settings();
        if !s.enabled {
            self.clicks.clear();
            return false;
        }
        self.sync_color(s.color);
        let total = Duration::from_millis(RING_DURATION_MS);
        self.clicks.retain(|c| c.start.elapsed() < total);
        !self.clicks.is_empty()
    }

    pub fn is_active(&self) -> bool {
        !self.clicks.is_empty()
    }

    pub fn render_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        cursor_size_px: u32,
        output_origin: Point<f64, Logical>,
        output_scale: f64,
    ) -> Vec<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let mut out = Vec::new();
        let size_scale = read_settings().size.max(0.0);
        if size_scale <= 0.0 {
            return out;
        }
        // All RING_COUNT rings spawn at the same instant at concentric
        // sizes. Each one's own diameter grows by `RING_GROWTH` over the
        // animation so the cluster expands outward like a soft pulse,
        // but the rings stay distinguishable from each other.
        let cursor_logical = cursor_size_px as f32;
        let base = cursor_logical * RING_START_FRAC * size_scale;
        let gap = cursor_logical * RING_GAP * size_scale;
        let growth = cursor_logical * (RING_END_FRAC - RING_START_FRAC) * size_scale;
        for click in &self.clicks {
            let elapsed_ms = click.start.elapsed().as_secs_f32() * 1000.0;
            if elapsed_ms < 0.0 || elapsed_ms > RING_DURATION_MS as f32 {
                continue;
            }
            let phys_origin = (
                (click.origin.x - output_origin.x) * output_scale,
                (click.origin.y - output_origin.y) * output_scale,
            );
            let t = elapsed_ms / RING_DURATION_MS as f32;
            let t_eased = 1.0 - (1.0 - t).powi(3);              // ease-out cubic
            // Smooth fade-in over RING_FADE_IN_MS, then ease-in fade-out
            // through the rest of the animation. Cosine fade-in is
            // smooth at both ends so the rings don't pop on.
            let fade_in = (elapsed_ms / RING_FADE_IN_MS as f32).clamp(0.0, 1.0);
            let fade_in = 0.5 - 0.5 * (fade_in * std::f32::consts::PI).cos();
            let fade_out = ((1.0 - t).max(0.0)).powi(2);
            let alpha = fade_in * fade_out;
            for i in 0..RING_COUNT {
                let diameter_logical = base + gap * i as f32 + growth * t_eased;
                if diameter_logical < 3.0 {
                    continue;
                }
                // Bake the buffer in *physical* pixels so the stroke
                // stays crisp on HiDPI outputs.
                let diameter_phys = ((diameter_logical as f64) * output_scale)
                    .round()
                    .clamp(4.0, RING_DIAMETER_CAP as f64) as u32;
                let color = self.color;
                let buffer = self.buffers
                    .entry(diameter_phys)
                    .or_insert_with(|| build_ring_buffer(diameter_phys, color, RING_STROKE_PX));
                let half_phys = diameter_phys as f64 * 0.5;
                let loc: Point<f64, Physical> = Point::from((
                    phys_origin.0 - half_phys,
                    phys_origin.1 - half_phys,
                ));
                if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    loc,
                    buffer,
                    Some(alpha),
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    out.push(elem);
                }
            }
        }
        out
    }
}

/// Rasterize a single ring outline at `diameter` pixels with constant
/// `stroke` pixel width and antialiased edges. Premultiplied BGRA so
/// downstream alpha modulation composes correctly.
fn build_ring_buffer(diameter: u32, color: (u8, u8, u8), stroke: f32) -> MemoryRenderBuffer {
    let size = diameter.max(4);
    let s = size as f32;
    let center = s * 0.5;
    // Half-px margin on the outer edge gives the AA fade room to land
    // without clipping against the buffer's own bounds — that clipping
    // is exactly the "cut off by bounding box" effect we kept seeing
    // before this rewrite.
    let outer = center - 0.75;
    let half_stroke = stroke.max(0.5) * 0.5;
    let inner_target = outer - stroke.max(0.5);
    let inner = inner_target.max(0.0);
    let pixel_count = (size * size) as usize;
    let mut rgba = vec![0u8; pixel_count * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let r = (dx * dx + dy * dy).sqrt();
            let outer_a = (outer - r + 0.5).clamp(0.0, 1.0);
            let inner_a = (r - inner + 0.5).clamp(0.0, 1.0);
            let a = outer_a * inner_a;
            if a <= 0.0 {
                continue;
            }
            let idx = ((y * size + x) * 4) as usize;
            rgba[idx]     = (color.2 as f32 * a) as u8; // B
            rgba[idx + 1] = (color.1 as f32 * a) as u8; // G
            rgba[idx + 2] = (color.0 as f32 * a) as u8; // R
            rgba[idx + 3] = (a * 255.0) as u8;          // A
        }
    }
    // half_stroke is referenced for clarity above; suppress unused warning
    // by binding it to itself. (The inner_target calc already encodes it.)
    let _ = half_stroke;
    MemoryRenderBuffer::from_slice(
        &rgba,
        Fourcc::Argb8888,
        (size as i32, size as i32),
        1,
        Transform::Normal,
        None,
    )
}
