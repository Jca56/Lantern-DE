//! Device registry: detect connected peripherals and bring them up as
//! capability-exposing [`Device`]s.
//!
//! Discovery is generalized — we probe *every* Logitech (0x046d) hidraw
//! node for a live HID++ endpoint, so the user's mouse, keyboard and
//! headset all surface without hardcoding product ids. A new vendor adds
//! its own driver + a scan branch here.

pub mod logitech;

use std::path::PathBuf;

use crate::caps::Device;

pub const VENDOR_LOGITECH: u16 = 0x046d;

/// Scan for all connected, controllable peripherals.
pub fn scan() -> Vec<Box<dyn Device>> {
    let mut out: Vec<Box<dyn Device>> = Vec::new();
    for node in hidraw_nodes_for_vendor(VENDOR_LOGITECH) {
        match logitech::LogitechDevice::open(&node) {
            Ok(Some(dev)) => out.push(Box::new(dev)),
            Ok(None) => {} // node didn't speak HID++ — a different interface
            Err(_) => {}   // unreadable / no permission — skip quietly
        }
    }
    out
}

/// All `/dev/hidrawN` whose backing USB device has vendor id `vendor`,
/// via `/sys/class/hidraw/*/device/uevent` (`HID_ID=bus:vendor:product`).
pub fn hidraw_nodes_for_vendor(vendor: u16) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/hidraw") else {
        return out;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("hidraw") {
            continue;
        }
        let uevent = entry.path().join("device/uevent");
        let Ok(content) = std::fs::read_to_string(&uevent) else {
            continue;
        };
        if uevent_vendor(&content) == Some(vendor) {
            out.push(PathBuf::from(format!("/dev/{name}")));
        }
    }
    out.sort();
    out
}

/// Parse the vendor id out of a uevent's `HID_ID=0003:0000046D:0000C08B`.
fn uevent_vendor(uevent: &str) -> Option<u16> {
    for line in uevent.lines() {
        let Some(rest) = line.strip_prefix("HID_ID=") else {
            continue;
        };
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() == 3 {
            if let Ok(v) = u32::from_str_radix(parts[1].trim(), 16) {
                return Some(v as u16);
            }
        }
    }
    None
}
