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
use std::time::{Duration, Instant};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

const POLL_INTERVAL: Duration = Duration::from_millis(750);
/// `wpctl status` is more expensive than get-volume; only re-fetch the
/// sink list every few seconds.
const SINK_LIST_POLL_INTERVAL: Duration = Duration::from_secs(3);

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
enum AudioCmd {
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
enum AudioEvent {
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
                .spawn(move || worker(event_tx, cmd_rx))
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

    /// Set the default sink to the given normalized volume. Clamps to
    /// `[0.0, 1.0]` so the user can't accidentally crank to 150 % via
    /// drag overshoot.
    pub fn set_volume(&mut self, v: f32) {
        let clamped = v.clamp(0.0, 1.0);
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
        let clamped = v.clamp(0.0, 1.0);
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

// ── Worker thread ───────────────────────────────────────────────────────────

fn worker(tx: mpsc::Sender<AudioEvent>, cmd_rx: mpsc::Receiver<AudioCmd>) {
    // Prime the UI with whatever we can read right away.
    poll_volumes(&tx);
    poll_devices(&tx);

    let mut last_poll = Instant::now();
    let mut last_sink_list_poll = Instant::now();

    loop {
        // Drain pending commands. Any setter triggers an immediate
        // re-poll so the cached state catches up without waiting for
        // the next tick.
        let mut force_volume_repoll = false;
        let mut force_devices_repoll = false;
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                AudioCmd::Rescan => {
                    force_volume_repoll = true;
                    force_devices_repoll = true;
                }
                AudioCmd::SetVolume(v) => {
                    let arg = format!("{:.2}", v);
                    let _ = Command::new("wpctl")
                        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &arg])
                        .status();
                    force_volume_repoll = true;
                }
                AudioCmd::SetInputVolume(v) => {
                    let arg = format!("{:.2}", v);
                    let _ = Command::new("wpctl")
                        .args(["set-volume", "@DEFAULT_AUDIO_SOURCE@", &arg])
                        .status();
                    force_volume_repoll = true;
                }
                AudioCmd::ToggleMute => {
                    let _ = Command::new("wpctl")
                        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
                        .status();
                    force_volume_repoll = true;
                }
                AudioCmd::ToggleInputMute => {
                    let _ = Command::new("wpctl")
                        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
                        .status();
                    force_volume_repoll = true;
                }
                AudioCmd::SetDefaultSink(id) => {
                    let _ = Command::new("wpctl")
                        .args(["set-default", &id.to_string()])
                        .status();
                    force_volume_repoll = true;
                    force_devices_repoll = true;
                }
                AudioCmd::SetDefaultSource(id) => {
                    let _ = Command::new("wpctl")
                        .args(["set-default", &id.to_string()])
                        .status();
                    force_volume_repoll = true;
                    force_devices_repoll = true;
                }
            }
        }

        if force_volume_repoll || last_poll.elapsed() >= POLL_INTERVAL {
            poll_volumes(&tx);
            last_poll = Instant::now();
        }
        if force_devices_repoll || last_sink_list_poll.elapsed() >= SINK_LIST_POLL_INTERVAL {
            poll_devices(&tx);
            last_sink_list_poll = Instant::now();
        }

        thread::sleep(Duration::from_millis(100));
    }
}

/// Read default sink + source volumes from wpctl and emit events.
fn poll_volumes(tx: &mpsc::Sender<AudioEvent>) {
    // Output sink.
    let out = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            if let Some((vol, muted)) = parse_get_volume(&s) {
                let _ = tx.send(AudioEvent::Output {
                    volume: vol.clamp(0.0, 1.5),
                    muted,
                    available: true,
                });
            }
        }
        _ => {
            let _ = tx.send(AudioEvent::Output {
                volume: 0.0,
                muted: false,
                available: false,
            });
        }
    }

    // Input source — failure here doesn't disable the tile.
    let out = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
        .output();
    if let Ok(o) = out {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout);
            if let Some((vol, muted)) = parse_get_volume(&s) {
                let _ = tx.send(AudioEvent::Input {
                    volume: vol.clamp(0.0, 1.5),
                    muted,
                });
            }
        }
    }
}

