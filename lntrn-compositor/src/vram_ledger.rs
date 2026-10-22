//! VRAM ledger: what the compositor holds on the GPU, by owner.
//!
//! Samples once a second and logs (target `vram`) when the owned total sets
//! a new peak, and every few minutes when anything has changed. Compare it
//! with `nvidia-smi`: the driver's figure for this process less the ledger is
//! the GL context's own baseline plus whatever the driver keeps after we free
//! it.
//!
//! Every format we allocate is 8888, so sizes count 4 bytes a pixel.
//! Swapchains are an upper bound: Smithay fills their slots lazily, up to
//! `SWAPCHAIN_SLOTS`. Client dmabufs are listed for reference only; that
//! memory is the client's, the compositor just imports it.

use std::collections::HashSet;
use std::fmt;
use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        buffer_type, gles::GlesTexture, utils::RendererSurfaceStateUserData, BufferType,
        ContextId, Renderer, Texture,
    },
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            LoopHandle,
        },
        wayland_server::protocol::wl_surface::WlSurface,
    },
    wayland::compositor::{with_surface_tree_downward, SurfaceData, TraversalAction},
};

use crate::Lantern;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const REPORT_INTERVAL: Duration = Duration::from_secs(300);
const MIB: u64 = 1024 * 1024;
/// Smithay's `allocator::swapchain::SLOT_CAP`, which it keeps private.
const SWAPCHAIN_SLOTS: u64 = 4;
/// An owned peak this far past the last one logged gets a line at once.
const PEAK_STEP: u64 = 32 * MIB;

/// Start sampling on the event loop.
pub fn install(handle: &LoopHandle<'static, Lantern>) {
    let mut ledger = VramLedger::default();
    let inserted = handle.insert_source(Timer::from_duration(SAMPLE_INTERVAL), move |_, _, state| {
        ledger.sample(state);
        TimeoutAction::ToDuration(SAMPLE_INTERVAL)
    });
    if let Err(e) = inserted {
        tracing::warn!("vram ledger: timer failed: {}", e.error);
    }
}

/// How many buffers, and the bytes they hold.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Tally {
    count: usize,
    bytes: u64,
}

impl Tally {
    fn add(&mut self, w: i32, h: i32) {
        self.add_bytes(w.max(0) as u64 * h.max(0) as u64 * 4);
    }

    fn add_bytes(&mut self, bytes: u64) {
        self.count += 1;
        self.bytes += bytes;
    }

    fn add_texture(&mut self, texture: &GlesTexture) {
        let size = texture.size();
        self.add(size.w, size.h);
    }

    fn keep_max(&mut self, other: Tally) {
        if other.bytes > self.bytes {
            *self = other;
        }
    }
}

impl fmt::Display for Tally {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} MiB ({})", mib(self.bytes), self.count)
    }
}

fn mib(bytes: u64) -> u64 {
    (bytes + MIB / 2) / MIB
}

/// One sample, by owner.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Breakdown {
    swapchains: Tally,
    wallpapers: Tally,
    blur: Tally,
    blur_results: Tally,
    snapshots: Tally,
    largest_snapshot: (i32, i32),
    screencopy: Tally,
    readback: Tally,
    shm_uploads: Tally,
    client_dmabufs: Tally,
}

impl Breakdown {
    /// Everything the compositor allocated itself.
    fn owned(&self) -> u64 {
        [
            self.swapchains,
            self.wallpapers,
            self.blur,
            self.blur_results,
            self.snapshots,
            self.screencopy,
            self.readback,
            self.shm_uploads,
        ]
        .iter()
        .map(|t| t.bytes)
        .sum()
    }

    fn keep_max(&mut self, other: &Breakdown) {
        self.swapchains.keep_max(other.swapchains);
        self.wallpapers.keep_max(other.wallpapers);
        self.blur.keep_max(other.blur);
        self.blur_results.keep_max(other.blur_results);
        if other.snapshots.bytes > self.snapshots.bytes {
            self.snapshots = other.snapshots;
            self.largest_snapshot = other.largest_snapshot;
        }
        self.screencopy.keep_max(other.screencopy);
        self.readback.keep_max(other.readback);
        self.shm_uploads.keep_max(other.shm_uploads);
        self.client_dmabufs.keep_max(other.client_dmabufs);
    }
}

impl fmt::Display for Breakdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (sw, sh) = self.largest_snapshot;
        write!(
            f,
            "swapchains <={} | wallpapers {} | blur {} + results {} | snapshots {}, largest {}x{} \
             | screencopy {} + readback {} | shm uploads {} | client dmabufs {} (theirs)",
            self.swapchains,
            self.wallpapers,
            self.blur,
            self.blur_results,
            self.snapshots,
            sw,
            sh,
            self.screencopy,
            self.readback,
            self.shm_uploads,
            self.client_dmabufs,
        )
    }
}

