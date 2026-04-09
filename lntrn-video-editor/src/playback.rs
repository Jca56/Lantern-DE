//! Playback state machine — bridges the decoder thread and the render loop.

use std::path::Path;
use anyhow::Result;
use crate::decoder::{DecodeCmd, DecodedFrame, DecoderHandle};

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
    pub position: f64,
    pub duration: f64,
    pub fps: f32,
    pub video_width: u32,
    pub video_height: u32,
    /// Set to true when a new frame was received this tick.
    pub frame_changed: bool,
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
        }
    }

    /// Open a video file and show the first frame (paused).
    pub fn open_file(&mut self, path: &Path) -> Result<()> {
        let handle = DecoderHandle::open(path)?;
        self.duration = handle.meta.duration;
        self.fps = handle.meta.fps;
        self.video_width = handle.meta.width;
        self.video_height = handle.meta.height;
        self.position = 0.0;

        // Seek to start to get the first frame
        handle.send(DecodeCmd::Seek(0.0));
        self.decoder = Some(handle);
        self.state = PlayState::Paused;
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

    pub fn toggle(&mut self) {
        match self.state {
            PlayState::Playing => self.pause(),
            PlayState::Paused => self.play(),
            PlayState::Empty => {}
        }
    }

    pub fn seek(&mut self, secs: f64) {
        let secs = secs.clamp(0.0, self.duration);
        if let Some(dec) = &self.decoder {
            dec.send(DecodeCmd::Seek(secs));
        }
        self.position = secs;
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
