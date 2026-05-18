use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use gstreamer::prelude::*;
use gstreamer::{self as gst, ClockTime, Element, SeekFlags, State as GstState};
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::fft::SpectrumAnalyzer;

const FFT_SIZE: usize = 1024;
const FFT_SAMPLE_RATE: u32 = 44_100;
const FFT_UPDATE_HZ: u32 = 30;

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
    spectrum_shared: Arc<Mutex<Vec<f32>>>,
    spectrum_dirty: Arc<AtomicBool>,
    spectrum: Vec<f32>,
    eos: bool,
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

        // ── Audio bin: tee playback + FFT analysis ─────────────────────
        // ghostpad-sink → tee ┬→ queue → autoaudiosink   (playback)
        //                     └→ queue → audioconvert → audioresample →
        //                          capsfilter(f32 mono 44.1k) → appsink (FFT)
        let tee = gst::ElementFactory::make("tee")
            .build()
            .map_err(|e| anyhow!("Failed to create tee: {e}"))?;
        let queue_play = gst::ElementFactory::make("queue")
            .build()
            .map_err(|e| anyhow!("Failed to create playback queue: {e}"))?;
        let audiosink = gst::ElementFactory::make("autoaudiosink")
            .build()
            .map_err(|e| anyhow!("Failed to create autoaudiosink: {e}"))?;
        let queue_fft = gst::ElementFactory::make("queue")
            .build()
            .map_err(|e| anyhow!("Failed to create fft queue: {e}"))?;
        let audioconvert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|e| anyhow!("Failed to create audioconvert: {e}"))?;
        let audioresample = gst::ElementFactory::make("audioresample")
            .build()
            .map_err(|e| anyhow!("Failed to create audioresample: {e}"))?;

        let fft_caps = gst::Caps::builder("audio/x-raw")
            .field("format", "F32LE")
            .field("channels", 1i32)
            .field("rate", FFT_SAMPLE_RATE as i32)
            .field("layout", "interleaved")
            .build();
        let audio_appsink = gst_app::AppSink::builder()
            .caps(&fft_caps)
            .max_buffers(2)
            .drop(true)
            .build();

        let spectrum_shared: Arc<Mutex<Vec<f32>>> =
            Arc::new(Mutex::new(vec![0.0; SPECTRUM_BANDS]));
        let spectrum_dirty = Arc::new(AtomicBool::new(false));
        let analyzer = Arc::new(Mutex::new(SpectrumAnalyzer::new(
            FFT_SIZE,
            SPECTRUM_BANDS,
            FFT_SAMPLE_RATE,
            FFT_UPDATE_HZ,
        )));

        let spec_ref = spectrum_shared.clone();
        let dirty_ref = spectrum_dirty.clone();
        let analyzer_ref = analyzer.clone();
        audio_appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let bytes = map.as_slice();
                    // SAFETY: caps pin format to F32LE interleaved mono, so the
                    // buffer is a packed array of native-endian f32 on LE hosts.
                    let samples: &[f32] = unsafe {
                        std::slice::from_raw_parts(
                            bytes.as_ptr() as *const f32,
                            bytes.len() / std::mem::size_of::<f32>(),
                        )
                    };
                    if let Ok(mut a) = analyzer_ref.lock() {
                        if let Some(bands) = a.push_samples(samples) {
                            drop(a);
                            if let Ok(mut s) = spec_ref.lock() {
                                *s = bands;
                                dirty_ref.store(true, Ordering::Release);
                            }
                        }
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let audio_bin = gst::Bin::new();
        let appsink_elem: &gst::Element = audio_appsink.upcast_ref();
        audio_bin.add_many([
            &tee,
            &queue_play,
            &audiosink,
            &queue_fft,
            &audioconvert,
            &audioresample,
            appsink_elem,
        ])?;
        gst::Element::link_many([&tee, &queue_play, &audiosink])?;
        gst::Element::link_many([
            &tee,
            &queue_fft,
            &audioconvert,
            &audioresample,
            appsink_elem,
        ])?;

        let pad = tee
            .static_pad("sink")
            .ok_or_else(|| anyhow!("No sink pad on tee"))?;
        audio_bin
            .add_pad(&gst::GhostPad::with_target(&pad)?)
            .map_err(|e| anyhow!("Failed to add ghost pad: {e}"))?;

        pipeline.set_property("audio-sink", &audio_bin.upcast::<gst::Element>());

        let spectrum = vec![0.0f32; SPECTRUM_BANDS];

        Ok(Self {
            pipeline,
            frame,
            spectrum_shared,
            spectrum_dirty,
            spectrum,
            eos: false,
        })
    }

    /// Drain pipeline bus for EOS, and pull the latest spectrum if the
    /// audio appsink has computed a fresh one. Call this each frame.
    pub fn poll_spectrum(&mut self) -> bool {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                if let gst::MessageView::Eos(_) = msg.view() {
                    self.eos = true;
                }
            }
        }
        if self.spectrum_dirty.swap(false, Ordering::Acquire) {
            if let Ok(s) = self.spectrum_shared.lock() {
                self.spectrum.clone_from(&s);
            }
            true
        } else {
            false
        }
    }

    /// Check playbin's n-video property to detect audio-only streams.
    /// Returns None if pipeline isn't ready yet, Some(true) for audio-only.
    pub fn is_audio_only(&self) -> bool {
        let n_video: i32 = self.pipeline.property("n-video");
        n_video == 0
    }

    pub fn is_eos(&self) -> bool { self.eos }

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
            SeekFlags::FLUSH | SeekFlags::ACCURATE,
            ClockTime::from_nseconds(position_ns),
        );
    }

    pub fn clear_eos(&mut self) {
        self.eos = false;
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
