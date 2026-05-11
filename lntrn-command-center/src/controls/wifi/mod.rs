//! WiFi control tile.
//!
//! Inline tile: a signal-strength icon (off / low / medium / high)
//! based on the currently-connected SSID's signal strength.
//!
//! Click-expand view: list of nearby networks. Each row shows a signal
//! icon, SSID, lock badge (for secured networks), and a "connected"
//! marker for the active one. Clicking a saved or open network triggers
//! a connect; secured networks the user hasn't connected to before
//! show a password modal.
//!
//! Backend: shells out to `nmcli`. Scans take ~500-1500 ms so we run
//! them on a dedicated background thread that pushes results through
//! mpsc channels — the panel render loop just `try_recv`s on tick.
//!
//! Layout of this module:
//! - `mod.rs` (this file): public types, [`Wifi`] state struct, password
//!   prompt, and the worker-bound enums.
//! - `worker.rs`: the background polling thread and its nmcli shellouts.
//! - `view.rs`: tile + click-expand drawing, hit-testing, and layout.
//! - `modal.rs`: password-prompt drawing and hit-testing.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use crate::search::input::Input;

mod modal;
mod tile;
mod view;
mod worker;

// Re-export the public surface so external callers (layershell, the
// tile dispatcher in `controls/mod.rs`, etc.) can still use
// `crate::controls::wifi::{...}` paths unchanged after the split.
pub use modal::{hit_test_modal, ModalHit};
// `modal_regions`/`ModalRegions` were part of the pre-split public
// surface; keep them addressable from this module so external callers
// can opt back in without touching the private submodule path.
#[allow(unused_imports)]
pub use modal::{modal_regions, ModalRegions};
pub use tile::{draw_inline, TILE_WIDTH};
pub use view::{draw_view, hit_test_network, NetworkHit};

// ── State types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiState {
    Off,
    Disconnected,
    Connected { ssid: String, signal: u32 },
}

/// Radio band a given AP is broadcasting on. Used to let the user pin
/// a connection to a specific band when a hotspot advertises the same
/// SSID on both 2.4 and 5 GHz radios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Band {
    G24,
    G5,
    G6,
}

impl Band {
    /// Short pill label, e.g. "2.4".
    pub fn short_label(self) -> &'static str {
        match self {
            Band::G24 => "2.4",
            Band::G5 => "5",
            Band::G6 => "6",
        }
    }

    /// Long-form label for the details panel, e.g. "2.4 GHz".
    pub fn long_label(self) -> &'static str {
        match self {
            Band::G24 => "2.4 GHz",
            Band::G5 => "5 GHz",
            Band::G6 => "6 GHz",
        }
    }

    /// Value to set on `wifi.band` in the NM connection profile.
    /// `bg` = 2.4 GHz only; `a` covers 5/6 GHz. (NetworkManager doesn't
    /// distinguish 5 from 6 here — for 6 GHz-only you'd need to pin a
    /// BSSID, which we don't do yet.)
    pub fn nm_band(self) -> &'static str {
        match self {
            Band::G24 => "bg",
            Band::G5 | Band::G6 => "a",
        }
    }

    pub(crate) fn from_mhz(mhz: u32) -> Option<Band> {
        Some(match mhz {
            2400..=2500 => Band::G24,
            4900..=5900 => Band::G5,
            5925..=7125 => Band::G6,
            _ => return None,
        })
    }

    pub(crate) fn from_freq_str(s: &str) -> Option<Band> {
        let mhz: u32 = s.split_whitespace().next()?.parse().ok()?;
        Band::from_mhz(mhz)
    }
}

/// One radio's worth of info for an SSID. A given network may have
/// multiple `BandEntry`s — typically one for 2.4 and one for 5 GHz when
/// a hotspot broadcasts both.
#[derive(Debug, Clone)]
pub struct BandEntry {
    pub band: Band,
    pub signal: u32,
    pub bssid: String,
    pub channel: String,
    pub frequency: String,
    pub rate: String,
}

