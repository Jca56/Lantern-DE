//! Prompt interface — `org.freedesktop.Secret.Prompt`.
//!
//! When the Service needs user interaction (unlock, lock, delete, create-
//! collection) it allocates a Prompt object and returns its path. The
//! client calls `Prompt.Prompt(window_id)` to start the interaction; we
//! contact lntrn-command-center, wait for a reply, then emit `Completed`
//! signal with the result variant.

use lntrn_dbus::{align_to, encode_string, Connection, Message};

use super::collection as coll_iface;
use super::ipc::{self, PromptResult};
use super::paths::{self, IFACE_PROMPT};
use super::service_iface;
use super::state::{Prompt, PromptKind, ServiceState};
use super::wire::{encode_object_paths, encode_variant_object_path};
use crate::log;
use crate::storage;

/// Allocate a new prompt entry and return its path. `_peer` is reserved for
/// per-sender access control checks once we wire SO_PEERCRED.
pub fn create(state: &mut ServiceState, _peer: String, kind: PromptKind) -> String {
    let id = state.allocate_prompt_id();
    state.prompts.insert(id, Prompt { kind, completed: false });
    paths::prompt_path(id)
}

/// Dispatch a method on a Prompt object. Returns true if handled.
pub fn dispatch(
    conn: &mut Connection,
    msg: &Message,
    state: &mut ServiceState,
    prompt_id: u64,
) -> bool {
    if msg.interface != IFACE_PROMPT { return false; }
    match msg.member.as_str() {
        "Prompt" => {
            // Body is `s` (window_id) but we ignore it.
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
            execute(conn, state, prompt_id);
            true
        }
        "Dismiss" => {
            if let Some(p) = state.prompts.get_mut(&prompt_id) {
                if !p.completed {
                    p.completed = true;
                    let path = paths::prompt_path(prompt_id);
                    emit_dismissed(conn, &path);
                }
            }
            state.prompts.remove(&prompt_id);
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
            true
        }
        _ => false,
    }
}

/// Run the prompt's interaction synchronously. Emits `Completed` and removes
/// the prompt from state.
fn execute(conn: &mut Connection, state: &mut ServiceState, prompt_id: u64) {
    let prompt = match state.prompts.remove(&prompt_id) {
        Some(p) => p,
        None => return,
    };
    let path = paths::prompt_path(prompt_id);

    match prompt.kind {
        PromptKind::Unlock { collection_ids } => {
            let unlocked = unlock_collections(state, &collection_ids);
            if unlocked.is_empty() {
                emit_dismissed(conn, &path);
            } else {
                let mut body = Vec::new();
                emit_bool(&mut body, false);
                emit_variant_ao(&mut body, &unlocked);
                send_signal(conn, &path, &body);
            }
        }
        PromptKind::CreateCollection { id, label, alias } => {
            match create_collection(state, &id, &label, alias.as_deref()) {
                Some(coll_path) => {
                    service_iface::emit_collection_created(conn, &coll_path);
                    let mut body = Vec::new();
                    emit_bool(&mut body, false);
                    emit_variant_o(&mut body, &coll_path);
                    send_signal(conn, &path, &body);
                }
                None => emit_dismissed(conn, &path),
            }
        }
        PromptKind::DeleteCollection { collection_id } => {
            let coll_path = paths::collection_path(&collection_id);
            match delete_collection(state, &collection_id) {
                true => {
                    service_iface::emit_collection_deleted(conn, &coll_path);
                    let mut body = Vec::new();
                    emit_bool(&mut body, false);
                    emit_variant_string(&mut body, "");
                    send_signal(conn, &path, &body);
                }
                false => emit_dismissed(conn, &path),
            }
        }
        PromptKind::DeleteItem { collection_id, item_id } => {
            let item_path = paths::item_path(&collection_id, &item_id);
            match delete_item(state, &collection_id, &item_id) {
                true => {
                    coll_iface::emit_item_deleted(conn, &collection_id, &item_path);
                    let mut body = Vec::new();
                    emit_bool(&mut body, false);
                    emit_variant_string(&mut body, "");
                    send_signal(conn, &path, &body);
                }
                false => emit_dismissed(conn, &path),
            }
        }
    }
}

