//! Worker thread for the WiFi tile.
//!
//! Owns all `nmcli` shellouts so the render thread never blocks on
//! network state. Receives commands over an mpsc channel; emits state
//! updates over another.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::{Band, BandEntry, Network, WifiCmd, WifiEvent, WifiState};

const POLL_INTERVAL: Duration = Duration::from_secs(8);

/// Worker entry point. Spawned from [`crate::controls::wifi::Wifi::new`].
pub(super) fn run(tx: mpsc::Sender<WifiEvent>, cmd_rx: mpsc::Receiver<WifiCmd>) {
    // Prime the UI with whatever we can read right away.
    let _ = tx.send(WifiEvent::Status(poll_status()));
    let _ = tx.send(WifiEvent::Networks(scan_networks()));

    let mut last_poll = Instant::now();
    loop {
        // Drain pending commands first.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                WifiCmd::Rescan => {
                    let _ = Command::new("nmcli").args(["dev", "wifi", "rescan"]).output();
                    thread::sleep(Duration::from_millis(500));
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                    last_poll = Instant::now();
                }
                WifiCmd::Connect {
                    ssid,
                    password,
                    band,
                } => {
                    // Phase 1: get a profile up. For saved networks we
                    // try `con up` first; for unsaved we let
                    // `dev wifi connect` create the profile.
                    let initial = if let Some(pw) = password {
                        Command::new("nmcli")
                            .args(["device", "wifi", "connect", &ssid, "password", &pw])
                            .output()
                    } else {
                        let r = Command::new("nmcli")
                            .args(["connection", "up", "id", &ssid])
                            .output();
                        if r.as_ref().map_or(true, |o| !o.status.success()) {
                            Command::new("nmcli")
                                .args(["device", "wifi", "connect", &ssid])
                                .output()
                        } else {
                            r
                        }
                    };

                    // Phase 2: if the user picked a band, pin it on the
                    // (now-existing) profile and reactivate. The brief
                    // band-flap is acceptable; the alternative is a
                    // pre-create dance that fails when the profile name
                    // already exists.
                    let result = match initial {
                        Ok(o) if o.status.success() => {
                            if let Some(b) = band {
                                let _ = Command::new("nmcli")
                                    .args([
                                        "connection",
                                        "modify",
                                        &ssid,
                                        "wifi.band",
                                        b.nm_band(),
                                    ])
                                    .output();
                                Command::new("nmcli")
                                    .args(["connection", "up", "id", &ssid])
                                    .output()
                            } else {
                                Ok(o)
                            }
                        }
                        other => other,
                    };

                    match result {
                        Ok(o) if o.status.success() => {
                            let _ = tx.send(WifiEvent::ConnectOk);
                        }
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr);
                            let msg = stderr
                                .lines()
                                .next()
                                .unwrap_or("Connection failed")
                                .to_string();
                            let _ = tx.send(WifiEvent::ConnectFail(msg));
                        }
                        Err(e) => {
                            let _ = tx.send(WifiEvent::ConnectFail(e.to_string()));
                        }
                    }
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                    last_poll = Instant::now();
                }
            }
        }

        // Periodic refresh.
        if last_poll.elapsed() >= POLL_INTERVAL {
            let _ = tx.send(WifiEvent::Status(poll_status()));
            let _ = tx.send(WifiEvent::Networks(scan_networks()));
            last_poll = Instant::now();
        }

        thread::sleep(Duration::from_millis(150));
    }
}

fn poll_status() -> WifiState {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
        .output();
    let Ok(out) = out else { return WifiState::Off };
    if !out.status.success() {
        return WifiState::Off;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut wifi_connected = false;
    let mut ssid = String::new();
    let mut has_wifi = false;
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.first().copied() == Some("wifi") {
            has_wifi = true;
            if parts.get(1).copied() == Some("connected") {
                wifi_connected = true;
                ssid = parts.get(2).copied().unwrap_or("").to_string();
            }
            break;
        }
    }
    if !has_wifi {
        return WifiState::Off;
    }
    if !wifi_connected {
        return WifiState::Disconnected;
    }
    let signal = signal_for_ssid(&ssid);
    WifiState::Connected { ssid, signal }
}

