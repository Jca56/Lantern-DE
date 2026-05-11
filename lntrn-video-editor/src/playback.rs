//! Playback state machine — bridges the decoder thread and the render loop.
//!
//! Two coordinate spaces meet here:
//!   * **Source time** (`position`) — PTS within the currently-loaded media.
//!   * **Timeline time** (`timeline_position`) — position along the project
//!     timeline as drawn in the NLE. Mapped to source via the active clip's
//!     `start` and `media_in`.

use crate::decoder::{DecodeCmd, DecodedFrame, DecoderHandle};
use crate::project::{ClipId, Project, TrackKind};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayState {
    Empty,
    Playing,
    Paused,
}

pub struct Playback {
    pub state: PlayState,
    pub decoder: Option<DecoderHandle>,
    pub current_frame: Option<DecodedFrame>,
    /// PTS of the latest decoded frame, in the loaded media's source time.
    pub position: f64,
    /// Total duration of the loaded media file (source time).
    pub duration: f64,
    pub fps: f32,
    pub video_width: u32,
    pub video_height: u32,
    /// Set to true when a new frame was received this tick.
    pub frame_changed: bool,
    /// Current position along the project timeline. Driven by the active
    /// clip's `start + (source_pts - media_in)` mapping during playback, and
    /// set directly by scrub / seek operations.
    pub timeline_position: f64,
    /// Which timeline clip is currently driving playback. None when no clip
    /// is loaded (preview-only mode after a media-browser click) or after the
    /// active clip was deleted.
    pub active_clip_id: Option<ClipId>,
    /// Path of the media file the decoder currently has open. Used to skip
    /// reopening when a clip transition targets the same source file.
    pub active_media_path: Option<PathBuf>,
    /// Paused decoder handles parked by path. A cross-media clip transition
    /// pulls the target out of here (or opens fresh if not cached) and parks
    /// the outgoing decoder back in. Threads stay alive but block on the
    /// command channel, so the only ongoing cost is the FFmpeg context and
    /// the thread stack. No eviction yet — TODO for long sessions.
    pub decoder_cache: HashMap<PathBuf, DecoderHandle>,
}

impl Playback {
    pub fn new() -> Self {
        Self {
            state: PlayState::Empty,
            decoder: None,
            current_frame: None,
            position: 0.0,
            duration: 0.0,
            fps: 0.0,
            video_width: 0,
            video_height: 0,
            frame_changed: false,
            timeline_position: 0.0,
            active_clip_id: None,
            active_media_path: None,
            decoder_cache: HashMap::new(),
        }
    }

    /// Swap the active decoder so it's open on `path`. If `path` is already
    /// active, returns Ok immediately. If it's in the cache, takes it back.
    /// Otherwise opens a fresh decoder. The previously-active decoder is
    /// paused and parked in the cache for future reuse.
    fn swap_decoder(&mut self, path: &Path) -> Result<()> {
        if self.active_media_path.as_deref() == Some(path) {
            return Ok(());
        }
        // Park the outgoing decoder.
        if let (Some(old_path), Some(old_dec)) =
            (self.active_media_path.take(), self.decoder.take())
        {
            old_dec.send(DecodeCmd::Pause);
            // Best-effort drain of frames the decoder already queued before
            // it sees the Pause. Any race-window stragglers will be drained
            // again on next `take`.
            while old_dec.frame_rx.try_recv().is_ok() {}
            self.decoder_cache.insert(old_path, old_dec);
        }
        // Take cached or open fresh.
        let handle = match self.decoder_cache.remove(path) {
            Some(h) => {
                while h.frame_rx.try_recv().is_ok() {}
                h
            }
            None => DecoderHandle::open(path)?,
        };
        self.duration = handle.meta.duration;
        self.fps = handle.meta.fps;
        self.video_width = handle.meta.width;
        self.video_height = handle.meta.height;
        self.decoder = Some(handle);
        self.active_media_path = Some(path.to_path_buf());
        Ok(())
    }

    /// Open a video file and show the first frame (paused). Detaches from any
    /// active timeline clip — used by the preview-on-import / media-browser
    /// click flows where we just want to peek at a source file. Runs through
    /// the decoder cache so repeat-opens of the same path are instant.
    pub fn open_file(&mut self, path: &Path) -> Result<()> {
        self.swap_decoder(path)?;
        if let Some(dec) = &self.decoder {
            dec.send(DecodeCmd::Seek(0.0));
        }
        self.position = 0.0;
        self.state = PlayState::Paused;
        self.active_clip_id = None;
        Ok(())
    }

