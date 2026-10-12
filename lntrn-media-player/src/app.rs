use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

mod playlist;
mod transport;

pub use transport::LoopMode;

use crate::event_pump::Waker;
use crate::pipeline::MediaPipeline;
use crate::position_store;
use crate::visualizer::Visualizer;
use playlist::{resolve, store_key};

/// Resume positions in the first/last `RESUME_MARGIN_NS` of a file are dropped:
/// the start isn't worth resuming, and the tail means the file basically
/// finished — both should reopen from the beginning instead.
const RESUME_MARGIN_NS: u64 = 5_000_000_000; // 5 seconds
/// Position drives the seek bar + timestamps; ~10 Hz is plenty for the UI and
/// avoids hammering the pipeline (each query can briefly grab its stream lock).
const QUERY_INTERVAL: Duration = Duration::from_millis(100);
/// How long a fresh pipeline is asked for its duration before we stop
/// waking up for it (a stream that never reports one would poll forever).
const DURATION_PATIENCE: Duration = Duration::from_secs(10);
/// After a seek the pipeline can still report the old position for a moment;
/// the position we asked for is shown until it has had time to land.
const SEEK_SETTLE: Duration = Duration::from_millis(300);
/// Controls stay up this long after the pointer last moved.
const CONTROLS_GRACE: Duration = Duration::from_millis(1500);
/// The volume readout: fully visible this long, then fading out.
const OSD_HOLD: Duration = Duration::from_millis(1200);
const OSD_FADE: Duration = Duration::from_millis(300);

pub struct App {
    pub pipeline: Option<MediaPipeline>,
    pub video_texture: Option<GpuTexture>,
    pub video_width: u32,
    pub video_height: u32,
    pub file_path: Option<PathBuf>,
    pub file_name: String,
    /// Proper (escaped) `file://` URI of the open file, for MPRIS.
    pub file_uri: String,
    pub volume: f64,
    pub position_ns: u64,
    pub duration_ns: u64,
    pub seeking: bool,
    pub seek_value: f32,
    /// Why nothing is playing ("No media loaded", an error). Drawn in the
    /// middle of the window whenever there is no pipeline.
    pub status_text: String,
    pub audio_only: bool,
    pub visualizer: Visualizer,
    // Playlist
    pub loop_mode: LoopMode,
    pub playlist: Vec<PathBuf>,
    pub playlist_index: usize,
    /// The track played to its end and stopped there (play-once, or the last
    /// file of the folder). Play starts it over.
    pub ended: bool,
    // Hover-reveal controls (0.0 = hidden, 1.0 = fully shown)
    pub controls_alpha: f32,
    pub controls_last_move: Instant,
    pub pointer_in_window: bool,
    // Visualizer theme (index into vis_theme::VIS_THEMES) + View dropdown state.
    pub vis_theme_index: usize,
    pub view_menu_open: bool,
    /// When the volume last changed: drives the on-screen readout.
    volume_changed_at: Option<Instant>,
    last_query: Instant,
    opened_at: Instant,
    /// Position reports are ignored until then (a seek is landing).
    position_hold_until: Instant,
    /// A seek queued at open time (resume where you left off), applied once
    /// the pipeline knows its duration — which means it's seekable.
    pending_resume_ns: Option<u64>,
    waker: Waker,
}

impl App {
    pub fn new(waker: Waker) -> Self {
        let now = Instant::now();
        Self {
            pipeline: None,
            video_texture: None,
            video_width: 0,
            video_height: 0,
            file_path: None,
            file_name: String::new(),
            file_uri: String::new(),
            volume: 1.0,
            position_ns: 0,
            duration_ns: 0,
            seeking: false,
            seek_value: 0.0,
            status_text: "No media loaded".into(),
            audio_only: false,
            visualizer: Visualizer::new(),
            loop_mode: LoopMode::Continue,
            playlist: Vec::new(),
            playlist_index: 0,
            ended: false,
            controls_alpha: 0.0,
            controls_last_move: now,
            pointer_in_window: false,
            vis_theme_index: crate::vis_theme::load_theme_index(),
            view_menu_open: false,
            volume_changed_at: None,
            last_query: now,
            opened_at: now,
            position_hold_until: now,
            pending_resume_ns: None,
            waker,
        }
    }

