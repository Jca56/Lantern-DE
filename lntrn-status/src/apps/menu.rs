//! `com.canonical.dbusmenu`, the menu behind an app's tray icon: its
//! layout read into a tree of [`MenuEntry`], and the calls asked of it.
//! Rows it hides are left out, `_Open`'s mnemonic underscore goes, and the
//! separators left doubled or at an end by what's hidden go too.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_dbus::{Value, align_to, encode_i32, encode_signature, encode_string, encode_u32};

/// One row of an app's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    /// What a click on it is told back as.
    pub id: i32,
    pub label: String,
    pub separator: bool,
    /// Greyed: nothing happens when it's chosen.
    pub enabled: bool,
    pub toggle: Toggle,
    /// A submenu's rows.
    pub children: Vec<MenuEntry>,
}

/// A row that is on or off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Toggle {
    #[default]
    None,
    Check(bool),
    /// One of a group, of which one is on.
    Radio(bool),
}

/// The rows of `GetLayout`'s reply, `(u(ia{sv}av))`: its root's children.
pub fn read_layout(values: &[Value]) -> Option<Vec<MenuEntry>> {
    entry(values.get(1)?).map(|root| root.children)
}

/// One `(ia{sv}av)` node, and what's under it; `None` for one hidden.
fn entry(value: &Value) -> Option<MenuEntry> {
    let Value::Struct(fields) = value else { return None };
    let empty = HashMap::new();
    let props = fields.get(1).and_then(Value::as_dict).unwrap_or(&empty);
    let text = |k: &str| props.get(k).and_then(Value::as_str).unwrap_or_default();
    let flag = |k: &str| props.get(k).and_then(Value::as_bool).unwrap_or(true);
    if !flag("visible") {
        return None;
    }
    let on = props.get("toggle-state").and_then(Value::as_i32) == Some(1);
    let toggle = match text("toggle-type") {
        "checkmark" => Toggle::Check(on),
        "radio" => Toggle::Radio(on),
        _ => Toggle::None,
    };
    let children = fields.get(2).and_then(Value::as_array).map(|kids| tidy(kids.iter().map(unwrap).filter_map(entry).collect())).unwrap_or_default();
    Some(MenuEntry { id: fields.first().and_then(Value::as_i32).unwrap_or(0), label: unmnemonic(text("label")), separator: text("type") == "separator", enabled: flag("enabled"), toggle, children })
}

/// A child is a variant holding its node.
fn unwrap(value: &Value) -> &Value {
    match value {
        Value::Variant(inner) => inner,
        other => other,
    }
}

/// `entries` without a separator first, last, or right after another.
fn tidy(entries: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut kept: Vec<MenuEntry> = Vec::with_capacity(entries.len());
    for e in entries {
        if e.separator && kept.last().is_none_or(|last| last.separator) {
            continue;
        }
        kept.push(e);
    }
    if kept.last().is_some_and(|e| e.separator) {
        kept.pop();
    }
    kept
}

/// GTK's mnemonics gone: `_Open` reads `Open`, `__` an underscore.
fn unmnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '_' {
            out.push(c);
        } else if chars.peek() == Some(&'_') {
            chars.next();
            out.push('_');
        }
    }
    out
}

/// `GetLayout(parentId 0, recursionDepth -1, propertyNames [])`: all of
/// it, every property.
pub fn get_layout() -> Vec<u8> {
    let mut body = Vec::new();
    encode_i32(&mut body, 0);
    encode_i32(&mut body, -1);
    align_to(&mut body, 4);
    encode_u32(&mut body, 0);
    body
}

/// `AboutToShow(id 0)`: some apps fill their menu only when told.
pub fn about_to_show() -> Vec<u8> {
    let mut body = Vec::new();
    encode_i32(&mut body, 0);
    body
}

/// `Event(id, "clicked", variant 0, timestamp)`.
pub fn clicked(id: i32) -> Vec<u8> {
    let mut body = Vec::new();
    encode_i32(&mut body, id);
    encode_string(&mut body, "clicked");
    encode_signature(&mut body, "i");
    align_to(&mut body, 4);
    encode_i32(&mut body, 0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
    encode_u32(&mut body, now);
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i32, props: &[(&str, Value)], children: Vec<Value>) -> Value {
        let props = props.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect();
        Value::Struct(vec![Value::Int32(id), Value::Dict(props), Value::Array(children.into_iter().map(|c| Value::Variant(Box::new(c))).collect())])
    }

    fn s(v: &str) -> Value {
        Value::String(v.to_owned())
    }

    #[test]
    fn a_layout_reads_as_rows_hidden_ones_and_stray_separators_gone() {
        let sep = |id| node(id, &[("type", s("separator"))], vec![]);
        let root = node(
            0,
            &[],
            vec![
                sep(1),
                node(2, &[("label", s("_Show Steam"))], vec![]),
                node(3, &[("label", s("Secret")), ("visible", Value::Bool(false))], vec![]),
                sep(4),
                sep(5),
                node(6, &[("label", s("Do Not Disturb")), ("toggle-type", s("checkmark")), ("toggle-state", Value::Int32(1))], vec![]),
                node(7, &[("label", s("Friends")), ("children-display", s("submenu"))], vec![node(8, &[("label", s("Offline")), ("enabled", Value::Bool(false))], vec![])]),
                sep(9),
            ],
        );
        let rows = read_layout(&[Value::Uint32(3), root]).unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| if r.separator { "—" } else { r.label.as_str() }).collect();
        assert_eq!(labels, ["Show Steam", "—", "Do Not Disturb", "Friends"]);
        assert_eq!(rows[2].toggle, Toggle::Check(true));
        assert!(!rows[3].children[0].enabled && rows[3].children[0].label == "Offline");
        assert_eq!(unmnemonic("Save__As"), "Save_As");
    }

    #[test]
    fn a_click_is_told_with_its_id() {
        let body = clicked(42);
        let values = lntrn_dbus::BodyReader::new(&body, "isvu").read_all();
        assert_eq!((values[0].as_i32(), values[1].as_str()), (Some(42), Some("clicked")));
    }
}
