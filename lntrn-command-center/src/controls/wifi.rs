//! WiFi control tile.
//!
//! Inline tile: a signal-strength icon (off / low / medium / high)
//! based on the currently-connected SSID's signal strength.
//!
//! Click-expand view: list of nearby networks. Each row shows a signal
//! icon, SSID, lock badge (for secured networks), and a "connected"
//! marker for the active one. Clicking a saved or open network triggers
//! a connect; secured networks the user hasn't connected to before
//! show an error (the password modal is a follow-up).
//!
//! Backend: shells out to `nmcli`. nmcli scans take ~500-1500 ms so we
//! run it on a dedicated background thread that pushes results through
//! mpsc channels — the panel render loop just `try_recv`s on tick.

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;
use crate::search::input::Input;

const POLL_INTERVAL: Duration = Duration::from_secs(8);

// ── State types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiState {
    Off,
    Disconnected,
    Connected { ssid: String, signal: u32 },
}

#[derive(Debug, Clone)]
pub struct Network {
    pub ssid: String,
    pub signal: u32,
    pub security: String,
    pub in_use: bool,
    pub saved: bool,
}

/// Commands sent from the render thread → worker thread.
enum WifiCmd {
    /// Force a fresh scan + status poll.
    Rescan,
    /// Connect to `ssid`. If `password` is `None`, we try saved-first
    /// then a bare open connect.
    Connect { ssid: String, password: Option<String> },
}

/// Events the worker thread emits.
enum WifiEvent {
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
                .spawn(move || worker(event_tx, cmd_rx))
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
        }
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
                WifiEvent::Networks(n) => self.networks = n,
                WifiEvent::ConnectOk => {
                    self.last_error = None;
                    // Successful connect → clear the modal if any.
                    self.prompt = None;
                }
                WifiEvent::ConnectFail(msg) => {
                    self.last_error = Some(msg);
                    if let Some(p) = &mut self.prompt {
                        // Stay in the modal so the user can retry.
                        p.connecting = false;
                    }
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
        let _ = self.cmd_tx.send(WifiCmd::Connect {
            ssid,
            password: Some(password),
        });
    }

    /// Ask the worker to rescan + repoll. Cheap to call; the worker
    /// rate-limits its own scan cadence.
    #[allow(dead_code)] // call from a future "refresh" button
    pub fn request_rescan(&self) {
        let _ = self.cmd_tx.send(WifiCmd::Rescan);
    }

    /// Attempt to connect to `ssid`. If `password` is `None`, the worker
    /// tries the saved connection first, then a bare open-network connect.
    pub fn connect(&self, ssid: &str, password: Option<String>) {
        let _ = self.cmd_tx.send(WifiCmd::Connect {
            ssid: ssid.to_string(),
            password,
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

// ── Worker thread ───────────────────────────────────────────────────────────

fn worker(tx: mpsc::Sender<WifiEvent>, cmd_rx: mpsc::Receiver<WifiCmd>) {
    // Prime the UI with whatever we can read right away.
    let _ = tx.send(WifiEvent::Status(poll_status()));
    let _ = tx.send(WifiEvent::Networks(scan_networks()));

    let mut last_poll = Instant::now();
    loop {
        // Drain pending commands first.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                WifiCmd::Rescan => {
                    let _ = Command::new("nmcli").args(["dev", "wifi", "rescan"]).output();
                    thread::sleep(Duration::from_millis(500));
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                    last_poll = Instant::now();
                }
                WifiCmd::Connect { ssid, password } => {
                    let result = if let Some(pw) = password {
                        Command::new("nmcli")
                            .args(["device", "wifi", "connect", &ssid, "password", &pw])
                            .output()
                    } else {
                        // Try saved connection first; fall back to bare connect
                        // (which only works for open networks).
                        let r = Command::new("nmcli")
                            .args(["connection", "up", "id", &ssid])
                            .output();
                        if r.as_ref().map_or(true, |o| !o.status.success()) {
                            Command::new("nmcli")
                                .args(["device", "wifi", "connect", &ssid])
                                .output()
                        } else {
                            r
                        }
                    };

                    match result {
                        Ok(o) if o.status.success() => {
                            let _ = tx.send(WifiEvent::ConnectOk);
                        }
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr);
                            let msg = stderr
                                .lines()
                                .next()
                                .unwrap_or("Connection failed")
                                .to_string();
                            let _ = tx.send(WifiEvent::ConnectFail(msg));
                        }
                        Err(e) => {
                            let _ = tx.send(WifiEvent::ConnectFail(e.to_string()));
                        }
                    }
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                    last_poll = Instant::now();
                }
            }
        }

        // Periodic refresh.
        if last_poll.elapsed() >= POLL_INTERVAL {
            let _ = tx.send(WifiEvent::Status(poll_status()));
            let _ = tx.send(WifiEvent::Networks(scan_networks()));
            last_poll = Instant::now();
        }

        thread::sleep(Duration::from_millis(150));
    }
}

