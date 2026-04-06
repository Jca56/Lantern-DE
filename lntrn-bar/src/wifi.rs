//! WiFi widget — icon in bar + popup with network list and password entry.
//! All nmcli interaction runs in a background thread.

use std::process::Command;
use std::sync::mpsc;

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, TextInput};

use crate::svg_icon::IconCache;

fn icon_dir() -> std::path::PathBuf { crate::lantern_icons_dir() }
const POLL_INTERVAL_MS: u64 = 10_000;

// Zone IDs (unique range 0xFF_xxxx)
pub const ZONE_WIFI_ICON: u32 = 0xFF_0000;
const ZONE_NETWORK_BASE: u32 = 0xFF_0100;
const ZONE_CONNECT_BTN: u32 = 0xFF_1000;
const ZONE_PASSWORD: u32 = 0xFF_1001;
const ZONE_REFRESH_BTN: u32 = 0xFF_1002;

// Key constants (evdev)
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum WifiState {
    Connected { ssid: String, signal: u32 },
    Disconnected,
    Off,
}

#[derive(Debug, Clone)]
pub struct NetworkInfo {
    pub ssid: String,
    pub signal: u32,
    pub security: String,
    pub in_use: bool,
    pub saved: bool,
}

/// Commands sent from render thread → background thread.
enum WifiCmd {
    Scan,
    Connect { ssid: String, password: Option<String> },
}

/// Events sent from background thread → render thread.
enum WifiEvent {
    Status(WifiState),
    Networks(Vec<NetworkInfo>),
    ConnectOk,
    ConnectFail(String),
}

// ── Widget ──────────────────────────────────────────────────────────────────

pub struct Wifi {
    state: WifiState,
    event_rx: mpsc::Receiver<WifiEvent>,
    cmd_tx: mpsc::Sender<WifiCmd>,
    icons_loaded: bool,
    // Popup state
    pub open: bool,
    networks: Vec<NetworkInfo>,
    selected_ssid: Option<String>,
    password_buf: String,
    pub password_focused: bool,
    cursor_pos: usize,
    connecting: bool,
    connect_error: Option<String>,
    scroll_offset: f32,
}

impl Wifi {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("wifi-poll".into())
            .spawn(move || poll_thread(event_tx, cmd_rx))
            .expect("spawn wifi poll thread");

