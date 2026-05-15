//! Object-path constants + helpers for the Secret Service tree.
//!
//! ```text
//! /org/freedesktop/secrets                              (Service)
//! /org/freedesktop/secrets/session/s<N>                 (Session)
//! /org/freedesktop/secrets/collection/<id>              (Collection)
//! /org/freedesktop/secrets/collection/<id>/<itemid>     (Item)
//! /org/freedesktop/secrets/aliases/<name>               (alias alias path)
//! /org/freedesktop/secrets/prompt/p<N>                  (Prompt)
//! ```

pub const SERVICE_PATH: &str = "/org/freedesktop/secrets";
pub const SESSION_PREFIX: &str = "/org/freedesktop/secrets/session/";
pub const COLLECTION_PREFIX: &str = "/org/freedesktop/secrets/collection/";
pub const ALIAS_PREFIX: &str = "/org/freedesktop/secrets/aliases/";
pub const PROMPT_PREFIX: &str = "/org/freedesktop/secrets/prompt/";

pub const IFACE_SERVICE: &str = "org.freedesktop.Secret.Service";
pub const IFACE_COLLECTION: &str = "org.freedesktop.Secret.Collection";
pub const IFACE_ITEM: &str = "org.freedesktop.Secret.Item";
pub const IFACE_SESSION: &str = "org.freedesktop.Secret.Session";
pub const IFACE_PROMPT: &str = "org.freedesktop.Secret.Prompt";

pub const IFACE_PROPS: &str = "org.freedesktop.DBus.Properties";
pub const IFACE_INTROSPECT: &str = "org.freedesktop.DBus.Introspectable";

pub fn session_path(id: u64) -> String {
    format!("{SESSION_PREFIX}s{id}")
}

pub fn collection_path(id: &str) -> String {
    format!("{COLLECTION_PREFIX}{id}")
}

pub fn item_path(coll: &str, item: &str) -> String {
    format!("{COLLECTION_PREFIX}{coll}/{item}")
}

pub fn prompt_path(id: u64) -> String {
    format!("{PROMPT_PREFIX}p{id}")
}

/// Parse a session path → numeric id. Returns None if it doesn't match.
pub fn parse_session(path: &str) -> Option<u64> {
    let s = path.strip_prefix(SESSION_PREFIX)?.strip_prefix('s')?;
    s.parse().ok()
}

/// Parse a prompt path → numeric id.
pub fn parse_prompt(path: &str) -> Option<u64> {
    let s = path.strip_prefix(PROMPT_PREFIX)?.strip_prefix('p')?;
    s.parse().ok()
}

/// Parse a collection path → collection id. None if it isn't a collection
/// path (or is an item inside a collection).
pub fn parse_collection(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(COLLECTION_PREFIX)?;
    if rest.contains('/') { return None; }
    if rest.is_empty() { return None; }
    Some(rest)
}

/// Parse an item path → (collection_id, item_id).
pub fn parse_item(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix(COLLECTION_PREFIX)?;
    let (coll, item) = rest.split_once('/')?;
    if coll.is_empty() || item.is_empty() { return None; }
    if item.contains('/') { return None; }
    Some((coll, item))
}

/// Parse an alias path → alias name.
pub fn parse_alias(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(ALIAS_PREFIX)?;
    if rest.is_empty() || rest.contains('/') { return None; }
    Some(rest)
}

/// Which kind of object does this path point at?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectKind<'a> {
    Service,
    Session(u64),
    Collection(&'a str),
    Item(&'a str, &'a str),
    Alias(&'a str),
    Prompt(u64),
    Unknown,
}

pub fn classify(path: &str) -> ObjectKind<'_> {
    if path == SERVICE_PATH { return ObjectKind::Service; }
    if let Some(id) = parse_session(path) { return ObjectKind::Session(id); }
    if let Some(id) = parse_prompt(path) { return ObjectKind::Prompt(id); }
    if let Some((c, i)) = parse_item(path) { return ObjectKind::Item(c, i); }
    if let Some(c) = parse_collection(path) { return ObjectKind::Collection(c); }
    if let Some(a) = parse_alias(path) { return ObjectKind::Alias(a); }
    ObjectKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_paths() {
        assert_eq!(classify("/org/freedesktop/secrets"), ObjectKind::Service);
        assert_eq!(classify("/org/freedesktop/secrets/session/s3"), ObjectKind::Session(3));
        assert_eq!(classify("/org/freedesktop/secrets/collection/login"), ObjectKind::Collection("login"));
        assert_eq!(classify("/org/freedesktop/secrets/collection/login/abc"), ObjectKind::Item("login", "abc"));
        assert_eq!(classify("/org/freedesktop/secrets/aliases/default"), ObjectKind::Alias("default"));
        assert_eq!(classify("/org/freedesktop/secrets/prompt/p7"), ObjectKind::Prompt(7));
        assert_eq!(classify("/unknown"), ObjectKind::Unknown);
    }
}
