use std::path::{Path, PathBuf};

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use crate::pipeline::MediaPipeline;

pub const VIS_BARS: usize = 32;

#[derive(Clone, Copy, PartialEq)]
pub enum VisMode {
    RadialBars,
    ConcentricRings,
}

impl VisMode {
    pub fn next(self) -> Self {
        match self {
            VisMode::RadialBars => VisMode::ConcentricRings,
            VisMode::ConcentricRings => VisMode::RadialBars,
        }
    }
}

pub struct App {
    pub pipeline: Option<MediaPipeline>,
    pub video_texture: Option<GpuTexture>,
    pub video_width: u32,
    pub video_height: u32,
    pub file_path: Option<PathBuf>,
    pub file_name: String,
    pub volume: f64,
    pub position_ns: u64,
    pub duration_ns: u64,
    pub seeking: bool,
    pub seek_value: f32,
    pub status_text: String,
    pub audio_only: bool,
    pub vis_mode: VisMode,
    pub vis_bars: Vec<f32>,
    pub vol_showing: bool,
    pub vol_dragging: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            pipeline: None,
            video_texture: None,
            video_width: 0,
            video_height: 0,
            file_path: None,
            file_name: String::new(),
            volume: 1.0,
            position_ns: 0,
            duration_ns: 0,
            seeking: false,
            seek_value: 0.0,
            status_text: "No media loaded".into(),
            audio_only: false,
            vis_mode: VisMode::ConcentricRings,
            vis_bars: vec![0.0; VIS_BARS],
            vol_showing: false,
            vol_dragging: false,
        }
    }

    pub fn open_file(&mut self, path: &str) {
        let abs = match Path::new(path).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                self.status_text = format!("File not found: {path} ({e})");
                return;
            }
        };

        let uri = format!("file://{}", abs.display());
        match MediaPipeline::new(&uri) {
            Ok(pipe) => {
                pipe.set_volume(self.volume);
                pipe.play();
                self.pipeline = Some(pipe);
                self.file_name = abs
                    .file_name()
                    .map(|n| n.to_string_lossy().into())
                    .unwrap_or_default();
                self.file_path = Some(abs.clone());
                self.status_text = abs.to_string_lossy().into();
                self.video_texture = None;
                self.video_width = 0;
                self.video_height = 0;
                self.position_ns = 0;
                self.duration_ns = 0;
                self.seeking = false;
                self.audio_only = false;
                self.vis_bars = vec![0.0; VIS_BARS];
            }
            Err(e) => {
                self.status_text = format!("Failed to open: {e}");
            }
        }
    }

    /// Grab the latest decoded frame and upload it as a GPU texture.
    /// Returns true if a new frame was uploaded or spectrum updated (needs redraw).
    pub fn tick(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) -> bool {
        let pipe = match &mut self.pipeline {
            Some(p) => p,
            None => return false,
        };

        // Update position/duration
        if let Some(pos) = pipe.position() {
            self.position_ns = pos;
        }
        if let Some(dur) = pipe.duration() {
            self.duration_ns = dur;
        }

        // Detect audio-only — only trust n-video after duration is known
        // (playbin needs time to demux before stream counts are valid)
        if !self.audio_only && self.duration_ns > 0 && pipe.is_audio_only() {
            self.audio_only = true;
        }

        // Poll bus for spectrum messages + log-scale into visual bars
        pipe.poll_spectrum();
        let raw = pipe.spectrum();
        let log_bars = log_group_spectrum(raw, VIS_BARS);
        for i in 0..VIS_BARS {
            let target = log_bars[i];
            let current = self.vis_bars[i];
            // Smooth: rise fast, fall moderate (snappy but not jittery)
            if target > current {
                self.vis_bars[i] = current + (target - current) * 0.7;
            } else {
                self.vis_bars[i] = current + (target - current) * 0.25;
            }
        }

        // Grab latest video frame — if we get one, it's definitely not audio-only
        if let Some(frame) = pipe.take_frame() {
            self.audio_only = false;
            self.video_width = frame.width;
            self.video_height = frame.height;
            self.video_texture = Some(tex_pass.upload(gpu, &frame.rgba, frame.width, frame.height));
            return true;
        }

        // For audio-only, always redraw (visualizer animates)
        self.audio_only
    }

    pub fn is_playing(&self) -> bool {
        self.pipeline.as_ref().map(|p| p.is_playing()).unwrap_or(false)
    }

    pub fn toggle_play_pause(&mut self) {
        if let Some(pipe) = &self.pipeline {
            pipe.toggle();
        }
    }

    pub fn seek_relative(&mut self, delta_ns: i64) {
        if let Some(pipe) = &self.pipeline {
            let pos = self.position_ns as i64 + delta_ns;
            let clamped = pos.max(0) as u64;
            let clamped = if self.duration_ns > 0 {
                clamped.min(self.duration_ns)
            } else {
                clamped
            };
            pipe.seek(clamped);
        }
    }

    pub fn seek_to_fraction(&mut self, frac: f32) {
        if let Some(pipe) = &self.pipeline {
            let target = (frac.clamp(0.0, 1.0) as f64 * self.duration_ns as f64) as u64;
            pipe.seek(target);
        }
        self.seeking = false;
    }

    pub fn adjust_volume(&mut self, delta: f64) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        if let Some(pipe) = &self.pipeline {
            pipe.set_volume(self.volume);
        }
    }

    pub fn progress_fraction(&self) -> f32 {
        if self.duration_ns == 0 {
            0.0
        } else {
            (self.position_ns as f64 / self.duration_ns as f64) as f32
        }
    }

    pub fn cycle_vis_mode(&mut self) {
        self.vis_mode = self.vis_mode.next();
    }

    pub fn format_time(ns: u64) -> String {
        let total_secs = ns / 1_000_000_000;
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;
        if hours > 0 {
            format!("{hours}:{mins:02}:{secs:02}")
        } else {
            format!("{mins}:{secs:02}")
        }
    }
}

/// Group linear FFT bins into log-spaced visual bars.
/// Low frequencies get fewer bins, high frequencies get more — matching human hearing.
fn log_group_spectrum(raw: &[f32], num_bars: usize) -> Vec<f32> {
    let n = raw.len();
    if n == 0 {
        return vec![0.0; num_bars];
    }

    let mut bars = Vec::with_capacity(num_bars);
    for i in 0..num_bars {
        // Log-spaced bin boundaries
        let lo = ((i as f64 / num_bars as f64).powf(2.0) * n as f64) as usize;
        let hi = (((i + 1) as f64 / num_bars as f64).powf(2.0) * n as f64) as usize;
        let lo = lo.min(n);
        let hi = hi.max(lo + 1).min(n);

        // Average the dB values in this group, then normalize
        let sum: f32 = raw[lo..hi].iter().sum();
        let avg = sum / (hi - lo) as f32;
        // Map -80..0 dB → 0.0..1.0, then boost a bit
        let normalized = ((avg + 80.0) / 80.0).clamp(0.0, 1.0);
        // Apply sqrt to compress the range — makes quieter bars more visible
        bars.push(normalized.sqrt());
    }
    bars
}