        Self {
            state: WifiState::Off,
            event_rx,
            cmd_tx,
            icons_loaded: false,
            open: false,
            networks: Vec::new(),
            selected_ssid: None,
            password_buf: String::new(),
            password_focused: false,
            cursor_pos: 0,
            connecting: false,
            connect_error: None,
            scroll_offset: 0.0,
        }
    }

    /// Drain background events. Returns `true` if any event was received.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.event_rx.try_recv() {
            changed = true;
            match event {
                WifiEvent::Status(s) => self.state = s,
                WifiEvent::Networks(n) => self.networks = n,
                WifiEvent::ConnectOk => {
                    self.connecting = false;
                    self.connect_error = None;
                    self.selected_ssid = None;
                    self.password_buf.clear();
                    self.password_focused = false;
                    self.cursor_pos = 0;
                }
                WifiEvent::ConnectFail(e) => {
                    self.connecting = false;
                    self.connect_error = Some(e);
                }
            }
        }
        changed
    }

    /// Request a rescan when opening the popup.
    pub fn request_scan(&self) {
        let _ = self.cmd_tx.send(WifiCmd::Scan);
    }

    /// Attempt to connect to a network.
    fn connect(&mut self, ssid: &str, password: Option<String>) {
        self.connecting = true;
        self.connect_error = None;
        let _ = self.cmd_tx.send(WifiCmd::Connect {
            ssid: ssid.to_string(),
            password,
        });
    }

    /// Handle a click on a network item.
    pub fn handle_network_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        if self.connecting { return; }
        if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
            if zone >= ZONE_NETWORK_BASE && zone < ZONE_NETWORK_BASE + 256 {
                let idx = (zone - ZONE_NETWORK_BASE) as usize;
                if let Some(net) = self.networks.get(idx) {
                    let ssid = net.ssid.clone();
                    let is_open = net.security.is_empty() || net.security == "--";
                    if net.saved || is_open {
                        self.connect(&ssid, None);
                        self.selected_ssid = Some(ssid);
                    } else {
                        // Needs password — show input
                        self.selected_ssid = Some(ssid);
                        self.password_buf.clear();
                        self.cursor_pos = 0;
                        self.password_focused = true;
                        self.connect_error = None;
                    }
                }
            } else if zone == ZONE_REFRESH_BTN {
                self.request_scan();
            } else if zone == ZONE_CONNECT_BTN {
                if let Some(ssid) = self.selected_ssid.clone() {
                    let pw = if self.password_buf.is_empty() { None } else { Some(self.password_buf.clone()) };
                    self.connect(&ssid, pw);
                }
            }
        }
    }

    /// Handle keyboard input when password field is focused.
    pub fn on_key(&mut self, key: u32, shift: bool) {
        if !self.password_focused { return; }
        match key {
            KEY_ESC => {
                self.password_focused = false;
                self.selected_ssid = None;
                self.password_buf.clear();
                self.cursor_pos = 0;
            }
            KEY_BACKSPACE => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.password_buf.remove(self.cursor_pos);
                }
            }
            KEY_ENTER => {
                if let Some(ssid) = self.selected_ssid.clone() {
                    let pw = if self.password_buf.is_empty() { None } else { Some(self.password_buf.clone()) };
                    self.connect(&ssid, pw);
                }
            }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    self.password_buf.insert(self.cursor_pos, ch);
                    self.cursor_pos += 1;
                }
            }
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if !self.open { return; }
        let max_visible = 6;
        let max_scroll = self.networks.len().saturating_sub(max_visible) as f32;
        self.scroll_offset = (self.scroll_offset + delta * 0.35).clamp(0.0, max_scroll);
    }

    /// Whether the password field wants keyboard input.
    pub fn wants_keyboard(&self) -> bool {
        self.open && self.password_focused
    }

    fn icon_key(&self) -> &'static str {
        match &self.state {
            WifiState::Off | WifiState::Disconnected => "wifi-off",
            WifiState::Connected { signal, .. } => match *signal {
                0..=33 => "wifi-low",
                34..=66 => "wifi-medium",
                _ => "wifi-high",
            },
        }
    }

    pub fn load_icons(
        &mut self, icons: &mut IconCache, tex_pass: &TexturePass, gpu: &GpuContext, size: u32,
    ) {
        if self.icons_loaded { return; }
        let dir = icon_dir();
        for (key, file) in [
            ("wifi-high", "spark-wifi-high.svg"),
            ("wifi-medium", "spark-wifi-medium.svg"),
            ("wifi-low", "spark-wifi-low.svg"),
            ("wifi-off", "spark-wifi-off.svg"),
        ] {
            icons.load(tex_pass, gpu, key, &dir.join(file), size, size);
        }
        self.icons_loaded = true;
    }

    pub fn measure(&self, bar_h: f32, scale: f32) -> f32 {
        let pad = 9.0 * scale;
        (bar_h - pad * 2.0).max(16.0)
    }

    /// Draw the bar icon. Returns (width, texture draws).
    pub fn draw<'a>(
        &self, _painter: &mut Painter, _text: &mut TextRenderer,
        ix: &mut InteractionContext, icons: &'a IconCache, _palette: &FoxPalette,
        x: f32, bar_y: f32, bar_h: f32, scale: f32, _screen_w: u32, _screen_h: u32,
    ) -> (f32, Vec<TextureDraw<'a>>) {
        let pad = 9.0 * scale;
        let icon_size = (bar_h - pad * 2.0).max(16.0);
        let icon_y = bar_y + pad;

        let mut tex_draws = Vec::new();
        if let Some(tex) = icons.get(self.icon_key()) {
            tex_draws.push(TextureDraw::new(tex, x, icon_y, icon_size, icon_size));
        }
        ix.add_zone(ZONE_WIFI_ICON, Rect::new(x, icon_y, icon_size, icon_size));
        (icon_size, tex_draws)
    }

    /// Draw the popup above/below the bar.
    pub fn draw_popup(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        wifi_x: f32, wifi_w: f32, bar_y: f32, bar_h: f32, position_top: bool, scale: f32,
        screen_w: u32, screen_h: u32,
    ) {
        if !self.open { return; }

        let pad = 20.0 * scale;
        let corner_r = 12.0 * scale;
        let gap = 8.0 * scale;
        let popup_w = 380.0 * scale;
        let title_font = 24.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let row_h = 48.0 * scale;
        let section_gap = 12.0 * scale;

        // Compute popup height
        let max_visible = 6;
        let net_count = self.networks.len().min(max_visible);
        let mut content_h = title_font + section_gap;
        content_h += net_count as f32 * row_h;
        if self.networks.len() > max_visible {
            content_h += small_font; // "scroll for more"
        }
        // Password section
        if self.selected_ssid.is_some() && self.password_focused {
            content_h += section_gap + 44.0 * scale + section_gap + 40.0 * scale;
        }
        // Connecting / error
        if self.connecting {
            content_h += section_gap + body_font;
        }
        if self.connect_error.is_some() {
            content_h += section_gap + small_font;
        }

        let popup_h = pad * 2.0 + content_h;
        let popup_x = (wifi_x + wifi_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = if position_top {
            bar_y + bar_h + gap
        } else {
            (bar_y - popup_h - gap).max(0.0)
        };

        // Shadow
        let shadow_expand = 3.0 * scale;
        painter.rect_filled(
            Rect::new(popup_x - shadow_expand, popup_y + shadow_expand,
                popup_w + shadow_expand * 2.0, popup_h + shadow_expand),
            corner_r + 2.0, Color::BLACK.with_alpha(0.35),
        );
        // Background
        let bg = Rect::new(popup_x, popup_y, popup_w, popup_h);
        painter.rect_filled(bg, corner_r, palette.bg);
        painter.rect_stroke_sdf(bg, corner_r, 1.0 * scale, Color::WHITE.with_alpha(0.08));

        let cx = popup_x + pad;
        let cw = popup_w - pad * 2.0;
        let mut y = popup_y + pad;

        // Title + refresh button
        let title = match &self.state {
            WifiState::Connected { ssid, .. } => format!("Wi-Fi — {ssid}"),
            WifiState::Disconnected => "Wi-Fi — Disconnected".to_string(),
            WifiState::Off => "Wi-Fi — Off".to_string(),
        };
        text.queue(&title, title_font, cx, y, palette.text, cw - title_font - 8.0 * scale, screen_w, screen_h);

        // Refresh button (right side of title row)
        let refresh_size = title_font;
        let refresh_x = cx + cw - refresh_size;
        let refresh_y = y;
        let refresh_rect = Rect::new(refresh_x - 4.0 * scale, refresh_y - 2.0 * scale,
            refresh_size + 8.0 * scale, refresh_size + 4.0 * scale);
        let refresh_state = ix.add_zone(ZONE_REFRESH_BTN, refresh_rect);
        if refresh_state.is_hovered() {
            painter.rect_filled(refresh_rect, 6.0 * scale, palette.muted.with_alpha(0.2));
        }
        text.queue("⟳", refresh_size, refresh_x, refresh_y, palette.text_secondary,
            refresh_size, screen_w, screen_h);

        y += title_font + section_gap;

        // Network list
        let scroll = self.scroll_offset as usize;
        let visible_nets: Vec<(usize, &NetworkInfo)> = self.networks.iter()
            .enumerate()
            .skip(scroll)
            .take(max_visible)
            .collect();

        for (orig_idx, net) in &visible_nets {
            let row_rect = Rect::new(cx, y, cw, row_h);
            let zone_id = ZONE_NETWORK_BASE + *orig_idx as u32;
            let state = ix.add_zone(zone_id, row_rect);
            let hovered = state.is_hovered();

            // Row background on hover
            if hovered {
                painter.rect_filled(row_rect, 8.0 * scale, palette.muted.with_alpha(0.2));
            }
            // Selected highlight
            if self.selected_ssid.as_deref() == Some(&net.ssid) {
                painter.rect_filled(row_rect, 8.0 * scale, palette.accent.with_alpha(0.15));
            }

            let text_y = y + (row_h - body_font) / 2.0;

            // Connected checkmark
            let mut label_x = cx + 8.0 * scale;
            if net.in_use {
                text.queue("✓", body_font, label_x, text_y, palette.accent, body_font, screen_w, screen_h);
                label_x += body_font + 4.0 * scale;
            }

            // SSID
            let ssid_color = if net.in_use { palette.text } else { palette.text_secondary };
            text.queue(&net.ssid, body_font, label_x, text_y, ssid_color, cw * 0.6, screen_w, screen_h);

            // Right side: signal + security
            let right_x = cx + cw - 100.0 * scale;
            let signal_text = format!("{}%", net.signal);
            text.queue(&signal_text, small_font, right_x, text_y + 2.0 * scale,
                palette.muted, 50.0 * scale, screen_w, screen_h);

            if !net.security.is_empty() && net.security != "--" {
                let lock_x = right_x + 52.0 * scale;
                text.queue("🔒", small_font, lock_x, text_y + 2.0 * scale,
                    palette.muted, 30.0 * scale, screen_w, screen_h);
            }

            if net.saved {
                let saved_x = right_x + 76.0 * scale;
                text.queue("★", small_font, saved_x, text_y + 2.0 * scale,
                    palette.accent, 20.0 * scale, screen_w, screen_h);
            }

            y += row_h;
        }

        if self.networks.len() > max_visible {
            text.queue("scroll for more…", small_font, cx, y,
                palette.muted, cw, screen_w, screen_h);
            y += small_font;
        }

        // Password input section
        if self.selected_ssid.is_some() && self.password_focused {
            y += section_gap;
            // Separator
            painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            let input_h = 44.0 * scale;
            let input_rect = Rect::new(cx, y, cw, input_h);
            ix.add_zone(ZONE_PASSWORD, input_rect);

            // Mask password with dots
            let masked: String = "●".repeat(self.password_buf.len());
            TextInput::new(input_rect)
                .text(&masked)
                .placeholder("Password")
                .focused(self.password_focused)
                .scale(scale)
                .cursor_pos(self.cursor_pos)
                .draw(painter, text, palette, screen_w, screen_h);

            y += input_h + section_gap;

            // Connect button
            let btn_h = 40.0 * scale;
            let btn_rect = Rect::new(cx, y, cw, btn_h);
            let btn_state = ix.add_zone(ZONE_CONNECT_BTN, btn_rect);
            let btn_hovered = btn_state.is_hovered();

            let btn_color = if btn_hovered { palette.accent } else { palette.accent.with_alpha(0.8) };
            painter.rect_filled(btn_rect, 8.0 * scale, btn_color);
            let btn_text_y = y + (btn_h - body_font) / 2.0;
            text.queue("Connect", body_font, cx + cw / 2.0 - 40.0 * scale, btn_text_y,
                palette.text, cw, screen_w, screen_h);
        }

        // Connecting indicator
        if self.connecting {
            y += section_gap;
            text.queue("Connecting…", body_font, cx, y, palette.accent, cw, screen_w, screen_h);
        }

        // Error message
        if let Some(ref err) = self.connect_error {
            y += section_gap;
            text.queue(err, small_font, cx, y, palette.danger, cw, screen_w, screen_h);
        }
    }

    /// Popup bounding rect for click-outside detection.
    pub fn popup_rect(
        &self, wifi_x: f32, wifi_w: f32, bar_y: f32, bar_h: f32, position_top: bool, scale: f32, screen_w: u32,
    ) -> Option<Rect> {
        if !self.open { return None; }

        let pad = 20.0 * scale;
        let gap = 8.0 * scale;
        let popup_w = 380.0 * scale;
        let title_font = 24.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let row_h = 48.0 * scale;
        let section_gap = 12.0 * scale;

        let max_visible = 6;
        let net_count = self.networks.len().min(max_visible);
        let mut content_h = title_font + section_gap;
        content_h += net_count as f32 * row_h;
        if self.networks.len() > max_visible {
            content_h += small_font;
        }
        if self.selected_ssid.is_some() && self.password_focused {
            content_h += section_gap + 44.0 * scale + section_gap + 40.0 * scale;
        }
        if self.connecting { content_h += section_gap + body_font; }
        if self.connect_error.is_some() { content_h += section_gap + small_font; }

        let popup_h = pad * 2.0 + content_h;
        let popup_x = (wifi_x + wifi_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = if position_top {
            bar_y + bar_h + gap
        } else {
            (bar_y - popup_h - gap).max(0.0)
        };

        Some(Rect::new(popup_x, popup_y, popup_w, popup_h))
    }
}

// ── Keycode → char (evdev) ──────────────────────────────────────────────────

fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    let ch = match key {
        2..=11 => {
            let base = b"1234567890"[(key - 2) as usize];
            if shift { b"!@#$%^&*()"[(key - 2) as usize] } else { base }
        }
        12 => if shift { b'_' } else { b'-' },
        13 => if shift { b'+' } else { b'=' },
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        26 => if shift { b'{' } else { b'[' },
        27 => if shift { b'}' } else { b']' },
        39 => if shift { b':' } else { b';' },
        40 => if shift { b'"' } else { b'\'' },
        41 => if shift { b'~' } else { b'`' },
        43 => if shift { b'|' } else { b'\\' },
        51 => if shift { b'<' } else { b',' },
        52 => if shift { b'>' } else { b'.' },
        53 => if shift { b'?' } else { b'/' },
        57 => b' ',
        _ => return None,
    };
    Some(ch as char)
}

