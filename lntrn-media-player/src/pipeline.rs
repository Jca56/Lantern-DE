use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use gstreamer::prelude::*;
use gstreamer::{self as gst, ClockTime, Element, SeekFlags, State as GstState};
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

// ── Video frame ─────────────────────────────────────────────────────────────

pub struct VideoFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

// ── Spectrum data ───────────────────────────────────────────────────────────

pub const SPECTRUM_BANDS: usize = 64;

// ── Pipeline ────────────────────────────────────────────────────────────────

pub struct MediaPipeline {
    pipeline: Element,
    frame: Arc<Mutex<Option<VideoFrame>>>,
    spectrum: Vec<f32>,
}

impl MediaPipeline {
    pub fn new(uri: &str) -> Result<Self> {
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri)
            .build()
            .map_err(|e| anyhow!("Failed to create playbin: {e}"))?;

        // ── Video appsink ──────────────────────────────────────────────
        let appsink = gst_app::AppSink::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::Rgba)
                    .build(),
            )
            .max_buffers(1)
            .drop(true)
            .build();

        pipeline.set_property("video-sink", &appsink);

        let frame: Arc<Mutex<Option<VideoFrame>>> = Arc::new(Mutex::new(None));
        let frame_ref = frame.clone();

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    // Skip the expensive copy if we already have an unconsumed frame
                    if let Ok(lock) = frame_ref.lock() {
                        if lock.is_some() {
                            return Ok(gst::FlowSuccess::Ok);
                        }
                    }

                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                    let info = gst_video::VideoInfo::from_caps(caps)
                        .map_err(|_| gst::FlowError::Error)?;

                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let width = info.width();
                    let height = info.height();

                    let stride = info.stride()[0] as usize;
                    let row_bytes = (width as usize) * 4;
                    let rgba = if stride == row_bytes {
                        map.as_slice().to_vec()
                    } else {
                        let mut rgba = Vec::with_capacity(row_bytes * height as usize);
                        for row in 0..height as usize {
                            let start = row * stride;
                            let end = start + row_bytes;
                            if end <= map.len() {
                                rgba.extend_from_slice(&map[start..end]);
                            }
                        }
                        rgba
                    };

                    if let Ok(mut lock) = frame_ref.lock() {
                        *lock = Some(VideoFrame { rgba, width, height });
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // ── Audio spectrum bin ─────────────────────────────────────────
        let spectrum_elem = gst::ElementFactory::make("spectrum")
            .property("bands", SPECTRUM_BANDS as u32)
            .property("threshold", -80i32)
            .property("post-messages", true)
            .property("interval", 33_333_333u64) // ~30fps
            .property("message-magnitude", true)
            .build()
            .map_err(|e| anyhow!("Failed to create spectrum element: {e}"))?;

        let audioconvert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|e| anyhow!("Failed to create audioconvert: {e}"))?;

        let audiosink = gst::ElementFactory::make("autoaudiosink")
            .build()
            .map_err(|e| anyhow!("Failed to create autoaudiosink: {e}"))?;

        let audio_bin = gst::Bin::new();
        audio_bin.add_many([&audioconvert, &spectrum_elem, &audiosink])?;
        gst::Element::link_many([&audioconvert, &spectrum_elem, &audiosink])?;

        let pad = audioconvert
            .static_pad("sink")
            .ok_or_else(|| anyhow!("No sink pad on audioconvert"))?;
        audio_bin
            .add_pad(&gst::GhostPad::with_target(&pad)?)
            .map_err(|e| anyhow!("Failed to add ghost pad: {e}"))?;

        pipeline.set_property("audio-sink", &audio_bin.upcast::<gst::Element>());

        let spectrum = vec![0.0f32; SPECTRUM_BANDS];

        Ok(Self { pipeline, frame, spectrum })
    }

    /// Poll bus for spectrum messages. Call this each frame.
    pub fn poll_spectrum(&mut self) -> bool {
        let bus = match self.pipeline.bus() {
            Some(b) => b,
            None => return false,
        };

        let mut updated = false;
        while let Some(msg) = bus.pop() {
            if let gst::MessageView::Element(elem) = msg.view() {
                if let Some(s) = elem.structure() {
                    if s.name() == "spectrum" {
                        if let Ok(magnitudes) = s.get::<gst::List>("magnitude") {
                            let vals: Vec<f32> = magnitudes
                                .iter()
                                .take(SPECTRUM_BANDS)
                                .filter_map(|v| v.get::<f32>().ok())
                                .collect();
                            if vals.len() == SPECTRUM_BANDS {
                                self.spectrum = vals;
                                updated = true;
                            }
                        }
                    }
                }
            }
        }
        updated
    }

    /// Check playbin's n-video property to detect audio-only streams.
    /// Returns None if pipeline isn't ready yet, Some(true) for audio-only.
    pub fn is_audio_only(&self) -> bool {
        let n_video: i32 = self.pipeline.property("n-video");
        n_video == 0
    }

    pub fn spectrum(&self) -> &[f32] {
        &self.spectrum
    }

    pub fn play(&self) {
        let _ = self.pipeline.set_state(GstState::Playing);
    }

    pub fn pause(&self) {
        let _ = self.pipeline.set_state(GstState::Paused);
    }

    pub fn is_playing(&self) -> bool {
        matches!(self.pipeline.current_state(), GstState::Playing)
    }

    pub fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn seek(&self, position_ns: u64) {
        let _ = self.pipeline.seek_simple(
            SeekFlags::FLUSH | SeekFlags::KEY_UNIT,
            ClockTime::from_nseconds(position_ns),
        );
    }

    pub fn position(&self) -> Option<u64> {
        self.pipeline
            .query_position::<ClockTime>()
            .map(|t| t.nseconds())
    }

    pub fn duration(&self) -> Option<u64> {
        self.pipeline
            .query_duration::<ClockTime>()
            .map(|t| t.nseconds())
    }

    pub fn take_frame(&self) -> Option<VideoFrame> {
        self.frame.lock().ok()?.take()
    }

    pub fn set_volume(&self, vol: f64) {
        self.pipeline.set_property("volume", vol.clamp(0.0, 1.0));
    }
}

impl Drop for MediaPipeline {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(GstState::Null);
    }
}
