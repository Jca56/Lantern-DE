//! Audio widget — icon in bar + full mixer popup (volume, mic, output switching).
//! All wpctl interaction runs in a background thread.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::svg_icon::IconCache;

const ICON_DIR: &str = "/home/alva/.config/lntrn-bar/icons";
const POLL_INTERVAL_MS: u64 = 5_000;

// Zone IDs
pub const ZONE_AUDIO_ICON: u32 = 0xAD_0000;
const ZONE_VOL_SLIDER: u32 = 0xAD_0001;
const ZONE_MUTE_BTN: u32 = 0xAD_0002;
const ZONE_MIC_SLIDER: u32 = 0xAD_0003;
const ZONE_MIC_MUTE: u32 = 0xAD_0004;
const ZONE_SINK_BASE: u32 = 0xAD_0100;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AudioSink {
    id: u32,
    name: String,
    is_default: bool,
}

#[derive(Debug, Clone)]
struct FullState {
    volume: f32,
    muted: bool,
    mic_volume: f32,
    mic_muted: bool,
    sinks: Vec<AudioSink>,
}

enum AudioCmd {
    SetVolume(f32),
    SetMicVolume(f32),
    ToggleMute,
    ToggleMicMute,
    SetDefaultSink(u32),
}

enum AudioEvent {
    State(FullState),
}

// ── Widget ──────────────────────────────────────────────────────────────────

pub struct Audio {
    volume: f32,
    muted: bool,
    mic_volume: f32,
    mic_muted: bool,
    sinks: Vec<AudioSink>,
    event_rx: mpsc::Receiver<AudioEvent>,
    cmd_tx: mpsc::Sender<AudioCmd>,
    icons_loaded: bool,
    pub open: bool,
    // Cached slider rects for click position calculation
    vol_slider_rect: Option<Rect>,
    mic_slider_rect: Option<Rect>,
    /// Which slider zone is being dragged (None = not dragging)
    dragging: Option<u32>,
}

impl Audio {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("audio-poll".into())
            .spawn(move || poll_thread(event_tx, cmd_rx))
            .expect("spawn audio poll thread");

