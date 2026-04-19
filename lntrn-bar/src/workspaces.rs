//! Workspace pills on the bar — tab-strip visual of per-output workspace state.
//!
//! Connects to the compositor at `/run/user/{uid}/lntrn-workspaces.sock`
//! and renders one segment per populated workspace. Active segment has
//! an accent underline and filled background.
//!
//! Click → switch, right-click → context menu (handled by layershell),
//! scroll → cycle prev/next.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

pub const ZONE_WS_BASE: u32 = 0x70_0000;

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-workspaces.sock", uid))
}

#[derive(Clone, Debug, Default)]
struct OutputState {
    active: u32,
    ids: Vec<u32>,
}

pub struct WorkspacesModule {
    reader: Option<BufReader<UnixStream>>,
    writer: Option<UnixStream>,
    state: HashMap<String, OutputState>,
    /// Output name this bar instance cares about. None = use first seen.
    target_output: Option<String>,
    /// Pill rects from the last draw, for scroll hit-testing.
    last_strip: Option<Rect>,
    /// Zone id → ws_id mapping for the latest draw.
    zone_map: Vec<(u32, u32)>,
    retry_deadline: std::time::Instant,
}

impl WorkspacesModule {
    pub fn new() -> Self {
        // Lazy-connect on first poll so we don't add startup latency that could
        // race against D-Bus services warming up.
        Self {
            reader: None,
            writer: None,
            state: HashMap::new(),
            target_output: None,
            last_strip: None,
            zone_map: Vec::new(),
            retry_deadline: std::time::Instant::now(),
        }
    }

    #[allow(dead_code)]
    pub fn set_target_output(&mut self, name: String) {
        self.target_output = Some(name);
    }

    fn try_connect(&mut self) {
        let now = std::time::Instant::now();
        if now < self.retry_deadline { return; }
        self.retry_deadline = now + std::time::Duration::from_secs(2);

        let path = socket_path();
        match UnixStream::connect(&path) {
            Ok(stream) => {
                stream.set_nonblocking(true).ok();
                let writer = match stream.try_clone() {
                    Ok(w) => w,
                    Err(_) => return,
                };
                self.reader = Some(BufReader::new(stream));
                self.writer = Some(writer);
                tracing::info!(?path, "connected to workspaces IPC");
            }
            Err(e) => {
                tracing::debug!(?e, "workspaces IPC not ready, will retry");
            }
        }
    }

    /// Poll for state updates from the compositor. Returns true if state changed
    /// and the bar should redraw.
    pub fn poll(&mut self) -> bool {
        if self.reader.is_none() {
            self.try_connect();
        }
        let Some(reader) = &mut self.reader else { return false; };

        let mut changed = false;
        let mut line = String::new();
        let mut disconnect = false;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => { disconnect = true; break; }
                Ok(_) => {
                    if parse_state_line(line.trim(), &mut self.state) {
                        changed = true;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => { disconnect = true; break; }
            }
        }
        if disconnect {
            self.reader = None;
            self.writer = None;
        }
        changed
    }

    fn current_output(&self) -> Option<String> {
        self.target_output.clone().or_else(|| self.state.keys().next().cloned())
    }

    fn output_state(&self) -> Option<(&String, &OutputState)> {
        let key = self.current_output()?;
        self.state.get_key_value(&key).map(|(k, v)| (k, v))
    }

    /// Total logical width the pill strip will consume, including outer padding.
    #[allow(dead_code)]
    pub fn measure_width(&self, bar_h: f32, scale: f32) -> f32 {
        let Some((_, os)) = self.output_state() else { return 0.0; };
        if os.ids.is_empty() { return 0.0; }
        let seg_w = pill_seg_width(bar_h, scale);
        let outer_pad = 6.0 * scale;
        os.ids.len() as f32 * seg_w + outer_pad * 2.0
    }

