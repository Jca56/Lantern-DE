//! Minimal D-Bus session bus client — wire protocol from scratch.
//!
//! Implements Unix socket connection, SASL EXTERNAL auth, message
//! encoding/decoding, method calls, replies, and signals.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

// ── D-Bus message types ─────────────────────────────────────────────────────

const MSG_METHOD_CALL: u8 = 1;
const MSG_METHOD_RETURN: u8 = 2;
const MSG_ERROR: u8 = 3;
const MSG_SIGNAL: u8 = 4;

// Header field codes
const FIELD_PATH: u8 = 1;
const FIELD_INTERFACE: u8 = 2;
const FIELD_MEMBER: u8 = 3;
const FIELD_DESTINATION: u8 = 6;
const FIELD_SENDER: u8 = 7;
const FIELD_SIGNATURE: u8 = 8;
const FIELD_REPLY_SERIAL: u8 = 5;

// ── Public types ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Message {
    pub msg_type: u8,
    pub serial: u32,
    pub reply_serial: u32,
    pub sender: String,
    pub path: String,
    pub member: String,
    pub interface: String,
    pub signature: String,
    pub destination: String,
    pub body: Vec<u8>,
}

impl Message {
    pub fn is_method_call(&self) -> bool { self.msg_type == MSG_METHOD_CALL }
    pub fn is_signal(&self) -> bool { self.msg_type == MSG_SIGNAL }
    pub fn is_error(&self) -> bool { self.msg_type == MSG_ERROR }
}

/// Parsed D-Bus value.
#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    ObjectPath(String),
    Int32(i32),
    Uint32(u32),
    Int64(i64),
    Bool(bool),
    Double(f64),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Struct(Vec<Value>),
    Dict(HashMap<String, Value>),
    Variant(Box<Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self { Value::String(s) | Value::ObjectPath(s) => Some(s), _ => None }
    }
    pub fn as_i32(&self) -> Option<i32> {
        match self { Value::Int32(v) => Some(*v), _ => None }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self { Value::Int64(v) => Some(*v), _ => None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self { Value::Double(v) => Some(*v), _ => None }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self { Value::Bytes(v) => Some(v), _ => None }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self { Value::Array(v) => Some(v), _ => None }
    }
    pub fn as_u32(&self) -> Option<u32> {
        match self { Value::Uint32(v) => Some(*v), _ => None }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self { Value::Bool(v) => Some(*v), _ => None }
    }
    pub fn as_dict(&self) -> Option<&HashMap<String, Value>> {
        match self { Value::Dict(d) => Some(d), _ => None }
    }
}

// ── Connection ──────────────────────────────────────────────────────────────

pub struct Connection {
    stream: UnixStream,
    serial: u32,
    unique_name: String,
}

