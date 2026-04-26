//! One LSP server process. Spawns the child, runs a reader thread that
//! forwards parsed messages to the winit event loop, and exposes a send API
//! for requests + notifications.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use serde::Serialize;
use serde_json::Value;

use super::framing;
use super::protocol::{
    ClientInfo, DidChangeParams, DidCloseParams, DidOpenParams, DidSaveParams, Incoming,
    InitializeParams, Notification, Request, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, VersionedTextDocumentIdentifier,
};
use super::{ServerId, ServerMessage};

/// What a request was asking for, so the main loop knows how to decode the
/// response it gets back. `uri` fields aren't consumed today but are kept
/// for future debug logging / context-aware handling of late responses.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum PendingKind {
    Initialize,
    Hover { uri: String },
    Completion { uri: String, line: u32, col: u32, prefix: String, tab_id: u64 },
    Definition { uri: String, tab_id: u64, new_tab: bool },
}

/// Configuration for one LSP server binary.
pub struct ServerConfig {
    pub id: ServerId,
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub language_id: &'static str,
}

/// A live LSP server process.
#[allow(dead_code)]
pub struct LspClient {
    pub id: ServerId,
    pub root_uri: String,
    pub language_id: &'static str,
    child: Child,
    stdin: BufWriter<ChildStdin>,
    _reader: JoinHandle<()>,
    _stderr: JoinHandle<()>,
    next_id: u64,
    pub pending: HashMap<u64, PendingKind>,
    /// False until the server responds to `initialize` and we've sent `initialized`.
    pub ready: bool,
    /// URI -> current doc version we've synced with the server.
    pub open_docs: HashMap<String, i32>,
    /// Pending didOpen/didChange calls buffered before the server was ready.
    pending_opens: Vec<(String, String)>,
    pending_changes: Vec<(String, String)>,
}