// ── Background thread ───────────────────────────────────────────────────────

fn poll_thread(tx: mpsc::Sender<WifiEvent>, cmd_rx: mpsc::Receiver<WifiCmd>) {
    // Initial poll
    let _ = tx.send(WifiEvent::Status(poll_status()));
    let _ = tx.send(WifiEvent::Networks(scan_networks()));

    let mut last_poll = std::time::Instant::now();

    loop {
        // Process commands (non-blocking)
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                WifiCmd::Scan => {
                    let _ = Command::new("nmcli").args(["dev", "wifi", "rescan"]).output();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                }
                WifiCmd::Connect { ssid, password } => {
                    let result = if let Some(pw) = password {
                        Command::new("nmcli")
                            .args(["device", "wifi", "connect", &ssid, "password", &pw])
                            .output()
                    } else {
                        // Try saved connection first, fall back to open connect
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
                        Ok(output) if output.status.success() => {
                            let _ = tx.send(WifiEvent::ConnectOk);
                            let _ = tx.send(WifiEvent::Status(poll_status()));
                            let _ = tx.send(WifiEvent::Networks(scan_networks()));
                        }
                        Ok(output) => {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            let msg = stderr.lines().next().unwrap_or("Connection failed").to_string();
                            let _ = tx.send(WifiEvent::ConnectFail(msg));
                        }
                        Err(e) => {
                            let _ = tx.send(WifiEvent::ConnectFail(e.to_string()));
                        }
                    }
                }
            }
        }

        // Periodic status poll
        if last_poll.elapsed().as_millis() >= POLL_INTERVAL_MS as u128 {
            let _ = tx.send(WifiEvent::Status(poll_status()));
            last_poll = std::time::Instant::now();
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn poll_status() -> WifiState {
    let output = Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
        .output();
    let Ok(output) = output else { return WifiState::Off };
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut wifi_connected = false;
    let mut ssid = String::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[0] == "wifi" {
            if parts[1] == "connected" {
                wifi_connected = true;
                ssid = parts[2].to_string();
            }
            break;
        }
    }
    if !wifi_connected {
        let has_wifi = stdout.lines().any(|l| l.starts_with("wifi:"));
        return if has_wifi { WifiState::Disconnected } else { WifiState::Off };
    }
    let signal = get_signal_strength(&ssid);
    WifiState::Connected { ssid, signal }
}

