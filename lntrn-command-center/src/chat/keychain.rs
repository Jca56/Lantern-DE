//! Minimal Secret Service client for fetching the Anthropic API key
//! from lntrn-keychain. Plain session over the user bus.
//!
//! Looks up the item with attribute `name=Claude API` in the default
//! collection — that's where the user stored their key.

use std::io;

use lntrn_dbus::{align_to, encode_string, encode_u32, BodyReader, Connection};

const BUS_NAME: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const DEFAULT_COLLECTION: &str = "/org/freedesktop/secrets/aliases/default";
const IFACE_SERVICE: &str = "org.freedesktop.Secret.Service";
const IFACE_COLLECTION: &str = "org.freedesktop.Secret.Collection";
const IFACE_ITEM: &str = "org.freedesktop.Secret.Item";

const LOOKUP_ATTR_KEY: &str = "name";
const LOOKUP_ATTR_VALUE: &str = "Claude API";

/// Fetch the Claude API key from the user keyring. Returns `Ok(None)` when
/// no matching item is found; only returns `Err` on bus / protocol failures.
pub fn fetch_api_key() -> io::Result<Option<String>> {
    let mut conn = Connection::connect()?;

    // OpenSession(algorithm="plain", input=variant<s>"") -> (output, session)
    let mut body = Vec::new();
    encode_string(&mut body, "plain");
    body.push(1); body.push(b's'); body.push(0);
    encode_string(&mut body, "");
    let serial = conn.method_call(
        BUS_NAME, SERVICE_PATH, IFACE_SERVICE, "OpenSession", "sv", &body,
    );
    let reply = conn.read_reply(serial)?;
    if reply.is_error() {
        return Err(io::Error::other("OpenSession failed"));
    }
    let mut r = BodyReader::new(&reply.body, &reply.signature);
    let _output = r.read_value("v");
    let session = r.read_value("o")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| io::Error::other("OpenSession: missing session path"))?;

    // SearchItems(attrs={name: "Claude API"}) on default collection -> ao
    let mut body = Vec::new();
    encode_dict_ss_one(&mut body, LOOKUP_ATTR_KEY, LOOKUP_ATTR_VALUE);
    let serial = conn.method_call(
        BUS_NAME, DEFAULT_COLLECTION, IFACE_COLLECTION, "SearchItems",
        "a{ss}", &body,
    );
    let reply = conn.read_reply(serial)?;
    if reply.is_error() {
        return Err(io::Error::other("SearchItems failed"));
    }
    let mut r = BodyReader::new(&reply.body, &reply.signature);
    let paths = read_ao(&mut r);
    let Some(path) = paths.first().cloned() else {
        return Ok(None);
    };

    // GetSecret(session) -> (oayays)
    let mut body = Vec::new();
    encode_string(&mut body, &session);
    let serial = conn.method_call(
        BUS_NAME, &path, IFACE_ITEM, "GetSecret", "o", &body,
    );
    let reply = conn.read_reply(serial)?;
    if reply.is_error() {
        return Err(io::Error::other("GetSecret failed"));
    }
    let mut r = BodyReader::new(&reply.body, &reply.signature);
    r.align(8);
    let _session_path = r.read_string();
    let _params = read_ay(&mut r);
    let value = read_ay(&mut r);
    let _content_type = r.read_string();

    let key = String::from_utf8(value)
        .map_err(|_| io::Error::other("API key isn't valid UTF-8"))?
        .trim()
        .to_string();
    if key.is_empty() {
        return Ok(None);
    }
    Ok(Some(key))
}

fn encode_dict_ss_one(out: &mut Vec<u8>, k: &str, v: &str) {
    align_to(out, 4);
    let len_pos = out.len();
    encode_u32(out, 0);
    align_to(out, 8);
    let body_start = out.len();
    align_to(out, 8);
    encode_string(out, k);
    encode_string(out, v);
    let body_len = (out.len() - body_start) as u32;
    out[len_pos..len_pos + 4].copy_from_slice(&body_len.to_le_bytes());
}

fn read_ao(r: &mut BodyReader<'_>) -> Vec<String> {
    r.align(4);
    let n = r.read_u32() as usize;
    r.align(4);
    let end = r.pos + n;
    let mut out = Vec::new();
    while r.pos < end {
        out.push(r.read_string());
    }
    r.pos = end;
    out
}

fn read_ay(r: &mut BodyReader<'_>) -> Vec<u8> {
    r.align(4);
    let n = r.read_u32() as usize;
    r.read_bytes(n)
}
