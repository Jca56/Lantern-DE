//! Audio control tile.
//!
//! Inline tile shows a speaker icon (drawn from polygons) + a thin
//! volume bar. Click-expand shows a large slider you can drag to set
//! the volume.
//!
//! Backend: shells out to `wpctl` (the WirePlumber CLI). All wpctl
//! calls happen on a background worker thread; the main render loop
//! only ever does `try_recv` on tick. Setters fire-and-forget commands
//! into the worker and optimistically update local state so the UI
//! feels instant. This keeps open/close animations smooth even when
//! pipewire/D-Bus is contended and a wpctl call spikes to 100+ ms.

use std::process::Command;
use std::sync::mpsc;
use std::thread;

mod icons;
mod view;
mod worker;

pub use view::{
    draw_inline, draw_view, hit_test_device_dir, hit_test_icon, hit_test_inline,
    inline_bar_rect, slider_rect_for, InlineHit, TILE_WIDTH,
};

/// One PipeWire sink as exposed by `wpctl status`.
#[derive(Debug, Clone)]
pub struct Sink {
    /// Numeric ID — what we pass to `wpctl set-default`.
    pub id: u32,
    /// Trimmed display name. May still be long for HDMI outputs; the
    /// renderer is responsible for truncating to fit.
    pub name: String,
    /// True for the current default sink (the one with `*` in `wpctl status`).
    pub is_default: bool,
}

/// Commands sent from the render thread → worker thread.
pub(super) enum AudioCmd {
    /// Force an immediate re-poll of volumes + sink list. No caller
    /// yet, but exposed so a future "refresh" affordance (device
    /// hot-plug, manual reload) can request fresh state without
    /// waiting for the next poll interval.
    #[allow(dead_code)]
    Rescan,
    SetVolume(f32),
    SetInputVolume(f32),
    ToggleMute,
    ToggleInputMute,
    SetDefaultSink(u32),
    SetDefaultSource(u32),
}

/// Events the worker thread emits.
pub(super) enum AudioEvent {
    /// Output sink state. `available=false` means wpctl is unreachable
    /// or returned garbage — the tile hides.
    Output {
        volume: f32,
        muted: bool,
        available: bool,
    },
    Input {
        volume: f32,
        muted: bool,
    },
    Devices {
        sinks: Vec<Sink>,
        sources: Vec<Sink>,
    },
}

/// Which audio direction a section addresses. Used so one piece of
/// layout code drives both the output and input sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Output,
    Input,
}

pub struct Audio {
    /// 0.0–1.0 normalized volume on the default sink.
    volume: f32,
    /// True when the default sink is explicitly muted.
    muted: bool,
    /// 0.0–1.0 normalized volume on the default source (mic).
    input_volume: f32,
    /// True when the default source is muted.
    input_muted: bool,
    /// True if `wpctl` is on PATH and returned a parseable volume.
    /// Off-PATH or failure → tile draws nothing.
    available: bool,
    /// Available output sinks (parsed from `wpctl status`). Used by the
    /// click-expand device picker.
    sinks: Vec<Sink>,
    /// Available input sources (mics). Same format as `sinks`.
    sources: Vec<Sink>,
    cmd_tx: mpsc::Sender<AudioCmd>,
    event_rx: mpsc::Receiver<AudioEvent>,
}

impl Audio {
    pub fn new() -> Self {
        // Availability check is the only synchronous wpctl call we do
        // on the main thread, and only at startup. After this, every
        // wpctl invocation lives on the worker. We probe with
        // `get-volume @DEFAULT_AUDIO_SINK@` because wpctl has no
        // `--version` flag — it errors out on it.
        let available = Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        if available {
            thread::Builder::new()
                .name("lcc-audio-poll".into())
                .spawn(move || worker::worker(event_tx, cmd_rx))
                .ok();
        }

        Self {
            volume: 0.0,
            muted: false,
            input_volume: 0.0,
            input_muted: false,
            available,
            sinks: Vec::new(),
            sources: Vec::new(),
            cmd_tx,
            event_rx,
        }
    }

    pub fn is_present(&self) -> bool {
        self.available
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn sinks(&self) -> &[Sink] {
        &self.sinks
    }

    pub fn input_volume(&self) -> f32 {
        self.input_volume
    }

    pub fn input_muted(&self) -> bool {
        self.input_muted
    }

    pub fn sources(&self) -> &[Sink] {
        &self.sources
    }

    /// Drain events from the worker. Non-blocking — does no I/O.
    pub fn tick(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                AudioEvent::Output {
                    volume,
                    muted,
                    available,
                } => {
                    self.volume = volume;
                    self.muted = muted;
                    self.available = available;
                }
                AudioEvent::Input { volume, muted } => {
                    self.input_volume = volume;
                    self.input_muted = muted;
                }
                AudioEvent::Devices { sinks, sources } => {
                    self.sinks = sinks;
                    self.sources = sources;
                }
            }
        }
    }

    /// Set the given sink ID as the default audio sink. Triggers an
    /// immediate sink-list re-poll on the worker so the UI updates the
    /// asterisk.
    pub fn set_default_sink(&mut self, id: u32) {
        // Optimistically flip the asterisk locally so the picker
        // doesn't visually lag behind the click.
        for s in &mut self.sinks {
            s.is_default = s.id == id;
        }
        let _ = self.cmd_tx.send(AudioCmd::SetDefaultSink(id));
    }

    /// Set the default sink's volume. Allows up to 120 % so the user
    /// can boost quiet sources (BT headphones, some HDMI sinks); wpctl
    /// is invoked with `--limit 1.5` so values above 1.0 take effect.
    pub fn set_volume(&mut self, v: f32) {
        let clamped = v.clamp(0.0, 1.2);
        self.volume = clamped;
        let _ = self.cmd_tx.send(AudioCmd::SetVolume(clamped));
    }

    /// Toggle the default sink's mute state.
    #[allow(dead_code)] // wired up later when we add the click-mute hit zone
    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        let _ = self.cmd_tx.send(AudioCmd::ToggleMute);
    }

    /// Set the default mic to the given normalized volume.
    pub fn set_input_volume(&mut self, v: f32) {
        let clamped = v.clamp(0.0, 1.2);
        self.input_volume = clamped;
        let _ = self.cmd_tx.send(AudioCmd::SetInputVolume(clamped));
    }

    /// Toggle the default source's mute state.
    #[allow(dead_code)] // wired up later when we add the input mute icon hit zone
    pub fn toggle_input_mute(&mut self) {
        self.input_muted = !self.input_muted;
        let _ = self.cmd_tx.send(AudioCmd::ToggleInputMute);
    }

    /// Set the given source ID as the default audio source.
    pub fn set_default_source(&mut self, id: u32) {
        for s in &mut self.sources {
            s.is_default = s.id == id;
        }
        let _ = self.cmd_tx.send(AudioCmd::SetDefaultSource(id));
    }
}