fn poll_status() -> WifiState {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
        .output();
    let Ok(out) = out else { return WifiState::Off };
    if !out.status.success() {
        return WifiState::Off;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut wifi_connected = false;
    let mut ssid = String::new();
    let mut has_wifi = false;
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.first().copied() == Some("wifi") {
            has_wifi = true;
            if parts.get(1).copied() == Some("connected") {
                wifi_connected = true;
                ssid = parts.get(2).copied().unwrap_or("").to_string();
            }
            break;
        }
    }
    if !has_wifi {
        return WifiState::Off;
    }
    if !wifi_connected {
        return WifiState::Disconnected;
    }
    let signal = signal_for_ssid(&ssid);
    WifiState::Connected { ssid, signal }
}

fn signal_for_ssid(ssid: &str) -> u32 {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "IN-USE,SSID,SIGNAL", "dev", "wifi", "list"])
        .output();
    let Ok(out) = out else { return 0 };
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Prefer the row with `*` (active BSSID); fall back to the first
    // row with a matching SSID otherwise.
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.first().copied() == Some("*") && parts.len() >= 3 {
            return parts[2].parse().unwrap_or(0);
        }
    }
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[1] == ssid {
            return parts[2].parse().unwrap_or(0);
        }
    }
    0
}

fn scan_networks() -> Vec<Network> {
    let out = Command::new("nmcli")
        .args([
            "-t", "-f", "SSID,SIGNAL,SECURITY,IN-USE",
            "dev", "wifi", "list",
        ])
        .output();
    let Ok(out) = out else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Saved connection names.
    let saved = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    let parts: Vec<&str> = l.split(':').collect();
                    if parts.len() >= 2 && parts[1] == "802-11-wireless" {
                        Some(parts[0].to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut nets = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        let ssid = parts[0].to_string();
        if ssid.is_empty() || !seen.insert(ssid.clone()) {
            continue;
        }
        let signal: u32 = parts[1].parse().unwrap_or(0);
        let security = parts[2].to_string();
        let in_use = parts[3] == "*";
        let saved = saved.iter().any(|n| n == &ssid);
        nets.push(Network { ssid, signal, security, in_use, saved });
    }

    // Sort: in-use first, then saved, then by signal desc.
    nets.sort_by(|a, b| {
        b.in_use
            .cmp(&a.in_use)
            .then(b.saved.cmp(&a.saved))
            .then(b.signal.cmp(&a.signal))
    });
    nets
}

// ── Inline tile ─────────────────────────────────────────────────────────────

const ICON_SIZE: f32 = 28.0;
const ICON_LEFT_PAD: f32 = 16.0;

pub const TILE_WIDTH: f32 = ICON_LEFT_PAD + ICON_SIZE;

pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    wifi: &Wifi,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    _surface_w: u32,
    _surface_h: u32,
) {
    if !wifi.is_present() {
        return;
    }
    let icon_size = ICON_SIZE * scale;
    let icon_x = layout.x + ICON_LEFT_PAD * scale;
    let icon_y = layout.y + (layout.h - icon_size) / 2.0;

    let bars = match wifi.state() {
        WifiState::Connected { signal, .. } => signal_to_bars(*signal),
        WifiState::Disconnected => 0,
        WifiState::Off => 0,
    };
    draw_signal_icon(painter, icon_x, icon_y, icon_size, icon_size, bars, alpha);
}

/// Convert a 0-100 signal value into a 0-3 bar count. 0 means "no
/// connection" (we draw a faded placeholder).
fn signal_to_bars(signal: u32) -> u32 {
    match signal {
        0 => 0,
        1..=33 => 1,
        34..=66 => 2,
        _ => 3,
    }
}

