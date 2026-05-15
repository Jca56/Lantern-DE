//! Session interface — `org.freedesktop.Secret.Session`.
//!
//! Sessions are negotiated up-front by `Service.OpenSession` and define
//! the transport encryption for secret payloads.
//!
//! Two algorithms:
//! - `plain` — no encryption (parameters empty, value = raw bytes)
//! - `dh-ietf1024-sha256-aes128-cbc-pkcs7` — DH-derived AES-128-CBC

use lntrn_dbus::{encode_string, BodyReader, Connection, Message, Value};

use super::dh;
use super::paths::{self, IFACE_SESSION};
use super::state::{Session, SessionAlgo, ServiceState};
use super::wire::encode_byte_array;

pub const ALGO_PLAIN: &str = "plain";
pub const ALGO_DH: &str = "dh-ietf1024-sha256-aes128-cbc-pkcs7";

/// Handle `Service.OpenSession(algorithm: String, input: Variant) -> (output: Variant, path: Object)`.
pub fn open_session(
    conn: &mut Connection,
    msg: &Message,
    state: &mut ServiceState,
) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let algorithm = r.read_string();
    let input = r.read_value("v");

    let (algo, output_payload): (SessionAlgo, OutputVariant) = match algorithm.as_str() {
        ALGO_PLAIN => (SessionAlgo::Plain, OutputVariant::EmptyString),
        ALGO_DH => {
            // Input variant must be an `ay` byte array — the client's public key.
            let peer_pub = match input.as_ref() {
                Some(Value::Bytes(b)) => b.clone(),
                Some(Value::Array(arr)) => array_to_bytes(arr),
                _ => Vec::new(),
            };
            if peer_pub.is_empty() {
                send_session_error(conn, msg, "missing peer public key");
                return;
            }
            let server = dh::generate_server_secret();
            let key = match dh::derive_shared_key(&server, &peer_pub) {
                Ok(k) => k,
                Err(_) => {
                    send_session_error(conn, msg, "bad peer public key");
                    return;
                }
            };
            (SessionAlgo::DhAesCbc { key }, OutputVariant::Bytes(server.public))
        }
        _ => {
            // org.freedesktop.DBus.Error.NotSupported
            conn.send_error(
                msg.serial,
                &msg.sender,
                "org.freedesktop.DBus.Error.NotSupported",
                "Unsupported session algorithm",
            );
            return;
        }
    };

    let id = state.allocate_session_id();
    let path = paths::session_path(id);
    state.sessions.insert(id, Session { algorithm: algo });

    let mut body = Vec::new();
    match output_payload {
        OutputVariant::EmptyString => {
            // Variant: `s` ""
            body.push(1);
            body.push(b's');
            body.push(0);
            encode_string(&mut body, "");
        }
        OutputVariant::Bytes(b) => {
            // Variant: `ay` <bytes>
            body.push(2);
            body.extend_from_slice(b"ay");
            body.push(0);
            encode_byte_array(&mut body, &b);
        }
    }
    encode_string(&mut body, &path);

    conn.send_reply(msg.serial, &msg.sender, "vo", &body);
}

enum OutputVariant {
    EmptyString,
    Bytes(Vec<u8>),
}

fn send_session_error(conn: &mut Connection, msg: &Message, why: &str) {
    conn.send_error(
        msg.serial,
        &msg.sender,
        "org.freedesktop.Secret.Error.NoSession",
        why,
    );
}

fn array_to_bytes(arr: &[Value]) -> Vec<u8> {
    arr.iter()
        .filter_map(|v| match v {
            Value::Uint32(n) => Some(*n as u8),
            Value::Int32(n) => Some(*n as u8),
            _ => None,
        })
        .collect()
}

/// Dispatch a method call on a Session object.
pub fn dispatch(
    conn: &mut Connection,
    msg: &Message,
    state: &mut ServiceState,
    session_id: u64,
) -> bool {
    if msg.interface != IFACE_SESSION { return false; }
    match msg.member.as_str() {
        "Close" => {
            state.sessions.remove(&session_id);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
            true
        }
        _ => false,
    }
}

/// Encode a Secret struct for the given session: returns the
/// (parameters, value) pair appropriate for the session's algorithm.
pub fn encrypt_for_session(
    state: &ServiceState,
    session_id: u64,
    plaintext: &[u8],
) -> Option<(Vec<u8>, Vec<u8>)> {
    let sess = state.sessions.get(&session_id)?;
    match &sess.algorithm {
        SessionAlgo::Plain => Some((Vec::new(), plaintext.to_vec())),
        SessionAlgo::DhAesCbc { key } => {
            let iv = dh::random_iv();
            let ct = dh::encrypt(key, &iv, plaintext);
            Some((iv.to_vec(), ct))
        }
    }
}

/// Decode an incoming Secret struct against the given session: returns
/// the plaintext.
pub fn decrypt_for_session(
    state: &ServiceState,
    session_id: u64,
    parameters: &[u8],
    value: &[u8],
) -> Option<Vec<u8>> {
    let sess = state.sessions.get(&session_id)?;
    match &sess.algorithm {
        SessionAlgo::Plain => Some(value.to_vec()),
        SessionAlgo::DhAesCbc { key } => {
            if parameters.len() != 16 { return None; }
            let mut iv = [0u8; 16];
            iv.copy_from_slice(parameters);
            dh::decrypt(key, &iv, value).ok()
        }
    }
}