/// Read sink + source device lists from `wpctl status`.
fn poll_devices(tx: &mpsc::Sender<AudioEvent>) {
    let out = Command::new("wpctl").arg("status").output();
    if let Ok(o) = out {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout);
            let sinks = parse_devices(&s, "Sinks:");
            let sources = parse_devices(&s, "Sources:");
            let _ = tx.send(AudioEvent::Devices { sinks, sources });
        }
    }
}

/// Parse `wpctl get-volume` output: "Volume: 0.65" or "Volume: 0.65 [MUTED]".
fn parse_get_volume(s: &str) -> Option<(f32, bool)> {
    // Format is consistent enough that a small ad-hoc parse is fine.
    let after = s.trim().strip_prefix("Volume:")?.trim();
    let muted = after.contains("[MUTED]");
    let num_str = after.split_whitespace().next()?;
    let v: f32 = num_str.parse().ok()?;
    Some((v, muted))
}

/// Substrings that mark a sink as an HDMI / DisplayPort audio output.
/// We filter these out of the sink picker — they're cluttery on most
/// laptops and the user's external monitor + speakers usually aren't
/// what they want to send audio to. Edit / clear this array if you do
/// want to send audio to a connected display.
const HIDDEN_SINK_SUBSTRINGS: &[&str] = &["HDMI", "DisplayPort"];

/// Parse one of the device sections (`Sinks:` or `Sources:`) of
/// `wpctl status`. Each device line looks like:
///   `│  *   61. Meteor Lake-P HD Audio Controller Speaker [vol: 0.65]`
/// The leading column may have a `*` for the default; the trailing
/// `[vol: ...]` is dropped.
///
/// `section_header` is the literal section name to scan for (`"Sinks:"`
/// or `"Sources:"`). Parsing stops at the next non-`Sinks` / non-`Sources`
/// header so we don't bleed into Filters/Streams.
fn parse_devices(status: &str, section_header: &str) -> Vec<Sink> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in status.lines() {
        let trimmed = line.trim_end();
        if trimmed.contains(section_header) {
            in_section = true;
            continue;
        }
        if in_section {
            // Any other top-level section header (Sinks/Sources/Filters/
            // Streams/Devices/...) ends our section.
            let is_other_header = trimmed.contains("Sinks:")
                || trimmed.contains("Sources:")
                || trimmed.contains("Filters:")
                || trimmed.contains("Streams:")
                || trimmed.contains("Devices:");
            if is_other_header {
                break;
            }
        }
        if !in_section {
            continue;
        }
        // Strip the leading box-drawing chars + whitespace + optional `*`.
        let stripped: String = line
            .chars()
            .skip_while(|c| !c.is_ascii_digit() && *c != '*')
            .collect();
        let stripped = stripped.trim_start_matches('*').trim();

        // Now we expect "ID. Name [vol: X.YZ]".
        let Some(dot_idx) = stripped.find('.') else { continue };
        let id_str = &stripped[..dot_idx];
        let id: u32 = match id_str.trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let after_dot = stripped[dot_idx + 1..].trim();
        // Drop trailing "[vol: ...]"
        let name = match after_dot.rfind('[') {
            Some(i) => after_dot[..i].trim().to_string(),
            None => after_dot.to_string(),
        };
        let is_default = line.contains('*');
        // Filter HDMI / DisplayPort sinks (output only — sources stay
        // unfiltered since extra mics are usually intentional).
        if section_header == "Sinks:"
            && HIDDEN_SINK_SUBSTRINGS.iter().any(|needle| name.contains(needle))
        {
            continue;
        }
        out.push(Sink { id, name, is_default });
    }
    out
}

// ── Inline tile ─────────────────────────────────────────────────────────────

