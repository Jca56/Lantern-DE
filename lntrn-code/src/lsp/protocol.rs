//! LSP wire types. We define only the messages we actually send or receive;
//! unknown fields fall through thanks to serde's default behavior.
//!
//! Response payloads (`result`) are left as raw `serde_json::Value` and
//! decoded lazily in `client.rs` per pending-request kind. That keeps this
//! file small and keeps optional fields from exploding the struct list.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC envelope ────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Request<'a, P: Serialize> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    pub params: P,
}

#[derive(Serialize)]
pub struct Notification<'a, P: Serialize> {
    pub jsonrpc: &'static str,
    pub method: &'a str,
    pub params: P,
}

/// Every inbound message — response, server-to-client request, or notification
/// — shares this shape. We branch on which fields are present.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct Incoming {
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: Option<String>,
    pub params: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<ResponseError>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    #[allow(dead_code)]
    pub data: Option<Value>,
}

// ── Positions ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

// ── textDocument identifiers ─────────────────────────────────────────────────

#[derive(Serialize)]
pub struct TextDocumentIdentifier {
    pub uri: String,
}

#[derive(Serialize)]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: i32,
}

#[derive(Serialize)]
pub struct TextDocumentItem {
    pub uri: String,
    #[serde(rename = "languageId")]
    pub language_id: &'static str,
    pub version: i32,
    pub text: String,
}

#[derive(Serialize)]
pub struct TextDocumentPositionParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

// ── Initialize ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct InitializeParams<'a> {
    #[serde(rename = "processId")]
    pub process_id: Option<u32>,
    #[serde(rename = "rootUri")]
    pub root_uri: Option<String>,
    /// Modern LSP servers (pyright in particular) prefer this over `rootUri`.
    /// When omitted, pyright invents a "<default workspace root>" placeholder
    /// and complains it can't find files under it.
    #[serde(rename = "workspaceFolders", skip_serializing_if = "Option::is_none")]
    pub workspace_folders: Option<Vec<WorkspaceFolder>>,
    pub capabilities: Value,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo<'a>,
    pub trace: &'static str,
}

#[derive(Serialize)]
pub struct WorkspaceFolder {
    pub uri: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct ClientInfo<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

// ── didOpen / didChange / didSave ────────────────────────────────────────────

#[derive(Serialize)]
pub struct DidOpenParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentItem,
}

#[derive(Serialize)]
pub struct DidChangeParams {
    #[serde(rename = "textDocument")]
    pub text_document: VersionedTextDocumentIdentifier,
    #[serde(rename = "contentChanges")]
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

/// Full-document replacement. rust-analyzer advertises `TextDocumentSyncKind::Full`
/// support even when it prefers incremental, so we keep it simple.
#[derive(Serialize)]
pub struct TextDocumentContentChangeEvent {
    pub text: String,
}

#[derive(Serialize)]
pub struct DidSaveParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub text: Option<String>,
}

#[derive(Serialize)]
pub struct DidCloseParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
}

// ── Diagnostics (inbound) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PublishDiagnosticsParams {
    pub uri: String,
    pub diagnostics: Vec<Diagnostic>,
    #[allow(dead_code)]
    #[serde(default)]
    pub version: Option<i32>,
}

/// `source` and `message` aren't consumed yet; they're surfaced when we add
/// the hover-on-diagnostic feature.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Diagnostic {
    pub range: Range,
    #[serde(default)]
    pub severity: Option<u8>,
    #[serde(default)]
    pub source: Option<String>,
    pub message: String,
}

/// LSP DiagnosticSeverity values. The spec says 1=Error, 2=Warning, 3=Info, 4=Hint.
pub const SEVERITY_ERROR: u8 = 1;
pub const SEVERITY_WARNING: u8 = 2;
pub const SEVERITY_INFO: u8 = 3;
pub const SEVERITY_HINT: u8 = 4;

// ── Hover / Completion / Definition params ──────────────────────────────────

pub type HoverParams = TextDocumentPositionParams;
pub type DefinitionParams = TextDocumentPositionParams;

#[derive(Serialize)]
pub struct CompletionParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
    pub context: CompletionContext,
}

#[derive(Serialize)]
pub struct CompletionContext {
    #[serde(rename = "triggerKind")]
    pub trigger_kind: u8,
    #[serde(rename = "triggerCharacter", skip_serializing_if = "Option::is_none")]
    pub trigger_character: Option<String>,
}

/// Only the fields we actually consume from a completion item. rust-analyzer
/// sends a lot more (additionalTextEdits, tags, etc.) — serde ignores them.
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    #[serde(default)]
    pub kind: Option<u8>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default, rename = "insertText")]
    pub insert_text: Option<String>,
    #[serde(default, rename = "filterText")]
    pub filter_text: Option<String>,
    #[serde(default, rename = "sortText")]
    pub sort_text: Option<String>,
}
