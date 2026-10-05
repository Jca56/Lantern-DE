//! Pinned favorites — persisted as plain newline-delimited lists of
//! app_ids (or absolute paths) under `~/.lantern/config/command-center/`.
//!
//! There are two lists, deliberately independent:
//!
//! - `pins.toml` — the "Pinned" grid on the main page.
//! - `dock.toml` — the apps kept in the mini-dock.
//!
//! Format (human-editable):
//!
//!   # comments allowed
//!   firefox
//!   code
//!   lntrn-terminal
//!   /home/alva/Documents
//!   /home/alva/notes.md
//!
//! Entries that start with `/` are treated as filesystem paths and
//! drawn alongside pinned apps on the main page. Everything else is
//! treated as an app_id. The dock holds apps only.
//!
//! We don't use the `toml` crate — a list of strings doesn't need a
//! schema, and Lantern prefers minimal deps. The `.toml` extension is
//! convention so editors syntax-highlight it sensibly.

use std::path::PathBuf;

/// Default pins shipped on first run. Curated to be useful out of the
/// box; user can edit pins.toml or right-click in the panel to change.
const DEFAULT_PINS: &[&str] = &[
    "lntrn-terminal",
    "lntrn-file-manager",
    "lntrn-code",
    "firefox",
];

const PINS_FILE: &str = "pins.toml";
const DOCK_FILE: &str = "dock.toml";

/// Comment block written at the top of each file on save.
const PINS_HEADER: &str = "# lntrn-command-center pinned apps\n\
# One app_id per line (the .desktop file's stem).\n\
# Lines starting with # are comments. Order matters — left to right.\n\n";
const DOCK_HEADER: &str = "# lntrn-command-center dock apps\n\
# One app_id per line (the .desktop file's stem).\n\
# Lines starting with # are comments. Order matters — left to right.\n\n";

pub struct Pins {
    /// app_ids in pin order (left → right in the row).
    items: Vec<String>,
    path: PathBuf,
    header: &'static str,
}

impl Pins {
    /// Load pins from disk. Creates the config directory and writes the
    /// default pins file on first run. Logs a warning if anything goes
    /// wrong but never fails — an empty pin list is a perfectly fine
    /// fallback.
    pub fn load() -> Self {
        let path = config_path(PINS_FILE);
        let items = read_or_init(&path, PINS_HEADER, || {
            DEFAULT_PINS.iter().map(|s| s.to_string()).collect()
        });
        Self {
            items,
            path,
            header: PINS_HEADER,
        }
    }

    /// Load the dock's list. The first run seeds it with the apps in
    /// `launcher_pins` — what the dock showed back when the two shared
    /// one list — so splitting them changes nothing until the user
    /// edits one side. Path pins stay behind; the dock holds apps only.
    pub fn load_dock(launcher_pins: &Pins) -> Self {
        let path = config_path(DOCK_FILE);
        let items = read_or_init(&path, DOCK_HEADER, || {
            launcher_pins
                .items
                .iter()
                .filter(|id| !super::is_path(id))
                .cloned()
                .collect()
        });
        Self {
            items,
            path,
            header: DOCK_HEADER,
        }
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    #[allow(dead_code)] // used by right-click handler in Phase 2.6
    pub fn is_pinned(&self, app_id: &str) -> bool {
        self.items.iter().any(|s| s == app_id)
    }

    /// Toggle pin state. Returns whether the app is now pinned.
    /// Persists the change immediately.
    #[allow(dead_code)] // Phase 2.3 wires this through right-click handler
    pub fn toggle(&mut self, app_id: &str) -> bool {
        let now_pinned = if let Some(pos) = self.items.iter().position(|s| s == app_id) {
            self.items.remove(pos);
            false
        } else {
            self.items.push(app_id.to_string());
            true
        };
        self.save();
        now_pinned
    }

    /// Move the pin at `from` to position `to`. Clamps both to the
    /// current item range. Persists on success. No-op when `from == to`
    /// or the index is out of bounds.
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to > self.items.len() || from == to {
            return;
        }
        let item = self.items.remove(from);
        // After removing `from`, indexes ≥ `from` shifted left by one —
        // adjust the destination accordingly.
        let dest = if to > from { to - 1 } else { to };
        let dest = dest.min(self.items.len());
        self.items.insert(dest, item);
        self.save();
    }

    fn save(&self) {
        let body = render(self.header, &self.items);
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(?e, ?parent, "failed to create pins dir");
                return;
            }
        }
        if let Err(e) = std::fs::write(&self.path, body) {
            tracing::warn!(?e, path = ?self.path, "failed to save pins");
        }
    }
}

fn config_path(file: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut p = PathBuf::from(home);
    p.push(".lantern/config/command-center");
    p.push(file);
    p
}

/// Try to read a pins file; if it's missing, create it from `defaults`.
fn read_or_init(
    path: &PathBuf,
    header: &str,
    defaults: impl FnOnce() -> Vec<String>,
) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(body) => parse(&body),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(?path, "pins file not found — writing defaults");
            let defaults = defaults();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, render(header, &defaults));
            defaults
        }
        Err(e) => {
            tracing::warn!(?e, ?path, "failed to read pins — starting with defaults");
            defaults()
        }
    }
}

fn parse(body: &str) -> Vec<String> {
    body.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn render(header: &str, items: &[String]) -> String {
    let mut out =
        String::with_capacity(header.len() + items.iter().map(|s| s.len() + 1).sum::<usize>());
    out.push_str(header);
    for app_id in items {
        out.push_str(app_id);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strips_comments_and_blanks() {
        let body = "# top comment\n\nfirefox\n# inline note\ncode\n  \n  vim  \n";
        let items = parse(body);
        assert_eq!(items, vec!["firefox", "code", "vim"]);
    }

    #[test]
    fn render_then_parse_roundtrip() {
        let items = vec!["firefox".to_string(), "code".to_string()];
        let body = render(PINS_HEADER, &items);
        assert_eq!(parse(&body), items);
    }

    #[test]
    fn dock_seed_takes_apps_and_leaves_paths() {
        let dir = std::env::temp_dir().join(format!("lntrn-cc-dock-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pins = vec![
            "firefox".to_string(),
            "/home/alva/Documents".to_string(),
            "lntrn-code".to_string(),
        ];
        let path = dir.join(DOCK_FILE);
        let seed = || pins.iter().filter(|id| !crate::launcher::is_path(id)).cloned().collect();
        assert_eq!(read_or_init(&path, DOCK_HEADER, seed), vec!["firefox", "lntrn-code"]);
        // Second load reads the file back — the seed must not run again.
        let reread = read_or_init(&path, DOCK_HEADER, || unreachable!("dock.toml already exists"));
        assert_eq!(reread, vec!["firefox", "lntrn-code"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