    /// Pick a visualizer color theme and persist it.
    pub fn set_vis_theme(&mut self, index: usize) {
        self.vis_theme_index = index.min(crate::vis_theme::theme_count() - 1);
        crate::vis_theme::save_theme_index(self.vis_theme_index);
    }

    // ── Opening files ───────────────────────────────────────────────────

    pub fn open_file(&mut self, path: &Path) {
        let Some(abs) = resolve(path) else {
            self.status_text = format!("File not found: {}", path.display());
            return;
        };
        self.open_file_internal(&abs);
        (self.playlist, self.playlist_index) = playlist::scan(&abs);
    }

    fn open_file_internal(&mut self, abs: &Path) {
        // Remember where we were in the outgoing file before swapping pipelines
        // (covers next/prev and folder auto-advance, not just app close).
        self.save_current_position();
        // The old pipeline comes down before the new one starts, so two
        // tracks never sound at once.
        self.pipeline = None;

        let now = Instant::now();
        self.file_name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into())
            .unwrap_or_default();
        self.file_path = Some(abs.to_path_buf());
        self.file_uri = gstreamer::glib::filename_to_uri(abs, None)
            .map(|uri| uri.to_string())
            .unwrap_or_default();
        self.video_texture = None;
        self.video_width = 0;
        self.video_height = 0;
        self.position_ns = 0;
        self.duration_ns = 0;
        self.seeking = false;
        self.audio_only = false;
        self.ended = false;
        self.visualizer.reset();
        self.opened_at = now;
        self.last_query = now;
        self.position_hold_until = now;
        self.pending_resume_ns = None;

