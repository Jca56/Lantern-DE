//! The recording: its own thread, its own GPU device (`lntrn-video` wants
//! its calls on the thread that submits to the device), frames from the
//! screen into H.264 on the GPU's video engine, into an MP4.
//!
//! Frames are asked for at most `fps` times a second, and after the first
//! only once the screen changed, so each is kept on screen as long as it
//! really was: a frame's length is known when the next one comes, so one
//! is always held back, in the other of two textures.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use lntrn_core::{log_info, log_warn};
use lntrn_video::wgpu;
use lntrn_video::{EncoderConfig, VideoEngine, VideoWriter};

use crate::capture::Screen;
use crate::control;

/// Failed copies in a row before the recording gives up.
const RETRIES: u32 = 5;
/// The smallest area worth recording, a side, in pixels.
pub const SMALLEST: u32 = 64;

/// A part of a monitor, in its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Area {
    /// Inside a `width` × `height` monitor, an even number of pixels a
    /// side (4:2:0 halves colour both ways).
    fn fit(self, width: u32, height: u32) -> Result<Area, String> {
        let x = self.x.min(width.saturating_sub(SMALLEST));
        let y = self.y.min(height.saturating_sub(SMALLEST));
        let w = self.w.min(width - x) & !1;
        let h = self.h.min(height - y) & !1;
        if w < SMALLEST || h < SMALLEST {
            return Err(format!("{w}×{h} is too small to record"));
        }
        Ok(Area { x, y, w, h })
    }
}

pub struct Job {
    pub output: String,
    pub area: Area,
    pub path: PathBuf,
    pub fps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Starting,
    Recording,
    Saving,
    Done,
}

/// How the recording is going, for the badge.
pub struct Status {
    phase: AtomicU8,
    /// When the first frame came.
    started: Mutex<Option<Instant>>,
}

impl Status {
    pub fn new() -> Arc<Status> {
        Arc::new(Status { phase: AtomicU8::new(Phase::Starting as u8), started: Mutex::new(None) })
    }

    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Acquire) {
            0 => Phase::Starting,
            1 => Phase::Recording,
            2 => Phase::Saving,
            _ => Phase::Done,
        }
    }

    fn set(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Release);
        control::wake();
    }

    /// Whether a frame ever came.
    pub fn began(&self) -> bool {
        self.started.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// How long it has recorded, from the first frame.
    pub fn elapsed(&self) -> Duration {
        self.started.lock().unwrap_or_else(|e| e.into_inner()).map_or(Duration::ZERO, |at| at.elapsed())
    }
}

/// A file written.
pub struct Saved {
    pub path: PathBuf,
    pub seconds: f64,
    pub bytes: u64,
    /// Why it ended before it was stopped, when it did.
    pub cut_short: Option<String>,
}

/// Start recording on a thread of its own. Its result says what was saved.
pub fn start(job: Job, status: Arc<Status>) -> JoinHandle<Result<Saved, String>> {
    std::thread::Builder::new()
        .name("lntrn-record".into())
        .spawn(move || {
            let out = record(&job, &status);
            if out.is_err() {
                // Nothing worth keeping: no half-written file left behind.
                let _ = std::fs::remove_file(&job.path);
            }
            status.set(Phase::Done);
            out
        })
        .expect("spawn the recording thread")
}

