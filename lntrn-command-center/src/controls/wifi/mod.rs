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
//! Backend: autodetected at startup — iwd (over D-Bus) on the Gentoo
//! desktop, NetworkManager (via `nmcli`) on the Arch laptop. Scans take
//! ~500-1500 ms either way so we run them on a dedicated background
//! thread that pushes results through mpsc channels — the panel render
//! loop just `try_recv`s on tick.
//!
//! Layout of this module:
//! - `mod.rs` (this file): [`Wifi`] state struct, password prompt, and
//!   the worker-bound enums.
//! - `types.rs`: network / band / profile data types.
//! - `worker/`: the background polling thread plus the iwd backend.
//! - `view/`: click-expand drawing, hit-testing, and layout.
//! - `modal.rs`: password-prompt drawing and hit-testing.

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::search::input::Input;

mod modal;
mod tile;
mod types;
mod view;
mod worker;

pub use types::{Band, BandEntry, Network, Profile, WifiState};

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
pub use view::{draw_view, hit_test_network, max_scroll, row_list_top_y, NetworkHit};

// ── Worker-bound enums ──────────────────────────────────────────────────────

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
        // iwd doesn't expose per-profile band / BSSID pinning yet — the
        // worker currently drops these. Kept on the variant so the UI's
        // pinning state has a wire path for when iwd support lands.
        #[allow(dead_code)]
        band: Option<Band>,
        #[allow(dead_code)]
        bssid: Option<String>,
    },
    /// `nmcli connection delete uuid <uuid>` — purge a saved profile.
    DeleteProfile { uuid: String },
    /// `nmcli connection up id <name>` — switch to this saved profile.
    ActivateProfile { name: String },
    /// Drop the active connection (iwd `Station.Disconnect`).
    Disconnect,
}

/// Events the worker thread emits.
pub(crate) enum WifiEvent {
    Status(WifiState),
    Networks(Vec<Network>),
    ConnectOk,
    ConnectFail(String),
    /// A user-requested rescan finished and its fresh network list has
    /// already been sent. Stops the refresh spinner.
    ScanDone,
    /// A disconnect request finished; `Err` carries a short message.
    DisconnectDone(Result<(), String>),
}