#[derive(Default)]
struct VramLedger {
    peak: u64,
    peak_logged: u64,
    /// Each owner's own high-water mark, whenever it happened.
    peaks: Breakdown,
    last_logged: Option<Breakdown>,
    last_report: Option<Instant>,
}

impl VramLedger {
    fn sample(&mut self, state: &Lantern) {
        let now = measure(state);
        let owned = now.owned();
        self.peaks.keep_max(&now);
        self.peak = self.peak.max(owned);

        if self.last_logged.is_none() {
            self.log("start", &now);
        } else if self.peak >= self.peak_logged + PEAK_STEP {
            self.log("new peak", &now);
        } else if self.last_report.is_some_and(|t| t.elapsed() >= REPORT_INTERVAL)
            && self.last_logged != Some(now)
        {
            self.log("now", &now);
            tracing::info!(target: "vram", "peaks: {}", self.peaks);
        }
    }

    fn log(&mut self, label: &str, now: &Breakdown) {
        self.peak_logged = self.peak;
        self.last_logged = Some(*now);
        self.last_report = Some(Instant::now());
        tracing::info!(
            target: "vram",
            "{label}: owned {} MiB, peak {} MiB | {now}",
            mib(now.owned()),
            mib(self.peak),
        );
    }
}

fn measure(state: &Lantern) -> Breakdown {
    let mut b = Breakdown::default();

    for (w, h) in state.wallpaper.buffer_sizes() {
        b.wallpapers.add(w, h);
    }
    for snapshot in state.window_snapshots.values() {
        b.snapshots.add(snapshot.size.w, snapshot.size.h);
        let (lw, lh) = b.largest_snapshot;
        if snapshot.size.w as i64 * snapshot.size.h as i64 > lw as i64 * lh as i64 {
            b.largest_snapshot = (snapshot.size.w, snapshot.size.h);
        }
    }
    for capture in state.screencopy_offscreen.values() {
        let size = capture.size();
        b.screencopy.add(size.w, size.h);
    }
    for pbo in state.screencopy_pbos.values() {
        b.readback.add_bytes(pbo.bytes());
    }

    let Some(udev) = state.udev.as_ref() else {
        return b;
    };
    for backend in udev.backends.values() {
        for surface in backend.surfaces.values() {
            let (w, h) = surface.drm_output.with_compositor(|c| c.pending_mode().size());
            b.swapchains
                .add_bytes(SWAPCHAIN_SLOTS * w as u64 * h as u64 * 4);
        }
    }
    for blur in udev.blur_states.values() {
        for texture in std::iter::once(&blur.scene).chain(&blur.textures) {
            b.blur.add_texture(texture);
        }
        for texture in &blur.results {
            b.blur_results.add_texture(texture);
        }
    }
    if let Some(renderer) = udev.renderer.as_ref() {
        client_textures(state, renderer.context_id(), &mut b);
    }
    b
}

/// The textures Smithay keeps for client buffers: an SHM buffer is copied
/// into one the compositor owns, a dmabuf is only imported.
fn client_textures(state: &Lantern, ctx: ContextId<GlesTexture>, b: &mut Breakdown) {
    let mut seen: HashSet<WlSurface> = HashSet::new();
    let mut visit = |surface: &WlSurface, states: &SurfaceData| {
        if !seen.insert(surface.clone()) {
            return;
        }
        let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() else {
            return;
        };
        let Ok(rss) = data.lock() else {
            return;
        };
        let Some(texture) = rss.texture::<GlesTexture>(ctx.clone()) else {
            return;
        };
        match rss.buffer().and_then(|buffer| buffer_type(buffer)) {
            Some(BufferType::Shm) => b.shm_uploads.add_texture(texture),
            Some(BufferType::Dma) => b.client_dmabufs.add_texture(texture),
            _ => {}
        }
    };

    // A window can sit in a workspace's Space and the global one at once;
    // `seen` counts each surface once.
    let windows = state
        .all_windows()
        .into_iter()
        .chain(state.space.elements().cloned())
        .chain(state.minimized_windows.iter().map(|m| m.window.clone()));
    for window in windows {
        window.with_surfaces(&mut visit);
    }
    for layer in &state.layer_surfaces {
        with_surface_tree_downward(
            layer.wl_surface(),
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |surface, states, _| visit(surface, states),
            |_, _, _| true,
        );
    }
}