const ICON_SIZE: f32 = 22.0;
const ICON_BAR_GAP: f32 = 10.0;
const BAR_WIDTH: f32 = 120.0;
const BAR_HEIGHT: f32 = 8.0;
const BAR_TRACK_RGB: (u8, u8, u8) = (60, 60, 60);
/// The expanded view's slider fill is gold (matches "lines = gold"),
/// but the inline tile keeps a tiny white bar so the row reads as a
/// neutral status strip without competing accents.
const BAR_FILL_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
/// Mute slash color — red so it's unambiguous.
const MUTE_SLASH_RGB: (u8, u8, u8) = (0xe0, 0x40, 0x40);

/// Logical px the audio tile asks for in the row layout — speaker icon
/// + small gap + 120pt bar.
pub const TILE_WIDTH: f32 = 22.0 + 10.0 + 120.0;

pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    audio: &Audio,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    _surface_w: u32,
    _surface_h: u32,
) {
    if !audio.is_present() {
        return;
    }

    let icon_size = ICON_SIZE * scale;
    let icon_bar_gap = ICON_BAR_GAP * scale;
    let bar_w = BAR_WIDTH * scale;
    let bar_h = BAR_HEIGHT * scale;

    // The slot is now content-sized (TILE_WIDTH), so left-align the
    // group: speaker at slot.x, bar to its right.
    let group_x = layout.x;

    let icon_y = layout.y + (layout.h - icon_size) / 2.0;
    draw_speaker(painter, group_x, icon_y, icon_size, icon_size, audio.is_muted(), alpha);

    // Volume bar.
    let bar_x = group_x + icon_size + icon_bar_gap;
    let bar_y = layout.y + (layout.h - bar_h) / 2.0;
    let radius = bar_h * 0.5;

    // Track.
    painter.rect_filled(
        Rect::new(bar_x, bar_y, bar_w, bar_h),
        radius,
        Color::from_rgb8(BAR_TRACK_RGB.0, BAR_TRACK_RGB.1, BAR_TRACK_RGB.2)
            .with_alpha(alpha),
    );

    // Fill — proportional to volume, clamped at 100% (anything over is
    // boost territory and the inline visual just sits at full).
    let v = if audio.is_muted() { 0.0 } else { audio.volume().min(1.0) };
    if v > 0.0 {
        let raw = bar_w * v;
        let fill_w = raw.max(bar_h);
        painter.rect_filled(
            Rect::new(bar_x, bar_y, fill_w, bar_h),
            radius,
            Color::from_rgb8(BAR_FILL_RGB.0, BAR_FILL_RGB.1, BAR_FILL_RGB.2)
                .with_alpha(alpha),
        );
    }
}

/// Draw a stylised speaker (cone + box). When `muted` is true, a red
/// diagonal slash is drawn across it. Pure polygons — same approach as
/// the lightning bolt.
fn draw_speaker(
    painter: &mut Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    muted: bool,
    alpha: f32,
) {
    let pt = |fx: f32, fy: f32| (x + fx * w, y + fy * h);
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);

    // Speaker silhouette: small box on the left + flared cone on the right.
    //
    //         ╱│
    //   ┌──┐ ╱ │
    //   │  │   │
    //   │  │   │
    //   └──┘ ╲ │
    //         ╲│
    //
    // Decompose into rectangle (the box part) + two triangles (cone).
    // Box: 0.0..0.35 horiz, 0.30..0.70 vert.
    let (bx0, by0) = pt(0.0, 0.30);
    let (bx1, by1) = pt(0.35, 0.70);
    painter.rect_filled(
        Rect::new(bx0, by0, bx1 - bx0, by1 - by0),
        0.0,
        color,
    );
    // Cone — two triangles forming a pentagon. Top half: top-left of box,
    // top-right of cone, midline-right.
    let cone_top_left = pt(0.35, 0.30);
    let cone_top_right = pt(0.95, 0.0);
    let cone_mid_right = pt(0.95, 0.5);
    let cone_bot_left = pt(0.35, 0.70);
    let cone_bot_right = pt(0.95, 1.0);
    painter.triangle(
        cone_top_left.0, cone_top_left.1,
        cone_top_right.0, cone_top_right.1,
        cone_mid_right.0, cone_mid_right.1,
        color,
    );
    painter.triangle(
        cone_top_left.0, cone_top_left.1,
        cone_mid_right.0, cone_mid_right.1,
        cone_bot_left.0, cone_bot_left.1,
        color,
    );
    painter.triangle(
        cone_bot_left.0, cone_bot_left.1,
        cone_mid_right.0, cone_mid_right.1,
        cone_bot_right.0, cone_bot_right.1,
        color,
    );

    if muted {
        // Diagonal red slash — bottom-left to top-right corner.
        let red = Color::from_rgb8(MUTE_SLASH_RGB.0, MUTE_SLASH_RGB.1, MUTE_SLASH_RGB.2)
            .with_alpha(alpha);
        let p1 = pt(0.0, 1.0);
        let p2 = pt(1.0, 0.0);
        painter.line(p1.0, p1.1, p2.0, p2.1, w * 0.12, red);
    }
}

