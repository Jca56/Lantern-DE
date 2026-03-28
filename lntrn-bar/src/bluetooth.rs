//! Bluetooth widget — icon in bar + popup for device management.
//! All bluetoothctl interaction runs in a background thread.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Toggle};

use crate::svg_icon::IconCache;

const ICON_DIR: &str = "/home/alva/.config/lntrn-bar/icons";
const POLL_INTERVAL_MS: u64 = 10_000;

// Zone IDs
pub const ZONE_BT_ICON: u32 = 0xDD_0000;
const ZONE_BT_POWER: u32 = 0xDD_0001;
const ZONE_BT_SCAN: u32 = 0xDD_0002;
const ZONE_DEVICE_BASE: u32 = 0xDD_0100;
const ZONE_DEVICE_REMOVE: u32 = 0xDD_0300;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    pub battery: Option<u8>,
}

enum BtCmd {
    Scan,
    Connect(String),
    Disconnect(String),
    Pair(String),
    Remove(String),
    SetPower(bool),
}

enum BtEvent {
    Status { powered: bool, devices: Vec<BtDevice> },
    Discovered(Vec<BtDevice>),
    ActionOk,
    ActionFail(String),
    ScanDone,
}

// ── Widget ──────────────────────────────────────────────────────────────────

pub struct Bluetooth {
    powered: bool,
    devices: Vec<BtDevice>,
    discovered: Vec<BtDevice>,
    event_rx: mpsc::Receiver<BtEvent>,
    cmd_tx: mpsc::Sender<BtCmd>,
    icons_loaded: bool,
    // Popup
    pub open: bool,
    scanning: bool,
    connecting_mac: Option<String>,
    connect_error: Option<String>,
    scroll_offset: f32,
    power_anim: f32,
}

impl Bluetooth {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("bt-poll".into())
            .spawn(move || poll_thread(event_tx, cmd_rx))
            .expect("spawn bt poll thread");

