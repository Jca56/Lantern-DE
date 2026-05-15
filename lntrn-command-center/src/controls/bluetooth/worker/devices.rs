//! Device polling — runs `bluetoothctl show`, `bluetoothctl devices ...`,
//! and `bluetoothctl info <mac>` to build the `Vec<Device>` snapshot the UI
//! consumes each tick.

use std::process::Command;
use std::sync::mpsc;

use crate::controls::bluetooth::{BtEvent, Device};

pub(super) fn forward_bt_result(
    tx: &mpsc::Sender<BtEvent>,
    result: Result<std::process::Output, std::io::Error>,
) {
    match result {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            // bluetoothctl reports failures via stdout sometimes.
            let line = stderr
                .lines()
                .chain(stdout.lines())
                .find(|l| l.to_lowercase().contains("fail")
                    || l.to_lowercase().contains("error")
                    || l.to_lowercase().contains("not available"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Bluetooth operation failed".to_string());
            let _ = tx.send(BtEvent::Error(line));
        }
        Err(e) => {
            let _ = tx.send(BtEvent::Error(e.to_string()));
        }
    }
}

pub(super) fn read_powered() -> bool {
    show_field_yes("Powered:")
}

pub(super) fn read_discoverable() -> bool {
    show_field_yes("Discoverable:")
}

fn show_field_yes(prefix: &str) -> bool {
    let out = Command::new("bluetoothctl").arg("show").output();
    let Ok(out) = out else { return false };
    if !out.status.success() {
        return false;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix(prefix) {
            return val.trim() == "yes";
        }
    }
    false
}

pub(super) fn read_devices() -> Vec<Device> {
    // `devices` (no filter) returns all known devices: paired ones
    // plus any currently visible from a running scan.
    let all = run_devices(&["devices"]);
    let paired = run_devices(&["devices", "Paired"]);
    let connected = run_devices(&["devices", "Connected"]);

    let paired_macs: std::collections::HashSet<String> =
        paired.iter().map(|(m, _)| m.clone()).collect();
    let connected_macs: std::collections::HashSet<String> =
        connected.iter().map(|(m, _)| m.clone()).collect();

    // Build off `all` so we get unpaired-but-discovered devices too.
    // Paired devices that aren't in `all` (rare, but possible if the
    // controller hasn't seen them since boot) get backfilled from the
    // paired list.
    let mut by_mac: std::collections::HashMap<String, Device> =
        std::collections::HashMap::new();
    for (mac, name) in all.into_iter().chain(paired.into_iter()) {
        let entry = by_mac.entry(mac.clone()).or_insert(Device {
            mac: mac.clone(),
            name: name.clone(),
            connected: connected_macs.contains(&mac),
            paired: paired_macs.contains(&mac),
            ..Device::default()
        });
        // Prefer the longer/non-MAC-style name when both lookups give
        // different strings (paired list usually has the real name).
        if entry.name.is_empty() || entry.name == entry.mac {
            entry.name = name;
        }
    }

    // Enrich each device with `bluetoothctl info <MAC>` — cheap (~10ms
    // per call) and only paired/visible devices, so worst case a handful.
    // We need this for OBEX/profile detection so the Send button knows
    // whether to render.
    for dev in by_mac.values_mut() {
        merge_device_info(dev);
    }

    let mut out: Vec<Device> = by_mac.into_values().collect();
    // Connected → paired → alphabetical. Within each tier, alpha.
    out.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Run `bluetoothctl info <mac>` and fold its output into `dev`. Skips
/// silently on any failure so the basic name/connected state we already
/// have stays usable.
fn merge_device_info(dev: &mut Device) {
    let out = Command::new("bluetoothctl").args(["info", &dev.mac]).output();
    let Ok(out) = out else { return };
    if !out.status.success() {
        return;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut uuids = Vec::new();
    for raw in s.lines() {
        let line = raw.trim_start();
        if let Some(rest) = raw.strip_prefix("Device ") {
            // "Device F0:05:1B:BE:1B:52 (public)"
            if let Some(open) = rest.find('(') {
                let close = rest[open..].find(')').map(|i| open + i).unwrap_or(rest.len());
                dev.address_type = rest[open + 1..close].to_string();
            }
            continue;
        }
        if let Some(v) = line.strip_prefix("Alias:") {
            dev.alias = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Icon:") {
            dev.icon = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Class:") {
            dev.class = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Trusted:") {
            dev.trusted = v.trim() == "yes";
        } else if let Some(v) = line.strip_prefix("Bonded:") {
            dev.bonded = v.trim() == "yes";
        } else if let Some(v) = line.strip_prefix("Blocked:") {
            dev.blocked = v.trim() == "yes";
        } else if let Some(v) = line.strip_prefix("RSSI:") {
            dev.rssi = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("Battery Percentage:") {
            // Format: "0x55 (85)" — prefer the parenthesised decimal.
            let trimmed = v.trim();
            if let Some(open) = trimmed.find('(') {
                let inside = &trimmed[open + 1..];
                if let Some(close) = inside.find(')') {
                    dev.battery_percent = inside[..close].trim().parse().ok();
                }
            }
        } else if let Some(v) = line.strip_prefix("UUID:") {
            // Format: "Audio Source              (0000110a-...)"
            // Take the human-friendly name (everything before the '(').
            let trimmed = v.trim();
            let name = match trimmed.find('(') {
                Some(i) => trimmed[..i].trim().to_string(),
                None => trimmed.to_string(),
            };
            if !name.is_empty() {
                uuids.push(name);
            }
        }
    }
    if !uuids.is_empty() {
        dev.uuids = uuids;
    }
}

fn run_devices(args: &[&str]) -> Vec<(String, String)> {
    let out = Command::new("bluetoothctl").args(args).output();
    let Ok(out) = out else { return Vec::new() };
    let s = String::from_utf8_lossy(&out.stdout);
    let mut v = Vec::new();
    for line in s.lines() {
        // "Device F0:05:1B:BE:1B:52 Julio's S24 Ultra"
        if let Some(rest) = line.strip_prefix("Device ") {
            if let Some((mac, name)) = rest.split_once(' ') {
                v.push((mac.to_string(), name.to_string()));
            }
        }
    }
    v
}
