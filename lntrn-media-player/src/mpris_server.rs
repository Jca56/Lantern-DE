//! MPRIS D-Bus server — advertises the media player on the session bus so
//! desktop bars and media key daemons can discover it and send transport commands.

use std::sync::mpsc;
use std::time::Duration;

use lntrn_dbus::{
    align_to, encode_string, encode_u32, encode_signature,
    encode_dict_entry_sv, encode_variant_string, encode_variant_bool,
    encode_variant_double, encode_variant_i64,
    Connection, BodyReader, Message,
};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.lntrn_media_player";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const IFACE_ROOT: &str = "org.mpris.MediaPlayer2";
const IFACE_PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const IFACE_PROPS: &str = "org.freedesktop.DBus.Properties";

// ── Public types ────────────────────────────────────────────────────────────

/// Snapshot of player state sent from the main thread.
#[derive(Clone)]
pub struct PlayerState {
    pub title: String,
    pub file_path: String,
    pub playing: bool,
    pub position_ns: u64,
    pub duration_ns: u64,
    pub volume: f64,
}

/// Commands the MPRIS server sends back to the main thread.
pub enum MprisCmd {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    Stop,
    SetVolume(f64),
    Seek(i64),
}

// ── Spawn ───────────────────────────────────────────────────────────────────