fn get_signal_strength(ssid: &str) -> u32 {
    let output = Command::new("nmcli")
        .args(["-t", "-f", "SSID,SIGNAL,IN-USE", "dev", "wifi", "list"])
        .output();
    let Ok(output) = output else { return 0 };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[2] == "*" {
            return parts[1].parse().unwrap_or(0);
        }
    }
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 2 && parts[0] == ssid {
            return parts[1].parse().unwrap_or(0);
        }
    }
    0
}

fn scan_networks() -> Vec<NetworkInfo> {
    // Get available networks
    let output = Command::new("nmcli")
        .args(["-t", "-f", "SSID,SIGNAL,SECURITY,IN-USE", "dev", "wifi", "list"])
        .output();
    let Ok(output) = output else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Get saved connections
    let saved_output = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
        .output();
    let saved_names: Vec<String> = saved_output
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
                .collect()
        })
        .unwrap_or_default();

    let mut networks = Vec::new();
    let mut seen_ssids = std::collections::HashSet::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 { continue; }
        let ssid = parts[0].to_string();
        if ssid.is_empty() || !seen_ssids.insert(ssid.clone()) { continue; }

        let signal: u32 = parts[1].parse().unwrap_or(0);
        let security = parts[2].to_string();
        let in_use = parts[3] == "*";
        let saved = saved_names.iter().any(|n| n == &ssid);

        networks.push(NetworkInfo { ssid, signal, security, in_use, saved });
    }

    // Sort: in-use first, then saved, then by signal strength
    networks.sort_by(|a, b| {
        b.in_use.cmp(&a.in_use)
            .then(b.saved.cmp(&a.saved))
            .then(b.signal.cmp(&a.signal))
    });

    networks
}
