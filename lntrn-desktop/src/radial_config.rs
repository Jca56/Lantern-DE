//! User-editable contents of the desktop right-click radial menu.
//!
//! Path: `~/.lantern/config/desktop-radial.json`. The file is a flat list of
//! buttons in clockwise order (starting from the top). On first run we write the
//! built-in defaults so there's always a file to discover and tweak; a missing
//! or malformed file falls back to defaults *without* clobbering whatever the
//! user has on disk.
//!
//! Each entry is `{ label, icon, action, command }`:
//!   - `action: "launch"`     → run `command` (program + args), cwd = ~/Desktop
//!   - `action: "new_folder"` → create a folder where the ring opened
//!   - `action: "refresh"`    → re-scan the desktop
//! `icon` is any name known to `lntrn_icons::get` (e.g. "lntrn-terminal.svg").

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::radial_menu::{RadialAction, RadialItem};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RadialItemCfg {
    label: String,
    icon: String,
    /// "launch" | "new_folder" | "refresh". Defaults to "launch" so a bare
    /// `{ label, icon, command }` entry just works.
    #[serde(default = "default_action")]
    action: String,
    /// Program + whitespace-separated args for `action: "launch"`.
    #[serde(default)]
    command: String,
}

fn default_action() -> String {
    "launch".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RadialCfg {
    items: Vec<RadialItemCfg>,
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".lantern/config/desktop-radial.json")
}

/// Load the ring contents. Writes the defaults on first run; on a parse error
/// keeps the user's file untouched and just uses defaults in memory.
pub fn load() -> Vec<RadialItem> {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<RadialCfg>(&s) {
            Ok(cfg) => {
                let items = to_items(cfg);
                if items.is_empty() {
                    default_items()
                } else {
                    items
                }
            }
            Err(e) => {
                tracing::warn!("desktop-radial.json parse error ({e}); using defaults");
                default_items()
            }
        },
        Err(_) => {
            // First run (or unreadable): seed the file with the defaults.
            write_defaults(&path);
            default_items()
        }
    }
}

fn to_items(cfg: RadialCfg) -> Vec<RadialItem> {
    cfg.items
        .into_iter()
        .map(|c| {
            let action = match c.action.as_str() {
                "new_folder" | "newfolder" => RadialAction::NewFolder,
                "refresh" => RadialAction::Refresh,
                // "launch" and anything unrecognised: treat as a command launch.
                _ => RadialAction::Launch(c.command.clone()),
            };
            RadialItem {
                action,
                label: c.label,
                icon: c.icon,
            }
        })
        // Drop launch entries with no command — they'd be dead buttons.
        .filter(|it| !matches!(&it.action, RadialAction::Launch(cmd) if cmd.trim().is_empty()))
        .collect()
}

fn write_defaults(path: &std::path::Path) {
    let cfg = RadialCfg {
        items: DEFAULTS
            .iter()
            .map(|(label, icon, action, command)| RadialItemCfg {
                label: label.to_string(),
                icon: icon.to_string(),
                action: action.to_string(),
                command: command.to_string(),
            })
            .collect(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&cfg) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, json);
    }
}

fn default_items() -> Vec<RadialItem> {
    DEFAULTS
        .iter()
        .map(|(label, icon, action, command)| {
            let action = match *action {
                "new_folder" => RadialAction::NewFolder,
                "refresh" => RadialAction::Refresh,
                _ => RadialAction::Launch(command.to_string()),
            };
            RadialItem {
                action,
                label: label.to_string(),
                icon: icon.to_string(),
            }
        })
        .collect()
}

/// (label, icon, action, command) — the seed ring, clockwise from the top.
const DEFAULTS: &[(&str, &str, &str, &str)] = &[
    ("Terminal", "lntrn-terminal.svg", "launch", "lntrn-terminal"),
    (
        "File Manager",
        "lntrn-file-manager.svg",
        "launch",
        "lntrn-file-manager",
    ),
    ("Firefox", "firefox", "launch", "firefox"),
    ("Notepad", "lntrn-notepad.svg", "launch", "lntrn-notepad"),
    (
        "Screenshot",
        "lntrn-screenshot.svg",
        "launch",
        "lntrn-screenshot",
    ),
    (
        "Settings",
        "lntrn-system-settings.svg",
        "launch",
        "lntrn-system-settings",
    ),
];