impl Connection {
    /// Connect to the session bus, authenticate, and complete Hello handshake.
    pub fn connect() -> io::Result<Self> {
        let addr = std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "DBUS_SESSION_BUS_ADDRESS not set"))?;

        let path = parse_bus_address(&addr)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "can't parse bus address"))?;

        let mut stream = UnixStream::connect(&path)?;
        sasl_auth(&mut stream)?;

        let mut conn = Self { stream, serial: 0, unique_name: String::new() };

        let serial = conn.method_call(
            "org.freedesktop.DBus", "/org/freedesktop/DBus",
            "org.freedesktop.DBus", "Hello", "", &[],
        );
        let reply = conn.read_reply(serial)?;
        if let Some(name) = BodyReader::new(&reply.body, &reply.signature).read_value("s") {
            conn.unique_name = name.as_str().unwrap_or("").to_string();
        }

        conn.stream.set_nonblocking(true)?;
        Ok(conn)
    }

    /// Connect to the system bus, authenticate, and complete Hello handshake.
    pub fn connect_system() -> io::Result<Self> {
        let path = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").ok()
            .and_then(|a| parse_bus_address(&a))
            .unwrap_or_else(|| "/run/dbus/system_bus_socket".to_string());
        let mut stream = UnixStream::connect(&path)?;
        sasl_auth(&mut stream)?;
        let mut conn = Self { stream, serial: 0, unique_name: String::new() };
        let serial = conn.method_call(
            "org.freedesktop.DBus", "/org/freedesktop/DBus",
            "org.freedesktop.DBus", "Hello", "", &[],
        );
        let reply = conn.read_reply(serial)?;
        if let Some(name) = BodyReader::new(&reply.body, &reply.signature).read_value("s") {
            conn.unique_name = name.as_str().unwrap_or("").to_string();
        }
        conn.stream.set_nonblocking(true)?;
        Ok(conn)
    }

    pub fn unique_name(&self) -> &str { &self.unique_name }
    pub fn as_raw_fd(&self) -> i32 { self.stream.as_raw_fd() }

    /// Send a method call. Returns the serial number.
    pub fn method_call(
        &mut self, dest: &str, path: &str, iface: &str, member: &str,
        sig: &str, args: &[u8],
    ) -> u32 {
        self.serial += 1;
        let serial = self.serial;
        let msg = encode_method_call(serial, dest, path, iface, member, sig, args);
        let _ = self.stream.write_all(&msg);
        serial
    }

    /// Request a well-known bus name. Returns true if granted.
    pub fn request_name(&mut self, name: &str) -> bool {
        // Switch to blocking before sending so write_all won't fail with WouldBlock
        self.stream.set_nonblocking(false).ok();
        let mut body = Vec::new();
        encode_string(&mut body, name);
        align_to(&mut body, 4);
        encode_u32(&mut body, 0x4); // DBUS_NAME_FLAG_DO_NOT_QUEUE
        self.serial += 1;
        let serial = self.serial;
        let msg = encode_method_call(
            serial, "org.freedesktop.DBus", "/org/freedesktop/DBus",
            "org.freedesktop.DBus", "RequestName", "su", &body,
        );
        if let Err(e) = self.stream.write_all(&msg) {
            self.stream.set_nonblocking(true).ok();
            eprintln!("[dbus] RequestName write failed: {e}");
            return false;
        }
        let ok = loop {
            match read_message(&mut self.stream) {
                Ok(msg) => {
                    if msg.reply_serial == serial {
                        if msg.is_error() {
                            eprintln!("[dbus] RequestName error reply");
                            break false;
                        }
                        let result = if msg.body.len() >= 4 {
                            u32::from_le_bytes([msg.body[0], msg.body[1], msg.body[2], msg.body[3]])
                        } else { 0 };
                        break result == 1;
                    }
                }
                Err(e) => {
                    eprintln!("[dbus] RequestName read error: {e}");
                    break false;
                }
            }
        };
        self.stream.set_nonblocking(true).ok();
        ok
    }

    /// Send a method return in reply to an incoming method call.
    pub fn send_reply(&mut self, reply_to_serial: u32, dest: &str, sig: &str, body: &[u8]) {
        self.serial += 1;
        let msg = encode_reply(self.serial, reply_to_serial, dest, sig, body);
        let _ = self.stream.write_all(&msg);
    }

    /// Send a D-Bus error reply (e.g. to reject an OBEX transfer).
    pub fn send_error(&mut self, reply_to_serial: u32, dest: &str, error_name: &str, message: &str) {
        self.serial += 1;
        let msg = encode_error(self.serial, reply_to_serial, dest, error_name, message);
        let _ = self.stream.write_all(&msg);
    }

    /// Emit a D-Bus signal.
    pub fn send_signal(&mut self, path: &str, iface: &str, member: &str, sig: &str, body: &[u8]) {
        self.serial += 1;
        let msg = encode_signal(self.serial, path, iface, member, sig, body);
        let _ = self.stream.write_all(&msg);
    }

    /// Send an AddMatch rule for signal subscription.
    pub fn add_match(&mut self, rule: &str) -> u32 {
        let mut body = Vec::new();
        encode_string(&mut body, rule);
        self.method_call(
            "org.freedesktop.DBus", "/org/freedesktop/DBus",
            "org.freedesktop.DBus", "AddMatch", "s", &body,
        )
    }

    /// Non-blocking read of one message. Returns None if no data available.
    pub fn try_read(&mut self) -> Option<Message> {
        match read_message(&mut self.stream) {
            Ok(msg) => Some(msg),
            Err(_) => None,
        }
    }

    /// Blocking read until a reply with the given serial arrives.
    pub fn read_reply(&mut self, serial: u32) -> io::Result<Message> {
        self.stream.set_nonblocking(false)?;
        loop {
            let msg = read_message(&mut self.stream)?;
            if msg.reply_serial == serial
                && (msg.msg_type == MSG_METHOD_RETURN || msg.msg_type == MSG_ERROR)
            {
                self.stream.set_nonblocking(true).ok();
                return Ok(msg);
            }
        }
    }
}

