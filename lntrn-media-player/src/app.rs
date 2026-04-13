use std::path::{Path, PathBuf};
use std::time::Instant;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use crate::pipeline::MediaPipeline;

pub const VIS_BARS: usize = 20;

const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "flv", "wmv", "m4v",
    "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus", "wma",
];

#[derive(Clone, Copy, PartialEq)]
pub enum VisMode {
    ConcentricRings,
    ClassicBars,
}

impl VisMode {
    pub fn next(self) -> Self {
        match self {
            VisMode::ConcentricRings => VisMode::ClassicBars,
            VisMode::ClassicBars => VisMode::ConcentricRings,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum LoopMode {
    Off,
    LoopOne,
    LoopAll,
}

impl LoopMode {
    pub fn next(self) -> Self {
        match self {
            LoopMode::Off => LoopMode::LoopOne,
            LoopMode::LoopOne => LoopMode::LoopAll,
            LoopMode::LoopAll => LoopMode::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LoopMode::Off => "\u{27F3}",
            LoopMode::LoopOne => "\u{27F3}1",
            LoopMode::LoopAll => "\u{27F3}A",
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
    // Playlist
    pub loop_mode: LoopMode,
    pub playlist: Vec<PathBuf>,
    pub playlist_index: usize,
    // Auto-hide controls
    pub controls_visible: bool,
    pub controls_last_move: Instant,
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
            loop_mode: LoopMode::Off,
            playlist: Vec::new(),
            playlist_index: 0,
            controls_visible: true,
            controls_last_move: Instant::now(),
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

        self.open_file_internal(&abs.clone());
        self.scan_directory(&abs);
    }

    fn open_file_internal(&mut self, abs: &Path) {
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
                self.file_path = Some(abs.to_path_buf());
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

    fn scan_directory(&mut self, file_path: &Path) {
        let dir = match file_path.parent() {
            Some(d) => d,
            None => return,
        };
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| MEDIA_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        self.playlist_index = files.iter().position(|p| p == file_path).unwrap_or(0);
        self.playlist = files;
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
        if !self.audio_only && self.duration_ns > 0 && pipe.is_audio_only() {
            self.audio_only = true;
        }

        // Poll bus for spectrum messages + EOS + log-scale into visual bars
        pipe.poll_spectrum();
        let raw = pipe.spectrum();
        let log_bars = log_group_spectrum(raw, VIS_BARS);
        for i in 0..VIS_BARS {
            let target = log_bars[i];
            let current = self.vis_bars[i];
            if target > current {
                self.vis_bars[i] = current + (target - current) * 0.7;
            } else {
                self.vis_bars[i] = current + (target - current) * 0.25;
            }
        }

        // Grab latest video frame
        if let Some(frame) = pipe.take_frame() {
            self.audio_only = false;
            self.video_width = frame.width;
            self.video_height = frame.height;
            self.video_texture = Some(tex_pass.upload(gpu, &frame.rgba, frame.width, frame.height));
            return true;
        }

        self.audio_only
    }

    /// Check for end-of-stream. Returns true if a new track was loaded.
    pub fn check_eos(&mut self) -> bool {
        let is_eos = self.pipeline.as_ref().map(|p| p.is_eos()).unwrap_or(false);
        if !is_eos { return false; }
        self.handle_eos()
    }

    fn handle_eos(&mut self) -> bool {
        match self.loop_mode {
            LoopMode::LoopOne => {
                if let Some(pipe) = &self.pipeline {
                    pipe.seek(0);
                    pipe.play();
                }
                true
            }
            LoopMode::Off | LoopMode::LoopAll => {
                self.next_track()
            }
        }
    }

    pub fn next_track(&mut self) -> bool {
        if self.playlist.len() <= 1 { return false; }
        if self.playlist_index + 1 < self.playlist.len() {
            self.playlist_index += 1;
        } else if self.loop_mode == LoopMode::LoopAll {
            self.playlist_index = 0;
        } else {
            return false;
        }
        let path = self.playlist[self.playlist_index].clone();
        self.open_file_internal(&path);
        true
    }

    pub fn prev_track(&mut self) -> bool {
        if self.playlist.len() <= 1 { return false; }
        // If we're past 3 seconds, restart current track instead
        if self.position_ns > 3_000_000_000 {
            if let Some(pipe) = &self.pipeline {
                pipe.seek(0);
            }
            return true;
        }
        if self.playlist_index > 0 {
            self.playlist_index -= 1;
        } else if self.loop_mode == LoopMode::LoopAll {
            self.playlist_index = self.playlist.len() - 1;
        } else {
            return false;
        }
        let path = self.playlist[self.playlist_index].clone();
        self.open_file_internal(&path);
        true
    }

    pub fn cycle_loop_mode(&mut self) {
        self.loop_mode = self.loop_mode.next();
    }

    pub fn reset_controls_timer(&mut self) {
        self.controls_visible = true;
        self.controls_last_move = Instant::now();
    }

    pub fn update_controls_visibility(&mut self) {
        if self.controls_visible && self.controls_last_move.elapsed().as_secs_f32() > 2.0 {
            if self.is_playing() && !self.audio_only {
                self.controls_visible = false;
            }
        }
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
        if self.duration_ns == 0 { 0.0 }
        else { (self.position_ns as f64 / self.duration_ns as f64) as f32 }
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
fn log_group_spectrum(raw: &[f32], num_bars: usize) -> Vec<f32> {
    let n = raw.len();
    if n == 0 {
        return vec![0.0; num_bars];
    }

    let mut bars = Vec::with_capacity(num_bars);
    for i in 0..num_bars {
        let lo = ((i as f64 / num_bars as f64).powf(2.0) * n as f64) as usize;
        let hi = (((i + 1) as f64 / num_bars as f64).powf(2.0) * n as f64) as usize;
        let lo = lo.min(n);
        let hi = hi.max(lo + 1).min(n);

        let sum: f32 = raw[lo..hi].iter().sum();
        let avg = sum / (hi - lo) as f32;
        let freq_t = i as f64 / num_bars as f64;
        let treble_boost = (freq_t * 25.0) as f32;
        let normalized = ((avg + treble_boost + 40.0) / 40.0).clamp(0.0, 1.0);
        bars.push(normalized.powf(0.8));
    }
    bars
}