/// Draw a stylised microphone — rounded "head" capsule + thin neck +
/// wide base. When muted, the same red diagonal slash as the speaker.
fn draw_mic(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, muted: bool, alpha: f32) {
    let pt = |fx: f32, fy: f32| (x + fx * w, y + fy * h);
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);

    // Head: rounded vertical capsule centered on x, y in [0.10, 0.65].
    let head_w = w * 0.45;
    let head_x = x + (w - head_w) / 2.0;
    let head_top = y + 0.10 * h;
    let head_h = 0.55 * h;
    painter.rect_filled(
        Rect::new(head_x, head_top, head_w, head_h),
        head_w * 0.5,
        color,
    );

    // Neck — thin vertical strip from head bottom to base top.
    let neck_w = w * 0.12;
    let neck_x = x + (w - neck_w) / 2.0;
    let neck_top = head_top + head_h;
    let neck_h = 0.18 * h;
    painter.rect_filled(
        Rect::new(neck_x, neck_top, neck_w, neck_h),
        0.0,
        color,
    );

    // Base — wider horizontal strip at the bottom.
    let base_w = w * 0.70;
    let base_x = x + (w - base_w) / 2.0;
    let base_top = neck_top + neck_h;
    let base_h = 0.08 * h;
    painter.rect_filled(
        Rect::new(base_x, base_top, base_w, base_h),
        base_h * 0.5,
        color,
    );

    if muted {
        let red = Color::from_rgb8(MUTE_SLASH_RGB.0, MUTE_SLASH_RGB.1, MUTE_SLASH_RGB.2)
            .with_alpha(alpha);
        let p1 = pt(0.05, 0.95);
        let p2 = pt(0.95, 0.05);
        painter.line(p1.0, p1.1, p2.0, p2.1, w * 0.12, red);
    }
}

// ── Click-expand panel ──────────────────────────────────────────────────────

const VIEW_TOP_PAD: f32 = 20.0;
const SECTION_HEADER_FONT: f32 = 22.0;
const SECTION_HEADER_BOTTOM_GAP: f32 = 10.0;
const SLIDER_PERCENT_FONT: f32 = 36.0;
const SLIDER_PERCENT_GAP: f32 = 16.0;
const SLIDER_HEIGHT: f32 = 12.0;
const SLIDER_BOTTOM_GAP: f32 = 16.0;
const DEVICE_ROW_HEIGHT: f32 = 44.0;
const DEVICE_FONT: f32 = 22.0;
const DEVICE_DOT_SIZE: f32 = 10.0;
const SECTION_GAP: f32 = 28.0;