// ── SASL authentication ─────────────────────────────────────────────────────

fn sasl_auth(stream: &mut UnixStream) -> io::Result<()> {
    stream.write_all(b"\0")?;
    let uid = unsafe { libc::getuid() };
    let hex_uid = format!("{}", uid)
        .bytes()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    let auth_cmd = format!("AUTH EXTERNAL {}\r\n", hex_uid);
    stream.write_all(auth_cmd.as_bytes())?;
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf)?;
    let resp = std::str::from_utf8(&buf[..n]).unwrap_or("");
    if !resp.starts_with("OK") {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "SASL auth failed"));
    }
    stream.write_all(b"BEGIN\r\n")?;
    Ok(())
}

fn parse_bus_address(addr: &str) -> Option<String> {
    for part in addr.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("unix:") {
            for kv in rest.split(';') {
                if let Some(p) = kv.strip_prefix("path=") {
                    return Some(p.to_string());
                }
                if let Some(p) = kv.strip_prefix("abstract=") {
                    let mut path = String::from("\0");
                    path.push_str(p);
                    return Some(path);
                }
            }
        }
    }
    None
}

// ── Wire format encoding ────────────────────────────────────────────────────

pub fn align_to(buf: &mut Vec<u8>, n: usize) {
    while buf.len() % n != 0 { buf.push(0); }
}

pub fn encode_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn encode_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn encode_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn encode_f64(buf: &mut Vec<u8>, v: f64) {
    buf.extend_from_slice(&v.to_bits().to_le_bytes());
}

