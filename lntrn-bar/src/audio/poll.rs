//! Background thread — all wpctl interaction lives here.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc;

use super::{AudioCmd, AudioEvent, FullState, AudioSink, AudioStream};

const POLL_INTERVAL_MS: u64 = 3_000;

pub fn poll_thread(tx: mpsc::Sender<AudioEvent>, cmd_rx: mpsc::Receiver<AudioCmd>) {
    let _ = tx.send(AudioEvent::State(poll_full_state()));
    let mut last_poll = std::time::Instant::now();

    loop {
        let mut did_command = false;
        while let Ok(cmd) = cmd_rx.try_recv() {
            did_command = true;
            match cmd {
                AudioCmd::SetVolume(vol) => {
                    let pct = format!("{:.0}%", vol * 100.0);
                    let _ = Command::new("wpctl")
                        .args(["set-volume", "--limit", "1.0", "@DEFAULT_AUDIO_SINK@", &pct])
                        .output();
                }
                AudioCmd::SetMicVolume(vol) => {
                    let pct = format!("{:.0}%", vol * 100.0);
                    let _ = Command::new("wpctl")
                        .args(["set-volume", "@DEFAULT_AUDIO_SOURCE@", &pct])
                        .output();
                }
                AudioCmd::ToggleMute => {
                    let _ = Command::new("wpctl")
                        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
                        .output();
                }
                AudioCmd::ToggleMicMute => {
                    let _ = Command::new("wpctl")
                        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
                        .output();
                }
                AudioCmd::SetDefaultSink(id) | AudioCmd::SetDefaultSource(id) => {
                    let _ = Command::new("wpctl")
                        .args(["set-default", &id.to_string()])
                        .output();
                }
                AudioCmd::SetStreamVolume(id, vol) => {
                    let pct = format!("{:.0}%", vol * 100.0);
                    let _ = Command::new("wpctl")
                        .args(["set-volume", &id.to_string(), &pct])
                        .output();
                }
            }
        }

        if did_command {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let _ = tx.send(AudioEvent::State(poll_full_state()));
            last_poll = std::time::Instant::now();
        } else if last_poll.elapsed().as_millis() >= POLL_INTERVAL_MS as u128 {
            let _ = tx.send(AudioEvent::State(poll_full_state()));
            last_poll = std::time::Instant::now();
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn poll_full_state() -> FullState {
    let (volume, muted) = get_volume("@DEFAULT_AUDIO_SINK@");
    let (mic_volume, mic_muted) = get_volume("@DEFAULT_AUDIO_SOURCE@");
    let default_sink_id = get_default_id("@DEFAULT_AUDIO_SINK@");
    let default_source_id = get_default_id("@DEFAULT_AUDIO_SOURCE@");
    let status = wpctl_status();
    let filters = parse_filters(&status);
    let sinks = parse_device_section(&status, "Sinks:", &filters, default_sink_id);
    let sources = parse_device_section(&status, "Sources:", &filters, default_source_id);
    let streams = parse_streams(&status);
    FullState { volume, muted, mic_volume, mic_muted, sinks, sources, streams }
}

/// Get the node ID for a default target.
/// `wpctl inspect @DEFAULT_AUDIO_SINK@` first line: "id 85, type ..."
fn get_default_id(target: &str) -> Option<u32> {
    let output = Command::new("wpctl").args(["inspect", target]).output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?;
    let after_id = first_line.strip_prefix("id ")?;
    let id_str = after_id.split(',').next()?.trim();
    id_str.parse().ok()
}

fn wpctl_status() -> String {
    Command::new("wpctl").arg("status").output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
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

/// Strip unicode box-drawing characters from a line.
fn strip_tree(s: &str) -> String {
    s.chars().filter(|c| !matches!(c,
        '│' | '├' | '└' | '─' | '┼' | '┬' | '┤' | '┐' | '┘' | '┌' | '┊'
    )).collect()
}

/// Shorten common long device name prefixes for display.
fn shorten_name(name: &str) -> String {
    // Strip common prefixes that make all devices look the same
    let name = name
        .replace("Meteor Lake-P HD Audio Controller ", "")
        .replace("Meteor Lake-P HD Audio Controller", "Built-in Audio");
    let name = name.trim().to_string();
    if name.is_empty() { "Built-in Audio".to_string() } else { name }
}

// ── Filter parsing ─────────────────────────────────────────────────────────

/// Info about a bluez loopback filter from the Filters: section.
/// Maps a bluetooth address to the filter node ID that can be set-default'd.
struct FilterInfo {
    /// The filter node ID (e.g. 85 for bluez_output)
    id: u32,
    /// "sink" or "source"
    kind: &'static str,
    /// Bluetooth MAC address (e.g. "BC:87:FA:48:B3:9A")
    address: String,
}

/// Parse the Filters: section to find bluez loopback filter IDs.
/// These are the IDs that `wpctl set-default` actually needs for bluetooth.
fn parse_filters(status: &str) -> Vec<FilterInfo> {
    let mut filters = Vec::new();
    let mut in_filters = false;

    for line in status.lines() {
        let clean = strip_tree(line);
        let trimmed = clean.trim();

        if trimmed.contains("Filters:") {
            in_filters = true;
            continue;
        }
        if in_filters && (trimmed.contains("Streams:") || trimmed.is_empty()) {
            break;
        }
        if !in_filters { continue; }

        let content = trimmed.trim_start_matches('*').trim();
        if let Some(dot_pos) = content.find('.') {
            let id_str = content[..dot_pos].trim();
            let Ok(id) = id_str.parse::<u32>() else { continue };

            let after_dot = content[dot_pos + 1..].trim();
            let name = if let Some(bracket) = after_dot.find('[') {
                after_dot[..bracket].trim()
            } else {
                after_dot.trim()
            };

            // Match bluez_output.XX:XX:XX or bluez_input.XX:XX:XX
            if let Some(addr) = name.strip_prefix("bluez_output.") {
                filters.push(FilterInfo { id, kind: "sink", address: addr.to_string() });
            } else if let Some(addr) = name.strip_prefix("bluez_input.") {
                filters.push(FilterInfo { id, kind: "source", address: addr.to_string() });
            }
        }
    }

    filters
}

/// For a device, find its bluetooth address (if any) by inspecting it.
fn get_bt_address(device_id: u32) -> Option<String> {
    let output = Command::new("wpctl")
        .args(["inspect", &device_id.to_string()])
        .output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("api.bluez5.address = \"") {
            return Some(rest.trim_end_matches('"').to_string());
        }
    }
    None
}

/// Parse Sinks: or Sources: section. For bluetooth devices, resolves the
/// filter ID so set-default works. Names are shortened for display.
/// Filters out HDMI/DisplayPort sinks since they're not useful outputs.
fn parse_device_section(
    status: &str, header: &str, filters: &[FilterInfo], default_id: Option<u32>,
) -> Vec<AudioSink> {
    let filter_kind = if header.contains("Sink") { "sink" } else { "source" };
    let mut devices = Vec::new();
    let mut in_section = false;
    let filter_map: HashMap<&str, u32> = filters.iter()
        .filter(|f| f.kind == filter_kind)
        .map(|f| (f.address.as_str(), f.id))
        .collect();

    for line in status.lines() {
        let clean = strip_tree(line);
        let trimmed = clean.trim();

        if trimmed.contains(header) {
            in_section = true;
            continue;
        }
        if in_section && (trimmed.is_empty()
            || (trimmed.contains("Sinks:") && !trimmed.contains(header))
            || (trimmed.contains("Sources:") && !trimmed.contains(header))
            || trimmed.contains("Filters:") || trimmed.contains("Streams:"))
        {
            break;
        }
        if !in_section { continue; }

        let is_star = trimmed.starts_with('*');
        let content = trimmed.trim_start_matches('*').trim();

        if let Some(dot_pos) = content.find('.') {
            let id_str = content[..dot_pos].trim();
            let Ok(raw_id) = id_str.parse::<u32>() else { continue };

            let after_dot = content[dot_pos + 1..].trim();
            let full_name = if let Some(bracket) = after_dot.find('[') {
                after_dot[..bracket].trim().to_string()
            } else {
                after_dot.trim().to_string()
            };
            if full_name.is_empty() { continue; }

            // Skip HDMI/DisplayPort outputs — not useful for most users
            if full_name.contains("HDMI") || full_name.contains("DisplayPort") {
                continue;
            }

            // For bluetooth devices, use the filter ID instead of the raw ID
            let settable_id = get_bt_address(raw_id)
                .and_then(|addr| filter_map.get(addr.as_str()).copied())
                .unwrap_or(raw_id);

            // Default detection: * marker OR settable_id matches the default
            let is_default = is_star || default_id == Some(settable_id);

            devices.push(AudioSink {
                id: settable_id,
                name: shorten_name(&full_name),
                is_default,
            });
        }
    }

    devices
}

/// Parse the Streams: section from wpctl status.
fn parse_streams(status: &str) -> Vec<AudioStream> {
    let mut streams = Vec::new();
    let mut in_streams = false;

    for line in status.lines() {
        let clean = strip_tree(line);
        let trimmed = clean.trim();

        if trimmed.contains("Streams:") {
            in_streams = true;
            continue;
        }
        if !in_streams { continue; }
        if trimmed.is_empty() || trimmed.contains("Video") { break; }
        if trimmed.contains('>') { continue; }

        let content = trimmed.trim_start_matches('*').trim();
        if let Some(dot_pos) = content.find('.') {
            let id_str = content[..dot_pos].trim();
            let Ok(id) = id_str.parse::<u32>() else { continue };

            let name = content[dot_pos + 1..].trim();
            if name.is_empty() || name.starts_with("bluez_") || name.starts_with("loopback") {
                continue;
            }

            let (volume, muted) = get_volume(&id.to_string());
            streams.push(AudioStream {
                id,
                name: name.to_string(),
                volume,
                muted,
            });
        }
    }

    streams
}
