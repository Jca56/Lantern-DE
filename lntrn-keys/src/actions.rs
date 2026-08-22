//! High-level actions that mutate the daemon: copy, delete, add.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

use crate::secret::Client;
use crate::state::{AddStage, Mode, State};

/// Copy the selected item's secret to the clipboard via `wl-copy`.
pub fn copy_selected(state: &mut State, client: &mut Client) {
    let Some(item) = state.selected() else {
        state.set_status("No selection to copy.");
        return;
    };
    let path = item.path.clone();
    let label = item.label.clone();
    match client.get_secret(&path) {
        Ok(bytes) => {
            if write_to_clipboard(&bytes) {
                state.set_status(format!("✓ Copied \"{label}\" to clipboard."));
            } else {
                state.set_status(format!(
                    "Got secret for \"{label}\" but wl-copy failed — try installing wl-clipboard.",
                ));
            }
        }
        Err(e) => state.set_status(format!("Failed to read secret: {e}")),
    }
}

pub fn reveal_selected(state: &mut State, client: &mut Client) {
    let Some(item) = state.selected() else {
        return;
    };
    let path = item.path.clone();
    match client.get_secret(&path) {
        Ok(bytes) => {
            let s = String::from_utf8_lossy(&bytes).to_string();
            state.mode = Mode::Revealing(s);
        }
        Err(e) => state.set_status(format!("Failed to read secret: {e}")),
    }
}

pub fn confirm_delete(state: &mut State, client: &mut Client) -> bool {
    let Some(item) = state.selected() else {
        state.set_status("No selection to delete.");
        return false;
    };
    let path = item.path.clone();
    let label = item.label.clone();
    match client.delete_item(&path) {
        Ok(()) => {
            state.set_status(format!("✓ Deleted \"{label}\"."));
            true
        }
        Err(e) => {
            state.set_status(format!("Delete failed: {e}"));
            false
        }
    }
}

/// Begin the add wizard.
pub fn start_add(state: &mut State) {
    state.mode = Mode::Adding(AddStage::Name(String::new()));
    state.set_status("Name the key — what you'll search for later (e.g. \"GitHub PAT\").");
}

/// Commit the wizard's final state — create the item in the daemon.
///
/// The "name" is stored as both the FDO `Label` (display) and a `name`
/// attribute (so `secret-tool lookup name "GitHub PAT"` works from scripts).
pub fn finish_add(state: &mut State, client: &mut Client, name: String, secret: String) -> bool {
    let mut attrs: HashMap<String, String> = HashMap::new();
    attrs.insert("name".into(), name.clone());
    match client.create_item(&name, &attrs, secret.as_bytes(), true) {
        Ok(_) => {
            state.set_status(format!(
                "✓ Stored \"{name}\". Look it up with: secret-tool lookup name \"{name}\""
            ));
            true
        }
        Err(e) => {
            state.set_status(format!("Store failed: {e}"));
            false
        }
    }
}

fn write_to_clipboard(bytes: &[u8]) -> bool {
    let mut child = match Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(bytes);
    }
    matches!(child.wait().map(|s| s.success()), Ok(true))
}
