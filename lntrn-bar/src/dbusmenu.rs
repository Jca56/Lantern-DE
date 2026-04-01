//! com.canonical.dbusmenu protocol — parse GetLayout responses and encode Event calls.
//! Used by tray items (like Steam) that expose a menu instead of an Activate method.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_ui::gpu::{MenuItem, FoxPalette};

use lntrn_dbus::{self as dbus, Value};

/// Base ID offset for dbusmenu items in the ContextMenu zone system.
/// Dbusmenu item IDs are offset by this so they don't collide with bar's own menu IDs.
pub const DBUSMENU_ID_BASE: u32 = 0xDB_0000;

// ── Parsed menu item ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DbusmenuItem {
    pub id: i32,
    pub label: String,
    pub item_type: String,
    pub enabled: bool,
    pub visible: bool,
    pub children: Vec<DbusmenuItem>,
}

// ── Parse GetLayout response ────────────────────────────────────────────────

/// Parse a `(ia{sv}av)` struct from a GetLayout reply into a DbusmenuItem tree.
/// The top-level response is `(u(ia{sv}av))` — call this on the inner struct.
pub fn parse_layout(value: &Value) -> Option<DbusmenuItem> {
    let fields = match value {
        Value::Struct(f) => f,
        _ => return None,
    };
    if fields.len() < 3 { return None; }

    let id = fields[0].as_i32().unwrap_or(0);

    let props = match &fields[1] {
        Value::Dict(d) => d,
        _ => &HashMap::new() as &HashMap<String, Value>,
    };

    let label = props.get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let item_type = props.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let enabled = props.get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let visible = props.get("visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let children = match &fields[2] {
        Value::Array(arr) => arr.iter()
            .filter_map(|child| parse_layout(child))
            .collect(),
        _ => Vec::new(),
    };

    Some(DbusmenuItem { id, label, item_type, enabled, visible, children })
}

// ── Encode GetLayout call body ──────────────────────────────────────────────

/// Encode arguments for `com.canonical.dbusmenu.GetLayout(parentId=0, depth=-1, props=[])`.
/// Signature: `iias`
pub fn encode_get_layout(buf: &mut Vec<u8>) {
    dbus::encode_i32(buf, 0);   // parentId: root
    dbus::encode_i32(buf, -1);  // recursionDepth: full tree
    // Empty string array: length 0
    dbus::align_to(buf, 4);
    dbus::encode_u32(buf, 0);
}

// ── Encode Event call body ──────────────────────────────────────────────────

/// Encode arguments for `com.canonical.dbusmenu.Event(id, "clicked", variant<i32 0>, timestamp)`.
/// Signature: `isvu`
pub fn encode_menu_event(buf: &mut Vec<u8>, item_id: i32) {
    dbus::encode_i32(buf, item_id);
    dbus::encode_string(buf, "clicked");
    // Variant: signature "i", value 0
    dbus::encode_signature(buf, "i");
    dbus::align_to(buf, 4); // align value after signature
    dbus::encode_i32(buf, 0);
    // Timestamp
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    dbus::encode_u32(buf, ts);
}

// ── Convert to lntrn-ui MenuItems ───────────────────────────────────────────

/// Strip underscore mnemonics from dbusmenu labels (e.g. "_Open" -> "Open").
fn strip_mnemonics(s: &str) -> String {
    s.replace('_', "")
}

/// Convert a list of DbusmenuItems to lntrn-ui MenuItems for the ContextMenu.
pub fn to_menu_items(items: &[DbusmenuItem], palette: &FoxPalette) -> Vec<MenuItem> {
    let _ = palette; // reserved for future icon tinting
    let mut result = Vec::new();
    for item in items {
        if !item.visible { continue; }
        if item.item_type == "separator" {
            result.push(MenuItem::separator());
            continue;
        }
        let label = strip_mnemonics(&item.label);
        if label.is_empty() { continue; }
        let menu_id = DBUSMENU_ID_BASE + item.id as u32;
        if !item.enabled {
            result.push(MenuItem::action_disabled(menu_id, &label));
        } else if !item.children.is_empty() {
            let children = to_menu_items(&item.children, palette);
            result.push(MenuItem::submenu(menu_id, &label, children));
        } else {
            result.push(MenuItem::action(menu_id, &label));
        }
    }
    result
}
