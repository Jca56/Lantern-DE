//! `com.canonical.dbusmenu` — the menu protocol behind tray right-clicks.
//!
//! Parses `GetLayout` replies into a small item tree, encodes the calls
//! we make (`GetLayout`, `Event`), and flattens the tree into the
//! Command Center's own [`MenuItem`] list so the existing context-menu
//! widget draws it.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_dbus::{self as dbus, Value};

use crate::launcher::context_menu::{MenuAction, MenuItem};

#[derive(Debug, Clone)]
pub struct DbusmenuItem {
    pub id: i32,
    pub label: String,
    /// `"standard"` (default) or `"separator"`.
    pub item_type: String,
    pub enabled: bool,
    pub visible: bool,
    /// `"checkmark"` / `"radio"` when the item toggles; empty otherwise.
    pub toggle_type: String,
    /// 1 = checked, 0 = unchecked, -1 = indeterminate / not a toggle.
    pub toggle_state: i32,
    pub children: Vec<DbusmenuItem>,
}

/// Parse one `(ia{sv}av)` layout node (recursively). The `GetLayout`
/// reply is `(u(ia{sv}av))` — hand this the inner struct.
pub fn parse_layout(value: &Value) -> Option<DbusmenuItem> {
    let Value::Struct(fields) = value else {
        return None;
    };
    if fields.len() < 3 {
        return None;
    }
    let id = fields[0].as_i32().unwrap_or(0);
    let empty = HashMap::new();
    let props = match &fields[1] {
        Value::Dict(d) => d,
        _ => &empty,
    };
    let s = |k: &str| {
        props
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let children = match &fields[2] {
        Value::Array(arr) => arr.iter().filter_map(parse_layout).collect(),
        _ => Vec::new(),
    };
    Some(DbusmenuItem {
        id,
        label: s("label"),
        item_type: s("type"),
        enabled: props
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        visible: props
            .get("visible")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        toggle_type: s("toggle-type"),
        toggle_state: props
            .get("toggle-state")
            .and_then(|v| v.as_i32())
            .unwrap_or(-1),
        children,
    })
}

/// Body for `GetLayout(parentId = 0, recursionDepth = -1, props = [])`.
pub fn encode_get_layout(buf: &mut Vec<u8>) {
    dbus::encode_i32(buf, 0);
    dbus::encode_i32(buf, -1);
    dbus::align_to(buf, 4);
    dbus::encode_u32(buf, 0);
}

/// Body for `Event(id, "clicked", variant<i32 0>, timestamp)`.
pub fn encode_menu_event(buf: &mut Vec<u8>, item_id: i32) {
    dbus::encode_i32(buf, item_id);
    dbus::encode_string(buf, "clicked");
    dbus::encode_signature(buf, "i");
    dbus::align_to(buf, 4);
    dbus::encode_i32(buf, 0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    dbus::encode_u32(buf, ts);
}

/// GTK-style mnemonics: `_Open` → `Open`, `__` → `_`.
fn strip_mnemonics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Flatten the tree into the context-menu widget's item list. Submenus
/// become an inline run of indented rows under their parent label —
/// the widget is single-level and tray menus are shallow enough that
/// this reads fine.
pub fn to_menu_items(items: &[DbusmenuItem]) -> Vec<MenuItem> {
    let mut out = Vec::new();
    flatten(items, 0, &mut out);
    // Trim leading / trailing / doubled separators left by hidden rows.
    let is_sep = |m: &MenuItem| m.action == MenuAction::TraySeparator;
    let mut cleaned: Vec<MenuItem> = Vec::with_capacity(out.len());
    for m in out {
        if is_sep(&m) && cleaned.last().map(is_sep).unwrap_or(true) {
            continue;
        }
        cleaned.push(m);
    }
    while cleaned.last().map(is_sep).unwrap_or(false) {
        cleaned.pop();
    }
    cleaned
}

fn flatten(items: &[DbusmenuItem], depth: usize, out: &mut Vec<MenuItem>) {
    let indent = "    ".repeat(depth);
    for item in items {
        if !item.visible {
            continue;
        }
        if item.item_type == "separator" {
            out.push(MenuItem {
                label: String::new(),
                action: MenuAction::TraySeparator,
            });
            continue;
        }
        let mut label = strip_mnemonics(&item.label);
        if label.is_empty() {
            continue;
        }
        match (item.toggle_type.as_str(), item.toggle_state) {
            ("checkmark", 1) => label = format!("✓ {label}"),
            ("checkmark", _) => label = format!("   {label}"),
            ("radio", 1) => label = format!("● {label}"),
            ("radio", _) => label = format!("○ {label}"),
            _ => {}
        }
        let label = format!("{indent}{label}");
        if !item.children.is_empty() {
            out.push(MenuItem {
                label,
                action: MenuAction::TrayDisabled,
            });
            flatten(&item.children, depth + 1, out);
        } else if !item.enabled {
            out.push(MenuItem {
                label,
                action: MenuAction::TrayDisabled,
            });
        } else {
            out.push(MenuItem {
                label,
                action: MenuAction::TrayMenuItem(item.id),
            });
        }
    }
}
