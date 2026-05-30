//! Throwaway G502 button-discovery logger — std only, no deps.
//!
//! Auto-detects the mouse's evdev nodes (any input device whose name
//! contains "G502") and prints each button press/release and wheel tilt,
//! so we can learn what code every physical button emits on THIS mouse.
//!
//! Build:  rustc -O tools/button_logger.rs -o /tmp/lntrn-button-logger
//! Run:    sudo /tmp/lntrn-button-logger
//!         (press each button / tilt the wheel once, Ctrl+C when done)
//!
//! Pass explicit device paths to override autodetect:
//!         sudo /tmp/lntrn-button-logger /dev/input/event6 /dev/input/event7

use std::fs::File;
use std::io::Read;
use std::thread;

// `struct input_event` ends with: type:u16, code:u16, value:i32 — always
// the last 8 bytes, whatever the leading timeval width is. So we size the
// record by pointer width and read the trailing fields by offset.
#[cfg(target_pointer_width = "64")]
const EV_SIZE: usize = 24;
#[cfg(target_pointer_width = "32")]
const EV_SIZE: usize = 16;

const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;

fn key_name(code: u16) -> String {
    let n = match code {
        0x110 => "BTN_LEFT",
        0x111 => "BTN_RIGHT",
        0x112 => "BTN_MIDDLE",
        0x113 => "BTN_SIDE",
        0x114 => "BTN_EXTRA",
        0x115 => "BTN_FORWARD",
        0x116 => "BTN_BACK",
        0x117 => "BTN_TASK",
        _ => return format!("UNKNOWN code {code} (0x{code:03x})  <-- note this one!"),
    };
    format!("{n} (0x{code:03x})")
}

fn rel_name(code: u16) -> Option<&'static str> {
    match code {
        0x06 => Some("REL_HWHEEL (wheel tilt)"),
        0x08 => Some("REL_WHEEL (scroll)"),
        _ => None, // skip REL_X / REL_Y movement + hi-res duplicates
    }
}

/// Scan /proc/bus/input/devices for any "G502" device and collect its
/// /dev/input/eventN handler paths.
fn discover_devices() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(content) = std::fs::read_to_string("/proc/bus/input/devices") else {
        return out;
    };
    for block in content.split("\n\n") {
        let mut is_g502 = false;
        let mut events = Vec::new();
        for line in block.lines() {
            if line.starts_with("N: Name=") && line.to_lowercase().contains("g502") {
                is_g502 = true;
            }
            if let Some(rest) = line.strip_prefix("H: Handlers=") {
                for tok in rest.split_whitespace() {
                    if tok.starts_with("event") {
                        events.push(format!("/dev/input/{tok}"));
                    }
                }
            }
        }
        if is_g502 {
            out.extend(events);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn short(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn read_loop(path: String) {
    let mut f = match File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("  ✗ can't open {path}: {e}  (need sudo?)");
            return;
        }
    };
    let label = short(&path).to_string();
    let mut buf = [0u8; EV_SIZE];
    loop {
        if f.read_exact(&mut buf).is_err() {
            eprintln!("  ✗ read ended on {path}");
            return;
        }
        let etype = u16::from_ne_bytes([buf[EV_SIZE - 8], buf[EV_SIZE - 7]]);
        let code = u16::from_ne_bytes([buf[EV_SIZE - 6], buf[EV_SIZE - 5]]);
        let value = i32::from_ne_bytes([
            buf[EV_SIZE - 4],
            buf[EV_SIZE - 3],
            buf[EV_SIZE - 2],
            buf[EV_SIZE - 1],
        ]);
        match etype {
            EV_KEY => {
                // 1 = press, 0 = release, 2 = autorepeat (skip the spam).
                let action = match value {
                    1 => "↓ press  ",
                    0 => "↑ release",
                    _ => continue,
                };
                println!("[{label}] {action}  {}", key_name(code));
            }
            EV_REL => {
                if let Some(name) = rel_name(code) {
                    println!("[{label}] ~ {name} = {value:+}");
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let devices = if args.is_empty() { discover_devices() } else { args };

    if devices.is_empty() {
        eprintln!("No G502 input devices found in /proc/bus/input/devices.");
        eprintln!("Pass event paths explicitly, e.g.:");
        eprintln!("  sudo {} /dev/input/event6 /dev/input/event7", "lntrn-button-logger");
        std::process::exit(1);
    }

    eprintln!("🖱  Lantern button discovery — watching:");
    for d in &devices {
        eprintln!("      {d}");
    }
    eprintln!("\nPress each physical button ONCE, tilt the wheel left/right,");
    eprintln!("and try the DPI-shift (sniper) button. UNKNOWN codes are the");
    eprintln!("interesting ones. Ctrl+C when done.\n");

    let handles: Vec<_> = devices
        .into_iter()
        .map(|d| thread::spawn(move || read_loop(d)))
        .collect();
    for h in handles {
        let _ = h.join();
    }
}