        match MediaPipeline::new(abs, self.waker.clone()) {
            Ok(mut pipe) => {
                pipe.set_volume(self.volume);
                pipe.play();
                self.pipeline = Some(pipe);
                self.status_text.clear();
                // Queue a resume to the remembered spot. Can't seek yet — the
                // pipeline hasn't prerolled — so `tick` applies it once a
                // duration is known. None if we've never seen this file.
                self.pending_resume_ns = position_store::get(&store_key(abs));
            }
            Err(e) => self.status_text = format!("Can't play {}: {e}", self.file_name),
        }
    }

    // ── Timers and pipeline upkeep ──────────────────────────────────────

    /// Pipeline upkeep for one wake-up: bus messages, position/duration,
    /// the queued resume seek, end-of-stream and errors. Returns true when
    /// something the window shows has changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self
            .volume_changed_at
            .is_some_and(|at| now >= at + OSD_HOLD + OSD_FADE)
        {
            self.volume_changed_at = None;
            changed = true;
        }
        let Some(pipe) = &mut self.pipeline else {
            return changed;
        };
        let fresh_spectrum = pipe.poll_bus();

        if now.duration_since(self.last_query) >= QUERY_INTERVAL {
            self.last_query = now;
            // Not while a seek is landing: the old position would yank the
            // seek bar back for a moment.
            if now >= self.position_hold_until && !pipe.seek_busy() {
                if let Some(pos) = pipe.position().filter(|&p| p != self.position_ns) {
                    self.position_ns = pos;
                    changed |= self.controls_alpha > 0.0;
                }
            }
            if self.duration_ns == 0 {
                if let Some(dur) = pipe.duration().filter(|&d| d > 0) {
                    self.duration_ns = dur;
                    changed = true;
                }
            }
            // Clamp the resume away from the very end and run it exactly once.
            if let Some(target) = self.pending_resume_ns.filter(|_| self.duration_ns > 0) {
                let clamped = target.min(self.duration_ns.saturating_sub(RESUME_MARGIN_NS));
                pipe.seek(clamped);
                self.position_ns = clamped;
                self.position_hold_until = now + SEEK_SETTLE;
                self.pending_resume_ns = None;
                changed = true;
            }
        }

        // Detect audio-only — only trust n-video after duration is known.
        if !self.audio_only && self.duration_ns > 0 && pipe.is_audio_only() {
            self.audio_only = true;
            changed = true;
        }
        if fresh_spectrum && self.audio_only && pipe.is_playing() {
            self.visualizer.set_spectrum(Some(pipe.spectrum()));
            changed = true;
        }
        changed |= pipe.has_frame();
        changed |= self.check_error();
        changed |= self.check_eos();
        changed
    }

    /// The next moment `tick` has something to do without being woken for it.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let mut due: Vec<Instant> = Vec::new();
        if let Some(pipe) = &self.pipeline {
            let wants_duration =
                self.duration_ns == 0 && now.duration_since(self.opened_at) < DURATION_PATIENCE;
            if pipe.is_playing() || wants_duration {
                due.push(self.last_query + QUERY_INTERVAL);
            }
            due.extend(pipe.preroll_deadline());
            due.extend(pipe.seek_deadline());
        }
        // The moments an animation starts on its own (a deadline already
        // past would spin the loop; `is_animating` covers those from there).
        let grace_end = self.controls_last_move + CONTROLS_GRACE;
        due.extend(Some(grace_end).filter(|&at| at > now && self.controls_alpha > 0.0));
        if let Some(at) = self.volume_changed_at {
            due.extend(Some(at + OSD_HOLD).filter(|&at| at > now));
        }
        due.into_iter().min()
    }

    /// Something is easing toward a new state and wants a frame every
    /// refresh until it gets there.
    pub fn is_animating(&self, now: Instant) -> bool {
        let controls = self.controls_alpha != self.controls_target(now);
        let bars = !self.video_mode() && !self.visualizer.is_settled();
        let osd = self.volume_changed_at.is_some_and(|at| now >= at + OSD_HOLD);
        controls || bars || osd
    }

    /// Bring everything that changes over time up to `now` and upload the
    /// newest video frame. Called right before drawing, never otherwise: a
    /// hidden window does no per-frame work.
    pub fn prepare_frame(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, now: Instant, dt: f32) {
        if let Some(pipe) = &self.pipeline {
            if let Some((w, h)) = pipe.upload_frame(gpu, tex_pass, &mut self.video_texture) {
                self.audio_only = false;
                self.video_width = w;
                self.video_height = h;
            }
        }

        // Fade rate: ~180ms in, ~250ms out
        let target = self.controls_target(now);
        let rate = if target > self.controls_alpha {
            1.0 / 0.18
        } else {
            1.0 / 0.25
        };
        let step = rate * dt;
        if (target - self.controls_alpha).abs() <= step {
            self.controls_alpha = target;
        } else if target > self.controls_alpha {
            self.controls_alpha += step;
        } else {
            self.controls_alpha -= step;
        }

        // The bars follow the music only while it plays; paused, ended or
        // with nothing loaded they sink back to rest.
        if !(self.audio_only && self.is_playing()) {
            self.visualizer.set_spectrum(None);
        }
        self.visualizer.step(dt);
    }

    /// Controls stay up while the pointer is in the window, briefly after it
    /// moves, and whenever playback is paused (so you can always see the seek
    /// bar and hit play again without fishing for the controls).
    fn controls_target(&self, now: Instant) -> f32 {
        let shown = self.pointer_in_window
            || !self.is_playing()
            || self.view_menu_open
            || now < self.controls_last_move + CONTROLS_GRACE;
        if shown {
            1.0
        } else {
            0.0
        }
    }

    /// Showing video rather than the visualizer.
    pub fn video_mode(&self) -> bool {
        !self.audio_only && self.pipeline.is_some()
    }

    /// Opacity of the volume readout right now (0 = not showing).
    pub fn volume_osd_alpha(&self, now: Instant) -> f32 {
        let Some(at) = self.volume_changed_at else {
            return 0.0;
        };
        let age = now.duration_since(at);
        if age <= OSD_HOLD {
            1.0
        } else {
            let fade = (age - OSD_HOLD).as_secs_f32() / OSD_FADE.as_secs_f32();
            (1.0 - fade).clamp(0.0, 1.0)
        }
    }

    pub fn progress_fraction(&self) -> f32 {
        if self.duration_ns == 0 {
            0.0
        } else {
            (self.position_ns as f64 / self.duration_ns as f64).min(1.0) as f32
        }
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
