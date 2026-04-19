//! `LspManager` — the single object `TextHandler` talks to. Lazily spawns
//! one `LspClient` per language, routes didOpen/didChange/didSave to the
//! right client, and caches diagnostics keyed by URI.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use super::client::{LspClient, PendingKind, ServerConfig};
use super::protocol::Diagnostic;
use super::{ServerId, ServerMessage};
use crate::syntax::Language;

/// Static list of servers we know how to spawn. `command` + `args` are
/// passed directly to `std::process::Command` — missing binaries surface
/// as a spawn error which we log to stderr and ignore.
const SERVERS: &[ServerConfig] = &[
    ServerConfig {
        id: ServerId::Rust,
        command: "rust-analyzer",
        args: &[],
        language_id: "rust",
    },
    ServerConfig {
        id: ServerId::Python,
        command: "pyright-langserver",
        args: &["--stdio"],
        language_id: "python",
    },
    ServerConfig {
        id: ServerId::TypeScript,
        command: "typescript-language-server",
        args: &["--stdio"],
        language_id: "typescript",
    },
];

fn config_for(id: ServerId) -> &'static ServerConfig {
    SERVERS.iter().find(|c| c.id == id).expect("ServerId configured")
}

/// Anything the manager tracks about a single open document, indexed by URI.
#[derive(Default)]
pub struct DocState {
    pub diagnostics: Vec<Diagnostic>,
}

pub struct LspManager {
    clients: HashMap<ServerId, LspClient>,
    /// Servers we've already tried to spawn and failed (e.g. binary not on
    /// PATH). We don't keep retrying every time the user opens a file.
    failed: HashMap<ServerId, String>,
    pub docs: HashMap<String, DocState>,
    tx: Sender<(ServerId, ServerMessage)>,
    rx: Receiver<(ServerId, ServerMessage)>,
    /// Shared callback reader threads invoke when a message arrives so the
    /// main loop wakes up to drain the channel. Cloned (cheaply) for every
    /// spawned client.
    wake: Arc<dyn Fn() + Send + Sync + 'static>,
}

impl LspManager {
    pub fn new<F: Fn() + Send + Sync + 'static>(wake: F) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            clients: HashMap::new(),
            failed: HashMap::new(),
            docs: HashMap::new(),
            tx,
            rx,
            wake: Arc::new(wake),
        }
    }

    /// Drain all inbound messages (one frame's worth). Consumer decides how
    /// to dispatch — typically by calling `handle_response` on each Response.
    pub fn drain(&mut self) -> Vec<(ServerId, ServerMessage)> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            out.push(msg);
        }
        out
    }

    /// Ensure a server exists for `server_id`. Returns None on spawn failure.
    pub fn ensure(&mut self, server_id: ServerId, root: &Path) -> Option<&mut LspClient> {
        if self.clients.contains_key(&server_id) {
            return self.clients.get_mut(&server_id);
        }
        if self.failed.contains_key(&server_id) {
            return None;
        }
        let cfg = config_for(server_id);
        let wake = self.wake.clone();
        let wake_box: Box<dyn Fn() + Send + 'static> = Box::new(move || (wake)());
        match LspClient::spawn(cfg, root, self.tx.clone(), wake_box) {
            Ok(client) => {
                self.clients.insert(server_id, client);
                self.clients.get_mut(&server_id)
            }
            Err(e) => {
                eprintln!(
                    "[lntrn-code] lsp: failed to spawn {} ({}): {e}",
                    cfg.command, server_id.label()
                );
                self.failed.insert(server_id, e.to_string());
                None
            }
        }
    }

    pub fn client_mut(&mut self, server_id: ServerId) -> Option<&mut LspClient> {
        self.clients.get_mut(&server_id)
    }

    // ── Document lifecycle (TextHandler calls these) ──────────────────

    pub fn did_open(
        &mut self,
        lang: Language,
        filename: &str,
        path: &Path,
        text: &str,
    ) -> Option<ServerId> {
        let id = super::server_for_language(lang, filename)?;
        let root = workspace_root(path);
        let client = self.ensure(id, &root)?;
        let uri = super::client::path_to_uri(path);
        let _ = client.did_open(&uri, text);
        self.docs.entry(uri).or_default();
        Some(id)
    }

    pub fn did_change(&mut self, path: &Path, text: &str) {
        let uri = super::client::path_to_uri(path);
        for client in self.clients.values_mut() {
            if client.open_docs.contains_key(&uri) {
                let _ = client.did_change(&uri, text);
            }
        }
    }

    pub fn did_save(&mut self, path: &Path, text: &str) {
        let uri = super::client::path_to_uri(path);
        for client in self.clients.values_mut() {
            if client.open_docs.contains_key(&uri) {
                let _ = client.did_save(&uri, Some(text));
            }
        }
    }

    pub fn did_close(&mut self, path: &Path) {
        let uri = super::client::path_to_uri(path);
        for client in self.clients.values_mut() {
            if client.open_docs.contains_key(&uri) {
                let _ = client.did_close(&uri);
            }
        }
        self.docs.remove(&uri);
    }

    /// Merge a fresh diagnostics batch from the server.
    pub fn set_diagnostics(&mut self, uri: String, diags: Vec<Diagnostic>) {
        self.docs.entry(uri).or_default().diagnostics = diags;
    }

    pub fn diagnostics_for_uri(&self, uri: &str) -> &[Diagnostic] {
        self.docs
            .get(uri)
            .map(|d| d.diagnostics.as_slice())
            .unwrap_or(&[])
    }

    /// (errors, warnings, infos, hints) across one document.
    pub fn diagnostic_counts(&self, uri: &str) -> (usize, usize, usize, usize) {
        let mut counts = (0, 0, 0, 0);
        for d in self.diagnostics_for_uri(uri) {
            match d.severity.unwrap_or(super::protocol::SEVERITY_ERROR) {
                super::protocol::SEVERITY_ERROR => counts.0 += 1,
                super::protocol::SEVERITY_WARNING => counts.1 += 1,
                super::protocol::SEVERITY_INFO => counts.2 += 1,
                super::protocol::SEVERITY_HINT => counts.3 += 1,
                _ => {}
            }
        }
        counts
    }

    pub fn take_pending(&mut self, server_id: ServerId, id: u64) -> Option<PendingKind> {
        self.clients.get_mut(&server_id).and_then(|c| c.pending.remove(&id))
    }
}

/// Find the project root for a given file. Walks upward looking for common
/// project markers; falls back to the file's parent dir so rust-analyzer
/// doesn't open at / and try to index the whole filesystem.
fn workspace_root(path: &Path) -> std::path::PathBuf {
    const MARKERS: &[&str] = &[
        "Cargo.toml",
        "pyproject.toml",
        "package.json",
        "tsconfig.json",
        "go.mod",
        ".git",
    ];
    let mut cur = path.parent().map(|p| p.to_path_buf());
    while let Some(dir) = cur {
        for m in MARKERS {
            if dir.join(m).exists() {
                return dir;
            }
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("."))
}