    /// Draw the tab strip. Returns consumed width (0 when no state).
    pub fn draw(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        x: f32, y: f32, bar_h: f32, scale: f32,
        screen_w: u32, screen_h: u32,
    ) -> f32 {
        self.zone_map.clear();
        self.last_strip = None;

        let Some((_, os)) = self.output_state().map(|(k, v)| (k.clone(), v.clone())) else {
            return 0.0;
        };
        if os.ids.is_empty() { return 0.0; }

        let seg_w = pill_seg_width(bar_h, scale);
        let outer_pad = 6.0 * scale;
        let strip_w = os.ids.len() as f32 * seg_w;
        let strip_x = x + outer_pad;
        let top_pad = bar_h * 0.15;
        let strip_y = y + top_pad;
        let strip_h = bar_h - top_pad * 2.0;
        let radius = strip_h * 0.22;

        // Background strip
        let strip_rect = Rect::new(strip_x, strip_y, strip_w, strip_h);
        painter.rect_filled(strip_rect, radius, palette.muted.with_alpha(0.18));
        self.last_strip = Some(strip_rect);

        let font_size = strip_h * 0.62;
        let char_w = font_size * 0.58;

        for (i, &id) in os.ids.iter().enumerate() {
            let seg_x = strip_x + i as f32 * seg_w;
            let seg_rect = Rect::new(seg_x, strip_y, seg_w, strip_h);
            let is_active = id == os.active;

            // Hover highlight
            let zone_id = ZONE_WS_BASE + id;
            let state = ix.add_zone(zone_id, seg_rect);
            self.zone_map.push((zone_id, id));

            if is_active {
                let fill = palette.accent.with_alpha(0.55);
                painter.rect_filled(seg_rect, radius, fill);
                // Underline accent at the bottom
                let underline_h = 3.0 * scale;
                let ul_pad = seg_w * 0.22;
                let ul_rect = Rect::new(
                    seg_x + ul_pad,
                    strip_y + strip_h - underline_h,
                    seg_w - ul_pad * 2.0,
                    underline_h,
                );
                painter.rect_filled(ul_rect, underline_h * 0.5, palette.accent);
            } else if state.is_hovered() {
                painter.rect_filled(seg_rect, radius, palette.muted.with_alpha(0.35));
            }

            // Number label
            let label = format!("{}", id);
            let label_w = label.len() as f32 * char_w;
            let label_x = seg_x + (seg_w - label_w) * 0.5;
            let label_y = strip_y + (strip_h - font_size) * 0.5 - font_size * 0.05;
            let label_color = if is_active {
                Color::WHITE
            } else {
                palette.text
            };
            text.queue(&label, font_size, label_x, label_y, label_color, 0.0, screen_w, screen_h);
        }

        // Separator between context items and common items visually handled by bar.
        strip_w + outer_pad * 2.0
    }

    /// Translate a clicked zone id to a workspace id, if it's ours.
    pub fn zone_to_ws(&self, zone: u32) -> Option<u32> {
        self.zone_map.iter().find(|(zid, _)| *zid == zone).map(|(_, ws)| *ws)
    }

    /// Send `switch` IPC command.
    pub fn send_switch(&mut self, ws: u32) {
        let Some(out) = self.current_output() else { return };
        self.send_line(&format!("switch:{}:{}", out, ws));
    }

    /// Send `move` IPC command (move focused window to ws).
    pub fn send_move(&mut self, ws: u32) {
        let Some(out) = self.current_output() else { return };
        self.send_line(&format!("move:{}:{}", out, ws));
    }

    /// Send `cycle` IPC command.
    pub fn send_cycle(&mut self, direction: i32) {
        let Some(out) = self.current_output() else { return };
        self.send_line(&format!("cycle:{}:{}", out, direction));
    }

    /// Returns true if the pointer is over the pill strip (for scroll routing).
    pub fn hit_strip(&self, x: f32, y: f32) -> bool {
        self.last_strip.map_or(false, |r| r.contains(x, y))
    }

    fn send_line(&mut self, line: &str) {
        let Some(writer) = self.writer.as_mut() else { return };
        let _ = writer.set_nonblocking(true);
        let payload = format!("{}\n", line);
        match writer.write(payload.as_bytes()) {
            Ok(n) if n == payload.len() => {}
            Ok(_) | Err(_) => {
                // Partial write or error — drop connection, will retry.
                self.writer = None;
                self.reader = None;
            }
        }
    }
}

fn pill_seg_width(bar_h: f32, scale: f32) -> f32 {
    // Big enough for two-digit numbers (WS 10+) and comfortable click target.
    (bar_h * 0.55).max(36.0 * scale)
}

fn parse_state_line(msg: &str, state: &mut HashMap<String, OutputState>) -> bool {
    let parts: Vec<&str> = msg.splitn(4, ':').collect();
    if parts.len() != 4 || parts[0] != "state" { return false; }
    let output = parts[1].to_string();
    let active: u32 = match parts[2].parse() { Ok(v) => v, Err(_) => return false };
    let ids: Vec<u32> = if parts[3].is_empty() {
        Vec::new()
    } else {
        parts[3].split(',').filter_map(|s| s.parse().ok()).collect()
    };
    let new_state = OutputState { active, ids };
    let old = state.get(&output);
    let changed = old.map_or(true, |o| o.active != new_state.active || o.ids != new_state.ids);
    state.insert(output, new_state);
    changed
}
