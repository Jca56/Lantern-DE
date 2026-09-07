//! `org.freedesktop.Secret.Service` method dispatch + signals.
//!
//! Service object lives at `/org/freedesktop/secrets`.

use lntrn_dbus::{encode_string, BodyReader, Connection, Message};

use super::paths::{self, IFACE_SERVICE};
use super::session;
use super::state::{PromptKind, ServiceState};
use super::wire::{
    encode_object_paths, encode_secret_struct, read_dict_ss, read_dict_sv, read_object_paths,
};

/// Dispatch a method on the Service interface. Returns true if handled.
pub fn dispatch(conn: &mut Connection, msg: &Message, state: &mut ServiceState) -> bool {
    if msg.interface != IFACE_SERVICE {
        return false;
    }
    match msg.member.as_str() {
        "OpenSession" => {
            session::open_session(conn, msg, state);
            true
        }
        "CreateCollection" => {
            create_collection(conn, msg, state);
            true
        }
        "SearchItems" => {
            search_items(conn, msg, state);
            true
        }
        "Unlock" => {
            unlock(conn, msg, state);
            true
        }
        "Lock" => {
            lock(conn, msg, state);
            true
        }
        "LockService" => {
            for coll in state.collections.values_mut() {
                coll.master_key = None;
            }
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
            true
        }
        "GetSecrets" => {
            get_secrets(conn, msg, state);
            true
        }
        "ReadAlias" => {
            read_alias(conn, msg, state);
            true
        }
        "SetAlias" => {
            set_alias(conn, msg, state);
            true
        }
        _ => false,
    }
}

// ── CreateCollection ────────────────────────────────────────────────────────

fn create_collection(conn: &mut Connection, msg: &Message, state: &mut ServiceState) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let props = read_dict_sv(&mut r);
    let alias = r.read_string();

    let label = props
        .get("org.freedesktop.Secret.Collection.Label")
        .and_then(|v| v.as_str())
        .unwrap_or("Keyring")
        .to_string();

    let id = slugify(&label, &state.collections);
    let prompt_path = super::prompt::create(
        state,
        msg.sender.clone(),
        PromptKind::CreateCollection {
            id,
            label,
            alias: opt(alias),
        },
    );

    // (collection, prompt) — return empty path for collection (prompt will
    // produce it). Per spec: when prompt is needed, collection is "/".
    let mut body = Vec::new();
    encode_string(&mut body, "/");
    encode_string(&mut body, &prompt_path);
    conn.send_reply(msg.serial, &msg.sender, "oo", &body);
}

// ── SearchItems ─────────────────────────────────────────────────────────────

fn search_items(conn: &mut Connection, msg: &Message, state: &ServiceState) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let attrs = read_dict_ss(&mut r);

    let mut unlocked = Vec::new();
    let mut locked = Vec::new();
    for (id, coll) in &state.collections {
        if coll.is_locked() {
            // We can't search a locked collection; report the collection path
            // itself so the client can call Unlock on it. Spec says: "the
            // service may include locked collections it knows about". libsecret
            // ignores entries here; secret-tool re-runs after unlock.
            locked.push(paths::collection_path(id));
            continue;
        }
        for (_, it) in &coll.items {
            if matches_attrs(&it.attributes, &attrs) {
                unlocked.push(paths::item_path(id, &it.id));
            }
        }
    }
    unlocked.sort();
    locked.sort();

    let mut body = Vec::new();
    encode_object_paths(&mut body, &unlocked);
    encode_object_paths(&mut body, &locked);
    conn.send_reply(msg.serial, &msg.sender, "aoao", &body);
}

fn matches_attrs(
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

// ── Unlock / Lock ───────────────────────────────────────────────────────────

fn unlock(conn: &mut Connection, msg: &Message, state: &mut ServiceState) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let objects = read_object_paths(&mut r);

    let mut already_unlocked = Vec::new();
    let mut need_unlock = Vec::new();
    for o in &objects {
        // Spec allows item paths — fold them up to their collection.
        let coll_id = match paths::classify(o) {
            super::paths::ObjectKind::Collection(id) => id.to_string(),
            super::paths::ObjectKind::Item(c, _) => c.to_string(),
            super::paths::ObjectKind::Alias(name) => match state.aliases.get(name) {
                Some(id) => id.clone(),
                None => continue,
            },
            _ => continue,
        };
        match state.collections.get(&coll_id) {
            Some(c) if !c.is_locked() => already_unlocked.push(paths::collection_path(&coll_id)),
            Some(_) => need_unlock.push(coll_id),
            None => {}
        }
    }
    need_unlock.sort();
    need_unlock.dedup();

    let prompt_path = if need_unlock.is_empty() {
        "/".to_string()
    } else {
        super::prompt::create(
            state,
            msg.sender.clone(),
            PromptKind::Unlock {
                collection_ids: need_unlock,
            },
        )
    };

    let mut body = Vec::new();
    encode_object_paths(&mut body, &already_unlocked);
    encode_string(&mut body, &prompt_path);
    conn.send_reply(msg.serial, &msg.sender, "aoo", &body);
}