fn record(job: &Job, status: &Status) -> Result<Saved, String> {
    let mut screen = Screen::open(&job.output)?;
    let area = job.area.fit(screen.width, screen.height)?;
    let (_adapter, device, queue) = lntrn_video::open_headless().map_err(|e| format!("the GPU: {e}"))?;
    let engine = VideoEngine::new(&device, &queue).map_err(|e| format!("the GPU's video engine: {e}"))?;
    if !engine.can_encode() {
        return Err("this GPU's video engine can't encode H.264 through Vulkan Video".into());
    }
    if let Some(dir) = job.path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let fps = job.fps.clamp(1, 240);
    let mut writer = VideoWriter::create(&job.path, &engine, EncoderConfig::new(area.w, area.h, fps)).map_err(|e| format!("the encoder: {e}"))?;
    log_info!("screen-capture: {}×{} at ({}, {}) of {}, {fps} fps, to {}", area.w, area.h, area.x, area.y, job.output, job.path.display());

    let pictures = [picture(&device, area), picture(&device, area)];
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let mut scratch = Vec::new();
    // The frame held back: which picture, and when it came.
    let mut held: Option<(usize, Duration)> = None;
    let mut next = 0;
    let mut asked: Option<Instant> = None;
    let mut failed = 0;
    let mut frames = 0u64;
    let mut cut_short = None;
    loop {
        if let Some(at) = asked
            && !wait_until(at + interval)
        {
            break;
        }
        asked = Some(Instant::now());
        let grab = match screen.grab(held.is_some()) {
            Ok(Some(grab)) => grab,
            Ok(None) => break,
            Err(e) if failed < RETRIES => {
                failed += 1;
                log_warn!("screen-capture: {e}; trying again");
                continue;
            }
            Err(e) => {
                cut_short = Some(e);
                break;
            }
        };
        failed = 0;
        upload(&queue, &pictures[next], &screen, area, grab.flipped, &mut scratch);
        match held {
            Some((slot, at)) => {
                writer.add(&pictures[slot], length(at, grab.at, interval)).map_err(|e| format!("encoding: {e}"))?;
                frames += 1;
            }
            None => {
                *status.started.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                status.set(Phase::Recording);
            }
        }
        held = Some((next, grab.at));
        next = 1 - next;
        // Lets wgpu take back the staging memory of finished uploads.
        let _ = device.poll(wgpu::PollType::Poll);
    }
    status.set(Phase::Saving);
    let Some((slot, at)) = held else {
        return Err(cut_short.unwrap_or_else(|| "stopped before the first frame".into()));
    };
    // The last frame stays until the moment it was stopped.
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    writer.add(&pictures[slot], length(at, now, interval)).map_err(|e| format!("encoding: {e}"))?;
    let mut out = writer.finish().map_err(|e| format!("finishing the file: {e}"))?;
    out.flush().map_err(|e| format!("writing {}: {e}", job.path.display()))?;
    let file = out.into_inner().map_err(|e| format!("writing {}: {}", job.path.display(), e.error()))?;
    file.sync_all().map_err(|e| format!("writing {}: {e}", job.path.display()))?;
    let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    let seconds = status.elapsed().as_secs_f64();
    log_info!("screen-capture: {} frames, {seconds:.1} s, {bytes} bytes", frames + 1);
    Ok(Saved { path: job.path.clone(), seconds, bytes, cut_short })
}

/// How long a frame that came `from` stayed until the next came `to`
/// (the compositor's clock): never less than one interval, so a clock
/// stepping back can't make a frame vanish.
fn length(from: Duration, to: Duration, interval: Duration) -> Duration {
    to.checked_sub(from).filter(|d| *d >= interval / 2).unwrap_or(interval)
}

/// Sleep until `at`, a little at a time. `false` when stopped first.
fn wait_until(at: Instant) -> bool {
    loop {
        if control::stopped() {
            return false;
        }
        let now = Instant::now();
        if now >= at {
            return true;
        }
        std::thread::sleep((at - now).min(Duration::from_millis(50)));
    }
}

/// A texture the size of `area`, in the compositor's byte order.
fn picture(device: &wgpu::Device, area: Area) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screen-capture frame"),
        size: wgpu::Extent3d { width: area.w, height: area.h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// The area of the frame just grabbed, into `texture`. Straight from the
/// shared buffer when the rows are top first; else turned the right way
/// up in `scratch` first.
fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, screen: &Screen, area: Area, flipped: bool, scratch: &mut Vec<u8>) {
    let pixels = screen.pixels();
    let stride = screen.stride as usize;
    let row = area.w as usize * 4;
    let size = wgpu::Extent3d { width: area.w, height: area.h, depth_or_array_layers: 1 };
    let (data, bytes_per_row) = if flipped {
        scratch.clear();
        for y in 0..area.h {
            let from = (screen.height - 1 - (area.y + y)) as usize * stride + area.x as usize * 4;
            scratch.extend_from_slice(&pixels[from..from + row]);
        }
        (&scratch[..], row as u32)
    } else {
        (&pixels[area.y as usize * stride + area.x as usize * 4..], screen.stride)
    };
    let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: None };
    queue.write_texture(texture.as_image_copy(), data, layout, size);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_fits_inside_its_monitor_and_is_even() {
        let a = Area { x: 3800, y: 2100, w: 501, h: 301 }.fit(3840, 2160).unwrap();
        assert!(a.x + a.w <= 3840 && a.y + a.h <= 2160);
        assert_eq!((a.w % 2, a.h % 2), (0, 0));
        assert!(Area { x: 0, y: 0, w: 20, h: 900 }.fit(1920, 1080).is_err());
        assert_eq!(Area { x: 10, y: 20, w: 1001, h: 601 }.fit(1920, 1080).unwrap(), Area { x: 10, y: 20, w: 1000, h: 600 });
    }

    #[test]
    fn a_frame_lasts_until_the_next_and_never_vanishes() {
        let i = Duration::from_millis(16);
        assert_eq!(length(Duration::from_secs(10), Duration::from_millis(10_500), i), Duration::from_millis(500));
        assert_eq!(length(Duration::from_secs(10), Duration::from_secs(9), i), i);
        assert_eq!(length(Duration::from_secs(10), Duration::from_millis(10_001), i), i);
    }
}
