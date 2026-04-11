//! Bluetooth widget — icon in bar + popup for device management and file transfer.

use std::io::Read as _;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::mpsc;

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Toggle};

use crate::bluetooth_worker::{BtCmd, BtDevice, BtEvent};
use crate::bluetooth_transfer::{Transfer, TransferCmd, TransferDir, TransferEvent};
use crate::svg_icon::IconCache;

// Zone IDs
pub const ZONE_BT_ICON: u32 = 0xDD_0000;
const ZONE_BT_POWER: u32 = 0xDD_0001;
const ZONE_BT_SCAN: u32 = 0xDD_0002;
const ZONE_BT_DISCOVERABLE: u32 = 0xDD_0003;
const ZONE_DEVICE_BASE: u32 = 0xDD_0100;
const ZONE_DEVICE_REMOVE: u32 = 0xDD_0300;
const ZONE_DEVICE_SEND: u32 = 0xDD_0400;
const ZONE_TRANSFER_CANCEL: u32 = 0xDD_0500;
const ZONE_PAIR_CONFIRM: u32 = 0xDD_0600;
const ZONE_PAIR_REJECT: u32 = 0xDD_0601;

pub struct Bluetooth {
    powered: bool,
    discoverable: bool,
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
    discoverable_anim: f32,
    // File transfer
    transfer_cmd_tx: mpsc::Sender<TransferCmd>,
    transfer_event_rx: mpsc::Receiver<TransferEvent>,
    transfers: Vec<Transfer>,
    pick_child: Option<(String, Child)>,
    obex_available: bool,
    // Pairing
    pair_request: Option<(String, u32)>, // (device_name, passkey) — 0 means no passkey
}

impl Bluetooth {
    pub fn new() -> Self {
        let (cmd_tx, event_rx) = crate::bluetooth_worker::spawn_bt_thread();
        let (transfer_cmd_tx, transfer_event_rx) = crate::bluetooth_transfer::spawn_obex_thread();

        Self {
            powered: false,
            discoverable: false,
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
            discoverable_anim: 0.0,
            transfer_cmd_tx,
            transfer_event_rx,
            transfers: Vec::new(),
            pick_child: None,
            obex_available: true,
            pair_request: None,
        }
    }

