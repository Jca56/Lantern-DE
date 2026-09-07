//! `org.freedesktop.Secret.Collection` method dispatch + signals.
//!
//! Each collection lives at `/org/freedesktop/secrets/collection/<id>`.

use lntrn_dbus::{encode_string, BodyReader, Connection, Message};

use super::paths::{self, IFACE_COLLECTION};
use super::session;
use super::state::{Item, PromptKind, ServiceState};
use super::wire::{encode_object_paths, read_dict_ss, read_dict_sv, read_secret_struct};
use crate::storage;
use crate::storage::crypto::MasterKey;

/// Dispatch a method on a Collection object. Returns true if handled.
pub fn dispatch(
    conn: &mut Connection,
    msg: &Message,
    state: &mut ServiceState,
    coll_id: &str,
) -> bool {
    if msg.interface != IFACE_COLLECTION {
        return false;
    }
    match msg.member.as_str() {
        "Delete" => {
            delete(conn, msg, state, coll_id);
            true
        }
        "SearchItems" => {
            search_items(conn, msg, state, coll_id);
            true
        }
        "CreateItem" => {
            create_item(conn, msg, state, coll_id);
            true
        }
        _ => false,
    }
}

fn delete(conn: &mut Connection, msg: &Message, state: &mut ServiceState, coll_id: &str) {
    let prompt_path = super::prompt::create(
        state,
        msg.sender.clone(),
        PromptKind::DeleteCollection {
            collection_id: coll_id.to_string(),
        },
    );
    let mut body = Vec::new();
    encode_string(&mut body, &prompt_path);
    conn.send_reply(msg.serial, &msg.sender, "o", &body);
}

fn search_items(conn: &mut Connection, msg: &Message, state: &ServiceState, coll_id: &str) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let attrs = read_dict_ss(&mut r);

    let mut out = Vec::new();
    if let Some(coll) = state.collections.get(coll_id) {
        if !coll.is_locked() {
            for (_, it) in &coll.items {
                if matches(&it.attributes, &attrs) {
                    out.push(paths::item_path(coll_id, &it.id));
                }
            }
        }
    }
    out.sort();

    let mut body = Vec::new();
    encode_object_paths(&mut body, &out);
    conn.send_reply(msg.serial, &msg.sender, "ao", &body);
}

fn create_item(conn: &mut Connection, msg: &Message, state: &mut ServiceState, coll_id: &str) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let props = read_dict_sv(&mut r);
    let (session_path, params, value, content_type) = read_secret_struct(&mut r);
    let replace = r.read_bool();

    let label = props
        .get("org.freedesktop.Secret.Item.Label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let attrs: std::collections::HashMap<String, String> = props
        .get("org.freedesktop.Secret.Item.Attributes")
        .and_then(|v| match v {
            lntrn_dbus::Value::Dict(d) => Some(
                d.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();

    let session_id = paths::parse_session(&session_path);
    let plaintext = match session_id
        .and_then(|sid| session::decrypt_for_session(state, sid, &params, &value))
    {
        Some(pt) => pt,
        None => {
            // Plain session, or unknown session — accept as cleartext.
            value
        }
    };

    let key: MasterKey = match state
        .collections
        .get(coll_id)
        .and_then(|c| c.master_key.as_ref())
    {
        Some(k) => clone_key(k),
        None => {
            conn.send_error(
                msg.serial,
                &msg.sender,
                "org.freedesktop.Secret.Error.IsLocked",
                "Collection is locked",
            );
            return;
        }
    };

    let coll = state.collections.get_mut(coll_id).unwrap();

    // Replace by exact attribute match (the spec's default lookup mode)
    if replace {
        let existing: Vec<String> = coll
            .items
            .iter()
            .filter(|(_, it)| it.attributes == attrs)
            .map(|(k, _)| k.clone())
            .collect();
        for id in existing {
            coll.items.remove(&id);
        }
    }

    let now = storage::unix_now();
    let id = state.next_item_id.to_string();
    state.next_item_id += 1;
    let item_id = format!("i{id}");
    let item = Item {
        id: item_id.clone(),
        label,
        attributes: attrs,
        content_type: if content_type.is_empty() {
            "text/plain".into()
        } else {
            content_type
        },
        secret: plaintext,
        created: now,
        modified: now,
    };
    let coll = state.collections.get_mut(coll_id).unwrap();
    coll.items.insert(item_id.clone(), item);
    coll.modified = now;
    if let Err(e) = super::persistence::persist_collection(coll, &key) {
        crate::log::error(&format!("persist after CreateItem failed: {e}"));
    }

    let item_path = paths::item_path(coll_id, &item_id);
    emit_item_created(conn, coll_id, &item_path);

    let mut body = Vec::new();
    encode_string(&mut body, &item_path);
    encode_string(&mut body, "/"); // prompt
    conn.send_reply(msg.serial, &msg.sender, "oo", &body);
}

fn matches(
    have: &std::collections::HashMap<String, String>,
    want: &std::collections::HashMap<String, String>,
) -> bool {
    for (k, v) in want {
        match have.get(k) {
            Some(hv) if hv == v => {}
            _ => return false,
        }
    }
    true
}

fn clone_key(k: &MasterKey) -> MasterKey {
    MasterKey {
        key: k.key,
        salt: k.salt,
        params: k.params.clone(),
    }
}

// ── Signals ─────────────────────────────────────────────────────────────────

pub fn emit_item_created(conn: &mut Connection, coll_id: &str, item_path: &str) {
    let mut body = Vec::new();
    encode_string(&mut body, item_path);
    let path = paths::collection_path(coll_id);
    conn.send_signal(&path, IFACE_COLLECTION, "ItemCreated", "o", &body);
}

pub fn emit_item_deleted(conn: &mut Connection, coll_id: &str, item_path: &str) {
    let mut body = Vec::new();
    encode_string(&mut body, item_path);
    let path = paths::collection_path(coll_id);
    conn.send_signal(&path, IFACE_COLLECTION, "ItemDeleted", "o", &body);
}

pub fn emit_item_changed(conn: &mut Connection, coll_id: &str, item_path: &str) {
    let mut body = Vec::new();
    encode_string(&mut body, item_path);
    let path = paths::collection_path(coll_id);
    conn.send_signal(&path, IFACE_COLLECTION, "ItemChanged", "o", &body);
}