/// Draw a 3-bar signal indicator at the given rect. `bars` in 0..=3
/// controls how many of the three bars are filled (rest are dim).
fn draw_signal_icon(
    painter: &mut Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bars: u32,
    alpha: f32,
) {
    let on = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    let off = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.20 * alpha);

    // Three vertical bars, increasing height left → right.
    let gap = w * 0.10;
    let bar_w = (w - gap * 2.0) / 3.0;
    let bottom = y + h * 0.95;
    for (i, frac_h) in [0.45, 0.70, 0.95].iter().enumerate() {
        let bar_x = x + i as f32 * (bar_w + gap);
        let bar_h_actual = h * frac_h;
        let bar_y = bottom - bar_h_actual;
        let color = if (i as u32) < bars { on } else { off };
        painter.rect_filled(
            Rect::new(bar_x, bar_y, bar_w, bar_h_actual),
            bar_w * 0.25,
            color,
        );
    }
}

// ── Click-expand view ───────────────────────────────────────────────────────

const VIEW_TOP_PAD: f32 = 24.0;
const VIEW_HEADER_FONT: f32 = 22.0;
const VIEW_HEADER_BOTTOM_GAP: f32 = 12.0;
const ROW_HEIGHT: f32 = 56.0;
const ROW_FONT: f32 = 22.0;
const ROW_SIGNAL_SIZE: f32 = 28.0;
const ROW_SIGNAL_GAP: f32 = 16.0;
const ROW_LOCK_SIZE: f32 = 20.0;
const ROW_RIGHT_GAP: f32 = 12.0;
const MAX_NETWORK_ROWS: usize = 6;

/// Y-coordinate (physical px) of the first network row.
fn row_list_top_y(panel_top_y: f32, scale: f32) -> f32 {
    panel_top_y + VIEW_TOP_PAD * scale + VIEW_HEADER_FONT * scale + VIEW_HEADER_BOTTOM_GAP * scale
}