impl LspClient {
    pub fn spawn(
        cfg: &ServerConfig,
        root_path: &Path,
        tx: Sender<(ServerId, ServerMessage)>,
        wake: Box<dyn Fn() + Send + 'static>,
    ) -> std::io::Result<Self> {
        let mut cmd = Command::new(cfg.command);
        cmd.args(cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let stdin = child.stdin.take().expect("stdin piped");

        let server_id = cfg.id;
        let reader_tx = tx.clone();
        let reader = std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                match framing::read_message(&mut r) {
                    Ok(Some(bytes)) => {
                        let msg = parse_incoming(&bytes);
                        if reader_tx.send((server_id, msg)).is_err() {
                            break;
                        }
                        wake();
                    }
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!(
                            "[lntrn-code] lsp read error ({:?}): {e}",
                            server_id
                        );
                        break;
                    }
                }
            }
        });

        // Drain stderr to a log file under ~/.lantern/log so we can see
        // what rust-analyzer is complaining about without it filling the
        // terminal we launched from.
        let log_path = log_path_for(cfg.id);
        let stderr_handle = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let mut writer: Option<std::fs::File> = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .ok();
            let r = BufReader::new(stderr);
            for line in r.lines().flatten() {
                if let Some(w) = writer.as_mut() {
                    let _ = writeln!(w, "{line}");
                }
            }
        });

        let root_uri = path_to_uri(root_path);
        let mut client = Self {
            id: cfg.id,
            root_uri: root_uri.clone(),
            language_id: cfg.language_id,
            child,
            stdin: BufWriter::new(stdin),
            _reader: reader,
            _stderr: stderr_handle,
            next_id: 1,
            pending: HashMap::new(),
            ready: false,
            open_docs: HashMap::new(),
            pending_opens: Vec::new(),
            pending_changes: Vec::new(),
        };

        client.send_initialize(&root_uri)?;
        Ok(client)
    }

    fn send_initialize(&mut self, root_uri: &str) -> std::io::Result<()> {
        let caps = serde_json::json!({
            "textDocument": {
                "synchronization": {
                    "dynamicRegistration": false,
                    "willSave": false,
                    "willSaveWaitUntil": false,
                    "didSave": true,
                },
                "publishDiagnostics": {
                    "relatedInformation": false,
                    "versionSupport": true,
                },
                "hover": {
                    "contentFormat": ["plaintext", "markdown"],
                },
                "completion": {
                    "completionItem": {
                        "snippetSupport": false,
                        "deprecatedSupport": false,
                    },
                    "contextSupport": true,
                },
                "definition": {
                    "linkSupport": false,
                },
            },
            "window": {
                "workDoneProgress": false,
            },
        });
        // Derive a friendly folder name from the URI's last path segment.
        let folder_name = root_uri
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("workspace")
            .to_string();
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.to_string()),
            workspace_folders: Some(vec![super::protocol::WorkspaceFolder {
                uri: root_uri.to_string(),
                name: folder_name,
            }]),
            capabilities: caps,
            client_info: ClientInfo { name: "lntrn-code", version: env!("CARGO_PKG_VERSION") },
            trace: "off",
        };
        let id = self.send_request("initialize", &params)?;
        self.pending.insert(id, PendingKind::Initialize);
        Ok(())
    }

    /// Called from the main loop when the `initialize` response arrives.
    pub fn on_initialized(&mut self) -> std::io::Result<()> {
        self.send_notification_raw("initialized", &serde_json::json!({}))?;
        self.ready = true;
        // Flush any docs that were queued before the server was ready.
        let opens = std::mem::take(&mut self.pending_opens);
        for (uri, text) in opens {
            self.send_did_open(&uri, &text)?;
        }
        let changes = std::mem::take(&mut self.pending_changes);
        for (uri, text) in changes {
            self.send_did_change(&uri, &text)?;
        }
        Ok(())
    }

    // ── High-level helpers ────────────────────────────────────────────

    pub fn did_open(&mut self, uri: &str, text: &str) -> std::io::Result<()> {
        if !self.ready {
            self.pending_opens.push((uri.to_string(), text.to_string()));
            return Ok(());
        }
        self.send_did_open(uri, text)
    }

    fn send_did_open(&mut self, uri: &str, text: &str) -> std::io::Result<()> {
        let params = DidOpenParams {
            text_document: TextDocumentItem {
                uri: uri.to_string(),
                language_id: self.language_id,
                version: 1,
                text: text.to_string(),
            },
        };
        self.open_docs.insert(uri.to_string(), 1);
        self.send_notification("textDocument/didOpen", &params)
    }

    pub fn did_change(&mut self, uri: &str, text: &str) -> std::io::Result<()> {
        if !self.ready {
            // Collapse duplicate opens/changes while buffering.
            self.pending_changes.retain(|(u, _)| u != uri);
            self.pending_changes.push((uri.to_string(), text.to_string()));
            return Ok(());
        }
        self.send_did_change(uri, text)
    }

    fn send_did_change(&mut self, uri: &str, text: &str) -> std::io::Result<()> {
        let version = self.open_docs.entry(uri.to_string()).or_insert(1);
        *version += 1;
        let params = DidChangeParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.to_string(),
                version: *version,
            },
            content_changes: vec![TextDocumentContentChangeEvent { text: text.to_string() }],
        };
        self.send_notification("textDocument/didChange", &params)
    }

    pub fn did_save(&mut self, uri: &str, text: Option<&str>) -> std::io::Result<()> {
        if !self.ready {
            return Ok(());
        }
        let params = DidSaveParams {
            text_document: TextDocumentIdentifier { uri: uri.to_string() },
            text: text.map(|s| s.to_string()),
        };
        self.send_notification("textDocument/didSave", &params)
    }

    pub fn did_close(&mut self, uri: &str) -> std::io::Result<()> {
        self.open_docs.remove(uri);
        if !self.ready {
            self.pending_opens.retain(|(u, _)| u != uri);
            self.pending_changes.retain(|(u, _)| u != uri);
            return Ok(());
        }
        let params = DidCloseParams {
            text_document: TextDocumentIdentifier { uri: uri.to_string() },
        };
        self.send_notification("textDocument/didClose", &params)
    }

    // ── Request senders ──────────────────────────────────────────────

    pub fn send_request<P: Serialize>(
        &mut self,
        method: &str,
        params: &P,
    ) -> std::io::Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let req = Request { jsonrpc: "2.0", id, method, params };
        let body = serde_json::to_vec(&req)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        framing::write_message(&mut self.stdin, &body)?;
        Ok(id)
    }

    pub fn send_notification<P: Serialize>(
        &mut self,
        method: &str,
        params: &P,
    ) -> std::io::Result<()> {
        let note = Notification { jsonrpc: "2.0", method, params };
        let body = serde_json::to_vec(&note)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        framing::write_message(&mut self.stdin, &body)?;
        Ok(())
    }

    fn send_notification_raw(&mut self, method: &str, params: &Value) -> std::io::Result<()> {
        let body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))?;
        framing::write_message(&mut self.stdin, &body)
    }

    pub fn shutdown(&mut self) {
        let _ = self.send_request("shutdown", &serde_json::json!(null));
        let _ = self.send_notification_raw("exit", &serde_json::json!(null));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn parse_incoming(bytes: &[u8]) -> ServerMessage {
    let Ok(raw) = serde_json::from_slice::<Incoming>(bytes) else {
        return ServerMessage::Unknown;
    };

    // Response?
    if let (Some(Value::Number(n)), true) = (&raw.id, raw.method.is_none()) {
        if let Some(id) = n.as_u64() {
            return ServerMessage::Response {
                id,
                result: raw.result,
                error: raw.error.map(|e| (e.code, e.message)),
            };
        }
    }

    // Notification / server-to-client request (we ignore server requests).
    if let Some(method) = raw.method {
        match method.as_str() {
            "textDocument/publishDiagnostics" => {
                if let Some(params) = raw.params {
                    if let Ok(d) = serde_json::from_value(params) {
                        return ServerMessage::Diagnostics(d);
                    }
                }
            }
            "window/logMessage" | "window/showMessage" => {
                if let Some(params) = raw.params {
                    let text = params
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    return ServerMessage::LogMessage(text);
                }
            }
            _ => {}
        }
    }

    ServerMessage::Unknown
}

pub fn path_to_uri(path: &Path) -> String {
    // rudimentary file:// encoder — handles spaces and a few reserved chars.
    let mut s = String::from("file://");
    for b in path.as_os_str().to_string_lossy().bytes() {
        match b {
            b'/' | b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                s.push(b as char)
            }
            _ => s.push_str(&format!("%{:02X}", b)),
        }
    }
    s
}

pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let body = uri.strip_prefix("file://")?;
    let mut out = Vec::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(std::path::PathBuf::from)
}

fn log_path_for(id: ServerId) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = std::path::PathBuf::from(home).join(".lantern/log");
    let _ = std::fs::create_dir_all(&dir);
    let name = match id {
        ServerId::Rust => "lntrn-code-lsp-rust.log",
        ServerId::Python => "lntrn-code-lsp-python.log",
        ServerId::TypeScript => "lntrn-code-lsp-ts.log",
    };
    dir.join(name)
}