/// Icon at the left of each slider row (logical px). Click toggles
/// mute for that section.
const ROW_ICON_SIZE: f32 = 28.0;
const ROW_ICON_GAP: f32 = 14.0;
/// Max devices we render per section. The view fits comfortably with
/// 4; if a system has more sinks/sources than that we just show the
/// top 4 (the default is guaranteed to be in the parsed list).
const MAX_DEVICE_ROWS: usize = 4;

/// Retained from the old expanded-panel sizing math; the panel-mode
/// rework made it unused. Keeping for reference.
#[allow(dead_code)]
pub const EXPANDED_HEIGHT: f32 = 0.0;

/// Which audio direction a section addresses. Used so one piece of
/// layout code drives both the output and input sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Output,
    Input,
}

/// Vertical offset (logical px) inside the audio view at which a
/// section begins. Output is first, then input below it.
fn section_top_logical(dir: Direction) -> f32 {
    let section_h = section_logical_height();
    match dir {
        Direction::Output => VIEW_TOP_PAD,
        Direction::Input => VIEW_TOP_PAD + section_h + SECTION_GAP,
    }
}

/// Logical height of one section (header + slider row + device rows).
fn section_logical_height() -> f32 {
    SECTION_HEADER_FONT
        + SECTION_HEADER_BOTTOM_GAP
        + SLIDER_PERCENT_FONT
        + SLIDER_BOTTOM_GAP
        + DEVICE_ROW_HEIGHT * MAX_DEVICE_ROWS as f32
}

/// Y coordinate (physical px) of the slider track for the given section.
fn slider_top_y(panel_top_y: f32, dir: Direction, scale: f32) -> f32 {
    let section_top = panel_top_y + section_top_logical(dir) * scale;
    section_top + (SECTION_HEADER_FONT + SECTION_HEADER_BOTTOM_GAP) * scale
}

/// Y coordinate (physical px) where the device list for the given
/// section starts (just below the slider row).
fn device_list_top_y_for(panel_top_y: f32, dir: Direction, scale: f32) -> f32 {
    slider_top_y(panel_top_y, dir, scale) + (SLIDER_PERCENT_FONT + SLIDER_BOTTOM_GAP) * scale
}

/// Mute icon (speaker / mic) rect for the given section. Click here
/// toggles mute for that direction.
pub fn icon_rect_for(panel: Rect, panel_top_y: f32, dir: Direction, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let percent_font = SLIDER_PERCENT_FONT * scale;
    let icon_size = ROW_ICON_SIZE * scale;
    let inner_x = panel.x + pad;
    // Vertically center the icon against the slider row (= percent_font tall).
    let row_top = slider_top_y(panel_top_y, dir, scale);
    let icon_y = row_top + (percent_font - icon_size) / 2.0;
    Rect::new(inner_x, icon_y, icon_size, icon_size)
}

/// Hit-test pointer position against either section's mute icon.
pub fn hit_test_icon(panel: Rect, panel_top_y: f32, scale: f32, x: f32, y: f32) -> Option<Direction> {
    for &dir in &[Direction::Output, Direction::Input] {
        let r = icon_rect_for(panel, panel_top_y, dir, scale);
        if x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h {
            return Some(dir);
        }
    }
    None
}

/// Layout helper: returns the slider's track rect (physical px) for the
/// given section, used by hit testing. The slider sits to the right of
/// the mute icon.
pub fn slider_rect_for(panel: Rect, panel_top_y: f32, dir: Direction, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let percent_font = SLIDER_PERCENT_FONT * scale;
    let percent_gap = SLIDER_PERCENT_GAP * scale;
    let percent_w = percent_font * 2.6;
    let icon_size = ROW_ICON_SIZE * scale;
    let icon_gap = ROW_ICON_GAP * scale;

    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let slider_x = inner_x + icon_size + icon_gap;
    let slider_w = inner_w - icon_size - icon_gap - percent_w - percent_gap;
    let slider_h = SLIDER_HEIGHT * scale;
    let slider_y = slider_top_y(panel_top_y, dir, scale) + (percent_font - slider_h) / 2.0;
    Rect::new(slider_x, slider_y, slider_w, slider_h)
}

