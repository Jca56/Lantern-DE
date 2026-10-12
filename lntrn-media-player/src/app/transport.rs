//! What playback does next: the end of a track, play / pause / seek,
//! track changes, volume, and saving the resume position.

use std::time::Instant;

use super::playlist::store_key;
use super::{App, RESUME_MARGIN_NS, SEEK_SETTLE};
use crate::position_store;

/// What happens when the current track reaches the end.
#[derive(Clone, Copy, PartialEq)]
pub enum LoopMode {
    /// Stop after the current track finishes ("play once").
    Once,
    /// Repeat the current track forever ("loop").
    RepeatOne,
    /// Advance to the next file in the folder ("continue").
    Continue,
}

impl LoopMode {
    /// Cycle order: Continue → RepeatOne → Once → Continue.
    pub fn next(self) -> Self {
        match self {
            LoopMode::Continue => LoopMode::RepeatOne,
            LoopMode::RepeatOne => LoopMode::Once,
            LoopMode::Once => LoopMode::Continue,
        }
    }
}

impl App {
    // ── End of stream / errors ──────────────────────────────────────────

    /// Surface a fatal pipeline error — a decoder failing, or the audio sink
    /// losing the PipeWire daemon under it. The pipeline is dropped (bounded,
    /// see `MediaPipeline`'s Drop) so nothing else in the UI can block on it.
    /// Returns true when an error was consumed.
    pub(super) fn check_error(&mut self) -> bool {
        let stalled = self.pipeline.as_ref().is_some_and(|p| p.preroll_stalled());
        let msg = match self.pipeline.as_mut().and_then(|p| p.take_error()) {
            Some(m) => m,
            None if stalled => {
                let secs = crate::pipeline::PREROLL_TIMEOUT.as_secs();
                eprintln!(
                    "[media-player] gst: no preroll after {secs}s — the audio sink never came up; dropping this pipeline"
                );
                format!("Audio device did not respond ({secs}s)")
            }
            None => return false,
        };
        self.status_text = format!("Can't play {}: {msg}", self.file_name);
        self.pipeline = None;
        self.video_texture = None;
        self.seeking = false;
        self.pending_resume_ns = None;
        true
    }

    /// Act on end-of-stream. Returns true when it was reached.
    pub(super) fn check_eos(&mut self) -> bool {
        match &mut self.pipeline {
            Some(p) if p.is_eos() => p.clear_eos(),
            _ => return false,
        }
        match self.loop_mode {
            LoopMode::RepeatOne => self.restart(),
            LoopMode::Once => self.stop_at_end(),
            LoopMode::Continue => {
                // Past the last file there is nothing to continue to.
                if !self.next_track() {
                    self.stop_at_end();
                }
            }
        }
        true
    }

    /// Park the finished track paused on its last frame — really paused, so
    /// the UI and MPRIS don't keep claiming it's playing.
    fn stop_at_end(&mut self) {
        if let Some(pipe) = &self.pipeline {
            pipe.pause();
        }
        self.ended = true;
    }

    /// Play the current track again from the top.
    fn restart(&mut self) {
        self.seek_to(0);
        if let Some(pipe) = &self.pipeline {
            pipe.play();
        }
    }

    // ── Transport ───────────────────────────────────────────────────────

    pub fn next_track(&mut self) -> bool {
        if self.playlist_index + 1 >= self.playlist.len() {
            return false;
        }
        self.playlist_index += 1;
        let path = self.playlist[self.playlist_index].clone();
        self.open_file_internal(&path);
        true
    }

    pub fn prev_track(&mut self) -> bool {
        // More than 3 seconds in, Previous restarts the current track (the
        // only thing it can do when the folder holds a single file).
        if self.pipeline.is_some() && self.position_ns > 3_000_000_000 {
            self.seek_to(0);
            return true;
        }
        if self.playlist_index == 0 || self.playlist.len() <= 1 {
            return false;
        }
        self.playlist_index -= 1;
        let path = self.playlist[self.playlist_index].clone();
        self.open_file_internal(&path);
        true
    }

    pub fn can_go_next(&self) -> bool {
        self.playlist_index + 1 < self.playlist.len()
    }

    pub fn can_go_previous(&self) -> bool {
        self.pipeline.is_some()
    }

    pub fn cycle_loop_mode(&mut self) {
        self.loop_mode = self.loop_mode.next();
    }

    pub fn note_pointer_activity(&mut self) {
        self.controls_last_move = Instant::now();
    }

    pub fn is_playing(&self) -> bool {
        self.pipeline.as_ref().is_some_and(|p| p.is_playing())
    }

    pub fn play(&mut self) {
        if self.ended {
            self.restart();
        } else if let Some(pipe) = &self.pipeline {
            pipe.play();
        }
    }

    pub fn pause(&mut self) {
        if let Some(pipe) = &self.pipeline {
            pipe.pause();
        }
    }

    pub fn toggle_play_pause(&mut self) {
        if self.ended {
            self.restart();
        } else if let Some(pipe) = &self.pipeline {
            pipe.toggle();
        }
    }

    /// Seek and show the new position straight away: the pipeline takes a
    /// moment to report it, and the next seek has to build on this one (two
    /// quick taps of → are ten seconds, not five).
    fn seek_to(&mut self, target_ns: u64) {
        let Some(pipe) = &mut self.pipeline else { return };
        pipe.seek(target_ns);
        self.position_ns = target_ns;
        self.position_hold_until = Instant::now() + SEEK_SETTLE;
        self.ended = false;
    }

    pub fn seek_relative(&mut self, delta_ns: i64) {
        let pos = (self.position_ns as i64).saturating_add(delta_ns).max(0) as u64;
        let target = if self.duration_ns > 0 {
            pos.min(self.duration_ns)
        } else {
            pos
        };
        self.seek_to(target);
    }

    pub fn seek_to_fraction(&mut self, frac: f32) {
        self.seeking = false;
        // No duration yet means no idea where `frac` is: don't jump to 0.
        if self.duration_ns > 0 {
            self.seek_to((frac.clamp(0.0, 1.0) as f64 * self.duration_ns as f64) as u64);
        }
    }

    pub fn adjust_volume(&mut self, delta: f64) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        if let Some(pipe) = &mut self.pipeline {
            pipe.set_volume(self.volume);
        }
        self.volume_changed_at = Some(Instant::now());
    }

    /// Persist the current playback position for the open file so reopening it
    /// later resumes from here. Positions in the first or last `RESUME_MARGIN_NS`
    /// are cleared instead — too trivial to resume, or the file effectively
    /// finished. Safe to call any time (no-op when nothing is open).
    pub fn save_current_position(&mut self) {
        let Some(path) = &self.file_path else { return };
        let (pos, dur) = (self.position_ns, self.duration_ns);
        let resumable =
            pos > RESUME_MARGIN_NS && (dur == 0 || pos < dur.saturating_sub(RESUME_MARGIN_NS));
        position_store::set(&store_key(path), resumable.then_some(pos));
    }
}
