//! MPRIS worker thread. Talks to whatever media player owns an
//! `org.mpris.MediaPlayer2.*` name on the **session** bus, reading the
//! Player interface's properties (status / metadata / position) and
//! forwarding transport commands. Blocking zbus API — this runs on its
//! own OS thread, mirroring the WiFi/iwd worker.
//!
//! MPRIS spec: <https://specifications.freedesktop.org/mpris-spec/latest/>

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use super::{MediaCmd, MediaEvent, PlaybackStatus, Track};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";

/// How often to re-read the active player's state while the panel is
/// on screen.
const POLL_INTERVAL: Duration = Duration::from_millis(600);
/// …and while it's hidden. Nothing is drawn then; this just keeps the
/// cached track from being wildly stale for the first frame after a
/// show (the show itself also triggers an immediate poll).
const HIDDEN_POLL_INTERVAL: Duration = Duration::from_secs(10);
/// Loop granularity — small so transport clicks feel instant.
const TICK: Duration = Duration::from_millis(120);
/// Backoff when the session bus isn't reachable yet.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(3);

pub(super) fn run(tx: mpsc::Sender<MediaEvent>, cmd_rx: mpsc::Receiver<MediaCmd>) {
    let mut conn: Option<Connection> = Connection::session().ok();
    let mut last_poll: Option<Instant> = None;
    let mut last_conn_try = Instant::now();
    // Bus name of the player we last read — transport commands target it.
    let mut current: Option<String> = None;
    let mut gate = crate::panel_visible::VisGate::new();

    loop {
        // (Re)establish the session connection if we lost it.
        if conn.is_none() && last_conn_try.elapsed() >= RECONNECT_INTERVAL {
            conn = Connection::session().ok();
            last_conn_try = Instant::now();
        }

        let (visible, just_shown) = gate.poll();
        let mut force_poll = just_shown;
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let (Some(c), Some(player)) = (&conn, &current) {
                let method = match cmd {
                    MediaCmd::PlayPause => "PlayPause",
                    MediaCmd::Next => "Next",
                    MediaCmd::Previous => "Previous",
                };
                let _ = c.call_method(
                    Some(player.as_str()),
                    MPRIS_PATH,
                    Some(PLAYER_IFACE),
                    method,
                    &(),
                );
            }
            // Reflect the new state quickly after acting on a command.
            force_poll = true;
        }

        let interval = if visible {
            POLL_INTERVAL
        } else {
            HIDDEN_POLL_INTERVAL
        };
        let due = last_poll.map(|t| t.elapsed() >= interval).unwrap_or(true);
        if conn.is_some() && (due || force_poll) {
            let c = conn.as_ref().unwrap();
            let (track, player) = poll(c);
            // A failed ListNames likely means the connection dropped —
            // tear it down so the reconnect path kicks in.
            if track.is_none() && player.is_none() && !bus_alive(c) {
                conn = None;
                last_conn_try = Instant::now();
            }
            current = player;
            let _ = tx.send(MediaEvent::Update(track));
            last_poll = Some(Instant::now());
        }

        thread::sleep(TICK);
    }
}

/// Pick the most relevant player and read its state. Prefers a player
/// that's actually Playing; otherwise the first one with real metadata.
fn poll(conn: &Connection) -> (Option<Track>, Option<String>) {
    let mut best: Option<(Track, String)> = None;
    for name in mpris_names(conn) {
        if let Some(track) = read_player(conn, &name) {
            if track.status == PlaybackStatus::Playing {
                return (Some(track), Some(name));
            }
            if best.is_none() {
                best = Some((track, name));
            }
        }
    }
    match best {
        Some((t, n)) => (Some(t), Some(n)),
        None => (None, None),
    }
}

/// All `org.mpris.MediaPlayer2.*` names currently on the bus.
fn mpris_names(conn: &Connection) -> Vec<String> {
    let names: Vec<String> = conn
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListNames",
            &(),
        )
        .ok()
        .and_then(|r| r.body().deserialize::<Vec<String>>().ok())
        .unwrap_or_default();
    names
        .into_iter()
        .filter(|n| n.starts_with(MPRIS_PREFIX))
        .collect()
}

/// True if the bus answers a trivial call — used to distinguish "no
/// players" from "connection dropped".
fn bus_alive(conn: &Connection) -> bool {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetId",
        &(),
    )
    .is_ok()
}

fn read_player(conn: &Connection, name: &str) -> Option<Track> {
    let reply = conn
        .call_method(
            Some(name),
            MPRIS_PATH,
            Some(PROPS_IFACE),
            "GetAll",
            &(PLAYER_IFACE,),
        )
        .ok()?;
    let props: HashMap<String, OwnedValue> = reply.body().deserialize().ok()?;

    let status = props
        .get("PlaybackStatus")
        .and_then(|v| String::try_from(v.clone()).ok())
        .map(|s| match s.as_str() {
            "Playing" => PlaybackStatus::Playing,
            "Paused" => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        })
        .unwrap_or(PlaybackStatus::Stopped);

    let meta: HashMap<String, OwnedValue> = props
        .get("Metadata")
        .and_then(|v| HashMap::<String, OwnedValue>::try_from(v.clone()).ok())
        .unwrap_or_default();

    let title = str_of(&meta, "xesam:title");
    let artist = artist_of(&meta);

    // A player with no track at all (e.g. an idle browser MPRIS shim)
    // shouldn't claim the centerpiece.
    if title.is_empty() && artist.is_empty() {
        return None;
    }

    let length_us = meta.get("mpris:length").and_then(as_i64).unwrap_or(0);
    let position_us = props.get("Position").and_then(as_i64).unwrap_or(0);

    Some(Track {
        title: if title.is_empty() {
            "Unknown".to_string()
        } else {
            title
        },
        artist,
        status,
        position_us,
        length_us,
    })
}

// ─── metadata extraction ─────────────────────────────────────────────────────

fn str_of(meta: &HashMap<String, OwnedValue>, key: &str) -> String {
    meta.get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

/// `xesam:artist` is normally `as` (array of strings); join with commas.
/// Falls back to a plain string, then to album artist.
fn artist_of(meta: &HashMap<String, OwnedValue>) -> String {
    if let Some(v) = meta.get("xesam:artist") {
        if let Ok(list) = Vec::<String>::try_from(v.clone()) {
            let joined = list
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            if !joined.is_empty() {
                return joined;
            }
        }
        if let Ok(s) = String::try_from(v.clone()) {
            return s;
        }
    }
    if let Some(v) = meta.get("xesam:albumArtist") {
        if let Ok(list) = Vec::<String>::try_from(v.clone()) {
            return list.join(", ");
        }
    }
    String::new()
}

/// Coerce any integer-ish variant to i64 (players are inconsistent about
/// whether length/position are signed).
fn as_i64(v: &OwnedValue) -> Option<i64> {
    if let Ok(x) = i64::try_from(v.clone()) {
        return Some(x);
    }
    if let Ok(x) = u64::try_from(v.clone()) {
        return Some(x as i64);
    }
    if let Ok(x) = i32::try_from(v.clone()) {
        return Some(x as i64);
    }
    if let Ok(x) = u32::try_from(v.clone()) {
        return Some(x as i64);
    }
    None
}
