//! PipeWire capture of the default sink's monitor stream.
//!
//! A background thread runs its own PipeWire mainloop and pumps mono
//! f32 samples into a shared ring buffer for the render thread to FFT.

use std::convert::TryInto;
use std::mem;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};
use std::thread;

use pipewire as pw;
use pw::{
    keys,
    properties::properties,
    spa,
    stream::{StreamBox, StreamFlags},
};
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::pod::Pod;

/// Number of f32 mono samples kept in the rolling capture buffer.
pub const RING_CAPACITY: usize = 4096;

/// FFT window size — must be a power of two. 512 gives ~10.7ms of audio
/// at 48 kHz, enough for the 32 log-binned bars without burning CPU.
pub const FFT_SIZE: usize = 512;

pub struct AudioCapture {
    inner: Arc<Inner>,
    _handle: thread::JoinHandle<()>,
}

struct Inner {
    ring: Mutex<Ring>,
    sample_rate: AtomicU32,
    quit: AtomicBool,
}

struct Ring {
    buf: Vec<f32>,
    head: usize,
    filled: bool,
}

impl Ring {
    fn new() -> Self {
        Self {
            buf: vec![0.0; RING_CAPACITY],
            head: 0,
            filled: false,
        }
    }

    fn push_mono(&mut self, samples: &[f32]) {
        for &s in samples {
            self.buf[self.head] = s;
            self.head = (self.head + 1) % RING_CAPACITY;
            if self.head == 0 {
                self.filled = true;
            }
        }
    }

    fn copy_recent(&self, out: &mut [f32]) {
        let n = out.len().min(RING_CAPACITY);
        if !self.filled && self.head < n {
            let avail = self.head;
            let pad = n - avail;
            out[..pad].fill(0.0);
            out[pad..pad + avail].copy_from_slice(&self.buf[..avail]);
            return;
        }
        let start = (self.head + RING_CAPACITY - n) % RING_CAPACITY;
        if start + n <= RING_CAPACITY {
            out.copy_from_slice(&self.buf[start..start + n]);
        } else {
            let first = RING_CAPACITY - start;
            out[..first].copy_from_slice(&self.buf[start..]);
            out[first..].copy_from_slice(&self.buf[..n - first]);
        }
    }
}

impl AudioCapture {
    pub fn start() -> Self {
        let inner = Arc::new(Inner {
            ring: Mutex::new(Ring::new()),
            sample_rate: AtomicU32::new(48_000),
            quit: AtomicBool::new(false),
        });
        let inner_thread = Arc::clone(&inner);
        let handle = thread::Builder::new()
            .name("lntrn-audio".into())
            .spawn(move || {
                if let Err(e) = run_capture(&inner_thread) {
                    tracing::warn!("[audio] capture failed: {e:#}");
                }
            })
            .expect("spawn audio thread");
        Self {
            inner,
            _handle: handle,
        }
    }

    pub fn snapshot(&self, out: &mut [f32; FFT_SIZE]) {
        let ring = self.inner.ring.lock().unwrap();
        ring.copy_recent(out);
    }

    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate.load(Ordering::Relaxed)
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.inner.quit.store(true, Ordering::Relaxed);
        // Thread observes the flag during its next 50 ms iterate tick
        // and returns. We don't join — process is exiting anyway.
    }
}

fn run_capture(inner: &Arc<Inner>) -> anyhow::Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let props = properties! {
        *keys::MEDIA_TYPE => "Audio",
        *keys::MEDIA_CATEGORY => "Capture",
        *keys::MEDIA_ROLE => "Music",
        *keys::APP_NAME => "Lantern Visualizer",
        *keys::NODE_NAME => "lntrn-visualizer",
        *keys::STREAM_CAPTURE_SINK => "true",
    };

    let stream = StreamBox::new(&core, "lntrn-visualizer", props)?;

    let inner_param = Arc::clone(inner);
    let inner_proc = Arc::clone(inner);

    // Shared "channels" count negotiated in param_changed and read by
    // the realtime process callback. Atomic so we don't lock on the RT
    // thread.
    let channels = Arc::new(AtomicU32::new(2));
    let channels_param = Arc::clone(&channels);
    let channels_proc = Arc::clone(&channels);

    // Reusable mono mixdown buffer for the realtime process callback.
    // Avoids per-callback heap allocations on the RT audio thread.
    let mut mono_buf: Vec<f32> = Vec::with_capacity(2048);

    let _listener = stream
        .add_local_listener::<()>()
        .param_changed(move |_stream, _user, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((mt, ms)) = format_utils::parse_format(param) else { return };
            if mt != MediaType::Audio || ms != MediaSubtype::Raw {
                return;
            }
            let mut info = spa::param::audio::AudioInfoRaw::new();
            if info.parse(param).is_err() {
                return;
            }
            if info.rate() > 0 {
                inner_param.sample_rate.store(info.rate(), Ordering::Relaxed);
            }
            if info.channels() > 0 {
                channels_param.store(info.channels(), Ordering::Relaxed);
            }
        })
        .process(move |stream, _user| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let nch = channels_proc.load(Ordering::Relaxed).max(1);
            let n_samples = data.chunk().size() as usize / mem::size_of::<f32>();
            let Some(bytes) = data.data() else { return };
            if n_samples == 0 {
                return;
            }
            let frames = n_samples / nch as usize;
            mono_buf.clear();
            if mono_buf.capacity() < frames {
                mono_buf.reserve(frames - mono_buf.capacity());
            }
            for f in 0..frames {
                let mut sum = 0.0f32;
                for c in 0..nch as usize {
                    let idx = f * nch as usize + c;
                    let start = idx * mem::size_of::<f32>();
                    let end = start + mem::size_of::<f32>();
                    if end <= bytes.len() {
                        if let Ok(arr) = bytes[start..end].try_into() {
                            sum += f32::from_le_bytes(arr);
                        }
                    }
                }
                mono_buf.push(sum / nch as f32);
            }
            if let Ok(mut ring) = inner_proc.ring.lock() {
                ring.push_mono(&mono_buf);
            }
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .map_err(|e| anyhow::anyhow!("pod serialize: {e}"))?
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).ok_or_else(|| anyhow::anyhow!("bad pod"))?];

    stream.connect(
        spa::utils::Direction::Input,
        None,
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    while !inner.quit.load(Ordering::Relaxed) {
        mainloop.loop_().iterate(std::time::Duration::from_millis(50));
    }
    Ok(())
}