pub fn encode_string(buf: &mut Vec<u8>, s: &str) {
    align_to(buf, 4);
    encode_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

pub fn encode_signature(buf: &mut Vec<u8>, s: &str) {
    buf.push(s.len() as u8);
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

/// Encode a variant: signature + value.
pub fn encode_variant_string(buf: &mut Vec<u8>, s: &str) {
    encode_signature(buf, "s");
    encode_string(buf, s);
}

pub fn encode_variant_bool(buf: &mut Vec<u8>, v: bool) {
    encode_signature(buf, "b");
    align_to(buf, 4);
    encode_u32(buf, v as u32);
}

pub fn encode_variant_double(buf: &mut Vec<u8>, v: f64) {
    encode_signature(buf, "d");
    align_to(buf, 8);
    encode_f64(buf, v);
}

pub fn encode_variant_i64(buf: &mut Vec<u8>, v: i64) {
    encode_signature(buf, "x");
    align_to(buf, 8);
    encode_i64(buf, v);
}

/// Encode a dict entry {string: variant} into the buffer.
pub fn encode_dict_entry_sv(buf: &mut Vec<u8>, key: &str, encode_variant: impl FnOnce(&mut Vec<u8>)) {
    align_to(buf, 8);
    encode_string(buf, key);
    encode_variant(buf);
}

fn encode_header_field(buf: &mut Vec<u8>, code: u8, sig: &str, value: &str) {
    align_to(buf, 8);
    buf.push(code);
    encode_signature(buf, sig);
    match sig {
        "s" | "o" => encode_string(buf, value),
        "g" => encode_signature(buf, value),
        _ => {}
    }
}

fn encode_method_call(
    serial: u32, dest: &str, path: &str, iface: &str, member: &str,
    body_sig: &str, body: &[u8],
) -> Vec<u8> {
    let mut fields = Vec::new();
    encode_header_field(&mut fields, FIELD_PATH, "o", path);
    encode_header_field(&mut fields, FIELD_INTERFACE, "s", iface);
    encode_header_field(&mut fields, FIELD_MEMBER, "s", member);
    encode_header_field(&mut fields, FIELD_DESTINATION, "s", dest);
    if !body_sig.is_empty() {
        encode_header_field(&mut fields, FIELD_SIGNATURE, "g", body_sig);
    }

    let mut msg = Vec::with_capacity(128 + fields.len() + body.len());
    msg.push(b'l');
    msg.push(MSG_METHOD_CALL);
    msg.push(0);
    msg.push(1);
    encode_u32(&mut msg, body.len() as u32);
    encode_u32(&mut msg, serial);
    encode_u32(&mut msg, fields.len() as u32);
    msg.extend_from_slice(&fields);
    align_to(&mut msg, 8);
    msg.extend_from_slice(body);
    msg
}

fn encode_reply(
    serial: u32, reply_to: u32, dest: &str, body_sig: &str, body: &[u8],
) -> Vec<u8> {
    let mut fields = Vec::new();
    align_to(&mut fields, 8);
    fields.push(FIELD_REPLY_SERIAL);
    encode_signature(&mut fields, "u");
    align_to(&mut fields, 4);
    encode_u32(&mut fields, reply_to);
    encode_header_field(&mut fields, FIELD_DESTINATION, "s", dest);
    if !body_sig.is_empty() {
        encode_header_field(&mut fields, FIELD_SIGNATURE, "g", body_sig);
    }

    let mut msg = Vec::with_capacity(128 + fields.len() + body.len());
    msg.push(b'l');
    msg.push(MSG_METHOD_RETURN);
    msg.push(1);
    msg.push(1);
    encode_u32(&mut msg, body.len() as u32);
    encode_u32(&mut msg, serial);
    encode_u32(&mut msg, fields.len() as u32);
    msg.extend_from_slice(&fields);
    align_to(&mut msg, 8);
    msg.extend_from_slice(body);
    msg
}

fn encode_error(
    serial: u32, reply_to: u32, dest: &str, error_name: &str, message: &str,
) -> Vec<u8> {
    let mut fields = Vec::new();
    align_to(&mut fields, 8);
    fields.push(FIELD_REPLY_SERIAL);
    encode_signature(&mut fields, "u");
    align_to(&mut fields, 4);
    encode_u32(&mut fields, reply_to);
    // Error name — field code 4, type "s"
    align_to(&mut fields, 8);
    fields.push(4);
    encode_signature(&mut fields, "s");
    encode_string(&mut fields, error_name);
    encode_header_field(&mut fields, FIELD_DESTINATION, "s", dest);

    let mut body = Vec::new();
    let body_sig = if message.is_empty() { "" } else {
        encode_string(&mut body, message);
        "s"
    };
    if !body_sig.is_empty() {
        encode_header_field(&mut fields, FIELD_SIGNATURE, "g", body_sig);
    }

    let mut msg = Vec::with_capacity(128 + fields.len() + body.len());
    msg.push(b'l');
    msg.push(MSG_ERROR);
    msg.push(1);
    msg.push(1);
    encode_u32(&mut msg, body.len() as u32);
    encode_u32(&mut msg, serial);
    encode_u32(&mut msg, fields.len() as u32);
    msg.extend_from_slice(&fields);
    align_to(&mut msg, 8);
    msg.extend_from_slice(&body);
    msg
}

fn encode_signal(
    serial: u32, path: &str, iface: &str, member: &str, body_sig: &str, body: &[u8],
) -> Vec<u8> {
    let mut fields = Vec::new();
    encode_header_field(&mut fields, FIELD_PATH, "o", path);
    encode_header_field(&mut fields, FIELD_INTERFACE, "s", iface);
    encode_header_field(&mut fields, FIELD_MEMBER, "s", member);
    if !body_sig.is_empty() {
        encode_header_field(&mut fields, FIELD_SIGNATURE, "g", body_sig);
    }

    let mut msg = Vec::with_capacity(128 + fields.len() + body.len());
    msg.push(b'l');
    msg.push(MSG_SIGNAL);
    msg.push(1);
    msg.push(1);
    encode_u32(&mut msg, body.len() as u32);
    encode_u32(&mut msg, serial);
    encode_u32(&mut msg, fields.len() as u32);
    msg.extend_from_slice(&fields);
    align_to(&mut msg, 8);
    msg.extend_from_slice(body);
    msg
}

// ── Wire format decoding ────────────────────────────────────────────────────

fn read_exact(stream: &mut UnixStream, buf: &mut [u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < buf.len() {
        match stream.read(&mut buf[offset..]) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "D-Bus EOF")),
            Ok(n) => offset += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn read_message(stream: &mut UnixStream) -> io::Result<Message> {
    let mut hdr = [0u8; 16];
    read_exact(stream, &mut hdr)?;

    let msg_type = hdr[1];
    let body_len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    let serial = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
    let fields_len = u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]) as usize;

    let padded_fields_len = (fields_len + 7) & !7;
    let mut fields_buf = vec![0u8; padded_fields_len];
    read_exact(stream, &mut fields_buf)?;

    let mut reply_serial = 0u32;
    let mut sender = String::new();
    let mut path = String::new();
    let mut member = String::new();
    let mut interface = String::new();
    let mut signature = String::new();
    let mut destination = String::new();
    let mut r = BodyReader::new(&fields_buf, "");
    while r.pos < fields_len {
        r.align(8);
        if r.pos >= fields_len { break; }
        let code = r.read_byte();
        let vsig_len = r.read_byte() as usize;
        let vsig = r.read_bytes(vsig_len);
        r.read_byte();
        let vsig_str = String::from_utf8_lossy(&vsig).to_string();
        match (code, vsig_str.as_str()) {
            (FIELD_REPLY_SERIAL, "u") => reply_serial = r.read_u32(),
            (FIELD_SENDER, "s") => sender = r.read_string(),
            (FIELD_PATH, "o") => path = r.read_string(),
            (FIELD_MEMBER, "s") => member = r.read_string(),
            (FIELD_INTERFACE, "s") => interface = r.read_string(),
            (FIELD_DESTINATION, "s") => destination = r.read_string(),
            (FIELD_SIGNATURE, "g") => {
                let slen = r.read_byte() as usize;
                let sbytes = r.read_bytes(slen);
                r.read_byte();
                signature = String::from_utf8_lossy(&sbytes).to_string();
            }
            (_, "s" | "o") => { r.read_string(); }
            (_, "u") => { r.read_u32(); }
            (_, "g") => { let n = r.read_byte() as usize; r.read_bytes(n); r.read_byte(); }
            _ => break,
        }
    }

    let mut body = vec![0u8; body_len];
    if body_len > 0 {
        read_exact(stream, &mut body)?;
    }

    Ok(Message { msg_type, serial, reply_serial, sender, path, member, interface, signature, destination, body })
}

// ── Body reader ─────────────────────────────────────────────────────────────

pub struct BodyReader<'a> {
    data: &'a [u8],
    pub pos: usize,
    sig: &'a str,
}

