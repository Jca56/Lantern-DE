//! Small render-pipeline helpers that don't need access to the whole
//! [`Lantern`] state — kept out of `surface.rs` so the giant
//! `render_surface` body stays focused on the pipeline.

use std::time::{Duration, Instant};

use smithay::backend::renderer::utils::{CommitCounter, RendererSurfaceStateUserData};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                surface::WaylandSurfaceRenderElement, AsRenderElements, Element, RenderElement,
            },
            gles::{GlesRenderer, GlesTexture},
            Bind, Color32F, Frame, Offscreen, Renderer,
        },
    },
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

// ── Window snapshots ──────────────────────────────────────────────────────

/// Minimum time between two snapshot captures of the same window. Captures
/// are additionally gated on the window's content actually changing, so a
/// static window costs nothing and an animating one is sampled at ~10Hz.
///
/// This replaced a `frame_counter % 6` gate: at 240Hz that was 40 captures
/// per second per visible window, each allocating a fresh full-size texture
/// and running an offscreen render + flush — the single biggest CPU/GPU cost
/// on an otherwise idle desktop.
pub const SNAPSHOT_MIN_INTERVAL: Duration = Duration::from_millis(100);

/// A window's last captured content, used by the close animation (client
/// died / closed itself) and as the crossfade source for resize animations.
pub struct WindowSnapshot {
    pub texture: GlesTexture,
    pub size: Size<i32, Physical>,
    /// [`window_content_key`] at capture time — recapture only when it moves.
    pub content_key: u64,
    pub captured_at: Instant,
}

/// Cheap fingerprint of a window's buffer contents: folds the commit counter
/// of every surface in the window tree (toplevel, subsurfaces, popups) into
/// one hash. Any client commit anywhere in the tree changes it.
pub(crate) fn window_content_key(window: &smithay::desktop::Window) -> u64 {
    // FNV-1a over the per-surface commit distances.
    let mut key: u64 = 0xcbf2_9ce4_8422_2325;
    window.with_surfaces(|_, data| {
        let commits = data
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .map(|s| {
                s.lock()
                    .unwrap()
                    .current_commit()
                    .distance(Some(CommitCounter::default()))
                    .unwrap_or(0) as u64
            })
            .unwrap_or(u64::MAX);
        key = (key ^ commits).wrapping_mul(0x0000_0100_0000_01b3);
        key = (key ^ 0xff).wrapping_mul(0x0000_0100_0000_01b3);
    });
    key
}

/// Whether `window` needs a fresh snapshot given its current content key.
pub(crate) fn snapshot_is_stale(previous: Option<&WindowSnapshot>, content_key: u64) -> bool {
    match previous {
        None => true,
        Some(p) => p.content_key != content_key && p.captured_at.elapsed() >= SNAPSHOT_MIN_INTERVAL,
    }
}