        Self {
            powered: false,
            devices: Vec::new(),
            discovered: Vec::new(),
            event_rx,
            cmd_tx,
            icons_loaded: false,
            open: false,
            scanning: false,
            connecting_mac: None,
            connect_error: None,
            scroll_offset: 0.0,
            power_anim: 0.0,
        }
    }

    pub fn tick(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                BtEvent::Status { powered, devices } => {
                    self.powered = powered;
                    self.devices = devices;
                }
                BtEvent::Discovered(devs) => {
                    // Merge discovered — skip devices already in paired list
                    for d in devs {
                        if !self.devices.iter().any(|e| e.mac == d.mac)
                            && !self.discovered.iter().any(|e| e.mac == d.mac)
                        {
                            self.discovered.push(d);
                        }
                    }
                }
                BtEvent::ActionOk => {
                    self.connecting_mac = None;
                    self.connect_error = None;
                }
                BtEvent::ActionFail(e) => {
                    self.connecting_mac = None;
                    self.connect_error = Some(e);
                }
                BtEvent::ScanDone => {
                    self.scanning = false;
                }
            }
        }
    }

    pub fn update_anim(&mut self, dt: f32) -> bool {
        let target = if self.powered { 1.0 } else { 0.0 };
        if (self.power_anim - target).abs() < 0.001 {
            self.power_anim = target;
            return false;
        }
        let step = dt / 0.2;
        if self.power_anim < target {
            self.power_anim = (self.power_anim + step).min(1.0);
        } else {
            self.power_anim = (self.power_anim - step).max(0.0);
        }
        true
    }

    pub fn request_scan(&self) {
        self.discovered.len(); // keep borrow checker happy — clear on open
        let _ = self.cmd_tx.send(BtCmd::Scan);
    }

    pub fn handle_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
            if zone == ZONE_BT_POWER {
                let _ = self.cmd_tx.send(BtCmd::SetPower(!self.powered));
            } else if zone == ZONE_BT_SCAN {
                if !self.scanning {
                    self.scanning = true;
                    self.discovered.clear();
                    let _ = self.cmd_tx.send(BtCmd::Scan);
                }
            } else if zone >= ZONE_DEVICE_REMOVE && zone < ZONE_DEVICE_REMOVE + 256 {
                let idx = (zone - ZONE_DEVICE_REMOVE) as usize;
                let mac = self.all_devices().get(idx).map(|d| d.mac.clone());
                if let Some(mac) = mac {
                    let _ = self.cmd_tx.send(BtCmd::Remove(mac));
                }
            } else if zone >= ZONE_DEVICE_BASE && zone < ZONE_DEVICE_BASE + 256 {
                let idx = (zone - ZONE_DEVICE_BASE) as usize;
                let all = self.all_devices();
                if let Some(dev) = all.get(idx) {
                    let mac = dev.mac.clone();
                    self.connecting_mac = Some(mac.clone());
                    self.connect_error = None;
                    if dev.connected {
                        let _ = self.cmd_tx.send(BtCmd::Disconnect(mac));
                    } else if dev.paired {
                        let _ = self.cmd_tx.send(BtCmd::Connect(mac));
                    } else {
                        let _ = self.cmd_tx.send(BtCmd::Pair(mac));
                    }
                }
            }
        }
    }

    fn all_devices(&self) -> Vec<BtDevice> {
        let mut all = self.devices.clone();
        all.extend(self.discovered.clone());
        all
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if !self.open { return; }
        self.scroll_offset = (self.scroll_offset + delta).max(0.0);
    }

    fn icon_key(&self) -> &'static str {
        if !self.powered { return "bt-off"; }
        if self.devices.iter().any(|d| d.connected) { "bt-connected" } else { "bt-on" }
    }

    pub fn load_icons(
        &mut self, icons: &mut IconCache, tex_pass: &TexturePass, gpu: &GpuContext, size: u32,
    ) {
        if self.icons_loaded { return; }
        let dir = Path::new(ICON_DIR);
        for (key, file) in [
            ("bt-on", "spark-bluetooth-on.svg"),
            ("bt-off", "spark-bluetooth-off.svg"),
            ("bt-connected", "spark-bluetooth-connected.svg"),
        ] {
            icons.load(tex_pass, gpu, key, &dir.join(file), size, size);
        }
        self.icons_loaded = true;
    }

    pub fn measure(&self, bar_h: f32, scale: f32) -> f32 {
        let pad = 5.0 * scale;
        (bar_h - pad * 2.0).max(16.0)
    }

    pub fn draw<'a>(
        &self, _painter: &mut Painter, _text: &mut TextRenderer,
        ix: &mut InteractionContext, icons: &'a IconCache, _palette: &FoxPalette,
        x: f32, bar_y: f32, bar_h: f32, scale: f32, _screen_w: u32, _screen_h: u32,
    ) -> (f32, Vec<TextureDraw<'a>>) {
        let pad = 5.0 * scale;
        let icon_size = (bar_h - pad * 2.0).max(16.0);
        let icon_y = bar_y + pad;
        let mut tex_draws = Vec::new();
        if let Some(tex) = icons.get(self.icon_key()) {
            tex_draws.push(TextureDraw::new(tex, x, icon_y, icon_size, icon_size));
        }
        ix.add_zone(ZONE_BT_ICON, Rect::new(x, icon_y, icon_size, icon_size));
        (icon_size, tex_draws)
    }

    pub fn draw_popup(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        bt_x: f32, bt_w: f32, bar_y: f32, scale: f32,
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
        let toggle_h = 28.0 * scale;

        // Content height
        let all_devs = self.all_devices();
        let max_visible = 6;
        let dev_count = all_devs.len().min(max_visible);
        let mut content_h = title_font.max(toggle_h) + section_gap; // title + power toggle
        content_h += 36.0 * scale + section_gap; // scan button
        content_h += 1.0 * scale + section_gap; // separator
        content_h += dev_count as f32 * row_h;
        if all_devs.len() > max_visible { content_h += small_font; }
        if self.scanning { content_h += section_gap + body_font; }
        if self.connect_error.is_some() { content_h += section_gap + small_font; }

        let popup_h = pad * 2.0 + content_h;
        let popup_x = (bt_x + bt_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = (bar_y - popup_h - gap).max(0.0);

        // Shadow + background
        let shadow_expand = 3.0 * scale;
        painter.rect_filled(
            Rect::new(popup_x - shadow_expand, popup_y + shadow_expand,
                popup_w + shadow_expand * 2.0, popup_h + shadow_expand),
            corner_r + 2.0, Color::BLACK.with_alpha(0.35),
        );
        let bg = Rect::new(popup_x, popup_y, popup_w, popup_h);
        painter.rect_filled(bg, corner_r, palette.surface_2);
        painter.rect_stroke(bg, corner_r, 1.0 * scale, Color::WHITE.with_alpha(0.08));

        let cx = popup_x + pad;
        let cw = popup_w - pad * 2.0;
        let mut y = popup_y + pad;

        // Title + power toggle
        text.queue("Bluetooth", title_font, cx, y, palette.text, cw * 0.6, screen_w, screen_h);

        let toggle_w = 52.0 * scale;
        let toggle_x = cx + cw - toggle_w;
        let toggle_rect = Rect::new(toggle_x, y, toggle_w, toggle_h);
        ix.add_zone(ZONE_BT_POWER, toggle_rect);
        let hovered = ix.is_hovered(&toggle_rect);
        Toggle::new(Rect::new(toggle_x, y, cw, toggle_h), self.powered)
            .hovered(hovered)
            .scale(scale)
            .transition(self.power_anim)
            .draw(painter, text, palette, screen_w, screen_h);

        y += title_font.max(toggle_h) + section_gap;

        // Scan button
        let scan_h = 36.0 * scale;
        let scan_rect = Rect::new(cx, y, cw, scan_h);
        let scan_state = ix.add_zone(ZONE_BT_SCAN, scan_rect);
        let scan_hovered = scan_state.is_hovered();
        let scan_color = if scan_hovered { palette.accent } else { palette.accent.with_alpha(0.7) };
        if self.scanning {
            painter.rect_filled(scan_rect, 8.0 * scale, palette.muted.with_alpha(0.3));
            let scan_ty = y + (scan_h - body_font) / 2.0;
            text.queue("Scanning…", body_font, cx + cw / 2.0 - 50.0 * scale, scan_ty,
                palette.muted, cw, screen_w, screen_h);
        } else {
            painter.rect_filled(scan_rect, 8.0 * scale, scan_color);
            let scan_ty = y + (scan_h - body_font) / 2.0;
            text.queue("Scan for devices", body_font, cx + cw / 2.0 - 80.0 * scale, scan_ty,
                palette.text, cw, screen_w, screen_h);
        }
        y += scan_h + section_gap;

        // Separator
        painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
        y += 1.0 * scale + section_gap;

        // Device list
        let scroll = self.scroll_offset as usize;
        let visible: Vec<(usize, &BtDevice)> = all_devs.iter()
            .enumerate()
            .skip(scroll)
            .take(max_visible)
            .collect();

        for (orig_idx, dev) in &visible {
            let row_rect = Rect::new(cx, y, cw, row_h);
            let zone_id = ZONE_DEVICE_BASE + *orig_idx as u32;
            let state = ix.add_zone(zone_id, row_rect);
            let row_hovered = state.is_hovered();

            if row_hovered {
                painter.rect_filled(row_rect, 8.0 * scale, palette.muted.with_alpha(0.2));
            }

            let is_connecting = self.connecting_mac.as_deref() == Some(&dev.mac);
            let text_y = y + (row_h - body_font) / 2.0;
            let mut label_x = cx + 8.0 * scale;

            // Connected indicator
            if dev.connected {
                text.queue("✓", body_font, label_x, text_y, palette.accent,
                    body_font, screen_w, screen_h);
                label_x += body_font + 4.0 * scale;
            }

            // Device name
            let name_color = if dev.connected { palette.text } else { palette.text_secondary };
            let display_name = if is_connecting {
                format!("{} — connecting…", dev.name)
            } else {
                dev.name.clone()
            };
            text.queue(&display_name, body_font, label_x, text_y, name_color,
                cw * 0.55, screen_w, screen_h);

            // Right side: battery + status + remove
            let right_x = cx + cw - 120.0 * scale;

            if let Some(batt) = dev.battery {
                let batt_text = format!("{}%", batt);
                text.queue(&batt_text, small_font, right_x, text_y + 2.0 * scale,
                    palette.muted, 40.0 * scale, screen_w, screen_h);
            }

            // Status label
            let status = if dev.connected { "Connected" }
                else if dev.paired { "Paired" }
                else { "New" };
            let status_x = right_x + 44.0 * scale;
            text.queue(status, small_font, status_x, text_y + 2.0 * scale,
                palette.muted, 60.0 * scale, screen_w, screen_h);

            // Remove button (×)
            if dev.paired {
                let rm_size = 24.0 * scale;
                let rm_x = cx + cw - rm_size - 2.0 * scale;
                let rm_y = y + (row_h - rm_size) / 2.0;
                let rm_rect = Rect::new(rm_x, rm_y, rm_size, rm_size);
                let rm_id = ZONE_DEVICE_REMOVE + *orig_idx as u32;
                let rm_state = ix.add_zone(rm_id, rm_rect);
                if rm_state.is_hovered() {
                    painter.rect_filled(rm_rect, 4.0 * scale, palette.danger.with_alpha(0.2));
                }
                let rm_ty = rm_y + (rm_size - small_font) / 2.0;
                text.queue("×", small_font, rm_x + 6.0 * scale, rm_ty,
                    palette.danger, rm_size, screen_w, screen_h);
            }

            y += row_h;
        }

        if all_devs.len() > max_visible {
            text.queue("scroll for more…", small_font, cx, y,
                palette.muted, cw, screen_w, screen_h);
            y += small_font;
        }

        if all_devs.is_empty() && !self.scanning {
            text.queue("No devices found", body_font, cx, y,
                palette.muted, cw, screen_w, screen_h);
        }

        // Error
        if let Some(ref err) = self.connect_error {
            y += section_gap;
            text.queue(err, small_font, cx, y, palette.danger, cw, screen_w, screen_h);
        }
    }

    pub fn popup_rect(
        &self, bt_x: f32, bt_w: f32, bar_y: f32, scale: f32, screen_w: u32,
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
        let toggle_h = 28.0 * scale;

        let all_devs = self.all_devices();
        let max_visible = 6;
        let dev_count = all_devs.len().min(max_visible);
        let mut content_h = title_font.max(toggle_h) + section_gap;
        content_h += 36.0 * scale + section_gap;
        content_h += 1.0 * scale + section_gap;
        content_h += dev_count as f32 * row_h;
        if all_devs.len() > max_visible { content_h += small_font; }
        if self.scanning { content_h += section_gap + body_font; }
        if self.connect_error.is_some() { content_h += section_gap + small_font; }

        let popup_h = pad * 2.0 + content_h;
        let popup_x = (bt_x + bt_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = (bar_y - popup_h - gap).max(0.0);

        Some(Rect::new(popup_x, popup_y, popup_w, popup_h))
    }
}

// ── Background thread ───────────────────────────────────────────────────────

fn poll_thread(tx: mpsc::Sender<BtEvent>, cmd_rx: mpsc::Receiver<BtCmd>) {
    let _ = tx.send(poll_status());

    let mut last_poll = std::time::Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                BtCmd::SetPower(on) => {
                    let arg = if on { "on" } else { "off" };
                    let _ = Command::new("bluetoothctl").args(["power", arg]).output();
                    let _ = tx.send(poll_status());
                }
                BtCmd::Scan => {
                    // Non-blocking scan with timeout
                    let output = Command::new("bluetoothctl")
                        .args(["--timeout", "5", "scan", "on"])
                        .output();
                    if let Ok(output) = output {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let mut found = Vec::new();
                        for line in stdout.lines() {
                            if let Some(rest) = line.strip_prefix("[NEW] Device ") {
                                if let Some((mac, name)) = rest.split_once(' ') {
                                    found.push(BtDevice {
                                        mac: mac.to_string(),
                                        name: name.to_string(),
                                        connected: false,
                                        paired: false,
                                        battery: None,
                                    });
                                }
                            }
                        }
                        let _ = tx.send(BtEvent::Discovered(found));
                    }
                    let _ = tx.send(BtEvent::ScanDone);
                    let _ = tx.send(poll_status());
                }
                BtCmd::Connect(mac) => {
                    let output = Command::new("bluetoothctl").args(["connect", &mac]).output();
                    send_action_result(&tx, output);
                    let _ = tx.send(poll_status());
                }
                BtCmd::Disconnect(mac) => {
                    let output = Command::new("bluetoothctl").args(["disconnect", &mac]).output();
                    send_action_result(&tx, output);
                    let _ = tx.send(poll_status());
                }
                BtCmd::Pair(mac) => {
                    let output = Command::new("bluetoothctl").args(["pair", &mac]).output();
                    send_action_result(&tx, output);
                    // Also trust + connect after pairing
                    let _ = Command::new("bluetoothctl").args(["trust", &mac]).output();
                    let _ = Command::new("bluetoothctl").args(["connect", &mac]).output();
                    let _ = tx.send(poll_status());
                }
                BtCmd::Remove(mac) => {
                    let output = Command::new("bluetoothctl").args(["remove", &mac]).output();
                    send_action_result(&tx, output);
                    let _ = tx.send(poll_status());
                }
            }
        }

        if last_poll.elapsed().as_millis() >= POLL_INTERVAL_MS as u128 {
            let _ = tx.send(poll_status());
            last_poll = std::time::Instant::now();
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn send_action_result(tx: &mpsc::Sender<BtEvent>, result: std::io::Result<std::process::Output>) {
    match result {
        Ok(output) if output.status.success() => { let _ = tx.send(BtEvent::ActionOk); }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr.lines().next().unwrap_or("Action failed").to_string();
            let _ = tx.send(BtEvent::ActionFail(msg));
        }
        Err(e) => { let _ = tx.send(BtEvent::ActionFail(e.to_string())); }
    }
}

