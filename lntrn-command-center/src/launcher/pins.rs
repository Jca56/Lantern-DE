//! Pinned favorites — persisted as a plain newline-delimited list of
//! app_ids at `~/.lantern/config/command-center/pins.toml`.
//!
//! Format (human-editable):
//!
//!   # comments allowed
//!   firefox
//!   code
//!   lntrn-terminal
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
    "lntrn-music-player",
];

pub struct Pins {
    /// app_ids in pin order (left → right in the row).
    items: Vec<String>,
    path: PathBuf,
}

impl Pins {
    /// Load pins from disk. Creates the config directory and writes the
    /// default pins file on first run. Logs a warning if anything goes
    /// wrong but never fails — an empty pin list is a perfectly fine
    /// fallback.
    pub fn load() -> Self {
        let path = config_path();
        let items = read_or_init(&path);
        Self { items, path }
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

    fn save(&self) {
        let body = render(&self.items);
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

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut p = PathBuf::from(home);
    p.push(".lantern/config/command-center/pins.toml");
    p
}

/// Try to read the pins file; if it's missing, create it with defaults.
fn read_or_init(path: &PathBuf) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(body) => parse(&body),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(?path, "pins file not found — writing defaults");
            let defaults: Vec<String> = DEFAULT_PINS.iter().map(|s| s.to_string()).collect();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, render(&defaults));
            defaults
        }
        Err(e) => {
            tracing::warn!(?e, ?path, "failed to read pins — starting with defaults");
            DEFAULT_PINS.iter().map(|s| s.to_string()).collect()
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

fn render(items: &[String]) -> String {
    let mut out = String::with_capacity(64 + items.iter().map(|s| s.len() + 1).sum::<usize>());
    out.push_str("# lntrn-command-center pinned apps\n");
    out.push_str("# One app_id per line (the .desktop file's stem).\n");
    out.push_str("# Lines starting with # are comments. Order matters — left to right.\n\n");
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
        let body = render(&items);
        assert_eq!(parse(&body), items);
    }
}