/// Capture a window's surface content into an offscreen texture so the
/// close animation can keep rendering after the surface has been
/// destroyed / unmapped. Reuses `previous`'s texture when the size is
/// unchanged instead of allocating a new one every capture.
pub(super) fn capture_window_snapshot(
    renderer: &mut GlesRenderer,
    window: &smithay::desktop::Window,
    win_size: Size<i32, Logical>,
    output_scale: f64,
    previous: Option<&WindowSnapshot>,
    content_key: u64,
) -> Option<WindowSnapshot> {
    let snap_w = (win_size.w as f64 * output_scale).round() as i32;
    let snap_h = (win_size.h as f64 * output_scale).round() as i32;
    // Tiny surfaces (< 16px) are usually transient bootstrap buffers from
    // Proton/Wine that resize themselves a frame later. Capturing them
    // racing against the client's realloc triggers GL_INVALID_VALUE.
    if snap_w < 16 || snap_h < 16 {
        return None;
    }

    let snap_size = Size::<i32, Physical>::from((snap_w, snap_h));
    let buf_size: Size<i32, BufferCoords> = Size::from((snap_w, snap_h));

    // Render the surface tree shifted by -geometry().loc so the texture
    // contains exactly the visible geometry box. Clients that draw CSD
    // shadow margins outside their geometry (Firefox/GTK) have a nonzero
    // offset — capturing at (0,0) shifts their content down-right and
    // crops it, which made the close animation visibly jump.
    let geo_loc = window.geometry().loc;
    let origin: Point<i32, Physical> = Point::from((
        -((geo_loc.x as f64 * output_scale).round() as i32),
        -((geo_loc.y as f64 * output_scale).round() as i32),
    ));
    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        window.render_elements(renderer, origin, Scale::from(output_scale), 1.0);
    if elements.is_empty() {
        return None;
    }

    // Same size as last time → draw over the existing texture. The snapshot
    // is only ever *read* during resize/close animations, during which no
    // capture happens, so nothing is sampling it while we overwrite.
    let mut tex = match previous.filter(|p| p.size == snap_size) {
        Some(p) => p.texture.clone(),
        None => Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buf_size).ok()?,
    };
    {
        let mut target = renderer.bind(&mut tex).ok()?;
        let mut frame = renderer
            .render(&mut target, snap_size, Transform::Normal)
            .ok()?;
        frame
            .clear(
                Color32F::from([0.0, 0.0, 0.0, 0.0]),
                &[Rectangle::from_size(snap_size)],
            )
            .ok()?;

        let scale = Scale::from(output_scale);
        for elem in &elements {
            let geo = elem.geometry(scale);
            let src = elem.src();
            let dst = Rectangle::<i32, Physical>::new(geo.loc, geo.size);
            if dst.size.w > 0 && dst.size.h > 0 {
                let _ = elem.draw(&mut frame, src, dst, &[dst], &[]);
            }
        }
        let _ = frame.finish();
    }

    Some(WindowSnapshot {
        texture: tex,
        size: snap_size,
        content_key,
        captured_at: Instant::now(),
    })
}

// ── Slow-render reporting ─────────────────────────────────────────────────

/// Where the time went in one over-budget frame, in milliseconds.
#[derive(Default, Clone, Copy, Debug)]
pub struct SlowRenderBreakdown {
    pub prelude_ms: f64,
    pub elements_ms: f64,
    pub chrome_ms: f64,
    pub config_ms: f64,
    pub blur_ms: f64,
    pub render_ms: f64,
}

/// Rate-limits the "slow render" warning. The old unconditional `warn!` at
/// >4ms fired on nearly every 4K frame at 240Hz (budget 4.17ms) — 64k lines
/// in compositor.log, each a synchronous file flush from inside the render
/// path. Now: the first over-budget frame in a quiet period logs at once,
/// then everything for the next second is folded into one summary line.
#[derive(Default)]
pub struct SlowRenderStats {
    window_start: Option<Instant>,
    count: u32,
    worst_total_ms: f64,
    worst: SlowRenderBreakdown,
}

/// What to log for a slow frame, if anything.
pub enum SlowRenderReport {
    /// First slow frame after a quiet period — log it as-is.
    Single(f64, SlowRenderBreakdown),
    /// Folded summary: (frames, window seconds, worst total ms, worst breakdown).
    Summary(u32, f64, f64, SlowRenderBreakdown),
}

impl SlowRenderStats {
    pub fn record(&mut self, total_ms: f64, breakdown: SlowRenderBreakdown) -> Option<SlowRenderReport> {
        let now = Instant::now();
        let Some(start) = self.window_start else {
            self.window_start = Some(now);
            self.count = 1;
            self.worst_total_ms = total_ms;
            self.worst = breakdown;
            return Some(SlowRenderReport::Single(total_ms, breakdown));
        };
        self.count += 1;
        if total_ms > self.worst_total_ms {
            self.worst_total_ms = total_ms;
            self.worst = breakdown;
        }
        let elapsed = now.duration_since(start);
        if elapsed < Duration::from_secs(1) {
            return None;
        }
        let report = SlowRenderReport::Summary(
            self.count,
            elapsed.as_secs_f64(),
            self.worst_total_ms,
            self.worst,
        );
        *self = Self::default();
        Some(report)
    }
}