impl<'a> BodyReader<'a> {
    pub fn new(data: &'a [u8], sig: &'a str) -> Self {
        Self { data, pos: 0, sig }
    }

    fn remaining(&self) -> usize { self.data.len().saturating_sub(self.pos) }

    pub fn align(&mut self, n: usize) {
        self.pos = (self.pos + n - 1) & !(n - 1);
    }

    pub fn read_byte(&mut self) -> u8 {
        if self.pos >= self.data.len() { return 0; }
        let v = self.data[self.pos];
        self.pos += 1;
        v
    }

    pub fn read_bytes(&mut self, n: usize) -> Vec<u8> {
        let end = (self.pos + n).min(self.data.len());
        let v = self.data[self.pos..end].to_vec();
        self.pos = end;
        v
    }

    pub fn read_u32(&mut self) -> u32 {
        self.align(4);
        if self.remaining() < 4 { return 0; }
        let v = u32::from_le_bytes([
            self.data[self.pos], self.data[self.pos+1],
            self.data[self.pos+2], self.data[self.pos+3],
        ]);
        self.pos += 4;
        v
    }

    pub fn read_i32(&mut self) -> i32 {
        self.align(4);
        if self.remaining() < 4 { return 0; }
        let v = i32::from_le_bytes([
            self.data[self.pos], self.data[self.pos+1],
            self.data[self.pos+2], self.data[self.pos+3],
        ]);
        self.pos += 4;
        v
    }

    pub fn read_string(&mut self) -> String {
        self.align(4);
        let len = self.read_u32() as usize;
        let end = (self.pos + len).min(self.data.len());
        let s = String::from_utf8_lossy(&self.data[self.pos..end]).to_string();
        self.pos = (end + 1).min(self.data.len());
        s
    }

    pub fn read_bool(&mut self) -> bool {
        self.read_u32() != 0
    }