#[derive(Debug, Clone)]
pub struct Network {
    pub ssid: String,
    /// Strongest signal across all bands — drives the row icon + sort.
    pub signal: u32,
    pub security: String,
    pub in_use: bool,
    pub saved: bool,
    /// BSSID of the strongest band. Kept for the details panel; the
    /// per-band BSSIDs live in `bands`.
    pub bssid: String,
    /// e.g. "Infra" / "Mesh".
    pub mode: String,
    /// Strongest band's channel (string, as nmcli reports).
    pub channel: String,
    /// Strongest band's frequency, e.g. "5180 MHz".
    pub frequency: String,
    /// Strongest band's negotiated bitrate, e.g. "270 Mbit/s".
    pub rate: String,
    /// All radios this SSID is advertised on, sorted by signal desc.
    pub bands: Vec<BandEntry>,
    /// Band the user has selected for connecting. Defaults to the
    /// strongest band; persists across rescans (see `Wifi::tick`).
    pub selected_band: Band,
}

/// Commands sent from the render thread → worker thread.
pub(crate) enum WifiCmd {
    /// Force a fresh scan + status poll.
    Rescan,
    /// Connect to `ssid`. If `password` is `None`, we try saved-first
    /// then a bare open connect. If `band` is `Some`, we pin the
    /// connection profile's `wifi.band` to that radio so future
    /// reconnects stay on it.
    Connect {
        ssid: String,
        password: Option<String>,
        band: Option<Band>,
    },
}

/// Events the worker thread emits.
pub(crate) enum WifiEvent {
    Status(WifiState),
    Networks(Vec<Network>),
    ConnectOk,
    ConnectFail(String),
}

pub struct Wifi {
    state: WifiState,
    networks: Vec<Network>,
    /// Last connect failure shown in the expanded view.
    last_error: Option<String>,
    /// Whether `nmcli` was available at startup. False → tile draws nothing.
    available: bool,
    cmd_tx: mpsc::Sender<WifiCmd>,
    event_rx: mpsc::Receiver<WifiEvent>,
    /// Optional password-prompt modal overlaying the network list.
    /// When `Some`, all keyboard input goes into the modal's `Input`
    /// instead of the launcher's search field.
    pub prompt: Option<PasswordPrompt>,
    /// SSID of the network we're currently trying to connect to, or
    /// `None` if no connect attempt is in flight. Drives the
    /// "Connecting…" status text + lets the click handler debounce
    /// duplicate clicks while a connect is pending.
    connecting_ssid: Option<String>,
    /// SSID under the pointer, used for the subtle hover-highlight on
    /// network rows. Set externally by the layershell pointer-motion
    /// handler; cleared when the cursor leaves the WiFi view.
    pub hovered_ssid: Option<String>,
    /// SSID of the row currently expanded to show details + Connect.
    /// Only one row may be expanded at a time. None = collapsed list.
    pub expanded_ssid: Option<String>,
}

/// State for the password-entry modal that overlays the WiFi view.
pub struct PasswordPrompt {
    pub ssid: String,
    pub input: Input,
    /// True between Submit and the next ConnectOk/Fail event.
    pub connecting: bool,
}

impl PasswordPrompt {
    fn new(ssid: String) -> Self {
        Self {
            ssid,
            input: Input::new(),
            connecting: false,
        }
    }
}

impl Wifi {
    pub fn new() -> Self {
        // Quick availability check up front — if nmcli is missing we
        // skip spawning the worker so the tile just hides.
        let available = Command::new("nmcli")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        if available {
            thread::Builder::new()
                .name("lcc-wifi-poll".into())
                .spawn(move || worker::run(event_tx, cmd_rx))
                .ok();
        }

        Self {
            state: WifiState::Off,
            networks: Vec::new(),
            last_error: None,
            available,
            cmd_tx,
            event_rx,
            prompt: None,
            connecting_ssid: None,
            hovered_ssid: None,
            expanded_ssid: None,
        }
    }

    /// True if a connect attempt to `ssid` is currently in flight.
    pub fn is_connecting_to(&self, ssid: &str) -> bool {
        self.connecting_ssid.as_deref() == Some(ssid)
    }

    pub fn is_present(&self) -> bool {
        self.available
    }

    pub fn state(&self) -> &WifiState {
        &self.state
    }

    pub fn networks(&self) -> &[Network] {
        &self.networks
    }

