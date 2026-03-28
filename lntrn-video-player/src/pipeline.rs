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

// ── Pipeline ────────────────────────────────────────────────────────────────

pub struct VideoPipeline {
    pipeline: Element,
    frame: Arc<Mutex<Option<VideoFrame>>>,
}

impl VideoPipeline {
    pub fn new(uri: &str) -> Result<Self> {
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri)
            .build()
            .map_err(|e| anyhow!("Failed to create playbin: {e}"))?;

        // Create appsink for video frames
        let appsink = gst_app::AppSink::builder()
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::Rgba)
                    .build(),
            )
            .max_buffers(1)
            .drop(true)
            .build();

        // Set video sink to our appsink
        pipeline.set_property("video-sink", &appsink);

        let frame: Arc<Mutex<Option<VideoFrame>>> = Arc::new(Mutex::new(None));
        let frame_ref = frame.clone();

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                    let info = gst_video::VideoInfo::from_caps(caps)
                        .map_err(|_| gst::FlowError::Error)?;

                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let width = info.width();
                    let height = info.height();

                    // Copy RGBA data — stride may differ from width*4
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

        Ok(Self { pipeline, frame })
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

    pub fn volume(&self) -> f64 {
        self.pipeline.property::<f64>("volume")
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(GstState::Null);
    }
}
