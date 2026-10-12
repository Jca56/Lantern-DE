//! MPRIS D-Bus server — advertises the media player on the session bus so
//! desktop bars and media key daemons can discover it and send transport commands.

use std::sync::mpsc;

use crate::event_pump::Waker;
use lntrn_dbus::{
    align_to, encode_dict_entry_sv, encode_signature, encode_string, encode_u32,
    encode_variant_bool, encode_variant_double, encode_variant_i64, encode_variant_string,
    BodyReader, Connection, Message,
};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.lntrn_media_player";
const IFACE_ROOT: &str = "org.mpris.MediaPlayer2";
const IFACE_PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const IFACE_PROPS: &str = "org.freedesktop.DBus.Properties";

// ── Public types ────────────────────────────────────────────────────────────

/// Snapshot of player state sent from the main thread.
#[derive(Clone, PartialEq)]
pub struct PlayerState {
    pub title: String,
    /// Proper (escaped) `file://` URI of the open file; empty when none.
    pub url: String,
    /// Something is loaded (otherwise the status is "Stopped").
    pub has_media: bool,
    pub playing: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
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
    Seek(i64),
}

// ── Spawn ───────────────────────────────────────────────────────────────────

/// Run the MPRIS server on its own thread: it connects to the session bus,
/// claims the bus name and answers calls there. `waker` is poked after every
/// command so the main loop acts on it at once instead of on its next timer.
pub fn spawn(waker: Waker) -> (mpsc::Sender<PlayerState>, mpsc::Receiver<MprisCmd>) {
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

            // Noted for diagnosing fd clashes with GStreamer's own D-Bus use.
            let raw_fd = conn.as_raw_fd();
            log(&format!("D-Bus socket fd={raw_fd}"));

            if !conn.request_name(BUS_NAME) {
                log("Failed to claim bus name");
                return;
            }
            log(&format!("Claimed {BUS_NAME}"));

            server_thread_with_conn(conn, state_rx, cmd_tx, waker);
        })
        .expect("spawn mpris server thread");

    (state_tx, cmd_rx)
}

// ── Server thread ───────────────────────────────────────────────────────────

/// The log starts over once it passes this size.
const LOG_CAP: u64 = 256 * 1024;

/// `~/.lantern/log/media-player-mpris.log`, with the other Lantern logs.
fn log(msg: &str) {
    use std::io::Write;
    use std::sync::OnceLock;
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    let path = PATH.get_or_init(|| {
        let dir = lntrn_theme::config::lantern_home()?.join("log");
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join("media-player-mpris.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_CAP) {
            let _ = std::fs::remove_file(&path);
        }
        Some(path)
    });
    let Some(path) = path else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{msg}");
    }
}

/// How long the server waits on the bus before looking at fresh player state.
const POLL_MS: i32 = 200;

fn server_thread_with_conn(
    mut conn: Connection,
    state_rx: mpsc::Receiver<PlayerState>,
    cmd_tx: mpsc::Sender<MprisCmd>,
    waker: Waker,
) {
    log("MPRIS server thread running");

    let mut state = PlayerState {
        title: String::new(),
        url: String::new(),
        has_media: false,
        playing: false,
        can_go_next: false,
        can_go_previous: false,
        position_ns: 0,
        duration_ns: 0,
        volume: 1.0,
    };
    loop {
        // Sleep until a call arrives (answered at once, not on a 50 ms tick).
        let mut fds = [libc::pollfd {
            fd: conn.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: `fds` is a live one-element array.
        unsafe { libc::poll(fds.as_mut_ptr(), 1, POLL_MS) };
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            log("D-Bus connection closed; MPRIS server stopping");
            return;
        }

        // Drain state updates (keep only the latest). Clients query Position /
        // PlaybackStatus on demand, so we just need the freshest snapshot.
        while let Ok(new_state) = state_rx.try_recv() {
            state = new_state;
        }

        // Handle incoming D-Bus method calls
        let mut commanded = false;
        while let Some(msg) = conn.try_read() {
            if msg.is_method_call() {
                commanded |= handle_method(&mut conn, &msg, &state, &cmd_tx);
            }
        }
        if commanded {
            waker.wake();
        }
    }
}

/// Answer one call. Returns true when it queued a command for the main thread.
fn handle_method(
    conn: &mut Connection,
    msg: &Message,
    state: &PlayerState,
    cmd_tx: &mpsc::Sender<MprisCmd>,
) -> bool {
    let is_command = msg.interface == IFACE_PLAYER;
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
                    msg.body[reader.pos],
                    msg.body[reader.pos + 1],
                    msg.body[reader.pos + 2],
                    msg.body[reader.pos + 3],
                    msg.body[reader.pos + 4],
                    msg.body[reader.pos + 5],
                    msg.body[reader.pos + 6],
                    msg.body[reader.pos + 7],
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
    is_command
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
            let s = match (state.has_media, state.playing) {
                (false, _) => "Stopped",
                (true, true) => "Playing",
                (true, false) => "Paused",
            };
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
        (IFACE_PLAYER, "CanGoNext") => encode_variant_bool(buf, state.can_go_next),
        (IFACE_PLAYER, "CanGoPrevious") => encode_variant_bool(buf, state.can_go_previous),
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
            "Identity",
            "DesktopEntry",
            "CanQuit",
            "CanRaise",
            "HasTrackList",
            "SupportedUriSchemes",
            "SupportedMimeTypes",
        ],
        IFACE_PLAYER => &[
            "PlaybackStatus",
            "Metadata",
            "Volume",
            "Position",
            "Rate",
            "MinimumRate",
            "MaximumRate",
            "CanGoNext",
            "CanGoPrevious",
            "CanPlay",
            "CanPause",
            "CanSeek",
            "CanControl",
        ],
        _ => &[],
    };

    // Encode as a{sv}: array length placeholder, then entries. The length
    // counts ONLY the element bytes, not the padding inserted to reach the
    // dict-entry (8-byte) boundary after the u32 length — so align first,
    // THEN mark the start. Counting the padding makes the array overrun its
    // own body and the bus daemon drops us for a malformed message.
    let len_pos = buf.len();
    encode_u32(&mut buf, 0); // placeholder
    align_to(&mut buf, 8);
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
    // Now encode the dict. Align to the dict-entry boundary BEFORE marking
    // the start so the length excludes the padding (see encode_all_properties).
    align_to(buf, 4);
    let len_pos = buf.len();
    encode_u32(buf, 0); // placeholder
    align_to(buf, 8);
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
    if !state.url.is_empty() {
        encode_dict_entry_sv(buf, "xesam:url", |b| {
            encode_variant_string(b, &state.url);
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
    <property name="Volume" type="d" access="read"/>
    <property name="Position" type="x" access="read"/>
    <property name="Rate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>"#
        .to_string()
}
