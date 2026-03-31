//! Audio widget — icon in bar + full mixer popup.
//! Split into mod.rs (types/state), draw.rs (rendering), poll.rs (wpctl backend).

mod draw;
mod poll;

use std::sync::mpsc;
use std::time::Instant;

use lntrn_render::Rect;
use lntrn_ui::gpu::InteractionContext;

// Zone IDs
pub const ZONE_AUDIO_ICON: u32 = 0xAD_0000;
const ZONE_VOL_SLIDER: u32 = 0xAD_0001;
const ZONE_MUTE_BTN: u32 = 0xAD_0002;
const ZONE_MIC_SLIDER: u32 = 0xAD_0003;
const ZONE_MIC_MUTE: u32 = 0xAD_0004;
const ZONE_SINK_BASE: u32 = 0xAD_0100;
const ZONE_SOURCE_BASE: u32 = 0xAD_0200;
const ZONE_STREAM_SLIDER_BASE: u32 = 0xAD_0300;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct AudioSink {
    pub id: u32,
    pub name: String,
    pub is_default: bool,
}

/// Same shape as AudioSink — kept as a type alias would lose clarity.
pub(crate) type AudioSource = AudioSink;

#[derive(Debug, Clone)]
pub(crate) struct AudioStream {
    pub id: u32,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FullState {
    pub volume: f32,
    pub muted: bool,
    pub mic_volume: f32,
    pub mic_muted: bool,
    pub sinks: Vec<AudioSink>,
    pub sources: Vec<AudioSource>,
    pub streams: Vec<AudioStream>,
}

pub(crate) enum AudioCmd {
    SetVolume(f32),
    SetMicVolume(f32),
    ToggleMute,
    ToggleMicMute,
    SetDefaultSink(u32),
    SetDefaultSource(u32),
    SetStreamVolume(u32, f32),
}

pub(crate) enum AudioEvent {
    State(FullState),
}

// ── Widget ──────────────────────────────────────────────────────────────────

pub struct Audio {
    pub(crate) volume: f32,
    pub(crate) muted: bool,
    pub(crate) mic_volume: f32,
    pub(crate) mic_muted: bool,
    pub(crate) sinks: Vec<AudioSink>,
    pub(crate) sources: Vec<AudioSource>,
    pub(crate) streams: Vec<AudioStream>,
    event_rx: mpsc::Receiver<AudioEvent>,
    cmd_tx: mpsc::Sender<AudioCmd>,
    pub(crate) icons_loaded: bool,
    pub open: bool,
    pub(crate) vol_slider_rect: Option<Rect>,
    pub(crate) mic_slider_rect: Option<Rect>,
    /// Per-stream slider rects: (stream_id, rect)
    pub(crate) stream_slider_rects: Vec<(u32, Rect)>,
    /// Which zone is being dragged (None = not dragging)
    dragging: Option<u32>,
    /// Debounce: ignore poll updates for volume/mic shortly after user changes them
    last_vol_change: Instant,
    last_mic_change: Instant,
    last_stream_change: Instant,
}

const DEBOUNCE_MS: u128 = 500;

impl Audio {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("audio-poll".into())
            .spawn(move || poll::poll_thread(event_tx, cmd_rx))
            .expect("spawn audio poll thread");

