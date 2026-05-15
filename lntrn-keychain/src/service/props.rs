//! `org.freedesktop.DBus.Properties` — Get / GetAll / Set router.
//!
//! Each Secret Service object class exposes a different set of properties.
//! This module decodes the (interface, property) tuple from the call and
//! reads/writes against the right state slot.

use lntrn_dbus::{align_to, encode_string, encode_u32, BodyReader, Connection, Message, Value};

use super::collection as coll_iface;
use super::paths::{self, IFACE_COLLECTION, IFACE_ITEM, IFACE_SERVICE, ObjectKind};
use super::service_iface;
use super::state::ServiceState;
use super::wire::{
    encode_variant_dict_ss, encode_variant_object_paths, encode_variant_u64,
};
use crate::storage;
use lntrn_dbus::{encode_variant_bool, encode_variant_string};

/// Dispatch a method on the org.freedesktop.DBus.Properties interface.
/// Returns true if handled.
pub fn dispatch(conn: &mut Connection, msg: &Message, state: &mut ServiceState) -> bool {
    match msg.member.as_str() {
        "Get" => {
            let mut r = BodyReader::new(&msg.body, &msg.signature);
            let iface = r.read_string();
            let prop = r.read_string();
            handle_get(conn, msg, state, &iface, &prop);
            true
        }
        "GetAll" => {
            let mut r = BodyReader::new(&msg.body, &msg.signature);
            let iface = r.read_string();
            handle_get_all(conn, msg, state, &iface);
            true
        }
        "Set" => {
            let mut r = BodyReader::new(&msg.body, &msg.signature);
            let iface = r.read_string();
            let prop = r.read_string();
            let value = r.read_value("v");
            handle_set(conn, msg, state, &iface, &prop, value);
            true
        }
        _ => false,
    }
}

fn handle_get(
    conn: &mut Connection, msg: &Message, state: &ServiceState,
    iface: &str, prop: &str,
) {
    let mut body = Vec::new();
    let ok = encode_property(&mut body, state, &msg.path, iface, prop);
    if ok {
        conn.send_reply(msg.serial, &msg.sender, "v", &body);
    } else {
        conn.send_error(
            msg.serial, &msg.sender,
            "org.freedesktop.DBus.Error.InvalidArgs",
            &format!("No such property {iface}.{prop}"),
        );
    }
}

fn handle_get_all(
    conn: &mut Connection, msg: &Message, state: &ServiceState, iface: &str,
) {
    let mut body = Vec::new();
    encode_all_properties(&mut body, state, &msg.path, iface);
    conn.send_reply(msg.serial, &msg.sender, "a{sv}", &body);
}

fn handle_set(
    conn: &mut Connection, msg: &Message, state: &mut ServiceState,
    iface: &str, prop: &str, value: Option<Value>,
) {
    let value = match value {
        Some(v) => v,
        None => {
            conn.send_error(msg.serial, &msg.sender,
                "org.freedesktop.DBus.Error.InvalidArgs", "missing value");
            return;
        }
    };
    let ok = apply_set(state, &msg.path, iface, prop, value);
    if !ok {
        conn.send_error(msg.serial, &msg.sender,
            "org.freedesktop.DBus.Error.InvalidArgs",
            &format!("Cannot set {iface}.{prop}"));
        return;
    }
    conn.send_reply(msg.serial, &msg.sender, "", &[]);
    // Mirror the change as a signal on the touched object's collection.
    match paths::classify(&msg.path) {
        ObjectKind::Collection(id) => {
            let p = paths::collection_path(id);
            service_iface::emit_collection_changed(conn, &p);
        }
        ObjectKind::Item(c, i) => {
            let p = paths::item_path(c, i);
            coll_iface::emit_item_changed(conn, c, &p);
        }
        _ => {}
    }
}

// ── Property table ──────────────────────────────────────────────────────────

