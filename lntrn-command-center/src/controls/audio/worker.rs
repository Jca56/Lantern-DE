//! Worker thread + wpctl invocations. The render thread never blocks
//! on wpctl; it talks to this module via `AudioCmd` / `AudioEvent` mpsc
//! channels.

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::{AudioCmd, AudioEvent, Sink};

const POLL_INTERVAL: Duration = Duration::from_millis(750);
/// `wpctl status` is more expensive than get-volume; only re-fetch the
/// sink list every few seconds.
const SINK_LIST_POLL_INTERVAL: Duration = Duration::from_secs(3);

pub(super) fn worker(tx: mpsc::Sender<AudioEvent>, cmd_rx: mpsc::Receiver<AudioCmd>) {
    // Prime the UI with whatever we can read right away.
    poll_volumes(&tx);
    poll_devices(&tx);

    let mut last_poll = Instant::now();
    let mut last_sink_list_poll = Instant::now();

    loop {
        // Drain pending commands. Any setter triggers an immediate
        // re-poll so the cached state catches up without waiting for
        // the next tick. Volume sets coalesce — slider drags emit one
        // SetVolume per drag-pixel, and each wpctl shellout blocks for
        // ~50ms; without coalescing the worker falls dozens of frames
        // behind a quick drag.
        let mut force_volume_repoll = false;
        let mut force_devices_repoll = false;
        let mut latest_sink_volume: Option<f32> = None;
        let mut latest_source_volume: Option<f32> = None;
        let mut toggle_mute_pending = false;
        let mut toggle_input_mute_pending = false;
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                AudioCmd::Rescan => {
                    force_volume_repoll = true;
                    force_devices_repoll = true;
                }
                AudioCmd::SetVolume(v) => {
                    latest_sink_volume = Some(v);
                }
                AudioCmd::SetInputVolume(v) => {
                    latest_source_volume = Some(v);
                }
                AudioCmd::ToggleMute => {
                    // XOR pending toggle — odd count toggles, even cancels.
                    toggle_mute_pending = !toggle_mute_pending;
                }
                AudioCmd::ToggleInputMute => {
                    toggle_input_mute_pending = !toggle_input_mute_pending;
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
        if let Some(v) = latest_sink_volume {
            let arg = format!("{:.2}", v);
            // `--limit 1.5` lifts wpctl's default 100 % cap so the
            // slider can boost up to 150 %; the UI clamps itself to 120 %.
            let _ = Command::new("wpctl")
                .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &arg, "--limit", "1.5"])
                .status();
            force_volume_repoll = true;
        }
        if let Some(v) = latest_source_volume {
            let arg = format!("{:.2}", v);
            let _ = Command::new("wpctl")
                .args([
                    "set-volume",
                    "@DEFAULT_AUDIO_SOURCE@",
                    &arg,
                    "--limit",
                    "1.5",
                ])
                .status();
            force_volume_repoll = true;
        }
        if toggle_mute_pending {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
                .status();
            force_volume_repoll = true;
        }
        if toggle_input_mute_pending {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
                .status();
            force_volume_repoll = true;
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
        let Some(dot_idx) = stripped.find('.') else {
            continue;
        };
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
            && HIDDEN_SINK_SUBSTRINGS
                .iter()
                .any(|needle| name.contains(needle))
        {
            continue;
        }
        out.push(Sink {
            id,
            name,
            is_default,
        });
    }
    out
}
