//! D-Bus client for `org.freedesktop.Secret.*` — talks to lntrn-keychain.
//!
//! We use the `plain` session algorithm because we're a local trusted client
//! on the same socket as the daemon — encrypting payloads to ourselves over
//! AF_UNIX wouldn't add anything.

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use lntrn_dbus::{
    align_to, encode_string, encode_u32, BodyReader, Connection, Value,
};

const BUS_NAME: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const DEFAULT_COLLECTION: &str = "/org/freedesktop/secrets/aliases/default";
const IFACE_SERVICE: &str = "org.freedesktop.Secret.Service";
const IFACE_COLLECTION: &str = "org.freedesktop.Secret.Collection";
const IFACE_ITEM: &str = "org.freedesktop.Secret.Item";
const IFACE_PROMPT: &str = "org.freedesktop.Secret.Prompt";
const IFACE_PROPS: &str = "org.freedesktop.DBus.Properties";

pub struct Client {
    conn: Connection,
    session: String,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub path: String,
    pub label: String,
    pub attributes: HashMap<String, String>,
}

impl Client {
    pub fn connect() -> io::Result<Self> {
        let mut conn = Connection::connect()?;
        // OpenSession(algorithm: plain, input: variant<s> "") → (output: variant, session: o)
        let mut body = Vec::new();
        encode_string(&mut body, "plain");
        // variant<s> ""
        body.push(1); body.push(b's'); body.push(0);
        encode_string(&mut body, "");
        let serial = conn.method_call(
            BUS_NAME, SERVICE_PATH, IFACE_SERVICE, "OpenSession",
            "sv", &body,
        );
        let reply = conn.read_reply(serial)?;
        if reply.is_error() {
            return Err(io::Error::new(io::ErrorKind::Other, "OpenSession returned error"));
        }
        let mut r = BodyReader::new(&reply.body, &reply.signature);
        let _output = r.read_value("v"); // unused for plain
        let session_val = r.read_value("o").ok_or_else(|| io::Error::new(
            io::ErrorKind::Other, "OpenSession: missing session path",
        ))?;
        let session = session_val.as_str().unwrap_or("").to_string();
        Ok(Self { conn, session })
    }

