//! LSP client integration for lntrn-code.
//!
//! Public surface:
//!   * `LspManager`     — owned by `TextHandler`, routes files to the right server.
//!   * `ServerMessage`  — inbound messages the main loop drains each frame.
//!   * `HoverState`, `CompletionState`, `DefinitionState` — popup UI state.
//!
//! No async runtime. Each server is a `std::process::Child` with a
//! dedicated reader thread that forwards parsed messages to the winit
//! event loop via `UserEvent::LspMessage`.

pub mod client;
pub mod framing;
pub mod glue;
pub mod manager;
pub mod protocol;
pub mod ui_render;

pub use client::path_to_uri;
pub use manager::LspManager;

use crate::syntax::Language;
use protocol::PublishDiagnosticsParams;

/// Which LSP server a given file should talk to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ServerId {
    Rust,
    Python,
    TypeScript,
}

impl ServerId {
    pub fn label(self) -> &'static str {
        match self {
            ServerId::Rust => "rust-analyzer",
            ServerId::Python => "pyright",
            ServerId::TypeScript => "ts-ls",
        }
    }
}

/// The parsed form of an inbound LSP message, forwarded from reader thread
/// to main thread via `UserEvent::LspMessage`.
#[derive(Debug)]
pub enum ServerMessage {
    Diagnostics(PublishDiagnosticsParams),
    /// Response to a request we sent.
    Response {
        id: u64,
        result: Option<serde_json::Value>,
        error: Option<(i32, String)>,
    },
    /// window/logMessage or window/showMessage — used to show "indexing…"
    /// and error notices in the status bar.
    LogMessage(String),
    /// Anything we don't handle yet.
    Unknown,
}

/// Map an editor `Language` to the LSP server that should handle it.
/// Returns `None` when the language has no configured server.
pub fn server_for_language(lang: Language, filename: &str) -> Option<ServerId> {
    match lang {
        Language::Rust => Some(ServerId::Rust),
        Language::Python => Some(ServerId::Python),
        Language::None => {
            // typescript/javascript are detected by extension here so we
            // don't have to plumb them through the Language enum yet.
            let lower = filename.to_ascii_lowercase();
            if lower.ends_with(".ts")
                || lower.ends_with(".tsx")
                || lower.ends_with(".js")
                || lower.ends_with(".jsx")
                || lower.ends_with(".mjs")
                || lower.ends_with(".cjs")
            {
                Some(ServerId::TypeScript)
            } else {
                None
            }
        }
    }
}

// ── UI popup state ───────────────────────────────────────────────────────────

/// Hover popup (Ctrl+mouse-over). Populated by response to
/// `textDocument/hover`, cleared when the user moves the cursor or releases Ctrl.
#[derive(Default)]
pub struct HoverState {
    pub visible: bool,
    pub text: String,
    /// Physical-pixel anchor where the popup should be drawn (top-left).
    pub anchor_x: f32,
    pub anchor_y: f32,
    /// We only want the latest request's response — track which id we're
    /// waiting on so older late-arriving responses are discarded.
    pub in_flight: Option<u64>,
}

impl HoverState {
    pub fn clear(&mut self) {
        self.visible = false;
        self.text.clear();
        self.in_flight = None;
    }
}

/// Completion popup state (Ctrl+Space).
pub struct CompletionState {
    pub visible: bool,
    pub items: Vec<CompletionItem>,
    /// Byte offset inside the line where the prefix being completed starts.
    pub prefix_start_col: usize,
    /// The document line the completion was triggered on.
    pub line: usize,
    /// Typed prefix used to filter `items`.
    pub prefix: String,
    pub selected: usize,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub in_flight: Option<u64>,
    /// Tab the completion was triggered on — responses for other tabs are
    /// ignored since the user has switched away.
    pub tab_id: u64,
}

impl Default for CompletionState {
    fn default() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
            prefix_start_col: 0,
            line: 0,
            prefix: String::new(),
            selected: 0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            in_flight: None,
            tab_id: 0,
        }
    }
}

impl CompletionState {
    pub fn clear(&mut self) {
        self.visible = false;
        self.items.clear();
        self.prefix.clear();
        self.selected = 0;
        self.in_flight = None;
    }

    /// Items filtered by the current prefix, in their LSP-provided sort order.
    pub fn filtered(&self) -> Vec<&CompletionItem> {
        let lower = self.prefix.to_ascii_lowercase();
        let mut v: Vec<&CompletionItem> = self
            .items
            .iter()
            .filter(|it| {
                let hay = it.filter_label();
                hay.to_ascii_lowercase().contains(&lower)
            })
            .collect();
        v.sort_by(|a, b| a.sort_text().cmp(b.sort_text()));
        v
    }
}

/// A single completion item in our own canonical form (decoded from the LSP
/// response). Stored on the popup state so the filter function doesn't have
/// to repeat the serde work on every keystroke.
#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Option<u8>,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
}

impl CompletionItem {
    pub fn from_proto(p: protocol::CompletionItem) -> Self {
        Self {
            label: p.label,
            kind: p.kind,
            detail: p.detail,
            insert_text: p.insert_text,
            filter_text: p.filter_text,
            sort_text: p.sort_text,
        }
    }
    fn filter_label(&self) -> &str {
        self.filter_text.as_deref().unwrap_or(&self.label)
    }
    fn sort_text(&self) -> &str {
        self.sort_text.as_deref().unwrap_or(&self.label)
    }
    /// The string actually inserted into the document on accept.
    pub fn insert(&self) -> &str {
        self.insert_text.as_deref().unwrap_or(&self.label)
    }
}
