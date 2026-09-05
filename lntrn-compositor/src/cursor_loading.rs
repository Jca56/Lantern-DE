//! Loading-cursor spinner: a ring of orbiting dots rendered alongside
//! the cursor whenever a client sets `CursorIcon::Wait` or
//! `CursorIcon::Progress`. The dots rotate around the cursor's bounding
//! box center at a constant angular velocity; alpha tapers so one dot
//! reads as "the head" of the spinner and the rest fade behind it.
//!
//! A single dot sprite is pre-rasterized at the configured color and
//! reused (with per-element alpha) for every orbit position. No
//! per-frame allocations.

use std::time::Instant;

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
    utils::{Logical, Physical, Point, Size, Transform},
};

/// How many dots in the orbit. More dots = smoother arc, more elements.
const DOT_COUNT: usize = 8;
/// Seconds per full revolution.
const REVOLUTION_SECONDS: f32 = 1.1;
/// Orbit radius as a fraction of the cursor's pixel size.
const ORBIT_RADIUS_FRAC: f32 = 0.65;
/// Each dot's diameter as a fraction of the cursor's pixel size.
const DOT_FRAC: f32 = 0.13;
/// Pre-rasterized dot resolution. Small → cheap to rebuild on color
/// change; big enough to look smooth when scaled up for huge cursors.
const DOT_BUFFER_PX: u32 = 32;

pub struct LoadingAnimState {
    active: bool,
    start: Instant,
    dot_buffer: MemoryRenderBuffer,
    color: (u8, u8, u8),
}

impl LoadingAnimState {
    pub fn new(color: (u8, u8, u8)) -> Self {
        Self {
            active: false,
            start: Instant::now(),
            dot_buffer: build_dot_buffer(DOT_BUFFER_PX, color),
            color,
        }
    }

    /// Toggle the spinner. `color` is the cursor's body-light hex parsed
    /// to RGB — re-rasterizes the dot sprite if it differs from the
    /// cached one. Setting `active = true` while already active leaves
    /// the rotation phase running so flicking the cursor icon back and
    /// forth doesn't restart the animation.
    pub fn set_active(&mut self, active: bool, color: (u8, u8, u8)) {
        if active && !self.active {
            self.start = Instant::now();
        }
        self.active = active;
        if color != self.color {
            self.dot_buffer = build_dot_buffer(DOT_BUFFER_PX, color);
            self.color = color;
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Build per-frame render elements. Caller passes the cursor's
    /// physical position (output-local), the configured cursor pixel
    /// size, and the output scale so dots match HiDPI sharpness.
    pub fn render_elements(
        &self,
        renderer: &mut GlesRenderer,
        cursor_pos: Point<f64, Physical>,
        cursor_size_px: u32,
        output_scale: f64,
    ) -> Vec<MemoryRenderBufferRenderElement<GlesRenderer>> {
        if !self.active {
            return Vec::new();
        }
        let elapsed = self.start.elapsed().as_secs_f32();
        let rotation = (elapsed / REVOLUTION_SECONDS) * std::f32::consts::TAU;
        let cursor_logical = cursor_size_px as f32;
        let radius_phys = cursor_logical * ORBIT_RADIUS_FRAC * output_scale as f32;
        let dot_logical = (cursor_logical * DOT_FRAC).max(4.0);
        let dot_size_i = dot_logical.round().max(1.0) as i32;
        // Orbit center sits at the cursor's bounding-box center, not on
        // the tip, so the spinner reads as "next to the arrow" rather
        // than impaling it.
        let center_phys = (
            cursor_pos.x + (cursor_logical * 0.5 * output_scale as f32) as f64,
            cursor_pos.y + (cursor_logical * 0.5 * output_scale as f32) as f64,
        );
        let mut out = Vec::with_capacity(DOT_COUNT);
        for i in 0..DOT_COUNT {
            // i=0 is "head" (brightest, leading the rotation), trailing
            // dots fade. Wraps around the orbit so the spinner reads as
            // a comet rather than a static ring.
            let phase = i as f32 / DOT_COUNT as f32;
            let angle = rotation - phase * std::f32::consts::TAU;
            let dx = angle.cos() * radius_phys;
            let dy = angle.sin() * radius_phys;
            // Comet trail: head is bright, alpha drops to ~0.15 at the
            // back of the trail. Quadratic falloff so the trail tapers.
            let alpha = (1.0 - phase).powi(2) * 0.85 + 0.15;
            let half = dot_size_i as f64 * 0.5;
            let loc: Point<f64, Physical> = Point::from((
                center_phys.0 + dx as f64 - half,
                center_phys.1 + dy as f64 - half,
            ));
            if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                loc,
                &self.dot_buffer,
                Some(alpha),
                None,
                Some(Size::from((dot_size_i, dot_size_i))),
                // Not Kind::Cursor — see cursor_click.rs.
                Kind::Unspecified,
            ) {
                out.push(elem);
            }
        }
        out
    }
}

fn build_dot_buffer(size: u32, color: (u8, u8, u8)) -> MemoryRenderBuffer {
    let s = size as f32;
    let center = s * 0.5;
    let radius = center - 1.0;
    let pixel_count = (size * size) as usize;
    let mut rgba = vec![0u8; pixel_count * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let r = (dx * dx + dy * dy).sqrt();
            // Anti-aliased filled disc: alpha fades over the last 1px.
            let a = (radius - r + 0.5).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let idx = ((y * size + x) * 4) as usize;
            // Premultiplied BGRA — matches Argb8888 fourcc with le order.
            rgba[idx] = (color.2 as f32 * a) as u8;
            rgba[idx + 1] = (color.1 as f32 * a) as u8;
            rgba[idx + 2] = (color.0 as f32 * a) as u8;
            rgba[idx + 3] = (a * 255.0) as u8;
        }
    }
    MemoryRenderBuffer::from_slice(
        &rgba,
        Fourcc::Argb8888,
        (size as i32, size as i32),
        1,
        Transform::Normal,
        None,
    )
}

// Suppress the unused-import warning when this module is compiled
// without the Logical type being referenced (used elsewhere in the
// crate for the cursor render path).
#[allow(dead_code)]
fn _logical_marker(_: Point<f64, Logical>) {}
