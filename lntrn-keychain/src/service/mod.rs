//! Top-level dispatcher for the Secret Service tree.
//!
//! Reads an incoming D-Bus method call and routes it to the right
//! interface module based on (path, interface, member).

use lntrn_dbus::{Connection, Message};

use crate::log;

pub mod collection;
pub mod dh;
pub mod introspect;
pub mod ipc;
pub mod item;
pub mod paths;
pub mod persistence;
pub mod prompt;
pub mod props;
pub mod service_iface;
pub mod session;
pub mod state;
pub mod wire;

use paths::{ObjectKind, IFACE_INTROSPECT, IFACE_PROPS};
use state::ServiceState;

/// Handle a single incoming D-Bus message.
pub fn handle(conn: &mut Connection, msg: &Message, state: &mut ServiceState) {
    if !msg.is_method_call() {
        return;
    }

    // Standard interfaces dispatch regardless of object kind.
    if msg.interface == IFACE_INTROSPECT && msg.member == "Introspect" {
        introspect::handle(conn, msg, state);
        return;
    }
    if msg.interface == IFACE_PROPS {
        if props::dispatch(conn, msg, state) {
            return;
        }
    }

    let handled = match paths::classify(&msg.path) {
        ObjectKind::Service => service_iface::dispatch(conn, msg, state),
        ObjectKind::Session(id) => session::dispatch(conn, msg, state, id),
        ObjectKind::Collection(id) => {
            let id = id.to_string();
            collection::dispatch(conn, msg, state, &id)
        }
        ObjectKind::Item(c, i) => {
            let c = c.to_string();
            let i = i.to_string();
            item::dispatch(conn, msg, state, &c, &i)
        }
        ObjectKind::Alias(name) => {
            // Aliases are aliases for a collection. Resolve and re-dispatch.
            match state.aliases.get(name).cloned() {
                Some(id) => collection::dispatch(conn, msg, state, &id),
                None => false,
            }
        }
        ObjectKind::Prompt(id) => prompt::dispatch(conn, msg, state, id),
        ObjectKind::Unknown => false,
    };

    if !handled {
        log::info(&format!(
            "unhandled method: path={} iface={} member={}",
            msg.path, msg.interface, msg.member,
        ));
        conn.send_error(
            msg.serial,
            &msg.sender,
            "org.freedesktop.DBus.Error.UnknownMethod",
            &format!("No such method {}.{}", msg.interface, msg.member),
        );
    }
}

/// Initialize state on boot — discover on-disk collections, set up the
/// default alias, and (if a passphrase is wired via the env-var fallback)
/// bootstrap a `login` collection so first-run write ops succeed.
pub fn init(state: &mut ServiceState) {
    persistence::discover_locked_collections(state);

    // Bootstrap "login" + eagerly unlock if env passphrase is set.
    if let Ok(pass) = std::env::var("LNTRN_KEYCHAIN_PASS") {
        if !state.collections.contains_key("login") {
            match crate::storage::create("login", "Login Keyring", &pass) {
                Ok(key) => {
                    let now = crate::storage::unix_now();
                    state.collections.insert(
                        "login".into(),
                        state::Collection {
                            id: "login".into(),
                            label: "Login Keyring".into(),
                            created: now,
                            modified: now,
                            items: Default::default(),
                            master_key: Some(key),
                        },
                    );
                    log::info("bootstrap: created Login Keyring");
                }
                Err(e) => log::error(&format!("bootstrap: could not create login: {e}")),
            }
        } else if let Some(coll) = state.collections.get_mut("login") {
            if coll.is_locked() {
                match crate::storage::unlock("login", &pass) {
                    Ok((dec, key)) => {
                        coll.label = dec.meta.label;
                        coll.created = dec.meta.created;
                        coll.modified = dec.meta.modified;
                        coll.items.clear();
                        for it in dec.items {
                            coll.items.insert(it.id.clone(), it.into());
                        }
                        coll.master_key = Some(key);
                        log::info("bootstrap: unlocked Login Keyring");
                    }
                    Err(e) => log::error(&format!("bootstrap: could not unlock login: {e}")),
                }
            }
        }
    }

    if state.aliases.get("default").is_none() {
        if state.collections.contains_key("login") {
            state.aliases.insert("default".into(), "login".into());
        } else if let Some(first) = state.collections.keys().next().cloned() {
            state.aliases.insert("default".into(), first);
        }
    }
}
