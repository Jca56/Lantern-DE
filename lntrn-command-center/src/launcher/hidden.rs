//! Hidden apps — user-curated list of `app_id`s to omit from the
//! all-apps grid (and search results). Persisted to
//! `~/.lantern/config/command-center/hidden.toml`. Same plain-text
//! format as `pins.toml`; one app_id per line.
//!
//! Hide is reversible: a hidden app stays in the apps index, just gets
//! filtered out of launcher views. Right-click → "Unhide" puts it back.

use std::path::PathBuf;

pub struct Hidden {
    items: Vec<String>,
    path: PathBuf,
}

impl Hidden {
    pub fn load() -> Self {
        let path = config_path();
        let items = read_or_empty(&path);
        Self { items, path }
    }

    pub fn is_hidden(&self, app_id: &str) -> bool {
        self.items.iter().any(|s| s == app_id)
    }

    /// Toggle hidden state. Returns whether the app is now hidden.
    pub fn toggle(&mut self, app_id: &str) -> bool {
        let now_hidden = if let Some(pos) = self.items.iter().position(|s| s == app_id) {
            self.items.remove(pos);
            false
        } else {
            self.items.push(app_id.to_string());
            true
        };
        self.save();
        now_hidden
    }

    fn save(&self) {
        let body = render(&self.items);
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(?e, ?parent, "failed to create hidden dir");
                return;
            }
        }
        if let Err(e) = std::fs::write(&self.path, body) {
            tracing::warn!(?e, path = ?self.path, "failed to save hidden");
        }
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut p = PathBuf::from(home);
    p.push(".lantern/config/command-center/hidden.toml");
    p
}

fn read_or_empty(path: &PathBuf) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(body) => parse(&body),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            tracing::warn!(?e, ?path, "failed to read hidden — starting empty");
            Vec::new()
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
    out.push_str("# lntrn-command-center hidden apps\n");
    out.push_str("# Apps listed here are filtered out of the all-apps grid\n");
    out.push_str("# and search results. Right-click → \"Unhide\" to restore,\n");
    out.push_str("# or just delete the line.\n\n");
    for app_id in items {
        out.push_str(app_id);
        out.push('\n');
    }
    out
}