/// Hit-test a click against the network list. Returns the SSID at the
/// clicked row (cloned), if any.
pub fn hit_test_network(
    wifi: &Wifi,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    if x < inner_x || x > inner_x + inner_w {
        return None;
    }

    let row_h = ROW_HEIGHT * scale;
    let list_top = row_list_top_y(panel_top_y, scale);
    for (i, net) in wifi.networks().iter().take(MAX_NETWORK_ROWS).enumerate() {
        let row_y = list_top + i as f32 * row_h;
        if y >= row_y && y <= row_y + row_h {
            return Some(net.ssid.clone());
        }
    }
    None
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    wifi: &Wifi,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let header_font = VIEW_HEADER_FONT * scale;
    let header_gap = VIEW_HEADER_BOTTOM_GAP * scale;
    let row_h = ROW_HEIGHT * scale;
    let row_font = ROW_FONT * scale;
    let signal_size = ROW_SIGNAL_SIZE * scale;
    let signal_gap = ROW_SIGNAL_GAP * scale;
    let lock_size = ROW_LOCK_SIZE * scale;
    let right_gap = ROW_RIGHT_GAP * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

    // ── Header: "Wi-Fi" + connected SSID ──
    let header = match wifi.state() {
        WifiState::Connected { ssid, .. } => format!("Wi-Fi · {}", ssid),
        WifiState::Disconnected => "Wi-Fi · Disconnected".to_string(),
        WifiState::Off => "Wi-Fi · Off".to_string(),
    };
    let header_y = panel_top_y + VIEW_TOP_PAD * scale;
    text.queue(
        &header,
        header_font,
        inner_x,
        header_y,
        white.with_alpha(alpha),
        inner_w,
        surface_w,
        surface_h,
    );

    // ── Network rows ──
    let list_top = header_y + header_font + header_gap;
    let _ = list_top; // keep symmetry with the layout helper above

    if wifi.networks().is_empty() {
        let msg_y = row_list_top_y(panel_top_y, scale);
        text.queue(
            "Scanning…",
            row_font,
            inner_x,
            msg_y,
            muted,
            inner_w,
            surface_w,
            surface_h,
        );
        return msg_y + row_font;
    }

    for (i, net) in wifi.networks().iter().take(MAX_NETWORK_ROWS).enumerate() {
        let row_y = row_list_top_y(panel_top_y, scale) + i as f32 * row_h;
        let row_rect = Rect::new(inner_x, row_y, inner_w, row_h);

        // Subtle row-stripe for readability + brighter highlight on the
        // currently-connected row.
        if net.in_use {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                gold.with_alpha(0.18 * alpha),
            );
        } else if i % 2 == 0 {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                white.with_alpha(0.04 * alpha),
            );
        }

        // Signal icon (left).
        let icon_x = inner_x + signal_gap;
        let icon_y = row_y + (row_h - signal_size) / 2.0;
        let bars = signal_to_bars(net.signal);
        draw_signal_icon(painter, icon_x, icon_y, signal_size, signal_size, bars, alpha);

        // SSID label.
        let label_x = icon_x + signal_size + signal_gap;
        let label_y = row_y + (row_h - row_font) / 2.0;
        let ssid_color = if net.in_use { white.with_alpha(alpha) } else { white.with_alpha(0.86 * alpha) };
        text.queue(
            &net.ssid,
            row_font,
            label_x,
            label_y,
            ssid_color,
            inner_w * 0.7,
            surface_w,
            surface_h,
        );

        // Right side: lock for secured + status badge.
        let mut right_x = inner_x + inner_w - right_gap;
        if !net.in_use && net.saved {
            // "Saved" hint to the right of the lock — small muted text.
            let saved_text = "Saved";
            let saved_w = text.measure_width(saved_text, row_font * 0.7);
            right_x -= saved_w;
            text.queue(
                saved_text,
                row_font * 0.7,
                right_x,
                row_y + (row_h - row_font * 0.7) / 2.0,
                muted,
                saved_w,
                surface_w,
                surface_h,
            );
            right_x -= right_gap;
        }
        if net.in_use {
            let conn_text = "Connected";
            let conn_w = text.measure_width(conn_text, row_font * 0.8);
            right_x -= conn_w;
            text.queue(
                conn_text,
                row_font * 0.8,
                right_x,
                row_y + (row_h - row_font * 0.8) / 2.0,
                gold.with_alpha(alpha),
                conn_w,
                surface_w,
                surface_h,
            );
            right_x -= right_gap;
        }
        if !net.security.is_empty() && net.security != "--" {
            let lock_x = right_x - lock_size;
            let lock_y = row_y + (row_h - lock_size) / 2.0;
            draw_lock(painter, lock_x, lock_y, lock_size, lock_size, alpha);
            right_x -= lock_size + right_gap;
        }
        let _ = right_x;
    }

    if let Some(err) = wifi.last_error() {
        let err_y = row_list_top_y(panel_top_y, scale)
            + wifi.networks().len().min(MAX_NETWORK_ROWS) as f32 * row_h
            + row_font * 0.5;
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        text.queue(
            err,
            row_font * 0.85,
            inner_x,
            err_y,
            red,
            inner_w,
            surface_w,
            surface_h,
        );
    }

    let bottom = row_list_top_y(panel_top_y, scale)
        + wifi.networks().len().min(MAX_NETWORK_ROWS) as f32 * row_h;

    // Draw the password prompt on top of the network list when active.
    // Layer 1 so the modal's painter rects can occlude already-queued
    // text from the network list — see lntrn-render/TEXT_OCCLUSION_FIX.
    if let Some(prompt) = &wifi.prompt {
        painter.set_layer(1);
        text.set_layer(1);
        draw_modal(
            painter,
            text,
            prompt,
            panel,
            panel_top_y,
            scale,
            alpha,
            wifi.last_error(),
            surface_w,
            surface_h,
        );
    }

    bottom
}

// ── Password prompt modal ──────────────────────────────────────────────────

const MODAL_W: f32 = 480.0;
const MODAL_H: f32 = 240.0;
const MODAL_RADIUS: f32 = 16.0;
const MODAL_PAD: f32 = 24.0;
const MODAL_TITLE_FONT: f32 = 22.0;
const MODAL_FIELD_HEIGHT: f32 = 48.0;
const MODAL_FIELD_FONT: f32 = 22.0;
const MODAL_BUTTON_HEIGHT: f32 = 44.0;
const MODAL_BUTTON_GAP: f32 = 12.0;
const MODAL_BUTTON_FONT: f32 = 18.0;
const MODAL_BACKDROP_ALPHA: f32 = 0.55;