fn poll_status() -> BtEvent {
    let output = Command::new("bluetoothctl").arg("show").output();
    let powered = output.as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines()
            .any(|l| l.contains("Powered:") && l.contains("yes")))
        .unwrap_or(false);

    let mut devices = Vec::new();
    if powered {
        let dev_output = Command::new("bluetoothctl").arg("devices").output();
        if let Ok(dev_output) = dev_output {
            let stdout = String::from_utf8_lossy(&dev_output.stdout);
            for line in stdout.lines() {
                if let Some(rest) = line.strip_prefix("Device ") {
                    if let Some((mac, name)) = rest.split_once(' ') {
                        let info = get_device_info(mac);
                        devices.push(BtDevice {
                            mac: mac.to_string(),
                            name: name.to_string(),
                            connected: info.0,
                            paired: info.1,
                            battery: info.2,
                        });
                    }
                }
            }
        }
        // Sort: connected first, then paired
        devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)));
    }

    BtEvent::Status { powered, devices }
}

fn get_device_info(mac: &str) -> (bool, bool, Option<u8>) {
    let output = Command::new("bluetoothctl").args(["info", mac]).output();
    let Ok(output) = output else { return (false, false, None) };
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut connected = false;
    let mut paired = false;
    let mut battery = None;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Connected:") {
            connected = trimmed.contains("yes");
        } else if trimmed.starts_with("Paired:") {
            paired = trimmed.contains("yes");
        } else if trimmed.starts_with("Battery Percentage:") {
            // Format: "Battery Percentage: 0x28 (40)"
            if let Some(paren) = trimmed.rfind('(') {
                let num_str = &trimmed[paren + 1..trimmed.len() - 1];
                battery = num_str.parse().ok();
            }
        }
    }

    (connected, paired, battery)
}
