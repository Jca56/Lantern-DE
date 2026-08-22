//! lntrn-gear CLI — drive connected peripherals through the capability
//! layer. Subcommands:
//!   list  (default) — connected devices + their capabilities
//!   info            — per-device detail (DPI current/range, LED zones)
//!   led <r> <g> <b> — set every lighting-capable device to a fixed color
//!   dpi <n>         — set DPI on every pointer (snapped to its range)
//!
//! hidraw is root-only until we ship a udev rule, so for now run with sudo.

use lntrn_gear::caps::Rgb;
use lntrn_gear::devices;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("list");

    let mut devs = devices::scan();
    if devs.is_empty() {
        eprintln!("✗ No controllable Logitech HID++ devices found.");
        eprintln!("  hidraw is root-only (0600) until we add a udev rule — try sudo.");
        std::process::exit(1);
    }

    match cmd {
        "list" => list(&mut devs),
        "info" => info(&mut devs),
        "led" => led(&mut devs, &args),
        "dpi" => dpi(&mut devs, &args),
        other => {
            eprintln!("unknown command '{other}'. Use: list | info | led <r> <g> <b> | dpi <n>");
            std::process::exit(2);
        }
    }
}

type Devices = Vec<Box<dyn lntrn_gear::caps::Device>>;

fn list(devs: &mut Devices) {
    println!("Connected devices:");
    for (i, d) in devs.iter_mut().enumerate() {
        let mut caps = Vec::new();
        if d.lighting().is_some() {
            caps.push("lighting");
        }
        if d.dpi().is_some() {
            caps.push("dpi");
        }
        let caps = if caps.is_empty() {
            "—".to_string()
        } else {
            caps.join(", ")
        };
        println!("  [{i}] {} ({})  caps: {caps}", d.name(), d.kind().label());
    }
}

fn info(devs: &mut Devices) {
    for (i, d) in devs.iter_mut().enumerate() {
        println!("[{i}] {} ({})", d.name(), d.kind().label());
        if let Some(dpi) = d.dpi() {
            let cur = dpi
                .get()
                .map(|v| v.to_string())
                .unwrap_or_else(|_| "?".into());
            let rng = dpi
                .range()
                .map(|r| format!("{}–{} step {}", r.min, r.max, r.step))
                .unwrap_or_else(|_| "?".into());
            println!("      DPI : {cur}  (range {rng})");
        }
        if let Some(light) = d.lighting() {
            println!("      LED : {} zone(s)", light.zone_count());
        }
    }
}

fn led(devs: &mut Devices, args: &[String]) {
    let parse = |s: Option<&String>| s.and_then(|v| v.parse::<u8>().ok());
    let (Some(r), Some(g), Some(b)) = (parse(args.get(2)), parse(args.get(3)), parse(args.get(4)))
    else {
        eprintln!("usage: lntrn-gear led <r> <g> <b>   (0–255 each)");
        return;
    };
    let color = Rgb::new(r, g, b);
    let mut any = false;
    for d in devs.iter_mut() {
        let name = d.name().to_string();
        if let Some(light) = d.lighting() {
            match light.set_all(color) {
                Ok(()) => {
                    println!("  {name} → {}", color.hex());
                    any = true;
                }
                Err(e) => eprintln!("  {name}: {e}"),
            }
        }
    }
    if !any {
        eprintln!("(no lighting-capable devices)");
    }
}

fn dpi(devs: &mut Devices, args: &[String]) {
    let Some(want) = args.get(2).and_then(|v| v.parse::<u16>().ok()) else {
        eprintln!("usage: lntrn-gear dpi <value>");
        return;
    };
    let mut any = false;
    for d in devs.iter_mut() {
        let name = d.name().to_string();
        if let Some(dpi) = d.dpi() {
            let before = dpi.get().unwrap_or(0);
            match dpi.set(want) {
                Ok(()) => {
                    let after = dpi.get().unwrap_or(0);
                    println!("  {name}: {before} → {after}");
                    any = true;
                }
                Err(e) => eprintln!("  {name}: {e}"),
            }
        }
    }
    if !any {
        eprintln!("(no DPI-capable devices)");
    }
}
