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

use super::{Band, BandEntry, Network, Profile, WifiCmd, WifiEvent, WifiState};

/// Cheap status poll — drives the toolbar icon (connected ssid + bars).
/// `nmcli device` returns instantly so we can run this often.
const STATUS_INTERVAL: Duration = Duration::from_secs(1);
/// Full scan — drives the expanded network list. Uses `--rescan yes`
/// which actually re-probes the air, so it's slow (3-5s blocking) and
/// runs less often.
const SCAN_INTERVAL: Duration = Duration::from_secs(8);

/// Worker entry point. Spawned from [`crate::controls::wifi::Wifi::new`].
pub(super) fn run(tx: mpsc::Sender<WifiEvent>, cmd_rx: mpsc::Receiver<WifiCmd>) {
    // Prime the UI with whatever we can read right away.
    let _ = tx.send(WifiEvent::Status(poll_status()));
    let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status()));
    let _ = tx.send(WifiEvent::Networks(scan_networks()));

    let mut last_status = Instant::now();
    let mut last_scan = Instant::now();
    loop {
        // Drain pending commands first.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                WifiCmd::Rescan => {
                    let _ = Command::new("nmcli").args(["dev", "wifi", "rescan"]).output();
                    thread::sleep(Duration::from_millis(500));
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                    last_status = Instant::now();
                    last_scan = Instant::now();
                }
                WifiCmd::Connect {
                    ssid,
                    password,
                    band,
                    bssid,
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

                    // Phase 2: pin band and/or BSSID on the (now-existing)
                    // profile and reactivate. The brief flap is acceptable;
                    // the alternative is a pre-create dance that fails
                    // when the profile name already exists.
                    let needs_reactivate = band.is_some() || bssid.is_some();
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
                            }
                            if let Some(mac) = &bssid {
                                // `""` clears the field; specific MAC pins it.
                                let value = if mac.is_empty() { "" } else { mac.as_str() };
                                let _ = Command::new("nmcli")
                                    .args([
                                        "connection",
                                        "modify",
                                        &ssid,
                                        "wifi.bssid",
                                        value,
                                    ])
                                    .output();
                            }
                            if needs_reactivate {
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
                    last_status = Instant::now();
                    last_scan = Instant::now();
                }
                WifiCmd::DeleteProfile { uuid } => {
                    let _ = Command::new("nmcli")
                        .args(["connection", "delete", "uuid", &uuid])
                        .output();
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                    last_status = Instant::now();
                    last_scan = Instant::now();
                }
                WifiCmd::ActivateProfile { name } => {
                    let r = Command::new("nmcli")
                        .args(["connection", "up", "id", &name])
                        .output();
                    match r {
                        Ok(o) if o.status.success() => {
                            let _ = tx.send(WifiEvent::ConnectOk);
                        }
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr);
                            let msg = stderr
                                .lines()
                                .next()
                                .unwrap_or("Activation failed")
                                .to_string();
                            let _ = tx.send(WifiEvent::ConnectFail(msg));
                        }
                        Err(e) => {
                            let _ = tx.send(WifiEvent::ConnectFail(e.to_string()));
                        }
                    }
                    let _ = tx.send(WifiEvent::Networks(scan_networks()));
                    let _ = tx.send(WifiEvent::Status(poll_status()));
                    last_status = Instant::now();
                    last_scan = Instant::now();
                }
            }
        }

        // Quick status poll → toolbar icon stays fresh every second.
        // Mullvad VPN status is cheap (~10ms IPC) so we piggyback it here.
        if last_status.elapsed() >= STATUS_INTERVAL {
            let _ = tx.send(WifiEvent::Status(poll_status()));
            let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status()));
            last_status = Instant::now();
        }
        // Full scan → expanded network list. Slow (radio rescan), so
        // runs less often and pinches the worker thread for a few sec.
        if last_scan.elapsed() >= SCAN_INTERVAL {
            let _ = tx.send(WifiEvent::Networks(scan_networks()));
            last_scan = Instant::now();
            // The scan we just ran also drains a status snapshot; reset
            // the status timer so we don't immediately fire another one.
            last_status = Instant::now();
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

/// Check Mullvad VPN connection state. Returns `None` if the `mullvad`
/// CLI isn't installed or the daemon isn't responding (so the indicator
/// can hide cleanly). `Some(true)` = Connected, `Some(false)` = anything
/// else (Disconnected, Connecting, Blocked).
fn poll_vpn_status() -> Option<bool> {
    let out = Command::new("mullvad").arg("status").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // First non-empty line is the tunnel state — typically "Connected",
    // "Disconnected", or "Connecting…".
    let first = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    Some(first.trim().starts_with("Connected"))
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
    // `--rescan yes` forces a fresh scan instead of using NM's cache
    // (which can hold APs for >60s after they disappear — that's why
    // the user kept seeing their phone hotspot at 100% even though the
    // phone was off).
    let out = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "SSID,BSSID,MODE,CHAN,FREQ,RATE,SIGNAL,SECURITY,WPA-FLAGS,RSN-FLAGS,IN-USE",
            "dev",
            "wifi",
            "list",
            "--rescan",
            "yes",
        ])
        .output();
    let Ok(out) = out else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Fetch every saved wireless profile up front and key them by SSID
    // so we can attach the matching ones to each network below.
    let profiles_by_ssid = fetch_profiles_by_ssid();
    let saved: Vec<String> = profiles_by_ssid.keys().cloned().collect();

    // Group rows by SSID. nmcli emits one row per BSSID, so an SSID
    // broadcast on both 2.4 and 5 GHz shows up twice — that's exactly
    // the case we want to collapse into one network with two bands.
    let mut groups: HashMap<String, Network> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for line in stdout.lines() {
        let parts = split_nmcli_t(line);
        if parts.len() < 11 {
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
        let wpa_flags = parts[8].clone();
        let rsn_flags = parts[9].clone();
        let in_use = parts[10] == "*";
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
            // Always append to `aps` — every distinct BSSID we hear is
            // a row in the "top BSSIDs" column.
            net.aps.push(entry.clone());
            // Strongest BandEntry per band wins (multiple BSSIDs on the
            // same band → keep the loudest) for the band-selector pills.
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
            let flags_summary = build_flags_summary(&security, &wpa_flags, &rsn_flags);
            let profiles = profiles_by_ssid
                .get(&ssid)
                .cloned()
                .unwrap_or_default();
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
                    profiles,
                    bands: vec![entry.clone()],
                    aps: vec![entry],
                    selected_band: band,
                    pinned_bssid: None,
                    flags_summary,
                },
            );
        }
    }

    // Finalize: sort bands + aps strongest-first, set headline fields
    // from the strongest, default `selected_band` to strongest, and
    // populate `pinned_bssid` from the active (or most recently used)
    // profile so the lock state reflects the NM profile on the way in.
    let mut nets: Vec<Network> = order
        .into_iter()
        .filter_map(|s| groups.remove(&s))
        .map(|mut net| {
            net.bands.sort_by(|a, b| b.signal.cmp(&a.signal));
            net.aps.sort_by(|a, b| b.signal.cmp(&a.signal));
            if let Some(top) = net.bands.first() {
                net.signal = top.signal;
                net.bssid = top.bssid.clone();
                net.channel = top.channel.clone();
                net.frequency = top.frequency.clone();
                net.rate = top.rate.clone();
                net.selected_band = top.band;
            }
            // Profile rows: active first, then most recently used.
            net.profiles.sort_by(|a, b| {
                b.active.cmp(&a.active).then(b.timestamp.cmp(&a.timestamp))
            });
            // Surface the active profile's pinned BSSID (if any).
            if let Some(p) = net.profiles.iter().find(|p| p.active) {
                if let Some(mac) = &p.pinned_bssid {
                    if net.aps.iter().any(|a| &a.bssid == mac) {
                        net.pinned_bssid = Some(mac.clone());
                    }
                }
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

/// Enumerate every wireless connection profile and return them grouped
/// by SSID. Per-profile we shell out once to pull the bssid + band +
/// timestamp; `nmcli -g` makes that a fast single-value read.
fn fetch_profiles_by_ssid() -> HashMap<String, Vec<Profile>> {
    let mut out: HashMap<String, Vec<Profile>> = HashMap::new();
    let list = Command::new("nmcli")
        .args(["-t", "-f", "NAME,UUID,TYPE,DEVICE,STATE", "connection", "show"])
        .output();
    let Ok(list) = list else { return out };
    if !list.status.success() {
        return out;
    }
    let stdout = String::from_utf8_lossy(&list.stdout);
    for line in stdout.lines() {
        let parts = split_nmcli_t(line);
        if parts.len() < 5 {
            continue;
        }
        let name = parts[0].clone();
        let uuid = parts[1].clone();
        let ctype = parts[2].clone();
        let device = parts[3].clone();
        let state = parts[4].clone();
        if ctype != "802-11-wireless" {
            continue;
        }
        let active = state == "activated" && !device.is_empty();
        let ssid = profile_field(&uuid, "802-11-wireless.ssid");
        let pinned_bssid = profile_field(&uuid, "802-11-wireless.bssid");
        let pinned_band = profile_field(&uuid, "802-11-wireless.band");
        let timestamp_s = profile_field(&uuid, "connection.timestamp");
        let timestamp: i64 = timestamp_s.as_deref().unwrap_or("0").parse().unwrap_or(0);
        let ssid_str = ssid.unwrap_or_default();
        if ssid_str.is_empty() {
            // Hidden / un-named profiles aren't matchable to a scan row;
            // skip rather than show ghost entries.
            continue;
        }
        let profile = Profile {
            name,
            uuid,
            pinned_bssid: pinned_bssid.filter(|s| !s.is_empty()),
            pinned_band: pinned_band.filter(|s| !s.is_empty()),
            timestamp,
            active,
        };
        out.entry(ssid_str).or_default().push(profile);
    }
    out
}

/// `nmcli -g <field> connection show <uuid>` — single-field read. Empty
/// strings come back as `Some("")` so callers can distinguish "missing"
/// (lookup failed) from "unset" (field is empty).
fn profile_field(uuid: &str, field: &str) -> Option<String> {
    let out = Command::new("nmcli")
        .args(["-g", field, "connection", "show", uuid])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    Some(raw.trim_end_matches('\n').to_string())
}

/// Distill `SECURITY` + `WPA-FLAGS` + `RSN-FLAGS` into a single line for
/// the details panel. nmcli's raw output looks like:
///   security  = "WPA2 WPA3 802.1X"
///   wpa-flags = "pair_ccmp group_ccmp psk"
///   rsn-flags = "pair_ccmp pair_gcmp256 group_ccmp sae"
/// We surface the protocol names (WPA2/WPA3) + auth (PSK/SAE/802.1X) +
/// cipher (CCMP/GCMP256) in a human-friendly order.
fn build_flags_summary(security: &str, wpa_flags: &str, rsn_flags: &str) -> String {
    let mut parts: Vec<String> = Vec::new();

    if security.is_empty() || security == "--" {
        return "Open (no encryption)".to_string();
    }
    parts.push(security.replace(' ', "/").to_string());

    let flags = format!("{} {}", wpa_flags, rsn_flags).to_ascii_lowercase();
    let mut auth: Vec<&str> = Vec::new();
    if flags.contains("sae") {
        auth.push("SAE");
    }
    if flags.contains("psk") {
        auth.push("PSK");
    }
    if flags.contains("802.1x") || flags.contains("eap") {
        auth.push("802.1X");
    }
    if !auth.is_empty() {
        parts.push(auth.join("/"));
    }

    let mut ciphers: Vec<&str> = Vec::new();
    if flags.contains("gcmp256") {
        ciphers.push("GCMP-256");
    }
    if flags.contains("ccmp") {
        ciphers.push("CCMP");
    }
    if flags.contains("tkip") {
        ciphers.push("TKIP");
    }
    if !ciphers.is_empty() {
        parts.push(ciphers.join("+"));
    }

    parts.join(" · ")
}
