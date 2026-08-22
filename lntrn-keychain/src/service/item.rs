//! `org.freedesktop.Secret.Item` method dispatch.

use lntrn_dbus::{encode_string, BodyReader, Connection, Message};

use super::collection as coll_iface;
use super::paths::{self, IFACE_ITEM};
use super::session;
use super::state::{PromptKind, ServiceState};
use super::wire::{encode_secret_struct, read_secret_struct};
use crate::storage;
use crate::storage::crypto::MasterKey;

/// Dispatch a method on an Item object. Returns true if handled.
pub fn dispatch(
    conn: &mut Connection,
    msg: &Message,
    state: &mut ServiceState,
    coll_id: &str,
    item_id: &str,
) -> bool {
    if msg.interface != IFACE_ITEM {
        return false;
    }
    match msg.member.as_str() {
        "Delete" => {
            delete(conn, msg, state, coll_id, item_id);
            true
        }
        "GetSecret" => {
            get_secret(conn, msg, state, coll_id, item_id);
            true
        }
        "SetSecret" => {
            set_secret(conn, msg, state, coll_id, item_id);
            true
        }
        _ => false,
    }
}

fn delete(
    conn: &mut Connection,
    msg: &Message,
    state: &mut ServiceState,
    coll_id: &str,
    item_id: &str,
) {
    let prompt_path = super::prompt::create(
        state,
        msg.sender.clone(),
        PromptKind::DeleteItem {
            collection_id: coll_id.to_string(),
            item_id: item_id.to_string(),
        },
    );
    let mut body = Vec::new();
    encode_string(&mut body, &prompt_path);
    conn.send_reply(msg.serial, &msg.sender, "o", &body);
}

fn get_secret(
    conn: &mut Connection,
    msg: &Message,
    state: &ServiceState,
    coll_id: &str,
    item_id: &str,
) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let session_path = r.read_string();
    let session_id = paths::parse_session(&session_path);

    let coll = match state.collections.get(coll_id) {
        Some(c) => c,
        None => {
            conn.send_error(
                msg.serial,
                &msg.sender,
                "org.freedesktop.Secret.Error.NoSuchObject",
                "collection gone",
            );
            return;
        }
    };
    if coll.is_locked() {
        conn.send_error(
            msg.serial,
            &msg.sender,
            "org.freedesktop.Secret.Error.IsLocked",
            "Collection is locked",
        );
        return;
    }
    let it = match coll.items.get(item_id) {
        Some(it) => it,
        None => {
            conn.send_error(
                msg.serial,
                &msg.sender,
                "org.freedesktop.Secret.Error.NoSuchObject",
                "no such item",
            );
            return;
        }
    };

    let (params, value) =
        match session_id.and_then(|sid| session::encrypt_for_session(state, sid, &it.secret)) {
            Some(pair) => pair,
            None => (Vec::new(), it.secret.clone()),
        };
    let resolved_session = session_id
        .map(paths::session_path)
        .unwrap_or_else(|| session_path.clone());

    let mut body = Vec::new();
    encode_secret_struct(
        &mut body,
        &resolved_session,
        &params,
        &value,
        &it.content_type,
    );
    conn.send_reply(msg.serial, &msg.sender, "(oayays)", &body);
}

fn set_secret(
    conn: &mut Connection,
    msg: &Message,
    state: &mut ServiceState,
    coll_id: &str,
    item_id: &str,
) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let (session_path, params, value, content_type) = read_secret_struct(&mut r);

    let session_id = paths::parse_session(&session_path);
    let plaintext = match session_id
        .and_then(|sid| session::decrypt_for_session(state, sid, &params, &value))
    {
        Some(pt) => pt,
        None => value,
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

    let coll = match state.collections.get_mut(coll_id) {
        Some(c) => c,
        None => {
            conn.send_error(
                msg.serial,
                &msg.sender,
                "org.freedesktop.Secret.Error.NoSuchObject",
                "no such collection",
            );
            return;
        }
    };
    let it = match coll.items.get_mut(item_id) {
        Some(it) => it,
        None => {
            conn.send_error(
                msg.serial,
                &msg.sender,
                "org.freedesktop.Secret.Error.NoSuchObject",
                "no such item",
            );
            return;
        }
    };
    it.secret = plaintext;
    if !content_type.is_empty() {
        it.content_type = content_type;
    }
    it.modified = storage::unix_now();
    coll.modified = it.modified;

    if let Err(e) = super::persistence::persist_collection(coll, &key) {
        crate::log::error(&format!("persist after SetSecret failed: {e}"));
    }

    coll_iface::emit_item_changed(conn, coll_id, &paths::item_path(coll_id, item_id));
    conn.send_reply(msg.serial, &msg.sender, "", &[]);
}

fn clone_key(k: &MasterKey) -> MasterKey {
    MasterKey {
        key: k.key,
        salt: k.salt,
        params: k.params.clone(),
    }
}