    /// Drain events from the worker. Returns true if anything changed
    /// (caller may want to redraw). Also auto-dismisses the password
    /// prompt on successful connect, and surfaces failures into the
    /// prompt's connecting state.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ev) = self.event_rx.try_recv() {
            changed = true;
            match ev {
                WifiEvent::Status(s) => self.state = s,
                WifiEvent::Networks(mut n) => {
                    // Preserve the user's band selection across rescans
                    // when the chosen band is still being broadcast.
                    let prev: HashMap<String, Band> = self
                        .networks
                        .iter()
                        .map(|net| (net.ssid.clone(), net.selected_band))
                        .collect();
                    for net in n.iter_mut() {
                        if let Some(b) = prev.get(&net.ssid) {
                            if net.bands.iter().any(|e| e.band == *b) {
                                net.selected_band = *b;
                            }
                        }
                    }
                    self.networks = n;
                }
                WifiEvent::ConnectOk => {
                    self.last_error = None;
                    // Successful connect → clear the modal if any.
                    self.prompt = None;
                    self.connecting_ssid = None;
                }
                WifiEvent::ConnectFail(msg) => {
                    self.last_error = Some(msg);
                    if let Some(p) = &mut self.prompt {
                        // Stay in the modal so the user can retry.
                        p.connecting = false;
                    }
                    self.connecting_ssid = None;
                }
            }
        }
        changed
    }

    /// Open a password-entry modal for the given SSID. Replaces any
    /// existing prompt.
    pub fn open_prompt(&mut self, ssid: &str) {
        self.prompt = Some(PasswordPrompt::new(ssid.to_string()));
        self.last_error = None;
    }

    /// Cancel/close the prompt without connecting.
    pub fn close_prompt(&mut self) {
        self.prompt = None;
    }

    /// Submit the current prompt's password and try to connect.
    /// No-op if the prompt is empty.
    pub fn submit_prompt(&mut self) {
        let Some(prompt) = &mut self.prompt else { return };
        if prompt.input.query().is_empty() {
            return;
        }
        let ssid = prompt.ssid.clone();
        let password = prompt.input.query().to_string();
        prompt.connecting = true;
        self.last_error = None;
        self.connecting_ssid = Some(ssid.clone());
        let band = self.band_pref_for(&ssid);
        let _ = self.cmd_tx.send(WifiCmd::Connect {
            ssid,
            password: Some(password),
            band,
        });
    }

    /// Return the band to force for `ssid`, if the user effectively has
    /// a choice. We only pass a band preference when the SSID is on
    /// more than one radio — single-band networks just let NM pick.
    fn band_pref_for(&self, ssid: &str) -> Option<Band> {
        let net = self.networks.iter().find(|n| n.ssid == ssid)?;
        if net.bands.len() > 1 {
            Some(net.selected_band)
        } else {
            None
        }
    }

    /// User clicked a band pill in the expanded row. Updates the
    /// in-memory selection so subsequent Connect uses it.
    pub fn select_band(&mut self, ssid: &str, band: Band) {
        if let Some(net) = self.networks.iter_mut().find(|n| n.ssid == ssid) {
            if net.bands.iter().any(|b| b.band == band) {
                net.selected_band = band;
            }
        }
    }

    /// Ask the worker to rescan + repoll. Cheap to call; the worker
    /// rate-limits its own scan cadence.
    #[allow(dead_code)] // call from a future "refresh" button
    pub fn request_rescan(&self) {
        let _ = self.cmd_tx.send(WifiCmd::Rescan);
    }

    /// Attempt to connect to `ssid`. If `password` is `None`, the worker
    /// tries the saved connection first, then a bare open-network connect.
    /// When the SSID is multi-band, the user's selected band is pinned
    /// on the resulting NM profile.
    pub fn connect(&mut self, ssid: &str, password: Option<String>) {
        self.connecting_ssid = Some(ssid.to_string());
        self.last_error = None;
        let band = self.band_pref_for(ssid);
        let _ = self.cmd_tx.send(WifiCmd::Connect {
            ssid: ssid.to_string(),
            password,
            band,
        });
    }

    /// Best-effort cached check for whether `ssid` is in NM's saved
    /// connection list. Used by the click handler to decide whether
    /// to attempt a passwordless connect or surface "needs password."
    #[allow(dead_code)] // utility kept for future click-handler refactors
    pub fn is_saved(&self, ssid: &str) -> bool {
        self.networks.iter().any(|n| n.ssid == ssid && n.saved)
    }

    /// Most recent connect-fail message, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}
