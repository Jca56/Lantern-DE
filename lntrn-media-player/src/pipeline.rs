use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use gstreamer::prelude::*;
use gstreamer::{self as gst, ClockTime, Element, SeekFlags, State as GstState};
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::fft::SpectrumAnalyzer;

const FFT_SIZE: usize = 1024;
const FFT_SAMPLE_RATE: u32 = 44_100;
const FFT_UPDATE_HZ: u32 = 30;

/// How long a Null transition may block the caller. Normal teardown is a few
/// ms; anything longer means the audio sink is stuck waiting on the PipeWire
/// daemon (graph stall, daemon restart) and will never come back.
pub const TEARDOWN_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long a fresh pipeline may sit un-prerolled before we give up on it.
/// A healthy open reaches PAUSED in well under a second; the failure this
/// guards against is the audio sink waiting forever for PipeWire to link its
/// stream, which left the player as a blank window with a deadlocked UI.
pub const PREROLL_TIMEOUT: Duration = Duration::from_secs(6);

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
    /// The autoaudiosink; kept so we can log which real sink it settled on.
    audiosink: Element,
    sink_logged: bool,
    /// First fatal bus error, until `take_error` consumes it.
    error: Option<String>,
    /// Set once the pipeline reports ASYNC_DONE (every sink has prerolled).
    /// Until then playbin's locks may be held by the sink activation on a
    /// streaming thread, so every synchronous query/seek/property call from
    /// the UI thread is refused — that is exactly the call that deadlocked.
    prerolled: bool,
    created: Instant,
    /// Volume requested before preroll; applied once it is safe to.
    pending_volume: Option<f64>,
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
                    let info =
                        gst_video::VideoInfo::from_caps(caps).map_err(|_| gst::FlowError::Error)?;

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
                        *lock = Some(VideoFrame {
                            rgba,
                            width,
                            height,
                        });
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

        let spectrum_shared: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(vec![0.0; SPECTRUM_BANDS]));
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
            audiosink,
            sink_logged: false,
            error: None,
            prerolled: false,
            created: Instant::now(),
            pending_volume: None,
        })
    }

    /// Drain the pipeline bus — EOS, errors, warnings, and a one-time note of
    /// which real sink autoaudiosink settled on — then pull the latest
    /// spectrum if the audio appsink has computed a fresh one. Call each frame.
    pub fn poll_bus(&mut self) -> bool {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                self.handle_message(&msg);
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

    fn handle_message(&mut self, msg: &gst::Message) {
        let src = || {
            msg.src()
                .map(|s| s.name().to_string())
                .unwrap_or_else(|| "?".into())
        };
        match msg.view() {
            gst::MessageView::Eos(_) => self.eos = true,
            gst::MessageView::AsyncDone(_) => {
                if !self.prerolled {
                    self.prerolled = true;
                    eprintln!(
                        "[media-player] gst: prerolled after {} ms",
                        self.created.elapsed().as_millis()
                    );
                    if let Some(v) = self.pending_volume.take() {
                        self.set_volume(v);
                    }
                }
            }
            gst::MessageView::Error(e) => {
                let debug = e.debug().map(|d| d.to_string()).unwrap_or_default();
                eprintln!("[media-player] gst error from {}: {} ({debug})", src(), e.error());
                // First error wins; the rest is usually fallout from it.
                self.error
                    .get_or_insert_with(|| format!("{}: {}", src(), e.error()));
            }
            gst::MessageView::Warning(w) => {
                let debug = w.debug().map(|d| d.to_string()).unwrap_or_default();
                eprintln!("[media-player] gst warning from {}: {} ({debug})", src(), w.error());
            }
            gst::MessageView::StateChanged(sc) => {
                let from_pipeline = msg.src() == Some(self.pipeline.upcast_ref());
                // Trail the transitions that matter for the "sink never came
                // up" diagnosis: the pipeline itself and the audio sink chain.
                let name = src();
                if from_pipeline || name.contains("audiosink") || name.contains("sink") && name.contains("pipewire") {
                    eprintln!(
                        "[media-player] gst: {name} {:?} → {:?}",
                        sc.old(),
                        sc.current()
                    );
                }
                if from_pipeline && sc.current() == GstState::Playing && !self.sink_logged {
                    self.log_audio_sink();
                }
            }
            _ => {}
        }
    }

    /// Note which sink autoaudiosink actually picked. If this ever says
    /// alsasink, the player went around PipeWire — the kind of thing that can
    /// take the sound card away from everyone else.
    fn log_audio_sink(&mut self) {
        self.sink_logged = true;
        let Some(bin) = self.audiosink.downcast_ref::<gst::Bin>() else {
            return;
        };
        let picked = bin
            .iterate_elements()
            .into_iter()
            .find_map(|e| e.ok())
            .and_then(|e| e.factory())
            .map(|f| f.name().to_string())
            .unwrap_or_else(|| "none".into());
        eprintln!("[media-player] audio sink: {picked}");
    }

    /// The first fatal error the bus reported, if any (consumed on read).
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    /// True when the pipeline has been un-prerolled for longer than
    /// `PREROLL_TIMEOUT` — the caller should drop it and tell the user.
    pub fn preroll_stalled(&self) -> bool {
        !self.prerolled && self.created.elapsed() > PREROLL_TIMEOUT
    }

    /// Check playbin's n-video property to detect audio-only streams.
    /// Returns None if pipeline isn't ready yet, Some(true) for audio-only.
    pub fn is_audio_only(&self) -> bool {
        if !self.prerolled {
            return false;
        }
        let n_video: i32 = self.pipeline.property("n-video");
        n_video == 0
    }

    pub fn is_eos(&self) -> bool {
        self.eos
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
        if !self.prerolled {
            return;
        }
        let _ = self.pipeline.seek_simple(
            SeekFlags::FLUSH | SeekFlags::ACCURATE,
            ClockTime::from_nseconds(position_ns),
        );
    }

    pub fn clear_eos(&mut self) {
        self.eos = false;
    }

    pub fn position(&self) -> Option<u64> {
        if !self.prerolled {
            return None;
        }
        self.pipeline
            .query_position::<ClockTime>()
            .map(|t| t.nseconds())
    }

    pub fn duration(&self) -> Option<u64> {
        if !self.prerolled {
            return None;
        }
        self.pipeline
            .query_duration::<ClockTime>()
            .map(|t| t.nseconds())
    }

    pub fn take_frame(&self) -> Option<VideoFrame> {
        self.frame.lock().ok()?.take()
    }

    /// Safe before `play()` (no streaming threads yet) and after preroll; in
    /// between the request is parked and applied on ASYNC_DONE.
    pub fn set_volume(&mut self, vol: f64) {
        let playing_or_pending = self.pipeline.current_state() != GstState::Null
            || self.pipeline.pending_state() != GstState::VoidPending;
        if playing_or_pending && !self.prerolled {
            self.pending_volume = Some(vol);
            return;
        }
        self.pipeline.set_property("volume", vol.clamp(0.0, 1.0));
    }
}