        Self {
            volume: 0.0, muted: false,
            mic_volume: 0.0, mic_muted: false,
            sinks: Vec::new(),
            sources: Vec::new(),
            streams: Vec::new(),
            event_rx, cmd_tx,
            icons_loaded: false,
            open: false,
            vol_slider_rect: None,
            mic_slider_rect: None,
            stream_slider_rects: Vec::new(),
            dragging: None,
            last_vol_change: Instant::now(),
            last_mic_change: Instant::now(),
            last_stream_change: Instant::now(),
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        let vol_locked = self.dragging == Some(ZONE_VOL_SLIDER)
            || now.duration_since(self.last_vol_change).as_millis() < DEBOUNCE_MS;
        let mic_locked = self.dragging == Some(ZONE_MIC_SLIDER)
            || now.duration_since(self.last_mic_change).as_millis() < DEBOUNCE_MS;
        let stream_locked = now.duration_since(self.last_stream_change).as_millis() < DEBOUNCE_MS;
        let dragging_stream = matches!(self.dragging, Some(z)
            if z >= ZONE_STREAM_SLIDER_BASE && z < ZONE_STREAM_SLIDER_BASE + 256);

        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AudioEvent::State(s) => {
                    if !vol_locked { self.volume = s.volume; }
                    self.muted = s.muted;
                    if !mic_locked { self.mic_volume = s.mic_volume; }
                    self.mic_muted = s.mic_muted;
                    self.sinks = s.sinks;
                    self.sources = s.sources;
                    if !stream_locked && !dragging_stream {
                        self.streams = s.streams;
                    } else {
                        // Preserve volumes for streams we're actively changing
                        let mut new_streams = s.streams;
                        for ns in &mut new_streams {
                            if let Some(old) = self.streams.iter().find(|o| o.id == ns.id) {
                                ns.volume = old.volume;
                            }
                        }
                        self.streams = new_streams;
                    }
                }
            }
        }
    }

    pub fn handle_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
            if zone == ZONE_MUTE_BTN {
                let _ = self.cmd_tx.send(AudioCmd::ToggleMute);
            } else if zone == ZONE_MIC_MUTE {
                let _ = self.cmd_tx.send(AudioCmd::ToggleMicMute);
            } else if zone == ZONE_VOL_SLIDER {
                self.dragging = Some(ZONE_VOL_SLIDER);
                self.set_vol_from_x(phys_cx);
            } else if zone == ZONE_MIC_SLIDER {
                self.dragging = Some(ZONE_MIC_SLIDER);
                self.set_mic_from_x(phys_cx);
            } else if zone >= ZONE_SINK_BASE && zone < ZONE_SINK_BASE + 256 {
                let idx = (zone - ZONE_SINK_BASE) as usize;
                if let Some(sink) = self.sinks.get(idx) {
                    let _ = self.cmd_tx.send(AudioCmd::SetDefaultSink(sink.id));
                }
            } else if zone >= ZONE_SOURCE_BASE && zone < ZONE_SOURCE_BASE + 256 {
                let idx = (zone - ZONE_SOURCE_BASE) as usize;
                if let Some(source) = self.sources.get(idx) {
                    let _ = self.cmd_tx.send(AudioCmd::SetDefaultSource(source.id));
                }
            } else if zone >= ZONE_STREAM_SLIDER_BASE && zone < ZONE_STREAM_SLIDER_BASE + 256 {
                self.dragging = Some(zone);
                self.set_stream_from_x(zone, phys_cx);
            }
        }
    }

    pub fn handle_drag(&mut self, phys_cx: f32) {
        match self.dragging {
            Some(ZONE_VOL_SLIDER) => self.set_vol_from_x(phys_cx),
            Some(ZONE_MIC_SLIDER) => self.set_mic_from_x(phys_cx),
            Some(z) if z >= ZONE_STREAM_SLIDER_BASE && z < ZONE_STREAM_SLIDER_BASE + 256 => {
                self.set_stream_from_x(z, phys_cx);
            }
            _ => {}
        }
    }

    pub fn on_release(&mut self) {
        self.dragging = None;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    fn snap_5(frac: f32) -> f32 {
        (frac * 20.0).round() / 20.0
    }

    fn set_vol_from_x(&mut self, phys_cx: f32) {
        if let Some(rect) = self.vol_slider_rect {
            let frac = ((phys_cx - rect.x) / rect.w).clamp(0.0, 1.0);
            let vol = Self::snap_5(frac);
            let _ = self.cmd_tx.send(AudioCmd::SetVolume(vol));
            self.volume = vol;
            self.last_vol_change = Instant::now();
        }
    }

    fn set_mic_from_x(&mut self, phys_cx: f32) {
        if let Some(rect) = self.mic_slider_rect {
            let frac = ((phys_cx - rect.x) / rect.w).clamp(0.0, 1.0);
            let snapped = Self::snap_5(frac);
            let _ = self.cmd_tx.send(AudioCmd::SetMicVolume(snapped));
            self.mic_volume = snapped;
            self.last_mic_change = Instant::now();
        }
    }

    fn set_stream_from_x(&mut self, zone: u32, phys_cx: f32) {
        let idx = (zone - ZONE_STREAM_SLIDER_BASE) as usize;
        if let Some(stream) = self.streams.get_mut(idx) {
            if let Some((_, rect)) = self.stream_slider_rects.iter().find(|(id, _)| *id == stream.id) {
                let frac = ((phys_cx - rect.x) / rect.w).clamp(0.0, 1.0);
                let vol = Self::snap_5(frac);
                let _ = self.cmd_tx.send(AudioCmd::SetStreamVolume(stream.id, vol));
                stream.volume = vol;
                self.last_stream_change = Instant::now();
            }
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if !self.open { return; }
        let step = -(delta * 0.005).clamp(-0.02, 0.02);
        let new_vol = Self::snap_5((self.volume + step).clamp(0.0, 1.0));
        if (new_vol - self.volume).abs() < 0.001 { return; }
        let _ = self.cmd_tx.send(AudioCmd::SetVolume(new_vol));
        self.volume = new_vol;
        self.last_vol_change = Instant::now();
    }

    pub(crate) fn icon_key(&self) -> &'static str {
        if self.muted || self.volume < 0.01 { return "sound-muted"; }
        match (self.volume * 100.0) as u32 {
            0..=25 => "sound-low",
            26..=60 => "sound-medium",
            _ => "sound-high",
        }
    }
}