    pub fn read_value(&mut self, sig: &str) -> Option<Value> {
        let c = sig.chars().next()?;
        Some(match c {
            'y' => Value::Uint32(self.read_byte() as u32),
            'b' => Value::Bool(self.read_bool()),
            'i' | 'n' => Value::Int32(self.read_i32()),
            'u' | 'q' => Value::Uint32(self.read_u32()),
            's' => Value::String(self.read_string()),
            'o' => Value::ObjectPath(self.read_string()),
            'v' => {
                let vsig_len = self.read_byte() as usize;
                let end = (self.pos + vsig_len).min(self.data.len());
                let vsig = String::from_utf8_lossy(&self.data[self.pos..end]).to_string();
                self.pos = (end + 1).min(self.data.len());
                self.read_value(&vsig)?
            }
            'a' => {
                let inner = &sig[1..];
                self.align(4);
                let array_len = self.read_u32() as usize;
                let array_end = self.pos + array_len;
                if inner.starts_with('{') {
                    let mut dict = HashMap::new();
                    self.align(8);
                    while self.pos < array_end {
                        self.align(8);
                        let key = self.read_string();
                        if let Some(val) = self.read_value("v") {
                            dict.insert(key, val);
                        }
                    }
                    self.pos = array_end;
                    return Some(Value::Dict(dict));
                }
                if inner == "s" {
                    let mut arr = Vec::new();
                    while self.pos < array_end {
                        arr.push(Value::String(self.read_string()));
                    }
                    return Some(Value::Array(arr));
                }
                if inner.starts_with("(iiay)") {
                    let mut arr = Vec::new();
                    while self.pos < array_end {
                        self.align(8);
                        let w = self.read_i32();
                        let h = self.read_i32();
                        self.align(4);
                        let byte_len = self.read_u32() as usize;
                        let bytes = self.read_bytes(byte_len);
                        arr.push(Value::Struct(vec![
                            Value::Int32(w), Value::Int32(h), Value::Bytes(bytes),
                        ]));
                    }
                    return Some(Value::Array(arr));
                }
                if inner == "v" {
                    let mut arr = Vec::new();
                    while self.pos < array_end {
                        if let Some(v) = self.read_value("v") {
                            arr.push(v);
                        } else { break; }
                    }
                    self.pos = array_end;
                    return Some(Value::Array(arr));
                }
                if inner.starts_with('(') {
                    let mut arr = Vec::new();
                    while self.pos < array_end {
                        if let Some(v) = self.read_value(inner) {
                            arr.push(v);
                        } else { break; }
                    }
                    self.pos = array_end;
                    return Some(Value::Array(arr));
                }
                self.pos = array_end;
                Value::Array(Vec::new())
            }
            '(' => {
                self.align(8);
                let inner = &sig[1..sig.len()-1];
                let mut fields = Vec::new();
                let mut i = 0;
                while i < inner.len() {
                    let (field_sig, consumed) = subsig_at(inner, i);
                    if let Some(v) = self.read_value(&field_sig) {
                        fields.push(v);
                    }
                    i += consumed;
                }
                Value::Struct(fields)
            }
            _ => return None,
        })
    }

    pub fn read_all(&mut self) -> Vec<Value> {
        let sig = self.sig.to_string();
        let mut values = Vec::new();
        let mut i = 0;
        while i < sig.len() {
            let (subsig, consumed) = subsig_at(&sig, i);
            if let Some(v) = self.read_value(&subsig) {
                values.push(v);
            }
            i += consumed;
        }
        values
    }
}

/// Extract a complete type signature starting at position `i`.
pub fn subsig_at(sig: &str, i: usize) -> (String, usize) {
    let bytes = sig.as_bytes();
    if i >= bytes.len() { return (String::new(), 1); }
    match bytes[i] {
        b'a' => {
            let (inner, inner_len) = subsig_at(sig, i + 1);
            (format!("a{}", inner), 1 + inner_len)
        }
        b'(' => {
            let mut depth = 1;
            let mut j = i + 1;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'(' { depth += 1; }
                if bytes[j] == b')' { depth -= 1; }
                j += 1;
            }
            (sig[i..j].to_string(), j - i)
        }
        b'{' => {
            let mut depth = 1;
            let mut j = i + 1;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'{' { depth += 1; }
                if bytes[j] == b'}' { depth -= 1; }
                j += 1;
            }
            (sig[i..j].to_string(), j - i)
        }
        _ => (String::from(bytes[i] as char), 1),
    }
}