    pub fn play(&mut self) {
        if self.state == PlayState::Paused {
            if let Some(dec) = &self.decoder {
                dec.send(DecodeCmd::Play);
            }
            self.state = PlayState::Playing;
        }
    }

    pub fn pause(&mut self) {
        if self.state == PlayState::Playing {
            if let Some(dec) = &self.decoder {
                dec.send(DecodeCmd::Pause);
            }
            self.state = PlayState::Paused;
        }
    }

    /// Source-level play/pause toggle. Superseded by `timeline_toggle` for
    /// the normal play path, kept for any caller that wants to operate purely
    /// on the loaded media without consulting the timeline.
    #[allow(dead_code)]
    pub fn toggle(&mut self) {
        match self.state {
            PlayState::Playing => self.pause(),
            PlayState::Paused => self.play(),
            PlayState::Empty => {}
        }
    }

    /// Source-level seek (PTS within the loaded media). Superseded by
    /// `timeline_seek` for normal scrub; kept for callers that already know
    /// the source-time target.
    #[allow(dead_code)]
    pub fn seek(&mut self, secs: f64) {
        let secs = secs.clamp(0.0, self.duration);
        if let Some(dec) = &self.decoder {
            dec.send(DecodeCmd::Seek(secs));
        }
        self.position = secs;
    }

