//! MPRIS media player integration — discovers active media players via D-Bus
//! and provides now-playing state + transport controls (play/pause/next/prev).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use lntrn_dbus::{self as dbus, Connection, BodyReader, Value, encode_string};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const FAST_POLL: Duration = Duration::from_millis(200);

// ── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct MediaState {
    pub player_name: String,
    pub title: String,
    pub artist: String,
    pub status: PlaybackStatus,
    pub bus_name: String,
}

pub enum MediaCmd {
    PlayPause,
    Next,
    Previous,
}

pub enum MprisEvent {
    State(Option<MediaState>),
}

// ── Spawn ──────────────────────────────────────────────────────────────────

pub fn spawn() -> (mpsc::Receiver<MprisEvent>, mpsc::Sender<MediaCmd>) {
    let (event_tx, event_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("mpris-poll".into())
        .spawn(move || mpris_thread(event_tx, cmd_rx))
        .expect("spawn mpris thread");
    (event_rx, cmd_tx)
}

// ── Background thread ──────────────────────────────────────────────────────

fn mpris_thread(tx: mpsc::Sender<MprisEvent>, rx: mpsc::Receiver<MediaCmd>) {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("mpris: D-Bus connect failed: {e}");
            return;
        }
    };

    // Subscribe to PropertiesChanged signals from MPRIS players
    conn.add_match(
        "type='signal',interface='org.freedesktop.DBus.Properties',\
         member='PropertiesChanged',arg0='org.mpris.MediaPlayer2.Player'",
    );

    let mut last_state: Option<MediaState> = None;
    let mut last_poll = Instant::now();
    let mut poll_fast_until = Instant::now();

    loop {
        // Handle commands (non-blocking)
        while let Ok(cmd) = rx.try_recv() {
            if let Some(ref state) = last_state {
                handle_cmd(&mut conn, &state.bus_name, cmd);
                // Poll fast after sending a command for responsive UI
                poll_fast_until = Instant::now() + Duration::from_secs(3);
            }
        }

        // Check for PropertiesChanged signals (non-blocking)
        let mut got_signal = false;
        while let Some(msg) = conn.try_read() {
            if msg.msg_type == 4 && msg.member == "PropertiesChanged" {
                got_signal = true;
            }
        }

        // Poll on interval or when signaled
        let interval = if Instant::now() < poll_fast_until { FAST_POLL } else { POLL_INTERVAL };
        if got_signal || last_poll.elapsed() >= interval {
            last_poll = Instant::now();
            let new_state = poll_players(&mut conn);

            // Only send if changed
            let changed = match (&last_state, &new_state) {
                (None, None) => false,
                (Some(_), None) | (None, Some(_)) => true,
                (Some(a), Some(b)) => {
                    a.title != b.title || a.artist != b.artist
                        || a.status != b.status || a.bus_name != b.bus_name
                }
            };
            if changed {
                let _ = tx.send(MprisEvent::State(new_state.clone()));
                last_state = new_state;
            }
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

fn handle_cmd(conn: &mut Connection, bus: &str, cmd: MediaCmd) {
    let method = match cmd {
        MediaCmd::PlayPause => "PlayPause",
        MediaCmd::Next => "Next",
        MediaCmd::Previous => "Previous",
    };
    conn.method_call(bus, MPRIS_PATH, MPRIS_PLAYER_IFACE, method, "", &[]);
}

// ── Player discovery & state reading ───────────────────────────────────────

fn poll_players(conn: &mut Connection) -> Option<MediaState> {
    let players = list_players(conn);
    if players.is_empty() {
        return None;
    }

    let mut best: Option<MediaState> = None;
    for bus in &players {
        if let Some(state) = get_player_state(conn, bus) {
            match state.status {
                PlaybackStatus::Playing => return Some(state),
                PlaybackStatus::Paused => {
                    if best.is_none() {
                        best = Some(state);
                    }
                }
                PlaybackStatus::Stopped => {}
            }
        }
    }
    best
}

fn list_players(conn: &mut Connection) -> Vec<String> {
    let serial = conn.method_call(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
        "",
        &[],
    );
    let reply = match conn.read_reply(serial) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    let Some(val) = reader.read_value("as") else { return Vec::new() };
    let Some(arr) = val.as_array() else { return Vec::new() };
    arr.iter()
        .filter_map(|v| v.as_str())
        .filter(|s| s.starts_with(MPRIS_PREFIX))
        .map(|s| s.to_string())
        .collect()
}

fn get_player_state(conn: &mut Connection, bus: &str) -> Option<MediaState> {
    let status_str = get_property_string(conn, bus, "PlaybackStatus")?;
    let status = match status_str.as_str() {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        _ => PlaybackStatus::Stopped,
    };

    let (title, artist) = get_metadata(conn, bus);

    // Extract friendly player name from bus name
    let player_name = bus
        .strip_prefix(MPRIS_PREFIX)
        .unwrap_or(bus)
        .split('.')
        .next()
        .unwrap_or("Unknown")
        .to_string();
    // Capitalize first letter
    let player_name = if let Some(first) = player_name.chars().next() {
        let mut s = first.to_uppercase().to_string();
        s.push_str(&player_name[first.len_utf8()..]);
        s
    } else {
        player_name
    };

    Some(MediaState {
        player_name,
        title,
        artist,
        status,
        bus_name: bus.to_string(),
    })
}

fn get_property_string(conn: &mut Connection, bus: &str, prop: &str) -> Option<String> {
    let mut body = Vec::new();
    encode_string(&mut body, MPRIS_PLAYER_IFACE);
    encode_string(&mut body, prop);
    let serial = conn.method_call(bus, MPRIS_PATH, PROPS_IFACE, "Get", "ss", &body);
    let reply = conn.read_reply(serial).ok()?;
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    let val = reader.read_value("v")?;
    val.as_str().map(|s| s.to_string())
}

fn get_metadata(conn: &mut Connection, bus: &str) -> (String, String) {
    let mut body = Vec::new();
    encode_string(&mut body, MPRIS_PLAYER_IFACE);
    encode_string(&mut body, "Metadata");
    let serial = conn.method_call(bus, MPRIS_PATH, PROPS_IFACE, "Get", "ss", &body);
    let reply = match conn.read_reply(serial) {
        Ok(r) => r,
        Err(_) => return (String::new(), String::new()),
    };
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    let Some(val) = reader.read_value("v") else {
        return (String::new(), String::new());
    };
    let Some(dict) = val.as_dict() else {
        return (String::new(), String::new());
    };

    let title = dict
        .get("xesam:title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let artist = dict
        .get("xesam:artist")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    (title, artist)
}