/// Region rects for the password modal — used both for drawing and
/// for routing clicks. All values in physical pixels.
pub struct ModalRegions {
    /// Backdrop — anything inside the panel but outside `box_rect`.
    /// Click here cancels the modal.
    pub backdrop: Rect,
    pub box_rect: Rect,
    pub field: Rect,
    pub connect_btn: Rect,
    pub cancel_btn: Rect,
}

/// Compute modal regions for the given panel + content top y.
pub fn modal_regions(panel: Rect, panel_top_y: f32, scale: f32) -> ModalRegions {
    let modal_w = MODAL_W * scale;
    let modal_h = MODAL_H * scale;
    let pad = MODAL_PAD * scale;
    let field_h = MODAL_FIELD_HEIGHT * scale;
    let title_font = MODAL_TITLE_FONT * scale;
    let btn_h = MODAL_BUTTON_HEIGHT * scale;
    let btn_gap = MODAL_BUTTON_GAP * scale;

    // Center the modal box horizontally inside the panel; vertically,
    // place it a bit below the controls row underline.
    let box_x = panel.x + (panel.w - modal_w) / 2.0;
    let box_y = panel_top_y + (panel.h - (panel_top_y - panel.y) - modal_h) * 0.30;
    let box_rect = Rect::new(box_x, box_y, modal_w, modal_h);

    // Field y = pad + title + breathing room.
    let field_y = box_y + pad + title_font + pad * 0.5;
    let field_rect = Rect::new(box_x + pad, field_y, modal_w - pad * 2.0, field_h);

    // Buttons at the bottom — Cancel left, Connect right (each ~half
    // width minus gap).
    let buttons_y = box_y + modal_h - pad - btn_h;
    let half_w = (modal_w - pad * 2.0 - btn_gap) / 2.0;
    let cancel_btn = Rect::new(box_x + pad, buttons_y, half_w, btn_h);
    let connect_btn = Rect::new(box_x + pad + half_w + btn_gap, buttons_y, half_w, btn_h);

    let backdrop = Rect::new(panel.x, panel_top_y, panel.w, panel.h - (panel_top_y - panel.y));

    ModalRegions { backdrop, box_rect, field: field_rect, connect_btn, cancel_btn }
}