fn encode_property(
    out: &mut Vec<u8>, state: &ServiceState,
    path: &str, iface: &str, prop: &str,
) -> bool {
    match (paths::classify(path), iface, prop) {
        (ObjectKind::Service, IFACE_SERVICE, "Collections") => {
            let mut paths_v: Vec<String> = state.collections.keys()
                .map(|k| paths::collection_path(k)).collect();
            paths_v.sort();
            encode_variant_object_paths(out, &paths_v);
            true
        }
        (ObjectKind::Collection(id), IFACE_COLLECTION, "Items") => {
            let coll = match state.collections.get(id) { Some(c) => c, None => return false };
            let mut item_paths: Vec<String> = coll.items.keys()
                .map(|k| paths::item_path(id, k)).collect();
            item_paths.sort();
            encode_variant_object_paths(out, &item_paths);
            true
        }
        (ObjectKind::Collection(id), IFACE_COLLECTION, "Label") => {
            let coll = match state.collections.get(id) { Some(c) => c, None => return false };
            encode_variant_string(out, &coll.label);
            true
        }
        (ObjectKind::Collection(id), IFACE_COLLECTION, "Locked") => {
            let coll = match state.collections.get(id) { Some(c) => c, None => return false };
            encode_variant_bool(out, coll.is_locked());
            true
        }
        (ObjectKind::Collection(id), IFACE_COLLECTION, "Created") => {
            let coll = match state.collections.get(id) { Some(c) => c, None => return false };
            encode_variant_u64(out, coll.created);
            true
        }
        (ObjectKind::Collection(id), IFACE_COLLECTION, "Modified") => {
            let coll = match state.collections.get(id) { Some(c) => c, None => return false };
            encode_variant_u64(out, coll.modified);
            true
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Locked") => {
            let coll = match state.collections.get(c) { Some(c) => c, None => return false };
            encode_variant_bool(out, coll.is_locked() || !coll.items.contains_key(i));
            true
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Attributes") => {
            let it = match state.collections.get(c).and_then(|coll| coll.items.get(i)) {
                Some(it) => it, None => return false,
            };
            encode_variant_dict_ss(out, &it.attributes);
            true
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Label") => {
            let it = match state.collections.get(c).and_then(|coll| coll.items.get(i)) {
                Some(it) => it, None => return false,
            };
            encode_variant_string(out, &it.label);
            true
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Type") => {
            let it = match state.collections.get(c).and_then(|coll| coll.items.get(i)) {
                Some(it) => it, None => return false,
            };
            encode_variant_string(out, &it.content_type);
            true
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Created") => {
            let it = match state.collections.get(c).and_then(|coll| coll.items.get(i)) {
                Some(it) => it, None => return false,
            };
            encode_variant_u64(out, it.created);
            true
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Modified") => {
            let it = match state.collections.get(c).and_then(|coll| coll.items.get(i)) {
                Some(it) => it, None => return false,
            };
            encode_variant_u64(out, it.modified);
            true
        }
        _ => false,
    }
}

fn encode_all_properties(
    out: &mut Vec<u8>, state: &ServiceState, path: &str, iface: &str,
) {
    align_to(out, 4);
    let len_pos = out.len();
    encode_u32(out, 0);
    align_to(out, 8);
    let body_start = out.len();

    let props: &[&str] = match (paths::classify(path), iface) {
        (ObjectKind::Service, IFACE_SERVICE) => &["Collections"],
        (ObjectKind::Collection(_), IFACE_COLLECTION) =>
            &["Items", "Label", "Locked", "Created", "Modified"],
        (ObjectKind::Item(_, _), IFACE_ITEM) =>
            &["Locked", "Attributes", "Label", "Type", "Created", "Modified"],
        _ => &[],
    };

    for prop in props {
        align_to(out, 8);
        encode_string(out, prop);
        if !encode_property(out, state, path, iface, prop) {
            // Should not happen — but if it does, emit an empty string variant
            // so the dict entry is still well-formed.
            out.push(1); out.push(b's'); out.push(0);
            encode_string(out, "");
        }
    }

    let body_len = (out.len() - body_start) as u32;
    out[len_pos..len_pos + 4].copy_from_slice(&body_len.to_le_bytes());
}

fn apply_set(
    state: &mut ServiceState,
    path: &str, iface: &str, prop: &str, value: Value,
) -> bool {
    match (paths::classify(path), iface, prop) {
        (ObjectKind::Collection(id), IFACE_COLLECTION, "Label") => {
            let s = match value.as_str() { Some(s) => s.to_string(), None => return false };
            let key = match state.collections.get(id).and_then(|c| c.master_key.as_ref()) {
                Some(k) => clone_master_key(k),
                None => return false,
            };
            let coll = match state.collections.get_mut(id) { Some(c) => c, None => return false };
            coll.label = s;
            coll.modified = storage::unix_now();
            super::persistence::persist_collection(coll, &key).is_ok()
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Label") => {
            mutate_item(state, c, i, |it| {
                if let Some(s) = value.as_str() { it.label = s.to_string(); }
            })
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Type") => {
            mutate_item(state, c, i, |it| {
                if let Some(s) = value.as_str() { it.content_type = s.to_string(); }
            })
        }
        (ObjectKind::Item(c, i), IFACE_ITEM, "Attributes") => {
            let dict = match value {
                Value::Dict(d) => d
                    .into_iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                    .collect(),
                _ => return false,
            };
            mutate_item(state, c, i, |it| { it.attributes = dict; })
        }
        _ => false,
    }
}

fn mutate_item(
    state: &mut ServiceState, coll_id: &str, item_id: &str,
    f: impl FnOnce(&mut super::state::Item),
) -> bool {
    let key = match state.collections.get(coll_id).and_then(|c| c.master_key.as_ref()) {
        Some(k) => clone_master_key(k),
        None => return false,
    };
    let coll = match state.collections.get_mut(coll_id) { Some(c) => c, None => return false };
    let it = match coll.items.get_mut(item_id) { Some(it) => it, None => return false };
    f(it);
    it.modified = storage::unix_now();
    coll.modified = storage::unix_now();
    super::persistence::persist_collection(coll, &key).is_ok()
}

fn clone_master_key(k: &storage::crypto::MasterKey) -> storage::crypto::MasterKey {
    storage::crypto::MasterKey {
        key: k.key,
        salt: k.salt,
        params: k.params.clone(),
    }
}