    /// Drain background events. Returns `true` if any event was received.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        // BT device events
        while let Ok(event) = self.event_rx.try_recv() {
            changed = true;
            match event {
                BtEvent::Status { powered, discoverable, devices } => {
                    self.powered = powered;
                    self.discoverable = discoverable;
                    self.devices = devices;
                }
                BtEvent::Discovered(devs) => {
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
                BtEvent::PairRequest { device_name, passkey } => {
                    self.pair_request = Some((device_name, passkey));
                    self.open = true; // auto-open popup to show pairing request
                }
                BtEvent::PairRequestCancelled => {
                    self.pair_request = None;
                }
            }
        }

        // Transfer events
        while let Ok(event) = self.transfer_event_rx.try_recv() {
            changed = true;
            match event {
                TransferEvent::Started { id, filename, total, direction } => {
                    self.transfers.push(Transfer {
                        id, filename, total, transferred: 0, direction, done: false,
                    });
                }
                TransferEvent::Progress { id, transferred, total } => {
                    if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
                        t.transferred = transferred;
                        if total > 0 { t.total = total; }
                    }
                }
                TransferEvent::Complete { id } => {
                    if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
                        t.transferred = t.total;
                        t.done = true;
                    }
                }
                TransferEvent::Failed { id, error } => {
                    self.transfers.retain(|t| t.id != id);
                    self.connect_error = Some(error);
                }
                TransferEvent::ObexUnavailable => {
                    self.obex_available = false;
                }
            }
        }
        // Clear completed transfers after 3 seconds (tracked by done flag)
        self.transfers.retain(|t| !t.done);

        // Check file picker subprocess
        if let Some((ref mac, ref mut child)) = self.pick_child {
            if let Ok(Some(status)) = child.try_wait() {
                if status.success() {
                    let mut stdout = String::new();
                    if let Some(ref mut out) = child.stdout {
                        let _ = out.read_to_string(&mut stdout);
                    }
                    let path = stdout.trim();
                    if !path.is_empty() {
                        let _ = self.transfer_cmd_tx.send(TransferCmd::SendFile {
                            mac: mac.clone(),
                            file_path: path.to_string(),
                        });
                    }
                }
                self.pick_child = None;
                changed = true;
            }
        }

        changed
    }

    pub fn update_anim(&mut self, dt: f32) -> bool {
        let step = dt / 0.2;
        let a = step_anim(&mut self.power_anim, self.powered, step);
        let b = step_anim(&mut self.discoverable_anim, self.discoverable, step);
        a || b
    }

    pub fn handle_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
            if zone == ZONE_BT_POWER {
                let _ = self.cmd_tx.send(BtCmd::SetPower(!self.powered));
            } else if zone == ZONE_BT_DISCOVERABLE {
                let _ = self.cmd_tx.send(BtCmd::SetDiscoverable(!self.discoverable));
            } else if zone == ZONE_PAIR_CONFIRM {
                let _ = self.cmd_tx.send(BtCmd::ConfirmPair);
                self.pair_request = None;
            } else if zone == ZONE_PAIR_REJECT {
                let _ = self.cmd_tx.send(BtCmd::RejectPair);
                self.pair_request = None;
            } else if zone == ZONE_BT_SCAN {
                if !self.scanning {
                    self.scanning = true;
                    self.discovered.clear();
                    let _ = self.cmd_tx.send(BtCmd::Scan);
                }
            } else if zone >= ZONE_TRANSFER_CANCEL && zone < ZONE_TRANSFER_CANCEL + 256 {
                let idx = (zone - ZONE_TRANSFER_CANCEL) as usize;
                if let Some(t) = self.transfers.get(idx) {
                    let _ = self.transfer_cmd_tx.send(TransferCmd::Cancel { id: t.id });
                }
            } else if zone >= ZONE_DEVICE_SEND && zone < ZONE_DEVICE_SEND + 256 {
                let idx = (zone - ZONE_DEVICE_SEND) as usize;
                if let Some(dev) = self.all_devices().get(idx) {
                    self.spawn_file_picker(&dev.mac);
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

    fn spawn_file_picker(&mut self, mac: &str) {
        if self.pick_child.is_some() { return; } // already picking
        let child = ProcessCommand::new("lntrn-file-manager")
            .arg("--pick")
            .arg("--title")
            .arg("Send via Bluetooth")
            .stdout(Stdio::piped())
            .spawn();
        match child {
            Ok(c) => { self.pick_child = Some((mac.to_string(), c)); }
            Err(e) => { self.connect_error = Some(format!("File picker failed: {e}")); }
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
        for (key, file) in [
            ("bt-on", "spark-bluetooth-on.svg"),
            ("bt-off", "spark-bluetooth-off.svg"),
            ("bt-connected", "spark-bluetooth-connected.svg"),
        ] {
            icons.load_embedded(tex_pass, gpu, key, file, size, size);
        }
        self.icons_loaded = true;
    }
    pub fn measure(&self, bar_h: f32, scale: f32) -> f32 {
        (bar_h - 18.0 * scale).max(16.0)
    }
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
        ix.add_zone(ZONE_BT_ICON, Rect::new(x, icon_y, icon_size, icon_size));
        (icon_size, tex_draws)
    }

    pub fn draw_popup(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        bt_x: f32, bt_w: f32, bar_y: f32, bar_h: f32, position_top: bool, scale: f32,
        screen_w: u32, screen_h: u32,
    ) {
        if !self.open { return; }

        let s = PopupSizes::new(scale);
        let all_devs = self.all_devices();
        let dev_count = all_devs.len().min(s.max_visible);
        let content_h = self.content_height(&s, &all_devs, dev_count);
        let popup_h = s.pad * 2.0 + content_h;
        let popup_w = s.popup_w;
        let gap = s.gap;
        let popup_x = (bt_x + bt_w / 2.0 - popup_w / 2.0)
            .max(gap).min(screen_w as f32 - popup_w - gap);
        let popup_y = if position_top { bar_y + bar_h + gap }
            else { (bar_y - popup_h - gap).max(0.0) };

        // Shadow + background
        let shadow_expand = 3.0 * scale;
        painter.rect_filled(
            Rect::new(popup_x - shadow_expand, popup_y + shadow_expand,
                popup_w + shadow_expand * 2.0, popup_h + shadow_expand),
            s.corner_r + 2.0, Color::BLACK.with_alpha(0.35),
        );
        let bg = Rect::new(popup_x, popup_y, popup_w, popup_h);
        painter.rect_filled(bg, s.corner_r, palette.bg);
        painter.rect_stroke_sdf(bg, s.corner_r, 3.0 * scale, Color::BLACK);

        let cx = popup_x + s.pad;
        let cw = popup_w - s.pad * 2.0;
        let mut y = popup_y + s.pad;

        // Title + power toggle + discoverable toggle
        let tw = 52.0 * scale;
        let tx = cx + cw - tw;
        text.queue("Bluetooth", s.title_font, cx, y, palette.text, cw * 0.6, screen_w, screen_h);
        let r = Rect::new(tx, y, tw, s.toggle_h);
        ix.add_zone(ZONE_BT_POWER, r);
        Toggle::new(Rect::new(tx, y, cw, s.toggle_h), self.powered)
            .hovered(ix.is_hovered(&r)).scale(scale).transition(self.power_anim)
            .draw(painter, text, palette, screen_w, screen_h);
        y += s.title_font.max(s.toggle_h) + s.section_gap;
        text.queue("Discoverable", s.body_font, cx, y + (s.toggle_h - s.body_font) / 2.0,
            palette.text_secondary, cw * 0.6, screen_w, screen_h);
        let r = Rect::new(tx, y, tw, s.toggle_h);
        ix.add_zone(ZONE_BT_DISCOVERABLE, r);
        Toggle::new(Rect::new(tx, y, cw, s.toggle_h), self.discoverable)
            .hovered(ix.is_hovered(&r)).scale(scale).transition(self.discoverable_anim)
            .draw(painter, text, palette, screen_w, screen_h);
        y += s.toggle_h + s.section_gap;

        // Pairing request (if any)
        if let Some((ref name, passkey)) = self.pair_request {
            let pr_h = s.body_font * 2.0 + s.scan_h + s.section_gap * 2.0;
            let pr_rect = Rect::new(cx, y, cw, pr_h);
            painter.rect_filled(pr_rect, 8.0 * scale, palette.accent.with_alpha(0.12));
            painter.rect_stroke_sdf(pr_rect, 8.0 * scale, 1.0 * scale, palette.accent.with_alpha(0.4));
            let py = y + s.section_gap * 0.5;
            text.queue(&format!("Pair with {}?", name), s.body_font, cx + 10.0 * scale,
                py, palette.text, cw - 20.0 * scale, screen_w, screen_h);
            if passkey > 0 {
                text.queue(&format!("Passkey: {:06}", passkey), s.body_font, cx + 10.0 * scale,
                    py + s.body_font + 4.0 * scale, palette.accent, cw, screen_w, screen_h);
            }
            let btn_y = y + pr_h - s.scan_h - s.section_gap * 0.5;
            let btn_w = (cw - 16.0 * scale) / 2.0;
            let conf_r = Rect::new(cx + 4.0 * scale, btn_y, btn_w, s.scan_h);
            let conf_s = ix.add_zone(ZONE_PAIR_CONFIRM, conf_r);
            let cc = if conf_s.is_hovered() { palette.accent } else { palette.accent.with_alpha(0.7) };
            painter.rect_filled(conf_r, 6.0 * scale, cc);
            text.queue("Confirm", s.body_font, conf_r.x + btn_w / 2.0 - 36.0 * scale,
                btn_y + (s.scan_h - s.body_font) / 2.0, palette.text, btn_w, screen_w, screen_h);
            let rej_r = Rect::new(cx + 12.0 * scale + btn_w, btn_y, btn_w, s.scan_h);
            let rej_s = ix.add_zone(ZONE_PAIR_REJECT, rej_r);
            let rc = if rej_s.is_hovered() { palette.danger } else { palette.danger.with_alpha(0.5) };
            painter.rect_filled(rej_r, 6.0 * scale, rc);
            text.queue("Reject", s.body_font, rej_r.x + btn_w / 2.0 - 30.0 * scale,
                btn_y + (s.scan_h - s.body_font) / 2.0, palette.text, btn_w, screen_w, screen_h);
            y += pr_h + s.section_gap;
        }

        // Scan button
        let scan_rect = Rect::new(cx, y, cw, s.scan_h);
        let scan_state = ix.add_zone(ZONE_BT_SCAN, scan_rect);
        let scan_hovered = scan_state.is_hovered();
        if self.scanning {
            painter.rect_filled(scan_rect, 8.0 * scale, palette.muted.with_alpha(0.3));
            text.queue("Scanning…", s.body_font, cx + cw / 2.0 - 50.0 * scale,
                y + (s.scan_h - s.body_font) / 2.0, palette.muted, cw, screen_w, screen_h);
        } else {
            let c = if scan_hovered { palette.accent } else { palette.accent.with_alpha(0.7) };
            painter.rect_filled(scan_rect, 8.0 * scale, c);
            text.queue("Scan for devices", s.body_font, cx + cw / 2.0 - 80.0 * scale,
                y + (s.scan_h - s.body_font) / 2.0, palette.text, cw, screen_w, screen_h);
        }
        y += s.scan_h + s.section_gap;

        // Separator
        painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
        y += 1.0 * scale + s.section_gap;

        // Device list
        let scroll = self.scroll_offset as usize;
        let visible: Vec<(usize, &BtDevice)> = all_devs.iter()
            .enumerate().skip(scroll).take(s.max_visible).collect();

        for (orig_idx, dev) in &visible {
            self.draw_device_row(painter, text, ix, palette, dev, *orig_idx,
                cx, y, cw, &s, screen_w, screen_h, scale);
            y += s.row_h;
        }

        if all_devs.len() > s.max_visible {
            text.queue("scroll for more…", s.small_font, cx, y, palette.muted, cw, screen_w, screen_h);
            y += s.small_font;
        }
        if all_devs.is_empty() && !self.scanning {
            text.queue("No devices found", s.body_font, cx, y, palette.muted, cw, screen_w, screen_h);
        }

        // Active transfers
        if !self.transfers.is_empty() {
            y += s.section_gap;
            painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + s.section_gap;
            text.queue("Transfers", s.body_font, cx, y, palette.text, cw, screen_w, screen_h);
            y += s.body_font + s.section_gap * 0.5;

            for (i, t) in self.transfers.iter().enumerate() {
                self.draw_transfer_row(painter, text, ix, palette, t, i,
                    cx, y, cw, &s, screen_w, screen_h, scale);
                y += s.row_h;
            }
        }

        // Error
        if let Some(ref err) = self.connect_error {
            y += s.section_gap;
            text.queue(err, s.small_font, cx, y, palette.danger, cw, screen_w, screen_h);
        }
    }

    fn draw_device_row(
        &self, p: &mut Painter, t: &mut TextRenderer, ix: &mut InteractionContext,
        pal: &FoxPalette, dev: &BtDevice, idx: usize,
        cx: f32, y: f32, cw: f32, s: &PopupSizes, sw: u32, sh: u32, sc: f32,
    ) {
        let row = Rect::new(cx, y, cw, s.row_h);
        if ix.add_zone(ZONE_DEVICE_BASE + idx as u32, row).is_hovered() {
            p.rect_filled(row, 8.0 * sc, pal.muted.with_alpha(0.2));
        }
        let conn_ing = self.connecting_mac.as_deref() == Some(&dev.mac);
        let ty = y + (s.row_h - s.body_font) / 2.0;
        let mut lx = cx + 8.0 * sc;
        t.queue(dev.type_label(), s.body_font, lx, ty, pal.muted, s.body_font * 1.5, sw, sh);
        lx += s.body_font + 8.0 * sc;
        if dev.connected {
            t.queue("✓", s.body_font, lx, ty, pal.accent, s.body_font, sw, sh);
            lx += s.body_font + 4.0 * sc;
        }
        let nc = if dev.connected { pal.text } else { pal.text_secondary };
        let nm = if conn_ing { format!("{}  connecting…", dev.name) } else { dev.name.clone() };
        t.queue(&nm, s.body_font, lx, ty, nc, cw * 0.35, sw, sh);
        let rx = cx + cw - 160.0 * sc;
        let mut rx2 = rx;
        if let Some(rssi) = dev.rssi {
            let b = match rssi { -50..=0 => "▂▄▆█", -65..=-51 => "▂▄▆", -80..=-66 => "▂▄", _ => "▂" };
            t.queue(b, s.small_font, rx2, ty + 2.0 * sc, pal.muted, 40.0 * sc, sw, sh);
            rx2 += 42.0 * sc;
        }
        if let Some(bat) = dev.battery {
            t.queue(&format!("{}%", bat), s.body_font, rx2, ty, pal.muted, 50.0 * sc, sw, sh);
        }
        if !dev.connected && !conn_ing {
            let st = if dev.paired { "Paired" } else { "New" };
            t.queue(st, s.small_font, rx + 55.0 * sc, ty + 2.0 * sc, pal.muted, 60.0 * sc, sw, sh);
        }
        if conn_ing { p.rect_filled(row, 8.0 * sc, pal.accent.with_alpha(0.08)); }
        if dev.connected && self.obex_available {
            icon_btn(p, t, ix, ZONE_DEVICE_SEND + idx as u32, "↑", s.body_font,
                pal.accent, cx + cw - 68.0 * sc, y, 32.0 * sc, s.row_h, sc, sw, sh);
        }
        if dev.paired {
            icon_btn(p, t, ix, ZONE_DEVICE_REMOVE + idx as u32, "×", s.body_font,
                pal.danger, cx + cw - 34.0 * sc, y, 32.0 * sc, s.row_h, sc, sw, sh);
        }
    }
    fn draw_transfer_row(
        &self, p: &mut Painter, t: &mut TextRenderer, ix: &mut InteractionContext,
        pal: &FoxPalette, tr: &Transfer, idx: usize,
        cx: f32, y: f32, cw: f32, s: &PopupSizes, sw: u32, sh: u32, sc: f32,
    ) {
        let ty = y + (s.row_h - s.body_font) / 2.0;
        let is_send = tr.direction == TransferDir::Send;
        let (arrow, ac) = if is_send { ("↑", pal.accent) } else { ("↓", pal.text) };
        t.queue(arrow, s.body_font, cx + 8.0 * sc, ty, ac, s.body_font, sw, sh);
        t.queue(&tr.filename, s.body_font, cx + s.body_font + 16.0 * sc, ty,
            pal.text, cw * 0.4, sw, sh);
        let bx = cx + cw * 0.55;
        let bw = cw * 0.3;
        let bh = 8.0 * sc;
        let by = y + (s.row_h - bh) / 2.0;
        p.rect_filled(Rect::new(bx, by, bw, bh), 4.0 * sc, pal.muted.with_alpha(0.3));
        let pct = if tr.total > 0 { (tr.transferred as f32 / tr.total as f32).min(1.0) } else { 0.0 };
        if pct > 0.0 { p.rect_filled(Rect::new(bx, by, bw * pct, bh), 4.0 * sc, pal.accent); }
        t.queue(&format!("{}%", (pct * 100.0) as u32), s.small_font,
            bx + bw + 6.0 * sc, ty + 2.0 * sc, pal.muted, 40.0 * sc, sw, sh);
        if is_send {
            icon_btn(p, t, ix, ZONE_TRANSFER_CANCEL + idx as u32, "×", s.small_font,
                pal.danger, cx + cw - 26.0 * sc, y, 24.0 * sc, s.row_h, sc, sw, sh);
        }
    }

    fn content_height(&self, s: &PopupSizes, all_devs: &[BtDevice], dev_count: usize) -> f32 {
        let mut h = s.title_font.max(s.toggle_h) + s.section_gap; // title
        h += s.toggle_h + s.section_gap; // discoverable toggle
        if self.pair_request.is_some() {
            h += s.body_font * 2.0 + s.scan_h + s.section_gap * 2.0 + s.section_gap; // pair request
        }
        h += s.scan_h + s.section_gap; // scan button
        h += 1.0 + s.section_gap; // separator (1px unscaled, but s values already scaled)
        h += dev_count as f32 * s.row_h;
        if all_devs.len() > s.max_visible { h += s.small_font; }
        if self.scanning { h += s.section_gap + s.body_font; }
        if !self.transfers.is_empty() {
            h += s.section_gap + 1.0 + s.section_gap; // separator
            h += s.body_font + s.section_gap * 0.5; // "Transfers" header
            h += self.transfers.len() as f32 * s.row_h;
        }
        if self.connect_error.is_some() { h += s.section_gap + s.small_font; }
        h
    }

    pub fn popup_rect(
        &self, bt_x: f32, bt_w: f32, bar_y: f32, bar_h: f32, position_top: bool, scale: f32, screen_w: u32,
    ) -> Option<Rect> {
        if !self.open { return None; }
        let s = PopupSizes::new(scale);
        let all_devs = self.all_devices();
        let dev_count = all_devs.len().min(s.max_visible);
        let content_h = self.content_height(&s, &all_devs, dev_count);
        let popup_h = s.pad * 2.0 + content_h;
        let popup_x = (bt_x + bt_w / 2.0 - s.popup_w / 2.0)
            .max(s.gap).min(screen_w as f32 - s.popup_w - s.gap);
        let popup_y = if position_top { bar_y + bar_h + s.gap }
            else { (bar_y - popup_h - s.gap).max(0.0) };
        Some(Rect::new(popup_x, popup_y, s.popup_w, popup_h))
    }
}

fn icon_btn(
    p: &mut Painter, t: &mut TextRenderer, ix: &mut InteractionContext,
    zone: u32, label: &str, font: f32, color: Color,
    x: f32, row_y: f32, size: f32, row_h: f32, sc: f32, sw: u32, sh: u32,
) {
    let by = row_y + (row_h - size) / 2.0;
    let r = Rect::new(x, by, size, size);
    if ix.add_zone(zone, r).is_hovered() { p.rect_filled(r, 4.0 * sc, color.with_alpha(0.2)); }
    t.queue(label, font, x + (size - font * 0.6) / 2.0, by + (size - font) / 2.0,
        color, size, sw, sh);
}

fn step_anim(anim: &mut f32, on: bool, step: f32) -> bool {
    let target = if on { 1.0 } else { 0.0 };
    if (*anim - target).abs() < 0.001 { *anim = target; return false; }
    *anim = if *anim < target { (*anim + step).min(1.0) } else { (*anim - step).max(0.0) };
    true
}

// ── Layout sizes (shared between draw_popup and popup_rect) ────────────────

struct PopupSizes {
    pad: f32, corner_r: f32, gap: f32, popup_w: f32,
    title_font: f32, body_font: f32, small_font: f32,
    row_h: f32, section_gap: f32, toggle_h: f32, scan_h: f32,
    max_visible: usize,
}

impl PopupSizes {
    fn new(scale: f32) -> Self {
        Self {
            pad: 20.0 * scale, corner_r: 12.0 * scale, gap: 8.0 * scale,
            popup_w: 380.0 * scale, title_font: 24.0 * scale, body_font: 20.0 * scale,
            small_font: 16.0 * scale, row_h: 48.0 * scale, section_gap: 12.0 * scale,
            toggle_h: 28.0 * scale, scan_h: 36.0 * scale, max_visible: 6,
        }
    }
}