fn lock(conn: &mut Connection, msg: &Message, state: &mut ServiceState) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let objects = read_object_paths(&mut r);

    let mut to_lock = Vec::new();
    for o in &objects {
        let coll_id = match paths::classify(o) {
            super::paths::ObjectKind::Collection(id) => id.to_string(),
            super::paths::ObjectKind::Item(c, _) => c.to_string(),
            super::paths::ObjectKind::Alias(name) => match state.aliases.get(name) {
                Some(id) => id.clone(),
                None => continue,
            },
            _ => continue,
        };
        if state.collections.contains_key(&coll_id) {
            to_lock.push(coll_id);
        }
    }
    to_lock.sort();
    to_lock.dedup();

    // Lock immediately (no prompt) — gnome-keyring behavior.
    let mut locked = Vec::new();
    for id in &to_lock {
        if let Some(c) = state.collections.get_mut(id) {
            c.master_key = None;
            locked.push(paths::collection_path(id));
        }
    }

    let mut body = Vec::new();
    encode_object_paths(&mut body, &locked);
    encode_string(&mut body, "/");
    conn.send_reply(msg.serial, &msg.sender, "aoo", &body);
}

// ── GetSecrets ──────────────────────────────────────────────────────────────

fn get_secrets(conn: &mut Connection, msg: &Message, state: &ServiceState) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let items = read_object_paths(&mut r);
    let session_path = r.read_string();
    let session_id = paths::parse_session(&session_path);

    let mut body = Vec::new();
    encode_secrets_dict(&mut body, state, &items, session_id);
    conn.send_reply(msg.serial, &msg.sender, "a{o(oayays)}", &body);
}

fn encode_secrets_dict(
    out: &mut Vec<u8>,
    state: &ServiceState,
    items: &[String],
    session_id: Option<u64>,
) {
    use lntrn_dbus::{align_to, encode_u32};
    align_to(out, 4);
    let len_pos = out.len();
    encode_u32(out, 0);
    align_to(out, 8);
    let body_start = out.len();
    for path in items {
        let (coll_id, item_id) = match paths::parse_item(path) {
            Some(x) => x,
            None => continue,
        };
        let coll = match state.collections.get(coll_id) {
            Some(c) => c,
            None => continue,
        };
        if coll.is_locked() {
            continue;
        }
        let it = match coll.items.get(item_id) {
            Some(it) => it,
            None => continue,
        };
        let (params, value) =
            match session_id.and_then(|sid| session::encrypt_for_session(state, sid, &it.secret)) {
                Some(pair) => pair,
                None => (Vec::new(), it.secret.clone()),
            };
        align_to(out, 8);
        encode_string(out, path);
        encode_secret_struct(
            out,
            &session_id
                .map(paths::session_path)
                .unwrap_or_else(|| "/".into()),
            &params,
            &value,
            &it.content_type,
        );
    }
    let body_len = (out.len() - body_start) as u32;
    out[len_pos..len_pos + 4].copy_from_slice(&body_len.to_le_bytes());
}

// ── Aliases ─────────────────────────────────────────────────────────────────

fn read_alias(conn: &mut Connection, msg: &Message, state: &ServiceState) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let name = r.read_string();
    let path = state
        .aliases
        .get(&name)
        .map(|id| paths::collection_path(id))
        .unwrap_or_else(|| "/".into());
    let mut body = Vec::new();
    encode_string(&mut body, &path);
    conn.send_reply(msg.serial, &msg.sender, "o", &body);
}

fn set_alias(conn: &mut Connection, msg: &Message, state: &mut ServiceState) {
    let mut r = BodyReader::new(&msg.body, &msg.signature);
    let name = r.read_string();
    let path = r.read_string();
    if path == "/" {
        state.aliases.remove(&name);
    } else if let super::paths::ObjectKind::Collection(id) = paths::classify(&path) {
        state.aliases.insert(name, id.to_string());
    } else {
        conn.send_error(
            msg.serial,
            &msg.sender,
            "org.freedesktop.DBus.Error.InvalidArgs",
            "not a collection path",
        );
        return;
    }
    conn.send_reply(msg.serial, &msg.sender, "", &[]);
}

// ── Signals ─────────────────────────────────────────────────────────────────

pub fn emit_collection_created(conn: &mut Connection, coll_path: &str) {
    let mut body = Vec::new();
    encode_string(&mut body, coll_path);
    conn.send_signal(
        paths::SERVICE_PATH,
        IFACE_SERVICE,
        "CollectionCreated",
        "o",
        &body,
    );
}

pub fn emit_collection_deleted(conn: &mut Connection, coll_path: &str) {
    let mut body = Vec::new();
    encode_string(&mut body, coll_path);
    conn.send_signal(
        paths::SERVICE_PATH,
        IFACE_SERVICE,
        "CollectionDeleted",
        "o",
        &body,
    );
}

pub fn emit_collection_changed(conn: &mut Connection, coll_path: &str) {
    let mut body = Vec::new();
    encode_string(&mut body, coll_path);
    conn.send_signal(
        paths::SERVICE_PATH,
        IFACE_SERVICE,
        "CollectionChanged",
        "o",
        &body,
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn slugify(
    label: &str,
    existing: &std::collections::HashMap<String, super::state::Collection>,
) -> String {
    let mut base: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if base.is_empty() {
        base = "keyring".into();
    }
    if !existing.contains_key(&base) {
        return base;
    }
    for n in 2..u32::MAX {
        let s = format!("{base}_{n}");
        if !existing.contains_key(&s) {
            return s;
        }
    }
    base
}

fn opt(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
