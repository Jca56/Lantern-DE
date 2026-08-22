//! D-Bus wire encoding/decoding helpers specific to Secret Service signatures.
//!
//! Builds on the primitive helpers from `lntrn_dbus`. Adds:
//! - `ao`  array of object paths
//! - `a{ss}` dict<string,string>
//! - `a{sv}` dict<string,variant>
//! - `(oayays)` the Secret struct
//! - `ay`  byte array

use std::collections::HashMap;

use lntrn_dbus::{align_to, encode_string, encode_u32, BodyReader, Value};

// ── Encoders ────────────────────────────────────────────────────────────────

/// Encode an array of object paths (`ao`).
pub fn encode_object_paths(buf: &mut Vec<u8>, paths: &[String]) {
    align_to(buf, 4);
    let len_pos = buf.len();
    encode_u32(buf, 0); // placeholder
    align_to(buf, 4);
    let body_start = buf.len();
    for p in paths {
        encode_string(buf, p);
    }
    let body_len = buf.len() - body_start;
    let len_bytes = (body_len as u32).to_le_bytes();
    buf[len_pos..len_pos + 4].copy_from_slice(&len_bytes);
}

/// Encode a variant containing an `ao` array.
pub fn encode_variant_object_paths(buf: &mut Vec<u8>, paths: &[String]) {
    buf.push(2);
    buf.extend_from_slice(b"ao");
    buf.push(0);
    encode_object_paths(buf, paths);
}

/// Encode a variant containing an `a{ss}` dict.
pub fn encode_variant_dict_ss(buf: &mut Vec<u8>, map: &HashMap<String, String>) {
    buf.push(5);
    buf.extend_from_slice(b"a{ss}");
    buf.push(0);
    encode_dict_ss(buf, map);
}

/// Encode an `a{ss}` dict body.
pub fn encode_dict_ss(buf: &mut Vec<u8>, map: &HashMap<String, String>) {
    align_to(buf, 4);
    let len_pos = buf.len();
    encode_u32(buf, 0);
    align_to(buf, 8);
    let body_start = buf.len();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        align_to(buf, 8);
        encode_string(buf, k);
        encode_string(buf, &map[k]);
    }
    let body_len = (buf.len() - body_start) as u32;
    buf[len_pos..len_pos + 4].copy_from_slice(&body_len.to_le_bytes());
}

/// Encode an `ay` byte array.
pub fn encode_byte_array(buf: &mut Vec<u8>, bytes: &[u8]) {
    align_to(buf, 4);
    encode_u32(buf, bytes.len() as u32);
    buf.extend_from_slice(bytes);
}

/// Encode the Secret struct `(oayays)` = (session_path, parameters, value, content_type).
pub fn encode_secret_struct(
    buf: &mut Vec<u8>,
    session: &str,
    parameters: &[u8],
    value: &[u8],
    content_type: &str,
) {
    align_to(buf, 8);
    encode_string(buf, session);
    encode_byte_array(buf, parameters);
    encode_byte_array(buf, value);
    encode_string(buf, content_type);
}

/// Variant containing a `t` (uint64). Wire layout: sig "t"\0 + align8 + le64.
pub fn encode_variant_u64(buf: &mut Vec<u8>, v: u64) {
    buf.push(1);
    buf.push(b't');
    buf.push(0);
    align_to(buf, 8);
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Variant containing an `o` (object path). Same wire shape as a string variant
/// but the signature byte must be `o`, not `s` — some D-Bus clients
/// (notably libsecret) reject the wrong tag.
pub fn encode_variant_object_path(buf: &mut Vec<u8>, path: &str) {
    buf.push(1);
    buf.push(b'o');
    buf.push(0);
    encode_string(buf, path);
}

// ── Decoders ────────────────────────────────────────────────────────────────

/// Read an `a{ss}` dict<string,string> at the current cursor position.
pub fn read_dict_ss(r: &mut BodyReader<'_>) -> HashMap<String, String> {
    r.align(4);
    let array_len = r.read_u32() as usize;
    r.align(8); // dict entry alignment
    let end = r.pos + array_len;
    let mut out = HashMap::new();
    while r.pos < end {
        r.align(8);
        if r.pos >= end {
            break;
        }
        let k = r.read_string();
        if r.pos >= end {
            break;
        }
        let v = r.read_string();
        out.insert(k, v);
    }
    r.pos = end;
    out
}

/// Read an `ao` array of object paths.
pub fn read_object_paths(r: &mut BodyReader<'_>) -> Vec<String> {
    r.align(4);
    let array_len = r.read_u32() as usize;
    r.align(4); // object-path alignment
    let end = r.pos + array_len;
    let mut out = Vec::new();
    while r.pos < end {
        out.push(r.read_string());
    }
    r.pos = end;
    out
}

/// Read an `ay` byte array.
pub fn read_byte_array(r: &mut BodyReader<'_>) -> Vec<u8> {
    r.align(4);
    let n = r.read_u32() as usize;
    // byte alignment is 1 — no extra padding.
    r.read_bytes(n)
}

/// Read a Secret struct `(oayays)` → (session_path, parameters, value, content_type).
pub fn read_secret_struct(r: &mut BodyReader<'_>) -> (String, Vec<u8>, Vec<u8>, String) {
    r.align(8);
    let session = r.read_string();
    let parameters = read_byte_array(r);
    let value = read_byte_array(r);
    let content_type = r.read_string();
    (session, parameters, value, content_type)
}

/// Read an `a{sv}` dict<string,variant>. Returns property name → Value.
pub fn read_dict_sv(r: &mut BodyReader<'_>) -> HashMap<String, Value> {
    r.align(4);
    let array_len = r.read_u32() as usize;
    r.align(8); // dict entry alignment
    let end = r.pos + array_len;
    let mut out = HashMap::new();
    while r.pos < end {
        r.align(8);
        if r.pos >= end {
            break;
        }
        let k = r.read_string();
        if r.pos >= end {
            break;
        }
        if let Some(v) = r.read_value("v") {
            out.insert(k, v);
        }
    }
    r.pos = end;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ao_roundtrip() {
        let mut buf = Vec::new();
        encode_object_paths(&mut buf, &["/a".into(), "/b/c".into()]);
        let mut r = BodyReader::new(&buf, "");
        let got = read_object_paths(&mut r);
        assert_eq!(got, vec!["/a".to_string(), "/b/c".to_string()]);
    }

    #[test]
    fn dict_ss_roundtrip() {
        let mut m = HashMap::new();
        m.insert("host".to_string(), "github.com".to_string());
        m.insert("user".to_string(), "alva".to_string());
        let mut buf = Vec::new();
        encode_dict_ss(&mut buf, &m);
        let mut r = BodyReader::new(&buf, "");
        let got = read_dict_ss(&mut r);
        assert_eq!(got, m);
    }

    #[test]
    fn secret_struct_roundtrip() {
        let mut buf = Vec::new();
        encode_secret_struct(
            &mut buf,
            "/session/s1",
            &[1, 2, 3],
            &[4, 5, 6],
            "text/plain",
        );
        let mut r = BodyReader::new(&buf, "");
        let (sess, params, val, ct) = read_secret_struct(&mut r);
        assert_eq!(sess, "/session/s1");
        assert_eq!(params, vec![1, 2, 3]);
        assert_eq!(val, vec![4, 5, 6]);
        assert_eq!(ct, "text/plain");
    }
}