/// Hit-test a click against the modal. Returns which region was hit.
pub fn hit_test_modal(panel: Rect, panel_top_y: f32, scale: f32, x: f32, y: f32) -> ModalHit {
    let r = modal_regions(panel, panel_top_y, scale);
    let inside = |rect: Rect| {
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    };
    if inside(r.connect_btn) {
        ModalHit::Connect
    } else if inside(r.cancel_btn) {
        ModalHit::Cancel
    } else if inside(r.field) {
        ModalHit::Field
    } else if inside(r.box_rect) {
        ModalHit::Box
    } else {
        ModalHit::Backdrop
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalHit {
    Connect,
    Cancel,
    Field,
    /// Inside the modal box but not on a specific control (no-op).
    Box,
    /// Inside the panel but outside the modal — cancel.
    Backdrop,
}

pub fn draw_modal(
    painter: &mut Painter,
    text: &mut TextRenderer,
    prompt: &PasswordPrompt,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    last_error: Option<&str>,
    surface_w: u32,
    surface_h: u32,
) {
    let r = modal_regions(panel, panel_top_y, scale);
    let pad = MODAL_PAD * scale;
    let title_font = MODAL_TITLE_FONT * scale;
    let field_font = MODAL_FIELD_FONT * scale;
    let btn_font = MODAL_BUTTON_FONT * scale;
    let radius = MODAL_RADIUS * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let muted = white.with_alpha(0.55 * alpha);

    // Backdrop dim — covers the network list behind the modal.
    painter.rect_filled(r.backdrop, 0.0, Color::BLACK.with_alpha(MODAL_BACKDROP_ALPHA * alpha));

    // Modal box.
    painter.rect_filled(
        r.box_rect,
        radius,
        Color::from_rgb8(0x24, 0x24, 0x24).with_alpha(alpha),
    );

    // Title: "Password for {SSID}".
    let title = format!("Password for {}", prompt.ssid);
    text.queue(
        &title,
        title_font,
        r.box_rect.x + pad,
        r.box_rect.y + pad,
        white.with_alpha(alpha),
        r.box_rect.w - pad * 2.0,
        surface_w,
        surface_h,
    );

    // Field background.
    painter.rect_filled(
        r.field,
        8.0 * scale,
        Color::from_rgb8(0x14, 0x14, 0x14).with_alpha(alpha),
    );
    painter.rect_stroke_sdf(
        r.field,
        8.0 * scale,
        1.5 * scale,
        white.with_alpha(0.18 * alpha),
    );

    // Masked password.
    let mask_count = prompt.input.query().chars().count();
    let mask = "•".repeat(mask_count);
    let display: &str = if mask_count == 0 { "Enter password" } else { &mask };
    let display_color = if mask_count == 0 { muted } else { white.with_alpha(alpha) };
    let field_text_x = r.field.x + 12.0 * scale;
    let field_text_y = r.field.y + (r.field.h - field_font) / 2.0;
    text.queue(
        display,
        field_font,
        field_text_x,
        field_text_y,
        display_color,
        r.field.w - 24.0 * scale,
        surface_w,
        surface_h,
    );

    // Cursor — only when the field has content (so it doesn't overlap
    // the placeholder).
    if mask_count > 0 && prompt.input.cursor_visible() {
        // Each `•` we emit advances by mask_w / mask_count, but since
        // glyph widths can vary slightly, measure the prefix directly.
        // Cursor lies on a char boundary in `prompt.input`'s real buffer;
        // count chars up to that point and use that many bullets.
        let cursor_char_idx = prompt
            .input
            .query()
            .char_indices()
            .take_while(|(b, _)| *b < prompt.input.cursor_byte())
            .count();
        let prefix: String = std::iter::repeat('•').take(cursor_char_idx).collect();
        let prefix_w = text.measure_width(&prefix, field_font);
        let cx = field_text_x + prefix_w;
        let cy = field_text_y - 2.0 * scale;
        let ch = field_font + 4.0 * scale;
        painter.rect_filled(
            Rect::new(cx, cy, 2.0 * scale, ch),
            1.0 * scale,
            gold.with_alpha(alpha),
        );
    }

    // Cancel button (subtle outline).
    painter.rect_filled(
        r.cancel_btn,
        8.0 * scale,
        white.with_alpha(0.06 * alpha),
    );
    let cancel_label = "Cancel";
    let cancel_w = text.measure_width(cancel_label, btn_font);
    text.queue(
        cancel_label,
        btn_font,
        r.cancel_btn.x + (r.cancel_btn.w - cancel_w) / 2.0,
        r.cancel_btn.y + (r.cancel_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        cancel_w,
        surface_w,
        surface_h,
    );

    // Connect button (gold). Dim when connecting or empty.
    let can_submit = !prompt.input.query().is_empty() && !prompt.connecting;
    let conn_alpha = if can_submit { alpha } else { 0.5 * alpha };
    painter.rect_filled(
        r.connect_btn,
        8.0 * scale,
        gold.with_alpha(conn_alpha),
    );
    let connect_label = if prompt.connecting { "Connecting…" } else { "Connect" };
    let connect_w = text.measure_width(connect_label, btn_font);
    text.queue(
        connect_label,
        btn_font,
        r.connect_btn.x + (r.connect_btn.w - connect_w) / 2.0,
        r.connect_btn.y + (r.connect_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        connect_w,
        surface_w,
        surface_h,
    );

    // Error below the buttons (last attempt failed).
    if let Some(err) = last_error {
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        text.queue(
            err,
            btn_font * 0.85,
            r.box_rect.x + pad,
            r.cancel_btn.y - btn_font - 4.0 * scale,
            red,
            r.box_rect.w - pad * 2.0,
            surface_w,
            surface_h,
        );
    }
}

/// Tiny lock icon — body rect + shackle arch. Pure shapes.
fn draw_lock(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.65 * alpha);
    // Body: bottom 60 % of the box, full width.
    let body_y = y + h * 0.40;
    let body_h = h * 0.60;
    painter.rect_filled(
        Rect::new(x, body_y, w, body_h),
        w * 0.16,
        color,
    );
    // Shackle: U-shape on top, drawn as a stroked rounded rect with the
    // bottom hidden behind the body. We approximate by drawing a rect
    // outline above the body.
    let shackle_x = x + w * 0.18;
    let shackle_w = w * 0.64;
    let shackle_h = h * 0.50;
    painter.rect_stroke_sdf(
        Rect::new(shackle_x, y, shackle_w, shackle_h),
        shackle_w * 0.45,
        w * 0.13,
        color,
    );
}
