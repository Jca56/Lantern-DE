//! Clipboard history tab — monitors clipboard and shows recent text entries.

use std::io::{Read, Write};
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::input::InteractionState;
use lntrn_ui::gpu::scroll::{ScrollArea, Scrollbar};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, TextInput};

const MAX_ENTRIES: usize = 50;
const MAX_READ: usize = 65536;
const TITLE_FONT: f32 = 24.0;
const ENTRY_FONT: f32 = 20.0;
const SMALL_FONT: f32 = 18.0;
const ITEM_H: f32 = 56.0;
const HEADER_H: f32 = 40.0;

const ZONE_CLIP_BASE: u32 = 0xBE_1000;
const ZONE_CLIP_DEL_BASE: u32 = 0xBE_1800;
const ZONE_CLIP_CLEAR: u32 = 0xBE_1F00;

struct ClipboardEntry {
    text: String,
    when: Instant,
}

enum ClipCmd {
    SetClipboard(String),
}

enum ClipEvent {
    NewEntry(String),
}

pub struct ClipboardHistory {
    entries: Vec<ClipboardEntry>,
    event_rx: mpsc::Receiver<ClipEvent>,
    cmd_tx: mpsc::Sender<ClipCmd>,
    pub scroll_offset: f32,
    pub search: String,
    pub search_focused: bool,
    pub search_cursor: usize,
}

impl ClipboardHistory {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("clipboard-monitor".into())
            .spawn(move || clipboard_thread(event_tx, cmd_rx))
            .expect("spawn clipboard thread");

