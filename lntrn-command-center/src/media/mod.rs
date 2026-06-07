//! Now-playing media state — the Command Center's centerpiece.
//!
//! A background worker speaks MPRIS over the session bus (find the active
//! `org.mpris.MediaPlayer2.*` player, read its metadata + playback state,
//! and forward transport commands). This module caches the latest track
//! for the render loop and exposes a tiny command API.
//!
//! See [`worker`] for the D-Bus side and [`render`] for the widget.

pub mod render;
mod worker;

use std::sync::mpsc;
use std::thread;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

/// A snapshot of whatever player is currently active.
#[derive(Debug, Clone)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub status: PlaybackStatus,
    /// Playback position at sample time, microseconds.
    pub position_us: i64,
    /// Track length, microseconds (0 if the player doesn't report it).
    pub length_us: i64,
}

/// Transport buttons the user can click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaButton {
    Previous,
    PlayPause,
    Next,
}

/// Render thread → worker commands.
pub(crate) enum MediaCmd {
    PlayPause,
    Next,
    Previous,
}

/// Worker → render thread events.
pub(crate) enum MediaEvent {
    /// Latest player snapshot, or `None` when no player is active.
    Update(Option<Track>),
}

pub struct Media {
    cmd_tx: mpsc::Sender<MediaCmd>,
    event_rx: mpsc::Receiver<MediaEvent>,
    pub track: Option<Track>,
    /// When the current `track` snapshot was taken — used to interpolate
    /// the playback position so the progress bar advances smoothly
    /// between worker polls.
    sampled_at: Option<Instant>,
    /// Transport button currently under the cursor (for hover highlight).
    /// Set by the hover pump each frame.
    pub hover: Option<MediaButton>,
}

impl Media {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::Builder::new()
            .name("lcc-media-mpris".into())
            .spawn(move || worker::run(event_tx, cmd_rx))
            .ok();
        Self {
            cmd_tx,
            event_rx,
            track: None,
            sampled_at: None,
            hover: None,
        }
    }

    /// Drain worker updates. Cheap; safe to call every loop iteration.
    pub fn tick(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                MediaEvent::Update(track) => {
                    self.track = track;
                    self.sampled_at = Some(Instant::now());
                }
            }
        }
    }

    /// Send a transport command to whatever player the worker last saw.
    pub fn command(&self, btn: MediaButton) {
        let cmd = match btn {
            MediaButton::Previous => MediaCmd::Previous,
            MediaButton::PlayPause => MediaCmd::PlayPause,
            MediaButton::Next => MediaCmd::Next,
        };
        let _ = self.cmd_tx.send(cmd);
    }

    /// Interpolated playback position (microseconds), clamped to length.
    /// Advances with wall-clock while playing so the bar doesn't tick in
    /// visible jumps at the worker's poll rate.
    pub fn effective_position_us(&self) -> i64 {
        let Some(t) = &self.track else {
            return 0;
        };
        let base = t.position_us.max(0);
        let pos = match (t.status, self.sampled_at) {
            (PlaybackStatus::Playing, Some(at)) => base + at.elapsed().as_micros() as i64,
            _ => base,
        };
        if t.length_us > 0 {
            pos.min(t.length_us)
        } else {
            pos
        }
    }
}

impl Default for Media {
    fn default() -> Self {
        Self::new()
    }
}