    /// Fetch all items in the default collection, with their metadata.
    pub fn list_items(&mut self) -> io::Result<Vec<Item>> {
        // SearchItems on /aliases/default returns ao
        let mut body = Vec::new();
        // empty a{ss}
        align_to(&mut body, 4);
        encode_u32(&mut body, 0);
        align_to(&mut body, 8);
        let serial = self.conn.method_call(
            BUS_NAME, DEFAULT_COLLECTION, IFACE_COLLECTION, "SearchItems",
            "a{ss}", &body,
        );
        let reply = self.conn.read_reply(serial)?;
        if reply.is_error() {
            return Err(io::Error::new(io::ErrorKind::Other, "SearchItems returned error"));
        }
        let mut r = BodyReader::new(&reply.body, &reply.signature);
        let paths = read_ao(&mut r);

        let mut items = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(it) = self.fetch_item(&path)? {
                items.push(it);
            }
        }
        items.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
        Ok(items)
    }

    fn fetch_item(&mut self, path: &str) -> io::Result<Option<Item>> {
        // Properties.GetAll("org.freedesktop.Secret.Item") → a{sv}
        let mut body = Vec::new();
        encode_string(&mut body, IFACE_ITEM);
        let serial = self.conn.method_call(
            BUS_NAME, path, IFACE_PROPS, "GetAll", "s", &body,
        );
        let reply = self.conn.read_reply(serial)?;
        if reply.is_error() {
            return Ok(None);
        }
        let mut r = BodyReader::new(&reply.body, &reply.signature);
        let props = match r.read_value("a{sv}") {
            Some(Value::Dict(d)) => d,
            _ => return Ok(None),
        };
        let label = props.get("Label").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let attributes = match props.get("Attributes") {
            Some(Value::Dict(d)) => d.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect(),
            _ => HashMap::new(),
        };
        Ok(Some(Item { path: path.to_string(), label, attributes }))
    }

    /// Get the secret bytes for one item.
    pub fn get_secret(&mut self, path: &str) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        encode_string(&mut body, &self.session);
        let serial = self.conn.method_call(
            BUS_NAME, path, IFACE_ITEM, "GetSecret", "o", &body,
        );
        let reply = self.conn.read_reply(serial)?;
        if reply.is_error() {
            return Err(io::Error::new(io::ErrorKind::Other, "GetSecret returned error"));
        }
        let mut r = BodyReader::new(&reply.body, &reply.signature);
        // (oayays) — session, parameters, value, content_type
        r.align(8);
        let _session = r.read_string();
        let _params = read_ay(&mut r);
        let value = read_ay(&mut r);
        let _ct = r.read_string();
        Ok(value)
    }

    /// Create a new item in the default collection. Replaces by-attributes if
    /// `replace` is true.
    pub fn create_item(
        &mut self,
        label: &str,
        attributes: &HashMap<String, String>,
        secret: &[u8],
        replace: bool,
    ) -> io::Result<String> {
        let mut body = Vec::new();
        // a{sv} properties
        encode_dict_sv(&mut body, &[
            ("org.freedesktop.Secret.Item.Label", PropVal::Str(label.into())),
            ("org.freedesktop.Secret.Item.Attributes", PropVal::DictSS(attributes.clone())),
        ]);
        // (oayays) — session, parameters=[], value=secret, ct="text/plain"
        encode_secret(&mut body, &self.session, &[], secret, "text/plain");
        // b — replace
        align_to(&mut body, 4);
        encode_u32(&mut body, replace as u32);

        let serial = self.conn.method_call(
            BUS_NAME, DEFAULT_COLLECTION, IFACE_COLLECTION, "CreateItem",
            "a{sv}(oayays)b", &body,
        );
        let reply = self.conn.read_reply(serial)?;
        if reply.is_error() {
            return Err(io::Error::new(io::ErrorKind::Other, "CreateItem returned error"));
        }
        let mut r = BodyReader::new(&reply.body, &reply.signature);
        let item_path = r.read_string();
        let _prompt = r.read_string();
        Ok(item_path)
    }

    /// Delete an item. Walks through the prompt protocol (our daemon
    /// completes prompts synchronously, so this returns quickly).
    pub fn delete_item(&mut self, path: &str) -> io::Result<()> {
        let serial = self.conn.method_call(
            BUS_NAME, path, IFACE_ITEM, "Delete", "", &[],
        );
        let reply = self.conn.read_reply(serial)?;
        if reply.is_error() {
            return Err(io::Error::new(io::ErrorKind::Other, "Delete returned error"));
        }
        let mut r = BodyReader::new(&reply.body, &reply.signature);
        let prompt_path = r.read_string();
        if prompt_path == "/" || prompt_path.is_empty() {
            return Ok(());
        }
        // Prompt.Prompt("") → empty reply, then wait for Completed signal
        let mut body = Vec::new();
        encode_string(&mut body, "");
        let _ = self.conn.method_call(
            BUS_NAME, &prompt_path, IFACE_PROMPT, "Prompt", "s", &body,
        );
        // Wait for Completed signal on our prompt path (up to 2s).
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(msg) = self.conn.try_read() {
                if msg.is_signal()
                    && msg.path == prompt_path
                    && msg.interface == IFACE_PROMPT
                    && msg.member == "Completed"
                {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
}

// ── Encoding helpers ────────────────────────────────────────────────────────

enum PropVal {
    Str(String),
    DictSS(HashMap<String, String>),
}

fn encode_dict_sv(out: &mut Vec<u8>, entries: &[(&str, PropVal)]) {
    align_to(out, 4);
    let len_pos = out.len();
    encode_u32(out, 0);
    align_to(out, 8);
    let body_start = out.len();
    for (k, v) in entries {
        align_to(out, 8);
        encode_string(out, k);
        match v {
            PropVal::Str(s) => {
                out.push(1); out.push(b's'); out.push(0);
                encode_string(out, s);
            }
            PropVal::DictSS(m) => {
                out.push(5);
                out.extend_from_slice(b"a{ss}");
                out.push(0);
                encode_dict_ss(out, m);
            }
        }
    }
    let body_len = (out.len() - body_start) as u32;
    out[len_pos..len_pos + 4].copy_from_slice(&body_len.to_le_bytes());
}

fn encode_dict_ss(out: &mut Vec<u8>, map: &HashMap<String, String>) {
    align_to(out, 4);
    let len_pos = out.len();
    encode_u32(out, 0);
    align_to(out, 8);
    let body_start = out.len();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        align_to(out, 8);
        encode_string(out, k);
        encode_string(out, &map[k]);
    }
    let body_len = (out.len() - body_start) as u32;
    out[len_pos..len_pos + 4].copy_from_slice(&body_len.to_le_bytes());
}

fn encode_secret(out: &mut Vec<u8>, session: &str, params: &[u8], value: &[u8], ct: &str) {
    align_to(out, 8);
    encode_string(out, session);
    align_to(out, 4);
    encode_u32(out, params.len() as u32);
    out.extend_from_slice(params);
    align_to(out, 4);
    encode_u32(out, value.len() as u32);
    out.extend_from_slice(value);
    encode_string(out, ct);
}

// ── Decoding helpers ────────────────────────────────────────────────────────

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