        Self {
            volume: 0.0, muted: false,
            mic_volume: 0.0, mic_muted: false,
            sinks: Vec::new(),
            event_rx, cmd_tx,
            icons_loaded: false,
            open: false,
            vol_slider_rect: None,
            mic_slider_rect: None,
            dragging: None,
        }
    }

    pub fn tick(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AudioEvent::State(s) => {
                    // Don't overwrite values we're actively dragging
                    if self.dragging != Some(ZONE_VOL_SLIDER) {
                        self.volume = s.volume;
                    }
                    self.muted = s.muted;
                    if self.dragging != Some(ZONE_MIC_SLIDER) {
                        self.mic_volume = s.mic_volume;
                    }
                    self.mic_muted = s.mic_muted;
                    self.sinks = s.sinks;
                }
            }
        }
    }

    pub fn handle_click(&mut self, ix: &InteractionContext, phys_cx: f32, _phys_cy: f32) {
        if let Some(zone) = ix.zone_at(phys_cx, _phys_cy) {
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
            }
        }
    }

    /// Call every frame with current cursor x while the mouse is held down.
    pub fn handle_drag(&mut self, phys_cx: f32) {
        match self.dragging {
            Some(ZONE_VOL_SLIDER) => self.set_vol_from_x(phys_cx),
            Some(ZONE_MIC_SLIDER) => self.set_mic_from_x(phys_cx),
            _ => {}
        }
    }

    pub fn on_release(&mut self) {
        self.dragging = None;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// Snap a 0.0–1.0 fraction to the nearest 5%.
    fn snap_5(frac: f32) -> f32 {
        (frac * 20.0).round() / 20.0
    }

    fn set_vol_from_x(&mut self, phys_cx: f32) {
        if let Some(rect) = self.vol_slider_rect {
            let frac = ((phys_cx - rect.x) / rect.w).clamp(0.0, 1.0);
            let vol = Self::snap_5(frac) * 1.2; // max 120%
            let _ = self.cmd_tx.send(AudioCmd::SetVolume(vol));
            self.volume = vol;
        }
    }

    fn set_mic_from_x(&mut self, phys_cx: f32) {
        if let Some(rect) = self.mic_slider_rect {
            let frac = ((phys_cx - rect.x) / rect.w).clamp(0.0, 1.0);
            let snapped = Self::snap_5(frac);
            let _ = self.cmd_tx.send(AudioCmd::SetMicVolume(snapped));
            self.mic_volume = snapped;
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if !self.open { return; }
        let step = if delta > 0.0 { -0.05 } else { 0.05 };
        let new_vol = Self::snap_5(((self.volume + step) / 1.2).clamp(0.0, 1.0)) * 1.2;
        let _ = self.cmd_tx.send(AudioCmd::SetVolume(new_vol));
        self.volume = new_vol;
    }

    fn icon_key(&self) -> &'static str {
        if self.muted || self.volume < 0.01 { return "sound-muted"; }
        match (self.volume * 100.0) as u32 {
            0..=25 => "sound-low",
            26..=60 => "sound-medium",
            _ => "sound-high",
        }
    }

    pub fn load_icons(
        &mut self, icons: &mut IconCache, tex_pass: &TexturePass, gpu: &GpuContext, size: u32,
    ) {
        if self.icons_loaded { return; }
        let dir = Path::new(ICON_DIR);
        for (key, file) in [
            ("sound-high", "spark-sound-high.svg"),
            ("sound-medium", "spark-sound-medium.svg"),
            ("sound-low", "spark-sound-low.svg"),
            ("sound-muted", "spark-sound-muted.svg"),
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
        ix.add_zone(ZONE_AUDIO_ICON, Rect::new(x, icon_y, icon_size, icon_size));
        (icon_size, tex_draws)
    }

    pub fn draw_popup(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        audio_x: f32, audio_w: f32, bar_y: f32, scale: f32,
        screen_w: u32, screen_h: u32,
    ) {
        if !self.open { return; }

        let pad = 20.0 * scale;
        let corner_r = 12.0 * scale;
        let gap = 8.0 * scale;
        let popup_w = 360.0 * scale;
        let title_font = 24.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let slider_h = 12.0 * scale;
        let section_gap = 16.0 * scale;
        let row_h = 42.0 * scale;
        let mute_btn_size = 32.0 * scale;

        // Content height
        let sink_count = self.sinks.len();
        let mut content_h = title_font + section_gap; // "Volume 80%"
        content_h += slider_h + section_gap;           // volume slider
        content_h += 1.0 * scale + section_gap;        // separator
        content_h += body_font + section_gap;           // "Microphone"
        content_h += slider_h + section_gap;            // mic slider
        if sink_count > 0 {
            content_h += 1.0 * scale + section_gap;     // separator
            content_h += small_font + section_gap * 0.5; // "Output"
            content_h += sink_count as f32 * row_h;
        }

        let popup_h = pad * 2.0 + content_h;
        let popup_x = (audio_x + audio_w / 2.0 - popup_w / 2.0)
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

        // ── Volume section ──
        let vol_pct = (self.volume * 100.0).round() as u32;
        let vol_label = if self.muted {
            "Volume — Muted".to_string()
        } else {
            format!("Volume — {}%", vol_pct)
        };
        text.queue(&vol_label, title_font, cx, y, palette.text, cw - mute_btn_size - gap, screen_w, screen_h);

        // Mute button (right side)
        let mute_x = cx + cw - mute_btn_size;
        let mute_y = y + (title_font - mute_btn_size) / 2.0;
        let mute_rect = Rect::new(mute_x, mute_y, mute_btn_size, mute_btn_size);
        let mute_state = ix.add_zone(ZONE_MUTE_BTN, mute_rect);
        let mute_hovered = mute_state.is_hovered();
        let mute_bg = if mute_hovered {
            palette.muted.with_alpha(0.3)
        } else if self.muted {
            palette.danger.with_alpha(0.2)
        } else {
            Color::TRANSPARENT
        };
        painter.rect_filled(mute_rect, 6.0 * scale, mute_bg);
        let mute_label = if self.muted { "🔇" } else { "🔊" };
        let mute_ty = mute_y + (mute_btn_size - small_font) / 2.0;
        text.queue(mute_label, small_font, mute_x + 6.0 * scale, mute_ty,
            if self.muted { palette.danger } else { palette.text }, mute_btn_size, screen_w, screen_h);

        y += title_font + section_gap;

        // Volume slider
        self.vol_slider_rect = Some(Rect::new(cx, y, cw, slider_h));
        self.draw_slider(painter, ix, palette, cx, y, cw, slider_h, scale,
            self.volume / 1.2, self.muted, ZONE_VOL_SLIDER);
        y += slider_h + section_gap;

        // Separator
        painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
        y += 1.0 * scale + section_gap;

        // ── Microphone section ──
        let mic_pct = (self.mic_volume * 100.0).round() as u32;
        let mic_label = if self.mic_muted {
            "Microphone — Muted".to_string()
        } else {
            format!("Microphone — {}%", mic_pct)
        };
        text.queue(&mic_label, body_font, cx, y, palette.text, cw - mute_btn_size - gap, screen_w, screen_h);

        // Mic mute button
        let mic_mute_x = cx + cw - mute_btn_size;
        let mic_mute_y = y + (body_font - mute_btn_size) / 2.0;
        let mic_mute_rect = Rect::new(mic_mute_x, mic_mute_y, mute_btn_size, mute_btn_size);
        let mic_state = ix.add_zone(ZONE_MIC_MUTE, mic_mute_rect);
        let mic_hovered = mic_state.is_hovered();
        let mic_bg = if mic_hovered {
            palette.muted.with_alpha(0.3)
        } else if self.mic_muted {
            palette.danger.with_alpha(0.2)
        } else {
            Color::TRANSPARENT
        };
        painter.rect_filled(mic_mute_rect, 6.0 * scale, mic_bg);
        let mic_mute_label = if self.mic_muted { "🔇" } else { "🎤" };
        let mic_mute_ty = mic_mute_y + (mute_btn_size - small_font) / 2.0;
        text.queue(mic_mute_label, small_font, mic_mute_x + 6.0 * scale, mic_mute_ty,
            if self.mic_muted { palette.danger } else { palette.text }, mute_btn_size, screen_w, screen_h);

        y += body_font + section_gap;

        // Mic slider
        self.mic_slider_rect = Some(Rect::new(cx, y, cw, slider_h));
        self.draw_slider(painter, ix, palette, cx, y, cw, slider_h, scale,
            self.mic_volume, self.mic_muted, ZONE_MIC_SLIDER);
        y += slider_h + section_gap;

        // ── Output devices ──
        if !self.sinks.is_empty() {
            painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            text.queue("Output", small_font, cx, y, palette.muted, cw, screen_w, screen_h);
            y += small_font + section_gap * 0.5;

            for (idx, sink) in self.sinks.iter().enumerate() {
                let row_rect = Rect::new(cx, y, cw, row_h);
                let zone_id = ZONE_SINK_BASE + idx as u32;
                let state = ix.add_zone(zone_id, row_rect);
                let hovered = state.is_hovered();

                if hovered {
                    painter.rect_filled(row_rect, 8.0 * scale, palette.muted.with_alpha(0.2));
                }
                if sink.is_default {
                    painter.rect_filled(row_rect, 8.0 * scale, palette.accent.with_alpha(0.12));
                }

                let text_y = y + (row_h - body_font) / 2.0;
                let mut lx = cx + 8.0 * scale;
                if sink.is_default {
                    text.queue("✓", body_font, lx, text_y, palette.accent, body_font, screen_w, screen_h);
                    lx += body_font + 4.0 * scale;
                }

                let name_color = if sink.is_default { palette.text } else { palette.text_secondary };
                text.queue(&sink.name, body_font, lx, text_y, name_color, cw * 0.85, screen_w, screen_h);

                y += row_h;
            }
        }
    }

    fn draw_slider(
        &self, painter: &mut Painter, ix: &mut InteractionContext, palette: &FoxPalette,
        x: f32, y: f32, w: f32, h: f32, scale: f32,
        frac: f32, is_muted: bool, zone_id: u32,
    ) {
        let track_r = h / 2.0;
        let track_rect = Rect::new(x, y, w, h);
        ix.add_zone(zone_id, track_rect);

        // Track background
        painter.rect_filled(track_rect, track_r, palette.surface);

        // Fill
        let fill_frac = frac.clamp(0.0, 1.0);
        if fill_frac > 0.0 {
            let fill_w = (w * fill_frac).max(h);
            let fill_color = if is_muted { palette.muted } else { palette.accent };
            painter.rect_filled(Rect::new(x, y, fill_w, h), track_r, fill_color);
        }

        // Thumb circle
        let thumb_x = x + w * fill_frac;
        let thumb_r = h * 0.9;
        painter.circle_filled(thumb_x, y + h / 2.0, thumb_r,
            if is_muted { palette.muted } else { palette.text });
    }

    pub fn popup_rect(
        &self, audio_x: f32, audio_w: f32, bar_y: f32, scale: f32, screen_w: u32,
    ) -> Option<Rect> {
        if !self.open { return None; }

        let pad = 20.0 * scale;
        let gap = 8.0 * scale;
        let popup_w = 360.0 * scale;
        let title_font = 24.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let slider_h = 12.0 * scale;
        let section_gap = 16.0 * scale;
        let row_h = 42.0 * scale;

        let sink_count = self.sinks.len();
        let mut content_h = title_font + section_gap + slider_h + section_gap;
        content_h += 1.0 * scale + section_gap + body_font + section_gap + slider_h + section_gap;
        if sink_count > 0 {
            content_h += 1.0 * scale + section_gap + small_font + section_gap * 0.5;
            content_h += sink_count as f32 * row_h;
        }

        let popup_h = pad * 2.0 + content_h;
        let popup_x = (audio_x + audio_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = (bar_y - popup_h - gap).max(0.0);

        Some(Rect::new(popup_x, popup_y, popup_w, popup_h))
    }
}

// ── Background thread ───────────────────────────────────────────────────────

fn poll_thread(tx: mpsc::Sender<AudioEvent>, cmd_rx: mpsc::Receiver<AudioCmd>) {
    let _ = tx.send(AudioEvent::State(poll_full_state()));
    let mut last_poll = std::time::Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                AudioCmd::SetVolume(vol) => {
                    let pct = format!("{:.0}%", vol * 100.0);
                    let _ = Command::new("wpctl")
                        .args(["set-volume", "--limit", "1.2", "@DEFAULT_AUDIO_SINK@", &pct])
                        .output();
                    let _ = tx.send(AudioEvent::State(poll_full_state()));
                }
                AudioCmd::SetMicVolume(vol) => {
                    let pct = format!("{:.0}%", vol * 100.0);
                    let _ = Command::new("wpctl")
                        .args(["set-volume", "@DEFAULT_AUDIO_SOURCE@", &pct])
                        .output();
                    let _ = tx.send(AudioEvent::State(poll_full_state()));
                }
                AudioCmd::ToggleMute => {
                    let _ = Command::new("wpctl")
                        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
                        .output();
                    let _ = tx.send(AudioEvent::State(poll_full_state()));
                }
                AudioCmd::ToggleMicMute => {
                    let _ = Command::new("wpctl")
                        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
                        .output();
                    let _ = tx.send(AudioEvent::State(poll_full_state()));
                }
                AudioCmd::SetDefaultSink(id) => {
                    let _ = Command::new("wpctl")
                        .args(["set-default", &id.to_string()])
                        .output();
                    let _ = tx.send(AudioEvent::State(poll_full_state()));
                }
            }
        }

        if last_poll.elapsed().as_millis() >= POLL_INTERVAL_MS as u128 {
            let _ = tx.send(AudioEvent::State(poll_full_state()));
            last_poll = std::time::Instant::now();
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn poll_full_state() -> FullState {
    let (volume, muted) = get_volume("@DEFAULT_AUDIO_SINK@");
    let (mic_volume, mic_muted) = get_volume("@DEFAULT_AUDIO_SOURCE@");
    let sinks = get_sinks();
    FullState { volume, muted, mic_volume, mic_muted, sinks }
}

fn get_volume(target: &str) -> (f32, bool) {
    let output = Command::new("wpctl").args(["get-volume", target]).output();
    let Ok(output) = output else { return (0.0, false) };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let muted = stdout.contains("[MUTED]");
    let volume = stdout.strip_prefix("Volume: ")
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    (volume, muted)
}

fn get_sinks() -> Vec<AudioSink> {
    let output = Command::new("wpctl").arg("status").output();
    let Ok(output) = output else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut sinks = Vec::new();
    let mut in_sinks = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        // Detect section headers
        if trimmed.contains("Sinks:") {
            in_sinks = true;
            continue;
        }
        if in_sinks && (trimmed.contains("Sources:") || trimmed.contains("Filters:")
            || trimmed.contains("Streams:") || trimmed.is_empty())
        {
            break;
        }
        if !in_sinks { continue; }

        // Parse sink lines like: "  * 100. Bose QC Ultra Headphones  [vol: 0.65]"
        // or:                    "     72. Meteor Land-P ... Speaker [vol: 0.65]"
        let is_default = trimmed.starts_with('*');
        let content = trimmed.trim_start_matches('*').trim();

        // Find the ID (number before the dot)
        if let Some(dot_pos) = content.find('.') {
            let id_str = content[..dot_pos].trim();
            let Ok(id) = id_str.parse::<u32>() else { continue };

            // Name is between the dot and the [vol: ...] bracket
            let after_dot = content[dot_pos + 1..].trim();
            let name = if let Some(bracket) = after_dot.find('[') {
                after_dot[..bracket].trim().to_string()
            } else {
                after_dot.trim().to_string()
            };

            if !name.is_empty() {
                sinks.push(AudioSink { id, name, is_default });
            }
        }
    }

    sinks
}