pub struct Wifi {
    state: WifiState,
    networks: Vec<Network>,
    /// Last connect failure shown in the expanded view.
    last_error: Option<String>,
    /// Whether a usable WiFi backend (iwd or NetworkManager) was detected
    /// at startup. False → tile draws nothing.
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
    /// When the in-flight user-requested rescan started, or `None` when
    /// idle. Drives the refresh button's spin angle.
    scan_started: Option<Instant>,
    /// True between a Disconnect click and the worker's reply.
    disconnecting: bool,
    /// SSID under the pointer, used for the subtle hover-highlight on
    /// network rows. Set externally by the layershell pointer-motion
    /// handler; cleared when the cursor leaves the WiFi view.
    pub hovered_ssid: Option<String>,
    /// Pointer is over the header's refresh button.
    pub hovered_refresh: bool,
    /// SSID of the row currently expanded to show details + Connect.
    /// Only one row may be expanded at a time. None = collapsed list.
    pub expanded_ssid: Option<String>,
    /// Vertical scroll offset of the network list in logical px. Set by
    /// the layershell wheel handler; clamped to `[0, max_scroll]` in
    /// `draw_view` so resizes / network-list changes can't strand it.
    pub scroll: f32,
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
        // iwd owns the airwaves on every Lantern host. Skip spawning the
        // worker if it isn't on the bus so the tile hides cleanly.
        let available = worker::is_available();

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        if available {
            tracing::info!("wifi worker spawning (backend=iwd)");
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
            scan_started: None,
            disconnecting: false,
            hovered_ssid: None,
            hovered_refresh: false,
            expanded_ssid: None,
            scroll: 0.0,
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
                    // Preserve the user's band selection + pinned BSSID
                    // across rescans when they're still valid (the band
                    // is still being broadcast / the BSSID is still on
                    // the list).
                    let prev_band: HashMap<String, Band> = self
                        .networks
                        .iter()
                        .map(|net| (net.ssid.clone(), net.selected_band))
                        .collect();
                    let prev_pin: HashMap<String, String> = self
                        .networks
                        .iter()
                        .filter_map(|net| {
                            net.pinned_bssid
                                .as_ref()
                                .map(|b| (net.ssid.clone(), b.clone()))
                        })
                        .collect();
                    for net in n.iter_mut() {
                        if let Some(b) = prev_band.get(&net.ssid) {
                            if net.bands.iter().any(|e| e.band == *b) {
                                net.selected_band = *b;
                            }
                        }
                        if let Some(mac) = prev_pin.get(&net.ssid) {
                            if net.aps.iter().any(|a| &a.bssid == mac) {
                                net.pinned_bssid = Some(mac.clone());
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
                WifiEvent::ScanDone => {
                    self.scan_started = None;
                }
                WifiEvent::DisconnectDone(result) => {
                    self.disconnecting = false;
                    if let Err(msg) = result {
                        self.last_error = Some(msg);
                    }
                }            }
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
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        if prompt.input.query().is_empty() {
            return;
        }
        let ssid = prompt.ssid.clone();
        let password = prompt.input.query().to_string();
        prompt.connecting = true;
        self.last_error = None;
        self.connecting_ssid = Some(ssid.clone());
        let band = self.band_pref_for(&ssid);
        let bssid = self.bssid_pref_for(&ssid);
        let _ = self.cmd_tx.send(WifiCmd::Connect {
            ssid,
            password: Some(password),
            band,
            bssid,
        });
    }

    /// Return the BSSID to force for `ssid` if the user pinned one.
    fn bssid_pref_for(&self, ssid: &str) -> Option<String> {
        let net = self.networks.iter().find(|n| n.ssid == ssid)?;
        net.pinned_bssid.clone()
    }

    /// Toggle a pin on `bssid` for `ssid`. Setting `Some(mac)` pins, the
    /// same mac again unpins. Also snaps `selected_band` to the pinned
    /// AP's band so the band-selector pill agrees with the lock state,
    /// and — when the SSID is currently saved — immediately reapplies
    /// the change to the NetworkManager profile (reconnecting to the
    /// new BSSID, or clearing the pin from the profile on unpin).
    pub fn toggle_pinned_bssid(&mut self, ssid: &str, bssid: &str) {
        let (new_pin, snap_band, saved, in_use) = {
            let Some(net) = self.networks.iter_mut().find(|n| n.ssid == ssid) else {
                return;
            };
            let was_pinned_here = net.pinned_bssid.as_deref() == Some(bssid);
            let new_pin = if was_pinned_here {
                None
            } else {
                Some(bssid.to_string())
            };
            // When pinning, sync selected_band to the AP's band so
            // wifi.band on the profile agrees with wifi.bssid.
            let snap_band = if !was_pinned_here {
                net.aps.iter().find(|a| a.bssid == bssid).map(|a| a.band)
            } else {
                None
            };
            net.pinned_bssid = new_pin.clone();
            if let Some(b) = snap_band {
                net.selected_band = b;
            }
            (new_pin, snap_band, net.saved || net.in_use, net.in_use)
        };
        let _ = snap_band;
        // Auto-apply on the live profile when possible. For unsaved
        // networks there's no profile to modify yet; the pin still gets
        // applied next time the user clicks Connect (and we'll write
        // wifi.bssid then).
        if saved {
            self.connecting_ssid = if in_use {
                Some(ssid.to_string())
            } else {
                self.connecting_ssid.clone()
            };
            self.last_error = None;
            let band = self.band_pref_for(ssid);
            // On unpin, pass `Some("")` so the worker writes an empty
            // value to wifi.bssid (i.e. clears the field on the profile).
            let bssid_arg = match &new_pin {
                Some(mac) => Some(mac.clone()),
                None => Some(String::new()),
            };
            let _ = self.cmd_tx.send(WifiCmd::Connect {
                ssid: ssid.to_string(),
                password: None,
                band,
                bssid: bssid_arg,
            });
        }
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

    /// Ask the worker for a fresh radio scan. Ignored while one is
    /// already in flight so mashing the button doesn't queue a backlog.
    pub fn request_rescan(&mut self) {
        if self.scan_started.is_some() {
            return;
        }
        // Only arm the spinner if a worker is there to eventually stop it.
        if self.cmd_tx.send(WifiCmd::Rescan).is_ok() {
            self.scan_started = Some(Instant::now());
        }
    }

    /// Seconds since the in-flight rescan started, or `None` when idle.
    pub fn scan_elapsed(&self) -> Option<f32> {
        self.scan_started.map(|t| t.elapsed().as_secs_f32())
    }

    /// Drop the active connection. No-op while a disconnect is pending.
    pub fn disconnect(&mut self) {
        if self.disconnecting {
            return;
        }
        self.last_error = None;
        self.disconnecting = self.cmd_tx.send(WifiCmd::Disconnect).is_ok();
    }

    pub fn is_disconnecting(&self) -> bool {
        self.disconnecting
    }

    /// Attempt to connect to `ssid`. If `password` is `None`, the worker
    /// tries the saved connection first, then a bare open-network connect.
    /// When the SSID is multi-band, the user's selected band is pinned
    /// on the resulting NM profile.
    pub fn connect(&mut self, ssid: &str, password: Option<String>) {
        self.connecting_ssid = Some(ssid.to_string());
        self.last_error = None;
        let band = self.band_pref_for(ssid);
        let bssid = self.bssid_pref_for(ssid);
        let _ = self.cmd_tx.send(WifiCmd::Connect {
            ssid: ssid.to_string(),
            password,
            band,
            bssid,
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

    /// Delete a saved NM profile by UUID. Refreshed via the next scan.
    pub fn delete_profile(&mut self, uuid: &str) {
        let _ = self.cmd_tx.send(WifiCmd::DeleteProfile {
            uuid: uuid.to_string(),
        });
    }

    /// Activate the saved profile with `name` (NM's `connection.id`).
    /// Useful when multiple profiles target the same SSID and the user
    /// wants to pick a specific one.
    pub fn activate_profile(&mut self, name: &str) {
        self.last_error = None;
        let _ = self.cmd_tx.send(WifiCmd::ActivateProfile {
            name: name.to_string(),
        });
    }
}