/// Backwards-compat wrapper for the layershell's left-click hit-test.
/// Defaults to the Output slider since that was the only one before
/// Input was added; the layershell now calls `slider_rect_for` directly
/// with both directions.
#[allow(dead_code)]
pub fn slider_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    slider_rect_for(panel, top_y, Direction::Output, scale)
}

/// Hit-test a click against either device list. Returns the device ID
/// + which direction it belongs to, if any.
pub fn hit_test_device_dir(
    audio: &Audio,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<(Direction, u32)> {
    for &dir in &[Direction::Output, Direction::Input] {
        let list_top = device_list_top_y_for(panel_top_y, dir, scale);
        let row_h = DEVICE_ROW_HEIGHT * scale;
        let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
        let inner_x = panel.x + pad;
        let inner_w = panel.w - pad * 2.0;
        if x < inner_x || x > inner_x + inner_w {
            continue;
        }
        let devices = match dir {
            Direction::Output => audio.sinks(),
            Direction::Input => audio.sources(),
        };
        for (i, dev) in devices.iter().take(MAX_DEVICE_ROWS).enumerate() {
            let row_y = list_top + i as f32 * row_h;
            if y >= row_y && y <= row_y + row_h {
                return Some((dir, dev.id));
            }
        }
    }
    None
}

/// Backwards-compat alias kept around so the layershell keeps building
/// while it migrates to `hit_test_device_dir`.
#[allow(dead_code)]
pub fn hit_test_device(audio: &Audio, panel: Rect, top_y: f32, scale: f32, x: f32, y: f32) -> Option<u32> {
    hit_test_device_dir(audio, panel, top_y, scale, x, y).map(|(_, id)| id)
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    audio: &Audio,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    draw_section(
        painter, text, audio, Direction::Output,
        panel, top_y, scale, alpha, surface_w, surface_h,
    );
    draw_section(
        painter, text, audio, Direction::Input,
        panel, top_y, scale, alpha, surface_w, surface_h,
    );
    top_y + (section_top_logical(Direction::Input) + section_logical_height()) * scale
}

/// Draw one of the two sections (Output or Input) into the audio view.
/// Each section is: header label + slider row (slider + percentage) +
/// device list. Layout math comes from the `_for` helpers above so
/// hit-testing and rendering stay in lockstep.
fn draw_section(
    painter: &mut Painter,
    text: &mut TextRenderer,
    audio: &Audio,
    dir: Direction,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let percent_font = SLIDER_PERCENT_FONT * scale;
    let percent_gap = SLIDER_PERCENT_GAP * scale;
    let percent_w = percent_font * 2.6;
    let header_font = SECTION_HEADER_FONT * scale;
    let header_gap = SECTION_HEADER_BOTTOM_GAP * scale;
    let device_row_h = DEVICE_ROW_HEIGHT * scale;
    let device_font = DEVICE_FONT * scale;
    let dot_size = DEVICE_DOT_SIZE * scale;
    let dot_pad_left = 6.0 * scale;
    let dot_text_gap = 14.0 * scale;

    // Per-direction values.
    let (label, vol, muted, devices) = match dir {
        Direction::Output => (
            "Output",
            audio.volume(),
            audio.is_muted(),
            audio.sinks(),
        ),
        Direction::Input => (
            "Input",
            audio.input_volume(),
            audio.input_muted(),
            audio.sources(),
        ),
    };

    let v = if muted { 0.0 } else { vol.min(1.0) };
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted_white = white.with_alpha(0.55 * alpha);

    // ── Section header ────────────────────────────────────────────────────
    let section_top = panel_top_y + section_top_logical(dir) * scale;
    text.queue(
        label,
        header_font,
        inner_x,
        section_top,
        muted_white,
        inner_w,
        surface_w,
        surface_h,
    );

    // ── Mute icon at the left of the slider row ──────────────────────────
    let icon = icon_rect_for(panel, panel_top_y, dir, scale);
    match dir {
        Direction::Output => draw_speaker(painter, icon.x, icon.y, icon.w, icon.h, muted, alpha),
        Direction::Input => draw_mic(painter, icon.x, icon.y, icon.w, icon.h, muted, alpha),
    }

    // ── Slider row ────────────────────────────────────────────────────────
    let track = slider_rect_for(panel, panel_top_y, dir, scale);
    let radius = track.h * 0.5;

    painter.rect_filled(
        track,
        radius,
        Color::from_rgb8(BAR_TRACK_RGB.0, BAR_TRACK_RGB.1, BAR_TRACK_RGB.2).with_alpha(alpha),
    );
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    if v > 0.0 {
        let fill_w = (track.w * v).max(track.h);
        painter.rect_filled(
            Rect::new(track.x, track.y, fill_w, track.h),
            radius,
            gold.with_alpha(alpha),
        );
    }

    // Knob — white circle at the current position.
    let knob_cx = track.x + track.w * v;
    let knob_cy = track.y + track.h * 0.5;
    let knob_r = track.h * 1.6;
    painter.circle_filled(
        knob_cx,
        knob_cy,
        knob_r,
        white.with_alpha(alpha),
    );

    // Percentage label on the right.
    let pct = (v * 100.0).round() as i32;
    let pct_str = if muted { "Muted".to_string() } else { format!("{}%", pct) };
    let pct_text_w = text.measure_width(&pct_str, percent_font);
    let pct_x = inner_x + inner_w - percent_w + (percent_w - pct_text_w);
    let pct_y = section_top + header_font + header_gap;
    text.queue(
        &pct_str,
        percent_font,
        pct_x,
        pct_y,
        white.with_alpha(alpha),
        percent_w,
        surface_w,
        surface_h,
    );
    let _ = percent_gap; // already baked into slider_rect_for

    // ── Device list ───────────────────────────────────────────────────────
    let list_top = device_list_top_y_for(panel_top_y, dir, scale);
    for (i, dev) in devices.iter().take(MAX_DEVICE_ROWS).enumerate() {
        let row_y = list_top + i as f32 * device_row_h;
        let text_y = row_y + (device_row_h - device_font) / 2.0;
        let label_alpha = if dev.is_default { alpha } else { 0.78 * alpha };
        let label_color = white.with_alpha(label_alpha);

        let dot_cx = inner_x + dot_pad_left + dot_size * 0.5;
        let dot_cy = row_y + device_row_h * 0.5;
        if dev.is_default {
            // Active = filled white circle.
            painter.circle_filled(
                dot_cx,
                dot_cy,
                dot_size * 0.5,
                white.with_alpha(alpha),
            );
        } else {
            // Available = hollow gold ring.
            painter.circle_stroke(
                dot_cx,
                dot_cy,
                dot_size * 0.5,
                1.5 * scale,
                gold.with_alpha(0.55 * alpha),
            );
        }

        let dev_label = truncate_for_width(
            &dev.name,
            &mut *text,
            device_font,
            inner_w - dot_pad_left - dot_size - dot_text_gap,
        );
        text.queue(
            &dev_label,
            device_font,
            dot_cx + dot_size * 0.5 + dot_text_gap,
            text_y,
            label_color,
            inner_w,
            surface_w,
            surface_h,
        );
    }
}

/// Truncate `s` with ellipsis so its width fits `max_w` at `font_size`.
/// The renderer truncates at character boundaries — good enough for
/// our space-separated sink names.
fn truncate_for_width(s: &str, text: &mut TextRenderer, font_size: f32, max_w: f32) -> String {
    if text.measure_width(s, font_size) <= max_w {
        return s.to_string();
    }
    let ellipsis = "…";
    let mut chars: Vec<char> = s.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + ellipsis;
        if text.measure_width(&candidate, font_size) <= max_w {
            return candidate;
        }
    }
    ellipsis.to_string()
}