        Self {
            entries: Vec::new(),
            event_rx,
            cmd_tx,
            scroll_offset: 0.0,
            search: String::new(),
            search_focused: false,
            search_cursor: 0,
        }
    }

    /// Drain clipboard events. Returns `true` if any new entry was received.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                ClipEvent::NewEntry(text) => {
                    if text.trim().is_empty() { continue; }
                    // Dedup: skip if top entry matches
                    if self.entries.first().map_or(false, |e| e.text == text) {
                        continue;
                    }
                    // Remove duplicate if it exists elsewhere
                    self.entries.retain(|e| e.text != text);
                    self.entries.insert(0, ClipboardEntry {
                        text,
                        when: Instant::now(),
                    });
                    if self.entries.len() > MAX_ENTRIES {
                        self.entries.pop();
                    }
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn wants_keyboard(&self) -> bool {
        self.search_focused
    }

    pub fn on_key(&mut self, key: u32, shift: bool) -> bool {
        match key {
            1 => { // Esc
                if !self.search.is_empty() {
                    self.search.clear();
                    self.search_cursor = 0;
                } else {
                    self.search_focused = false;
                    return false; // let menu handle ESC
                }
                true
            }
            14 => { // Backspace
                if self.search_cursor > 0 {
                    self.search_cursor -= 1;
                    self.search.remove(self.search_cursor);
                }
                true
            }
            _ => {
                if let Some(ch) = super::keycode_to_char(key, shift) {
                    self.search.insert(self.search_cursor, ch);
                    self.search_cursor += 1;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn on_left_click(&mut self, ix: &InteractionContext, phys_x: f32, phys_y: f32) {
        if let Some(zone) = ix.zone_at(phys_x, phys_y) {
            if zone == ZONE_CLIP_CLEAR {
                self.entries.clear();
                self.scroll_offset = 0.0;
                return;
            }
            if zone >= ZONE_CLIP_DEL_BASE && zone < ZONE_CLIP_DEL_BASE + MAX_ENTRIES as u32 {
                let idx = (zone - ZONE_CLIP_DEL_BASE) as usize;
                let filtered = self.filtered_indices();
                if let Some(&real_idx) = filtered.get(idx) {
                    self.entries.remove(real_idx);
                }
                return;
            }
            if zone >= ZONE_CLIP_BASE && zone < ZONE_CLIP_BASE + MAX_ENTRIES as u32 {
                let idx = (zone - ZONE_CLIP_BASE) as usize;
                let filtered = self.filtered_indices();
                if let Some(&real_idx) = filtered.get(idx) {
                    let text = self.entries[real_idx].text.clone();
                    let _ = self.cmd_tx.send(ClipCmd::SetClipboard(text));
                }
                return;
            }
        }
        self.search_focused = true;
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let filter = self.search.to_lowercase();
        self.entries.iter().enumerate()
            .filter(|(_, e)| filter.is_empty() || e.text.to_lowercase().contains(&filter))
            .map(|(i, _)| i)
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        area: Rect, scale: f32, screen_w: u32, screen_h: u32,
    ) {
        let pad = 16.0 * scale;
        let sf = TITLE_FONT * scale;
        let ef = ENTRY_FONT * scale;
        let smf = SMALL_FONT * scale;
        let item_h = ITEM_H * scale;
        let header_h = HEADER_H * scale;
        let input_h = 44.0 * scale;
        let lg = 8.0 * scale;

        let gr = Rect::new(area.x + pad, area.y + pad, area.w - pad * 2.0, area.h - pad * 2.0);

        // Header: title + clear button
        let hx = gr.x;
        let mut hy = gr.y;
        text.queue(
            "Clipboard", sf, hx, hy, palette.text,
            gr.w * 0.6, screen_w, screen_h,
        );

        if !self.entries.is_empty() {
            let clear_text = "Clear All";
            let clear_w = text.measure_width(clear_text, smf);
            let clear_x = gr.x + gr.w - clear_w - pad;
            let clear_rect = Rect::new(clear_x - 6.0 * scale, hy, clear_w + 12.0 * scale, header_h);
            let cs = ix.add_zone(ZONE_CLIP_CLEAR, clear_rect);
            if cs.is_hovered() {
                painter.rect_filled(clear_rect, 6.0 * scale, palette.muted.with_alpha(0.2));
            }
            let clear_color = if cs.is_hovered() { palette.danger } else { palette.text_secondary };
            text.queue(
                clear_text, smf, clear_x, hy + (header_h - smf) * 0.5, clear_color,
                clear_w + 4.0, screen_w, screen_h,
            );
        }
        hy += header_h;

        // Search input
        let input_rect = Rect::new(hx, hy, gr.w, input_h);
        TextInput::new(input_rect)
            .text(&self.search)
            .placeholder("Search clipboard...")
            .focused(self.search_focused)
            .scale(scale)
            .cursor_pos(self.search_cursor)
            .draw(painter, text, palette, screen_w, screen_h);
        hy += input_h + lg;

        // Scrollable entry list
        let list_area = Rect::new(gr.x, hy, gr.w, gr.y + gr.h - hy);
        let filtered = self.filtered_indices();
        let content_h = filtered.len() as f32 * item_h;
        let scroll = ScrollArea::new(list_area, content_h, &mut self.scroll_offset);
        scroll.begin(painter, text);

        let mut y = scroll.content_y();
        let clip = [list_area.x, list_area.y, list_area.w, list_area.h];

        if filtered.is_empty() {
            let msg = if self.entries.is_empty() { "No clipboard history yet" } else { "No matches" };
            text.queue_clipped(msg, ef, gr.x, y + pad, palette.muted, gr.w, clip);
        }

        for (vis_idx, &real_idx) in filtered.iter().enumerate() {
            let entry = &self.entries[real_idx];
            let row_rect = Rect::new(gr.x - 4.0 * scale, y, gr.w + 8.0 * scale, item_h);
            let zone_id = ZONE_CLIP_BASE + vis_idx as u32;
            let row_state = ix.add_zone(zone_id, row_rect);

            if row_state.is_hovered() {
                painter.rect_filled(row_rect, 6.0 * scale, palette.muted.with_alpha(0.12));
            }

            // Preview text (truncated)
            let preview = entry.text.replace('\n', " ");
            let preview = if preview.len() > 80 {
                format!("{}...", &preview[..77])
            } else {
                preview
            };
            text.queue_clipped(&preview, ef, gr.x, y + 4.0 * scale, palette.text, gr.w - 40.0 * scale, clip);

            // Time ago
            let ago = format_ago(entry.when);
            let ago_w = text.measure_width(&ago, smf);
            text.queue_clipped(
                &ago, smf, gr.x + gr.w - ago_w - 30.0 * scale,
                y + item_h - smf - 6.0 * scale, palette.muted, ago_w + 4.0, clip,
            );

            // Delete button on hover
            if row_state.is_hovered() {
                let del_x = gr.x + gr.w - 24.0 * scale;
                let del_rect = Rect::new(del_x - 4.0 * scale, y + 4.0 * scale, 24.0 * scale, item_h - 8.0 * scale);
                let del_state = ix.add_zone(ZONE_CLIP_DEL_BASE + vis_idx as u32, del_rect);
                if del_state.is_hovered() {
                    painter.rect_filled(del_rect, 4.0 * scale, palette.danger.with_alpha(0.2));
                }
                text.queue_clipped("x", ef, del_x, y + (item_h - ef) * 0.5, palette.danger, 20.0 * scale, clip);
            }

            // Separator
            if vis_idx + 1 < filtered.len() {
                painter.rect_filled(
                    Rect::new(gr.x, y + item_h - 1.0, gr.w, 1.0),
                    0.0, palette.muted.with_alpha(0.1),
                );
            }

            y += item_h;
        }

        scroll.end(painter, text);
        if scroll.is_scrollable() {
            let sb = Scrollbar::new(&list_area, content_h, self.scroll_offset);
            sb.draw(painter, InteractionState::Idle, palette);
        }
    }
}

fn format_ago(when: Instant) -> String {
    let secs = when.elapsed().as_secs();
    if secs < 5 { "just now".into() }
    else if secs < 60 { format!("{}s ago", secs) }
    else if secs < 3600 { format!("{}m ago", secs / 60) }
    else { format!("{}h ago", secs / 3600) }
}

// ── Background clipboard monitor thread ─────────────────────────────

fn clipboard_thread(tx: mpsc::Sender<ClipEvent>, cmd_rx: mpsc::Receiver<ClipCmd>) {
    use wayland_client::{
        globals::{registry_queue_init, GlobalListContents},
        protocol::{wl_registry, wl_seat},
        Connection, Dispatch, QueueHandle,
    };
    use wayland_protocols_wlr::data_control::v1::client::{
        zwlr_data_control_device_v1, zwlr_data_control_manager_v1,
        zwlr_data_control_offer_v1, zwlr_data_control_source_v1,
    };

    struct MonState {
        latest_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
        latest_mimes: Vec<String>,
        clipboard_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
        clipboard_mimes: Vec<String>,
        got_selection: bool,
        just_set: bool,
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for MonState {
        fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event,
            _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<wl_seat::WlSeat, ()> for MonState {
        fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event,
            _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, ()> for MonState {
        fn event(_: &mut Self, _: &zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
            _: zwlr_data_control_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for MonState {
        fn event(state: &mut Self, _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
            event: zwlr_data_control_device_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            match event {
                zwlr_data_control_device_v1::Event::DataOffer { .. } => {
                    state.latest_mimes.clear();
                }
                zwlr_data_control_device_v1::Event::Selection { id } => {
                    if id.is_some() && state.latest_offer.is_some() {
                        state.clipboard_offer = state.latest_offer.take();
                        state.clipboard_mimes = std::mem::take(&mut state.latest_mimes);
                    }
                    state.got_selection = true;
                }
                _ => {}
            }
        }
        wayland_client::event_created_child!(MonState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
            zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ())
        ]);
    }
    impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for MonState {
        fn event(state: &mut Self, proxy: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
            event: zwlr_data_control_offer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
                if state.latest_offer.is_none() {
                    state.latest_offer = Some(proxy.clone());
                }
                state.latest_mimes.push(mime_type);
            }
        }
    }
    impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, Arc<Vec<u8>>> for MonState {
        fn event(state: &mut Self, _: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
            event: zwlr_data_control_source_v1::Event, data: &Arc<Vec<u8>>,
            _: &Connection, _: &QueueHandle<Self>) {
            match event {
                zwlr_data_control_source_v1::Event::Send { fd, .. } => {
                    let mut file = std::fs::File::from(fd);
                    let _ = file.write_all(data);
                }
                zwlr_data_control_source_v1::Event::Cancelled => {
                    state.just_set = false; // done serving
                }
                _ => {}
            }
        }
    }

    let Ok(conn) = Connection::connect_to_env() else {
        tracing::warn!("clipboard monitor: cannot connect to Wayland");
        return;
    };
    let Ok((globals, mut queue)) = registry_queue_init::<MonState>(&conn) else { return };
    let qh = queue.handle();
    let Ok(seat) = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=9, ()) else { return };
    let Ok(manager) = globals.bind::<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, _, _>(&qh, 1..=2, ()) else {
        tracing::warn!("clipboard monitor: wlr-data-control not available");
        return;
    };
    let _device = manager.get_data_device(&seat, &qh, ());
    let _ = conn.flush();
    tracing::info!("clipboard monitor: connected and listening");

    let mut state = MonState {
        latest_offer: None, latest_mimes: Vec::new(),
        clipboard_offer: None, clipboard_mimes: Vec::new(),
        got_selection: false, just_set: false,
    };

    // Skip the initial selection event (current clipboard content on connect)
    let mut skip_first = true;
    let text_mimes = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING"];

    loop {
        // Check for set-clipboard commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ClipCmd::SetClipboard(text) => {
                    let data = Arc::new(text.into_bytes());
                    let source = manager.create_data_source(&qh, data);
                    source.offer("text/plain;charset=utf-8".to_string());
                    source.offer("text/plain".to_string());
                    source.offer("UTF8_STRING".to_string());
                    source.offer("TEXT".to_string());
                    source.offer("STRING".to_string());
                    let device = manager.get_data_device(&seat, &qh, ());
                    device.set_selection(Some(&source));
                    state.just_set = true;
                    let _ = conn.flush();
                }
            }
        }

        // Dispatch with timeout so we can check cmd_rx periodically
        if let Some(guard) = queue.prepare_read() {
            use std::os::fd::AsRawFd;
            let raw_fd = guard.connection_fd().as_raw_fd();
            let mut poll_fd = libc::pollfd {
                fd: raw_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ret = unsafe { libc::poll(&mut poll_fd, 1, 100) }; // 100ms timeout
            if ret > 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
        }
        if queue.dispatch_pending(&mut state).is_err() {
            break;
        }

        // Process new selection
        if state.got_selection {
            state.got_selection = false;
            if skip_first {
                skip_first = false;
                continue;
            }
            if state.just_set {
                // Skip — this is our own set_selection echoing back
                state.just_set = false;
                continue;
            }
            if let Some(offer) = state.clipboard_offer.as_ref() {
                let has_text = state.clipboard_mimes.iter()
                    .any(|m| text_mimes.contains(&m.as_str()));
                if has_text {
                    let mime = text_mimes.iter()
                        .find(|t| state.clipboard_mimes.contains(&t.to_string()))
                        .unwrap_or(&"text/plain");
                    if let Ok((read_fd, write_fd)) = pipe() {
                        offer.receive(mime.to_string(), write_fd.as_fd());
                        let _ = conn.flush();
                        drop(write_fd);
                        let mut data = Vec::new();
                        let mut reader = std::fs::File::from(read_fd);
                        let _ = reader.read_to_end(&mut data);
                        data.truncate(MAX_READ);
                        if let Ok(text) = String::from_utf8(data) {
                            if !text.is_empty() {
                                tracing::debug!("clipboard: new entry ({}b)", text.len());
                                if tx.send(ClipEvent::NewEntry(text)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if ret != 0 { return Err(std::io::Error::last_os_error()); }
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}