/// Try to unlock the listed collections. Returns the list of *paths* that
/// are now unlocked.
fn unlock_collections(state: &mut ServiceState, ids: &[String]) -> Vec<String> {
    let mut unlocked = Vec::new();
    for id in ids {
        let label = state.collections.get(id).map(|c| c.label.clone())
            .unwrap_or_else(|| id.clone());
        let already_unlocked = state.collections.get(id)
            .map(|c| !c.is_locked())
            .unwrap_or(false);
        if already_unlocked {
            unlocked.push(paths::collection_path(id));
            continue;
        }
        match ipc::request_passphrase(&format!("Unlock keyring: {label}")) {
            PromptResult::Passphrase(pass) => {
                match storage::unlock(id, &pass) {
                    Ok((coll, key)) => {
                        if let Some(slot) = state.collections.get_mut(id) {
                            slot.label = coll.meta.label;
                            slot.modified = coll.meta.modified;
                            slot.items.clear();
                            for it in coll.items {
                                slot.items.insert(it.id.clone(), it.into());
                            }
                            slot.master_key = Some(key);
                            unlocked.push(paths::collection_path(id));
                        }
                    }
                    Err(storage::Error::BadPassphrase) => {
                        log::info(&format!("unlock: bad passphrase for {id}"));
                    }
                    Err(e) => log::error(&format!("unlock: storage error for {id}: {e}")),
                }
            }
            PromptResult::Dismissed => {
                log::info(&format!("unlock: user dismissed prompt for {id}"));
            }
        }
    }
    unlocked
}

fn create_collection(
    state: &mut ServiceState,
    id: &str,
    label: &str,
    alias: Option<&str>,
) -> Option<String> {
    if state.collections.contains_key(id) {
        return None;
    }
    let pass = match ipc::request_passphrase(&format!("Create keyring: {label}")) {
        PromptResult::Passphrase(p) => p,
        PromptResult::Dismissed => return None,
    };
    let key = storage::create(id, label, &pass).ok()?;
    let now = storage::unix_now();
    state.collections.insert(id.to_string(), super::state::Collection {
        id: id.to_string(),
        label: label.to_string(),
        created: now,
        modified: now,
        items: Default::default(),
        master_key: Some(key),
    });
    if let Some(a) = alias {
        state.aliases.insert(a.to_string(), id.to_string());
    }
    Some(paths::collection_path(id))
}

fn delete_collection(state: &mut ServiceState, id: &str) -> bool {
    let path = storage::collection_path(id);
    if path.exists() {
        if std::fs::remove_file(&path).is_err() { return false; }
    }
    state.collections.remove(id);
    state.aliases.retain(|_, v| v != id);
    true
}

fn delete_item(state: &mut ServiceState, coll_id: &str, item_id: &str) -> bool {
    let key = match state.collections.get(coll_id).and_then(|c| c.master_key.as_ref()) {
        Some(k) => clone_master_key(k),
        None => return false,
    };
    let coll = match state.collections.get_mut(coll_id) {
        Some(c) => c,
        None => return false,
    };
    coll.items.remove(item_id);
    coll.modified = storage::unix_now();
    super::persistence::persist_collection(coll, &key).is_ok()
}

fn clone_master_key(k: &crate::storage::crypto::MasterKey) -> crate::storage::crypto::MasterKey {
    crate::storage::crypto::MasterKey {
        key: k.key,
        salt: k.salt,
        params: k.params.clone(),
    }
}

// ── Signal emission helpers ─────────────────────────────────────────────────

fn emit_dismissed(conn: &mut Connection, path: &str) {
    let mut body = Vec::new();
    emit_bool(&mut body, true);
    emit_variant_string(&mut body, "");
    send_signal(conn, path, &body);
}

fn emit_bool(buf: &mut Vec<u8>, v: bool) {
    align_to(buf, 4);
    buf.extend_from_slice(&(v as u32).to_le_bytes());
}

fn emit_variant_string(buf: &mut Vec<u8>, s: &str) {
    buf.push(1);
    buf.push(b's');
    buf.push(0);
    encode_string(buf, s);
}

fn emit_variant_o(buf: &mut Vec<u8>, p: &str) {
    encode_variant_object_path(buf, p);
}

fn emit_variant_ao(buf: &mut Vec<u8>, paths: &[String]) {
    buf.push(2);
    buf.extend_from_slice(b"ao");
    buf.push(0);
    encode_object_paths(buf, paths);
}

fn send_signal(conn: &mut Connection, path: &str, body: &[u8]) {
    conn.send_signal(path, IFACE_PROMPT, "Completed", "bv", body);
}
