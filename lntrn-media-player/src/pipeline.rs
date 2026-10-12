use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use gstreamer::prelude::*;
use gstreamer::{self as gst, ClockTime, Element, SeekFlags, State as GstState};
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use crate::audio_tap::{self, AudioTap};
use crate::event_pump::Waker;

/// How long a Null transition may block the caller. Normal teardown is a few
/// ms; anything longer means the audio sink is stuck waiting on the PipeWire
/// daemon (graph stall, daemon restart) and will never come back.
pub const TEARDOWN_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long a fresh pipeline may sit un-prerolled before we give up on it.
/// A healthy open reaches PAUSED in well under a second; the failure this
/// guards against is the audio sink waiting forever for PipeWire to link its
/// stream, which left the player as a blank window with a deadlocked UI.
pub const PREROLL_TIMEOUT: Duration = Duration::from_secs(6);

/// A seek normally reports back (ASYNC_DONE) within a frame or two; after
/// this long without one the next seek goes out anyway.
const SEEK_PATIENCE: Duration = Duration::from_secs(1);

/// The newest decoded video frame, straight from the appsink. Held as the
/// GStreamer sample (a refcount, no pixel copy); the UI thread maps and
/// uploads it only when it's about to draw.
type FrameSlot = Arc<Mutex<Option<gst::Sample>>>;

// ── Pipeline ────────────────────────────────────────────────────────────────

pub struct MediaPipeline {
    pipeline: Element,
    frame: FrameSlot,
    tap: AudioTap,
    spectrum: Vec<f32>,
    eos: bool,
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
    /// When the seek now in flight was sent (None once ASYNC_DONE answers it).
    seek_started: Option<Instant>,
    /// Latest position asked for while a seek was in flight. Only the newest
    /// matters: holding → sends one seek at a time instead of a pile-up.
    queued_seek: Option<u64>,
}

impl MediaPipeline {
    /// Build a playbin for `path`. `waker` is poked from GStreamer's threads
    /// whenever the UI has something to pick up: a video frame, a spectrum,
    /// a bus message.
    pub fn new(path: &Path, waker: Waker) -> Result<Self> {
        // A real file URI: `#`, `%` and `?` in a file name must be escaped or
        // the source can't find the file.
        let uri = gst::glib::filename_to_uri(path, None)
            .map_err(|e| anyhow!("Can't make a URI for {}: {e}", path.display()))?;
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri.as_str())
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