impl Drop for MediaPipeline {
    fn drop(&mut self) {
        stop_bounded(self.pipeline.clone(), TEARDOWN_TIMEOUT);
    }
}

/// Drive `pipeline` to Null on a helper thread, waiting at most `timeout`.
///
/// The Null transition is synchronous and waits on the audio sink; when the
/// PipeWire daemon has stopped serving us that wait never returns. Blocking
/// the UI thread on it is how the player used to turn into an invisible
/// zombie that kept its audio stream — so on timeout the helper (and the
/// pipeline it holds) is simply abandoned. Returns whether teardown finished.
pub fn stop_bounded(pipeline: Element, timeout: Duration) -> bool {
    let (done_tx, done_rx) = mpsc::channel();
    let handle = pipeline.clone();
    let spawned = std::thread::Builder::new()
        .name("gst-teardown".into())
        .spawn(move || {
            let _ = handle.set_state(GstState::Null);
            let _ = done_tx.send(());
        });
    if spawned.is_err() {
        // No thread to lean on — fall back to the old inline transition.
        let _ = pipeline.set_state(GstState::Null);
        return true;
    }
    match done_rx.recv_timeout(timeout) {
        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            eprintln!(
                "[media-player] pipeline teardown still blocked after {timeout:?}; abandoning it"
            );
            false
        }
    }
}