fn signal_for_ssid(ssid: &str) -> u32 {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "IN-USE,SSID,SIGNAL", "dev", "wifi", "list"])
        .output();
    let Ok(out) = out else { return 0 };
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Prefer the row with `*` (active BSSID); fall back to the first
    // row with a matching SSID otherwise.
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.first().copied() == Some("*") && parts.len() >= 3 {
            return parts[2].parse().unwrap_or(0);
        }
    }
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[1] == ssid {
            return parts[2].parse().unwrap_or(0);
        }
    }
    0
}

fn scan_networks() -> Vec<Network> {
    let out = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "SSID,BSSID,MODE,CHAN,FREQ,RATE,SIGNAL,SECURITY,IN-USE",
            "dev",
            "wifi",
            "list",
        ])
        .output();
    let Ok(out) = out else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Saved connection names.
    let saved = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    let parts = split_nmcli_t(l);
                    if parts.len() >= 2 && parts[1] == "802-11-wireless" {
                        Some(parts[0].clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Group rows by SSID. nmcli emits one row per BSSID, so an SSID
    // broadcast on both 2.4 and 5 GHz shows up twice — that's exactly
    // the case we want to collapse into one network with two bands.
    let mut groups: HashMap<String, Network> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for line in stdout.lines() {
        let parts = split_nmcli_t(line);
        if parts.len() < 9 {
            continue;
        }
        let ssid = parts[0].clone();
        if ssid.is_empty() {
            continue;
        }
        let bssid = parts[1].clone();
        let mode = parts[2].clone();
        let channel = parts[3].clone();
        let frequency = parts[4].clone();
        let rate = parts[5].clone();
        let signal: u32 = parts[6].parse().unwrap_or(0);
        let security = parts[7].clone();
        let in_use = parts[8] == "*";
        let Some(band) = Band::from_freq_str(&frequency) else {
            continue;
        };
        let entry = BandEntry {
            band,
            signal,
            bssid: bssid.clone(),
            channel: channel.clone(),
            frequency: frequency.clone(),
            rate: rate.clone(),
        };

        if let Some(net) = groups.get_mut(&ssid) {
            net.in_use = net.in_use || in_use;
            // Strongest BandEntry per band wins (multiple BSSIDs on the
            // same band → keep the loudest).
            if let Some(existing) = net.bands.iter_mut().find(|b| b.band == band) {
                if signal > existing.signal {
                    *existing = entry;
                }
            } else {
                net.bands.push(entry);
            }
        } else {
            order.push(ssid.clone());
            let saved = saved.iter().any(|n| n == &ssid);
            groups.insert(
                ssid.clone(),
                Network {
                    ssid,
                    signal,
                    security,
                    in_use,
                    saved,
                    bssid,
                    mode,
                    channel,
                    frequency,
                    rate,
                    bands: vec![entry],
                    selected_band: band,
                },
            );
        }
    }

    // Finalize: sort each network's bands strongest-first, set headline
    // fields from the strongest, default `selected_band` to strongest.
    let mut nets: Vec<Network> = order
        .into_iter()
        .filter_map(|s| groups.remove(&s))
        .map(|mut net| {
            net.bands.sort_by(|a, b| b.signal.cmp(&a.signal));
            if let Some(top) = net.bands.first() {
                net.signal = top.signal;
                net.bssid = top.bssid.clone();
                net.channel = top.channel.clone();
                net.frequency = top.frequency.clone();
                net.rate = top.rate.clone();
                net.selected_band = top.band;
            }
            net
        })
        .collect();

    // Sort: in-use first, then saved, then by signal desc.
    nets.sort_by(|a, b| {
        b.in_use
            .cmp(&a.in_use)
            .then(b.saved.cmp(&a.saved))
            .then(b.signal.cmp(&a.signal))
    });
    nets
}

/// Split a single line of `nmcli -t` output, respecting backslash
/// escapes so colons inside fields (like BSSIDs) survive intact.
fn split_nmcli_t(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Escape: take the next char literally.
                if let Some(&next) = chars.peek() {
                    cur.push(next);
                    chars.next();
                } else {
                    cur.push(c);
                }
            }
            ':' => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}