        let frame: FrameSlot = Arc::new(Mutex::new(None));
        // Playing frames arrive as samples; a paused pipeline (a seek while
        // paused, the first frame of a paused open) delivers its picture as
        // a preroll. Both land in the same slot.
        let store = |slot: FrameSlot, waker: Waker| {
            move |sample: Result<gst::Sample, gst::glib::BoolError>| {
                let sample = sample.map_err(|_| gst::FlowError::Eos)?;
                if let Ok(mut lock) = slot.lock() {
                    *lock = Some(sample);
                }
                waker.wake();
                Ok(gst::FlowSuccess::Ok)
            }
        };
        let on_sample = store(frame.clone(), waker.clone());
        let on_preroll = store(frame.clone(), waker.clone());
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| on_sample(sink.pull_sample()))
                .new_preroll(move |sink| on_preroll(sink.pull_preroll()))
                .build(),
        );

        let tap = audio_tap::build(waker.clone())?;
        pipeline.set_property("audio-sink", &tap.bin);

        // EOS, errors and ASYNC_DONE reach the UI thread promptly instead of
        // on its next timer. `Pass` leaves the message queued for `poll_bus`.
        if let Some(bus) = pipeline.bus() {
            bus.set_sync_handler(move |_, _| {
                waker.wake();
                gst::BusSyncReply::Pass
            });
        }

        Ok(Self {
            pipeline,
            frame,
            spectrum: vec![crate::fft::DB_FLOOR; audio_tap::SPECTRUM_BANDS],
            tap,
            eos: false,
            sink_logged: false,
            error: None,
            prerolled: false,
            created: Instant::now(),
            pending_volume: None,
            seek_started: None,
            queued_seek: None,
        })
    }

    /// Drain the pipeline bus — EOS, errors, warnings, and a one-time note of
    /// which real sink autoaudiosink settled on — then pull the latest
    /// spectrum if the audio appsink has computed a fresh one (returns true
    /// when it did). Call on every wake-up.
    pub fn poll_bus(&mut self) -> bool {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                self.handle_message(&msg);
            }
        }
        // A seek that never reported back mustn't hold the queue forever.
        if self.queued_seek.is_some() && !self.seek_in_flight() {
            if let Some(target) = self.queued_seek.take() {
                self.start_seek(target);
            }
        }
        if self.tap.dirty.swap(false, Ordering::Acquire) {
            if let Ok(s) = self.tap.spectrum.lock() {
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
                // The seek in flight has landed; send the newest queued one.
                self.seek_started = None;
                if let Some(target) = self.queued_seek.take() {
                    self.start_seek(target);
                }
            }
            gst::MessageView::Error(e) => {
                let debug = e.debug().map(|d| d.to_string()).unwrap_or_default();
                eprintln!("[media-player] gst error from {}: {} ({debug})", src(), e.error());
                // First error wins; the rest is usually fallout from it.
                self.error.get_or_insert_with(|| e.error().to_string());
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
        let Some(bin) = self.tap.sink.downcast_ref::<gst::Bin>() else {
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

    /// When `preroll_stalled` would turn true, while still waiting.
    pub fn preroll_deadline(&self) -> Option<Instant> {
        (!self.prerolled).then(|| self.created + PREROLL_TIMEOUT)
    }

    /// When a queued seek stops waiting for the one in flight.
    pub fn seek_deadline(&self) -> Option<Instant> {
        self.queued_seek
            .and(self.seek_started)
            .map(|at| at + SEEK_PATIENCE)
    }

    /// Check playbin's n-video property to detect audio-only streams. False
    /// until the pipeline has prerolled.
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

    pub fn clear_eos(&mut self) {
        self.eos = false;
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

    /// Seek to `position_ns`. While an earlier seek is still settling the
    /// request is remembered (newest wins) and sent the moment that one
    /// lands, so rapid seeks coalesce instead of queueing up in the decoder.
    pub fn seek(&mut self, position_ns: u64) {
        if !self.prerolled {
            return;
        }
        if self.seek_in_flight() {
            self.queued_seek = Some(position_ns);
        } else {
            self.start_seek(position_ns);
        }
    }

    /// A seek is still landing (or waiting its turn): position reports
    /// aren't trustworthy yet.
    pub fn seek_busy(&self) -> bool {
        self.seek_in_flight() || self.queued_seek.is_some()
    }

    fn seek_in_flight(&self) -> bool {
        self.seek_started
            .is_some_and(|at| at.elapsed() < SEEK_PATIENCE)
    }

    fn start_seek(&mut self, position_ns: u64) {
        self.eos = false;
        let sent = self.pipeline.seek_simple(
            SeekFlags::FLUSH | SeekFlags::ACCURATE,
            ClockTime::from_nseconds(position_ns),
        );
        self.seek_started = sent.is_ok().then(Instant::now);
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

    /// A decoded frame is waiting to be shown.
    pub fn has_frame(&self) -> bool {
        self.frame.lock().is_ok_and(|f| f.is_some())
    }

    /// Upload the newest decoded frame into `slot`, reusing its texture when
    /// the resolution is unchanged (the common case) so playback doesn't
    /// churn VRAM. Returns the frame's size when one was uploaded. Called
    /// only when a frame is about to be drawn: a hidden window costs nothing.
    pub fn upload_frame(
        &self,
        gpu: &GpuContext,
        tex_pass: &TexturePass,
        slot: &mut Option<GpuTexture>,
    ) -> Option<(u32, u32)> {
        let sample = self.frame.lock().ok()?.take()?;
        let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
        let map = sample.buffer()?.map_readable().ok()?;
        let (width, height) = (info.width(), info.height());
        let stride = *info.stride().first()? as usize;
        let row_bytes = width as usize * 4;
        let rows = height as usize;
        if width == 0 || rows == 0 || stride < row_bytes {
            return None;
        }
        if stride == row_bytes {
            // Tightly packed: straight from the decoder's buffer to the GPU.
            let pixels = map.as_slice().get(..row_bytes * rows)?;
            tex_pass.upload_reuse(gpu, slot, pixels, width, height);
        } else {
            // Padded rows: repack. A short buffer is skipped whole rather
            // than uploaded at the wrong size.
            let mut rgba = Vec::with_capacity(row_bytes * rows);
            for row in 0..rows {
                let start = row * stride;
                rgba.extend_from_slice(map.as_slice().get(start..start + row_bytes)?);
            }
            tex_pass.upload_reuse(gpu, slot, &rgba, width, height);
        }
        Some((width, height))
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