    /// Find the clip at (or following) `timeline_secs`, load its media if the
    /// path differs from what's loaded, and seek the source decoder to the
    /// matching in-point. Preserves play state across the swap.
    /// Returns true if a clip was activated.
    pub fn activate_clip_at(&mut self, timeline_secs: f64, project: &Project) -> bool {
        // Playback is driven by video clips — audio clips don't seek a video
        // decoder. Prefer a video clip that *contains* the time; fall back to
        // the next one after it (so seeks past the end land on the next clip).
        let is_video = |c: &&crate::project::TimelineClip| {
            project
                .track_by_id(c.track_id)
                .map(|t| t.kind == TrackKind::Video)
                .unwrap_or(false)
        };
        let target = project
            .timeline_clips
            .iter()
            .filter(is_video)
            .find(|c| timeline_secs >= c.start && timeline_secs < c.start + c.duration)
            .or_else(|| {
                project
                    .timeline_clips
                    .iter()
                    .filter(is_video)
                    .filter(|c| c.start >= timeline_secs - 1e-6)
                    .min_by(|a, b| {
                        a.start
                            .partial_cmp(&b.start)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
        let Some(clip) = target else {
            return false;
        };
        let clip_id = clip.id;
        let clip_start = clip.start;
        let clip_duration = clip.duration;
        let clip_media_in = clip.media_in;
        let clip_speed = clip.speed.max(0.05) as f64;
        let media_id = clip.media_id;

        let path = match project.media_by_id(media_id) {
            Some(m) => m.path.clone(),
            None => return false,
        };

        let was_playing = self.state == PlayState::Playing;
        let needs_open = self.active_media_path.as_deref() != Some(path.as_path());
        if needs_open {
            if let Err(e) = self.swap_decoder(&path) {
                eprintln!("[playback] failed to open clip media {}: {e}", path.display());
                return false;
            }
            // The new decoder comes out of swap_decoder in whatever state it
            // was parked at. We override state below; the Seek that follows
            // moves it to the right source offset.
            self.state = PlayState::Paused;
            self.active_clip_id = None;
        }

        let source_offset =
            if timeline_secs >= clip_start && timeline_secs < clip_start + clip_duration {
                clip_media_in + (timeline_secs - clip_start) * clip_speed
            } else {
                clip_media_in
            };
        if let Some(dec) = &self.decoder {
            dec.send(DecodeCmd::Seek(source_offset));
        }
        self.position = source_offset;
        self.timeline_position = if timeline_secs >= clip_start {
            timeline_secs.max(0.0)
        } else {
            clip_start
        };
        self.active_clip_id = Some(clip_id);

        if was_playing {
            if let Some(dec) = &self.decoder {
                dec.send(DecodeCmd::Play);
            }
            self.state = PlayState::Playing;
        }
        true
    }

    /// Scrub destination: set timeline position and update the source decoder
    /// to match. Play state is preserved.
    pub fn timeline_seek(&mut self, secs: f64, project: &Project) {
        let max = project.timeline_duration().max(0.0);
        let clamped = secs.clamp(0.0, max);
        if !self.activate_clip_at(clamped, project) {
            // No clip available — park the playhead but don't disturb decoder.
            self.timeline_position = clamped;
        }
    }

    /// Toggle play/pause in timeline mode. If no clip is active, picks the
    /// clip containing (or following) the current `timeline_position`.
    pub fn timeline_toggle(&mut self, project: &Project) {
        match self.state {
            PlayState::Playing => self.pause(),
            PlayState::Paused | PlayState::Empty => {
                let needs_activate = self
                    .active_clip_id
                    .map(|id| !project.timeline_clips.iter().any(|c| c.id == id))
                    .unwrap_or(true);
                if needs_activate
                    && !self.activate_clip_at(self.timeline_position, project)
                {
                    return; // empty timeline — nothing to play
                }
                self.play();
            }
        }
    }

    /// Per-frame tick. Drains decoder frames and updates `timeline_position`
    /// based on the active clip's source-to-timeline mapping. When the
    /// decoder crosses the active clip's source-out point, jump to the next
    /// clip on the timeline (or pause at the end).
    pub fn tick(&mut self, project: &Project) {
        self.poll_frame();

        let Some(clip_id) = self.active_clip_id else {
            return;
        };
        let clip = project.timeline_clips.iter().find(|c| c.id == clip_id);
        let (clip_start, clip_duration, clip_media_in, clip_speed) = match clip {
            Some(c) => (c.start, c.duration, c.media_in, c.speed.max(0.05) as f64),
            None => {
                // Active clip was deleted out from under us.
                self.pause();
                self.active_clip_id = None;
                return;
            }
        };

        // Map source PTS → timeline time inside the active clip. With clip
        // speed > 1, source runs faster than wall-clock, so timeline_position
        // advances slower per source second. Clamp on both ends so a running
        // trim past the playhead doesn't produce nonsense for one frame.
        let source_offset_in_clip = (self.position - clip_media_in)
            .clamp(0.0, (clip_duration * clip_speed).max(0.0));
        self.timeline_position = clip_start + source_offset_in_clip / clip_speed;

        // Boundary crossing: when source PTS reaches the clip's effective
        // out-point, jump to the next video clip on the timeline. Audio
        // clips are not considered as playback targets.
        let source_out = clip_media_in + clip_duration * clip_speed;
        if self.position >= source_out - 1e-3 {
            let cur_end = clip_start + clip_duration;
            let next_start = project
                .timeline_clips
                .iter()
                .filter(|c| {
                    c.id != clip_id
                        && c.start >= cur_end - 1e-3
                        && project
                            .track_by_id(c.track_id)
                            .map(|t| t.kind == TrackKind::Video)
                            .unwrap_or(false)
                })
                .map(|c| c.start)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            match next_start {
                Some(next) => {
                    self.activate_clip_at(next, project);
                }
                None => {
                    self.timeline_position = cur_end;
                    self.pause();
                }
            }
        }
    }

    /// Poll for a new decoded frame (non-blocking).
    /// Returns true if a new frame arrived.
    pub fn poll_frame(&mut self) -> bool {
        self.frame_changed = false;
        let dec = match &self.decoder {
            Some(d) => d,
            None => return false,
        };

        // Drain to latest frame (skip stale frames)
        let mut got_frame = false;
        while let Ok(frame) = dec.frame_rx.try_recv() {
            self.position = frame.pts;
            self.current_frame = Some(frame);
            got_frame = true;
        }
        self.frame_changed = got_frame;
        got_frame
    }

    pub fn is_playing(&self) -> bool {
        self.state == PlayState::Playing
    }

    pub fn has_media(&self) -> bool {
        self.state != PlayState::Empty
    }
}

/// Format seconds as "HH:MM:SS:FF" timecode.
pub fn format_timecode(secs: f64, fps: f32) -> String {
    let total_secs = secs.max(0.0);
    let h = (total_secs / 3600.0) as u32;
    let m = ((total_secs % 3600.0) / 60.0) as u32;
    let s = (total_secs % 60.0) as u32;
    let f = ((total_secs % 1.0) * fps as f64) as u32;
    format!("{h:02}:{m:02}:{s:02}:{f:02}")
}