/// Create the D-Bus connection and claim the bus name on the CALLING thread,
/// then hand the connection off to a background thread for message handling.
pub fn spawn() -> (mpsc::Sender<PlayerState>, mpsc::Receiver<MprisCmd>) {
    let (state_tx, state_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();

    // Connect and claim name on the main thread, keep it alive by
    // NOT handing off — run the server loop on a dedicated thread that
    // also owns the connection from creation.
    std::thread::Builder::new()
        .name("mpris-server".into())
        .spawn(move || {
            let mut conn = match Connection::connect() {
                Ok(c) => c,
                Err(e) => {
                    log(&format!("D-Bus connect failed: {e}"));
                    return;
                }
            };
            log(&format!("D-Bus connected as {}", conn.unique_name()));

            // Dup the fd to a high number so GStreamer can't accidentally close it
            let raw_fd = conn.as_raw_fd();
            log(&format!("D-Bus socket fd={raw_fd}"));

            if !conn.request_name(BUS_NAME) {
                log("Failed to claim bus name");
                return;
            }
            log(&format!("Claimed {BUS_NAME}"));

            server_thread_with_conn(conn, state_rx, cmd_tx);
        })
        .expect("spawn mpris server thread");

    (state_tx, cmd_rx)
}

// ── Server thread ───────────────────────────────────────────────────────────

fn log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open("/tmp/lntrn-mpris.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

fn server_thread_with_conn(
    mut conn: Connection,
    state_rx: mpsc::Receiver<PlayerState>,
    cmd_tx: mpsc::Sender<MprisCmd>,
) {
    log("MPRIS server thread running");
    let mut heartbeat_count = 0u32;
    let mut first_loop = true;

    let mut state = PlayerState {
        title: String::new(),
        file_path: String::new(),
        playing: false,
        position_ns: 0,
        duration_ns: 0,
        volume: 1.0,
    };
    let mut prev_playing = false;
    let mut prev_title = String::new();

    loop {
        // Drain state updates (take latest)
        let mut changed = false;
        while let Ok(new_state) = state_rx.try_recv() {
            if new_state.playing != state.playing || new_state.title != state.title
                || (new_state.volume - state.volume).abs() > 0.01
            {
                changed = true;
            }
            state = new_state;
        }

        // PropertiesChanged disabled for debugging
        if changed {
            prev_playing = state.playing;
            prev_title = state.title.clone();
        }

        // Handle incoming D-Bus method calls
        while let Some(msg) = conn.try_read() {
            if msg.is_method_call() {
                handle_method(&mut conn, &msg, &state, &cmd_tx);
            }
        }

        // Heartbeat: actually call GetNameOwner to verify we own the name
        heartbeat_count += 1;
        if heartbeat_count >= 40 {
            heartbeat_count = 0;
            let mut body = Vec::new();
            lntrn_dbus::encode_string(&mut body, BUS_NAME);
            let serial = conn.method_call(
                "org.freedesktop.DBus", "/org/freedesktop/DBus",
                "org.freedesktop.DBus", "GetNameOwner", "s", &body,
            );
            // Try to read the reply (blocking briefly)
            match conn.read_reply(serial) {
                Ok(reply) => {
                    let mut r = BodyReader::new(&reply.body, &reply.signature);
                    let owner = r.read_string();
                    log(&format!("heartbeat: owner of {BUS_NAME} = '{owner}' (err={})", reply.is_error()));
                }
                Err(e) => {
                    log(&format!("heartbeat: GetNameOwner FAILED: {e}"));
                }
            }
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

fn handle_method(
    conn: &mut Connection, msg: &Message, state: &PlayerState,
    cmd_tx: &mpsc::Sender<MprisCmd>,
) {
    match (msg.interface.as_str(), msg.member.as_str()) {
        (IFACE_PROPS, "Get") => {
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            let iface = reader.read_string();
            let prop = reader.read_string();
            let mut body = Vec::new();
            encode_property(&mut body, &iface, &prop, state);
            conn.send_reply(msg.serial, &msg.sender, "v", &body);
        }
        (IFACE_PROPS, "GetAll") => {
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            let iface = reader.read_string();
            let body = encode_all_properties(&iface, state);
            conn.send_reply(msg.serial, &msg.sender, "a{sv}", &body);
        }
        (IFACE_PLAYER, "PlayPause") => {
            let _ = cmd_tx.send(MprisCmd::PlayPause);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        (IFACE_PLAYER, "Play") => {
            let _ = cmd_tx.send(MprisCmd::Play);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        (IFACE_PLAYER, "Pause") => {
            let _ = cmd_tx.send(MprisCmd::Pause);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        (IFACE_PLAYER, "Next") => {
            let _ = cmd_tx.send(MprisCmd::Next);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        (IFACE_PLAYER, "Previous") => {
            let _ = cmd_tx.send(MprisCmd::Previous);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        (IFACE_PLAYER, "Stop") => {
            let _ = cmd_tx.send(MprisCmd::Stop);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        (IFACE_PLAYER, "Seek") => {
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            reader.align(8);
            if reader.pos + 8 <= msg.body.len() {
                let offset = i64::from_le_bytes([
                    msg.body[reader.pos], msg.body[reader.pos+1],
                    msg.body[reader.pos+2], msg.body[reader.pos+3],
                    msg.body[reader.pos+4], msg.body[reader.pos+5],
                    msg.body[reader.pos+6], msg.body[reader.pos+7],
                ]);
                let _ = cmd_tx.send(MprisCmd::Seek(offset));
            }
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        (IFACE_ROOT, "Quit") | (IFACE_ROOT, "Raise") => {
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        ("org.freedesktop.DBus.Introspectable", "Introspect") => {
            let xml = introspect_xml();
            let mut body = Vec::new();
            encode_string(&mut body, &xml);
            conn.send_reply(msg.serial, &msg.sender, "s", &body);
        }
        _ => {
            // Unknown method — reply empty
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
    }
}

// ── Property encoding ───────────────────────────────────────────────────────

fn encode_property(buf: &mut Vec<u8>, iface: &str, prop: &str, state: &PlayerState) {
    match (iface, prop) {
        (IFACE_ROOT, "Identity") => encode_variant_string(buf, "Lantern Media Player"),
        (IFACE_ROOT, "DesktopEntry") => encode_variant_string(buf, "org.lantern.MediaPlayer"),
        (IFACE_ROOT, "CanQuit") => encode_variant_bool(buf, false),
        (IFACE_ROOT, "CanRaise") => encode_variant_bool(buf, false),
        (IFACE_ROOT, "HasTrackList") => encode_variant_bool(buf, false),
        (IFACE_ROOT, "SupportedUriSchemes") => encode_variant_empty_array_string(buf),
        (IFACE_ROOT, "SupportedMimeTypes") => encode_variant_empty_array_string(buf),
        (IFACE_PLAYER, "PlaybackStatus") => {
            let s = if state.playing { "Playing" } else { "Paused" };
            encode_variant_string(buf, s);
        }
        (IFACE_PLAYER, "Metadata") => encode_variant_metadata(buf, state),
        (IFACE_PLAYER, "Volume") => encode_variant_double(buf, state.volume),
        (IFACE_PLAYER, "Position") => {
            // MPRIS uses microseconds
            encode_variant_i64(buf, (state.position_ns / 1000) as i64);
        }
        (IFACE_PLAYER, "Rate") => encode_variant_double(buf, 1.0),
        (IFACE_PLAYER, "MinimumRate") => encode_variant_double(buf, 1.0),
        (IFACE_PLAYER, "MaximumRate") => encode_variant_double(buf, 1.0),
        (IFACE_PLAYER, "CanGoNext") => encode_variant_bool(buf, false),
        (IFACE_PLAYER, "CanGoPrevious") => encode_variant_bool(buf, false),
        (IFACE_PLAYER, "CanPlay") => encode_variant_bool(buf, true),
        (IFACE_PLAYER, "CanPause") => encode_variant_bool(buf, true),
        (IFACE_PLAYER, "CanSeek") => encode_variant_bool(buf, true),
        (IFACE_PLAYER, "CanControl") => encode_variant_bool(buf, true),
        _ => encode_variant_string(buf, ""),
    }
}

fn encode_all_properties(iface: &str, state: &PlayerState) -> Vec<u8> {
    let mut buf = Vec::new();
    let props: &[&str] = match iface {
        IFACE_ROOT => &[
            "Identity", "DesktopEntry", "CanQuit", "CanRaise",
            "HasTrackList", "SupportedUriSchemes", "SupportedMimeTypes",
        ],
        IFACE_PLAYER => &[
            "PlaybackStatus", "Metadata", "Volume", "Position", "Rate",
            "MinimumRate", "MaximumRate", "CanGoNext", "CanGoPrevious",
            "CanPlay", "CanPause", "CanSeek", "CanControl",
        ],
        _ => &[],
    };

    // Encode as a{sv}: array length placeholder, then entries
    let len_pos = buf.len();
    encode_u32(&mut buf, 0); // placeholder
    let array_start = buf.len();

    for prop in props {
        encode_dict_entry_sv(&mut buf, prop, |b| {
            encode_property(b, iface, prop, state);
        });
    }

    let array_len = (buf.len() - array_start) as u32;
    buf[len_pos..len_pos + 4].copy_from_slice(&array_len.to_le_bytes());
    buf
}

fn encode_variant_metadata(buf: &mut Vec<u8>, state: &PlayerState) {
    // variant signature: a{sv}
    encode_signature(buf, "a{sv}");
    // Now encode the dict
    align_to(buf, 4);
    let len_pos = buf.len();
    encode_u32(buf, 0); // placeholder
    let array_start = buf.len();

    // mpris:trackid (required)
    encode_dict_entry_sv(buf, "mpris:trackid", |b| {
        encode_signature(b, "o");
        encode_string(b, "/org/lantern/MediaPlayer/Track");
    });

    // xesam:title
    if !state.title.is_empty() {
        encode_dict_entry_sv(buf, "xesam:title", |b| {
            encode_variant_string(b, &state.title);
        });
    }

    // mpris:length (microseconds)
    if state.duration_ns > 0 {
        encode_dict_entry_sv(buf, "mpris:length", |b| {
            encode_variant_i64(b, (state.duration_ns / 1000) as i64);
        });
    }

    // xesam:url
    if !state.file_path.is_empty() {
        encode_dict_entry_sv(buf, "xesam:url", |b| {
            encode_variant_string(b, &format!("file://{}", state.file_path));
        });
    }

    let array_len = (buf.len() - array_start) as u32;
    buf[len_pos..len_pos + 4].copy_from_slice(&array_len.to_le_bytes());
}

fn encode_variant_empty_array_string(buf: &mut Vec<u8>) {
    encode_signature(buf, "as");
    align_to(buf, 4);
    encode_u32(buf, 0); // empty array
}

fn emit_properties_changed(conn: &mut Connection, state: &PlayerState) {
    // Signal body: STRING interface_name, DICT changed_properties, ARRAY invalidated
    let mut body = Vec::new();
    encode_string(&mut body, IFACE_PLAYER);

    // Changed properties dict: a{sv}
    align_to(&mut body, 4);
    let len_pos = body.len();
    encode_u32(&mut body, 0);
    let array_start = body.len();

    encode_dict_entry_sv(&mut body, "PlaybackStatus", |b| {
        let s = if state.playing { "Playing" } else { "Paused" };
        encode_variant_string(b, s);
    });
    encode_dict_entry_sv(&mut body, "Metadata", |b| {
        encode_variant_metadata(b, state);
    });

    let array_len = (body.len() - array_start) as u32;
    body[len_pos..len_pos + 4].copy_from_slice(&array_len.to_le_bytes());

    // Invalidated properties: empty array of strings
    align_to(&mut body, 4);
    encode_u32(&mut body, 0);

    conn.send_signal(
        OBJECT_PATH, IFACE_PROPS, "PropertiesChanged",
        "sa{sv}as", &body,
    );
}

fn introspect_xml() -> String {
    r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg direction="in" type="x" name="Offset"/></method>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="Rate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>"#.to_string()
}
