//! The audio side of the pipeline: a bin that plays the sound and, on a
//! second branch, feeds the visualizer's FFT.
//!
//! ghostpad-sink → tee ┬→ queue → autoaudiosink   (playback)
//!                     └→ queue → audioconvert → audioresample →
//!                          capsfilter(f32 mono 44.1k) → appsink (FFT)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use gstreamer::prelude::*;
use gstreamer::{self as gst, Element};
use gstreamer_app as gst_app;

use crate::event_pump::Waker;
use crate::fft::SpectrumAnalyzer;

const FFT_SIZE: usize = 1024;
const FFT_SAMPLE_RATE: u32 = 44_100;
const FFT_UPDATE_HZ: u32 = 30;

pub const SPECTRUM_BANDS: usize = 64;

pub struct AudioTap {
    /// The whole bin, to hand to playbin as its audio sink.
    pub bin: Element,
    /// The autoaudiosink inside it; kept to log which real sink it picked.
    pub sink: Element,
    /// Latest spectrum (dB per band) and whether it changed since last read.
    pub spectrum: Arc<Mutex<Vec<f32>>>,
    pub dirty: Arc<AtomicBool>,
}

fn make(factory: &str) -> Result<Element> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|e| anyhow!("Failed to create {factory}: {e}"))
}

/// Build the bin. `waker` is poked whenever a fresh spectrum is ready, so
/// the visualizer redraws without the main loop polling for it.
pub fn build(waker: Waker) -> Result<AudioTap> {
    let tee = make("tee")?;
    let queue_play = make("queue")?;
    let sink = make("autoaudiosink")?;
    let queue_fft = make("queue")?;
    let audioconvert = make("audioconvert")?;
    let audioresample = make("audioresample")?;

    let fft_caps = gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("channels", 1i32)
        .field("rate", FFT_SAMPLE_RATE as i32)
        .field("layout", "interleaved")
        .build();
    let appsink = gst_app::AppSink::builder()
        .caps(&fft_caps)
        .max_buffers(2)
        .drop(true)
        .build();

    // Silence until the first analysis lands (0 dB would read as full blast).
    let spectrum: Arc<Mutex<Vec<f32>>> =
        Arc::new(Mutex::new(vec![crate::fft::DB_FLOOR; SPECTRUM_BANDS]));
    let dirty = Arc::new(AtomicBool::new(false));
    let analyzer = Mutex::new(SpectrumAnalyzer::new(
        FFT_SIZE,
        SPECTRUM_BANDS,
        FFT_SAMPLE_RATE,
        FFT_UPDATE_HZ,
    ));

    let spec_ref = spectrum.clone();
    let dirty_ref = dirty.clone();
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                // The caps pin the format to F32LE interleaved mono. Decoded
                // by value: a mapped buffer needn't be 4-byte aligned.
                let samples: Vec<f32> = map
                    .as_slice()
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                let bands = analyzer
                    .lock()
                    .ok()
                    .and_then(|mut a| a.push_samples(&samples));
                if let Some(bands) = bands {
                    if let Ok(mut s) = spec_ref.lock() {
                        *s = bands;
                        dirty_ref.store(true, Ordering::Release);
                        waker.wake();
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let bin = gst::Bin::new();
    let appsink_elem: &Element = appsink.upcast_ref();
    bin.add_many([
        &tee,
        &queue_play,
        &sink,
        &queue_fft,
        &audioconvert,
        &audioresample,
        appsink_elem,
    ])?;
    Element::link_many([&tee, &queue_play, &sink])?;
    Element::link_many([&tee, &queue_fft, &audioconvert, &audioresample, appsink_elem])?;

    let pad = tee
        .static_pad("sink")
        .ok_or_else(|| anyhow!("No sink pad on tee"))?;
    bin.add_pad(&gst::GhostPad::with_target(&pad)?)
        .map_err(|e| anyhow!("Failed to add ghost pad: {e}"))?;

    Ok(AudioTap {
        bin: bin.upcast::<Element>(),
        sink,
        spectrum,
        dirty,
    })
}
