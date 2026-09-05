//! One language's server: found on the path and started for the
//! project, the `initialize` handshake, the open documents kept in step
//! (full text on every change), requests with what they were for, and
//! the messages it sends on its own (diagnostics, progress, questions).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lntrn_app::Waker;

use super::client::{Client, Incoming};
use super::pos::{path_to_uri, to_units};
use super::{Event, LspDiag, parse};
use crate::buffer::Pos;
use crate::doc::Doc;
use crate::json::Json;
use crate::obj;
use crate::syntax::Language;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Starting,
    Ready,
    Failed(String),
    Gone,
}

enum Pending {
    Init,
    Hover(PathBuf, Pos),
    Definition,
    Completion(PathBuf, Pos),
}

struct Synced {
    version: i64,
    buffer: u64,
    saved: u64,
}

pub struct Server {
    pub lang: Language,
    pub name: &'static str,
    language_id: &'static str,
    client: Option<Client>,
    pub state: State,
    pub utf16: bool,
    open: HashMap<PathBuf, Synced>,
    /// Requests in flight: what for, the parameters (to ask again when
    /// the server says its content changed), and how many tries so far.
    pending: HashMap<u64, (Pending, Json, u8)>,
    progress: Vec<(String, String)>,
    /// What rust-analyzer says about itself when something is off.
    pub health: Option<String>,
    root: PathBuf,
}

/// The program for a language: name, arguments, LSP language id.
fn program(lang: Language) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    Some(match lang {
        Language::Rust => ("rust-analyzer", &[], "rust"),
        Language::Python => ("pyright-langserver", &["--stdio"], "python"),
        Language::JavaScript => ("typescript-language-server", &["--stdio"], "javascript"),
        Language::C => ("clangd", &[], "c"),
        Language::Toml => ("taplo", &["lsp", "stdio"], "toml"),
        _ => return None,
    })
}

pub fn has_server(lang: Language) -> bool {
    program(lang).is_some()
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

impl Server {
    /// Start the server for `lang` in `root`; a missing program is a
    /// server that failed.
    pub fn spawn(lang: Language, root: &Path, waker: Option<Waker>) -> Option<Self> {
        let (name, args, language_id) = program(lang)?;
        let mut s = Self { lang, name, language_id, client: None, state: State::Starting, utf16: true, open: HashMap::new(), pending: HashMap::new(), progress: Vec::new(), health: None, root: root.to_path_buf() };
        if !on_path(name) {
            s.state = State::Failed(format!("{name} is not installed"));
            return Some(s);
        }
        match Client::spawn(name, args, root, waker, name) {
            Ok(mut c) => {
                let params = s.initialize_params();
                let id = c.request("initialize", params.clone());
                s.pending.insert(id, (Pending::Init, params, 0));
                s.client = Some(c);
            }
            Err(e) => s.state = State::Failed(format!("{name}: {e}")),
        }
        Some(s)
    }

    fn initialize_params(&self) -> Json {
        let uri = path_to_uri(&self.root);
        let name = self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let options = match self.lang {
            // A target dir of its own, so its checks never wait on a build in the terminal.
            Language::Rust => obj! { "cargo" => obj! { "targetDir" => true }, "checkOnSave" => true },
            _ => Json::Null,
        };
        obj! {
            "processId" => std::process::id(),
            "clientInfo" => obj! { "name" => "lntrn-code", "version" => env!("CARGO_PKG_VERSION") },
            "rootUri" => uri.as_str(),
            "rootPath" => self.root.display().to_string(),
            "workspaceFolders" => vec![obj! { "uri" => uri.as_str(), "name" => name }],
            "initializationOptions" => options,
            "capabilities" => obj! {
                "general" => obj! { "positionEncodings" => vec![Json::from("utf-8"), Json::from("utf-16")] },
                "textDocument" => obj! {
                    "synchronization" => obj! { "didSave" => true },
                    "publishDiagnostics" => obj! { "relatedInformation" => false },
                    "hover" => obj! { "contentFormat" => vec![Json::from("markdown"), Json::from("plaintext")] },
                    "completion" => obj! { "completionItem" => obj! { "snippetSupport" => false, "insertReplaceSupport" => true }, "contextSupport" => true },
                    "definition" => obj! { "linkSupport" => true },
                },
                "window" => obj! { "workDoneProgress" => true },
                "workspace" => obj! { "workspaceFolders" => true, "configuration" => true },
            },
        }
    }

    pub fn ready(&self) -> bool {
        self.state == State::Ready && self.client.as_ref().is_some_and(Client::alive)
    }

    fn text_document(path: &Path) -> Json {
        obj! { "uri" => path_to_uri(path) }
    }

    /// Open, change, save and close notifications so the server sees the
    /// documents of its language as the editor has them.
    pub fn sync(&mut self, docs: &[Doc]) {
        if !self.ready() {
            return;
        }
        let Some(c) = self.client.as_mut() else {
            return;
        };
        let mut seen: Vec<PathBuf> = Vec::new();
        for d in docs.iter().filter(|d| d.lang() == self.lang) {
            let Some(path) = &d.path else {
                continue;
            };
            seen.push(path.clone());
            let (buffer, saved) = (d.buffer.version(), d.saved_version());
            match self.open.get_mut(path) {
                None => {
                    c.notify("textDocument/didOpen", obj! { "textDocument" => obj! { "uri" => path_to_uri(path), "languageId" => self.language_id, "version" => 1, "text" => d.buffer.to_text() } });
                    self.open.insert(path.clone(), Synced { version: 1, buffer, saved });
                }
                Some(s) => {
                    if s.buffer != buffer {
                        s.version += 1;
                        s.buffer = buffer;
                        c.notify("textDocument/didChange", obj! { "textDocument" => obj! { "uri" => path_to_uri(path), "version" => s.version }, "contentChanges" => vec![obj! { "text" => d.buffer.to_text() }] });
                    }
                    if s.saved != saved {
                        s.saved = saved;
                        c.notify("textDocument/didSave", obj! { "textDocument" => Self::text_document(path) });
                    }
                }
            }
        }
        let gone: Vec<PathBuf> = self.open.keys().filter(|p| !seen.contains(p)).cloned().collect();
        for p in gone {
            self.open.remove(&p);
            c.notify("textDocument/didClose", obj! { "textDocument" => Self::text_document(&p) });
        }
    }

    fn position_params(&self, path: &Path, line_text: &str, pos: Pos) -> Json {
        obj! { "textDocument" => Self::text_document(path), "position" => obj! { "line" => pos.line, "character" => to_units(line_text, pos.col, self.utf16) } }
    }

    pub fn hover(&mut self, path: &Path, line_text: &str, pos: Pos) {
        if !self.ready() || !self.open.contains_key(path) {
            return;
        }
        let params = self.position_params(path, line_text, pos);
        if let Some(c) = self.client.as_mut() {
            let id = c.request("textDocument/hover", params.clone());
            self.pending.insert(id, (Pending::Hover(path.to_path_buf(), pos), params, 0));
        }
    }

    pub fn definition(&mut self, path: &Path, line_text: &str, pos: Pos) {
        if !self.ready() || !self.open.contains_key(path) {
            return;
        }
        let params = self.position_params(path, line_text, pos);
        if let Some(c) = self.client.as_mut() {
            let id = c.request("textDocument/definition", params.clone());
            self.pending.insert(id, (Pending::Definition, params, 0));
        }
    }

    pub fn complete(&mut self, path: &Path, line_text: &str, pos: Pos, trigger: Option<char>) {
        if !self.ready() || !self.open.contains_key(path) {
            return;
        }
        let mut params = self.position_params(path, line_text, pos);
        let context = match trigger {
            Some(ch) => obj! { "triggerKind" => 2, "triggerCharacter" => ch.to_string() },
            None => obj! { "triggerKind" => 1 },
        };
        if let Json::Obj(pairs) = &mut params {
            pairs.push(("context".to_owned(), context));
        }
        if let Some(c) = self.client.as_mut() {
            let id = c.request("textDocument/completion", params.clone());
            self.pending.insert(id, (Pending::Completion(path.to_path_buf(), pos), params, 0));
        }
    }

    /// Whether the server answers requests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn serving(&self) -> bool {
        self.ready()
    }

    /// What the server is busy with, if anything.
    pub fn progress(&self) -> Option<&str> {
        self.progress.last().map(|(_, t)| t.as_str())
    }

    /// Take in the server's messages. Returns whether diagnostics changed.
    pub fn pump(&mut self, diags: &mut HashMap<PathBuf, Vec<LspDiag>>, events: &mut Vec<Event>) -> bool {
        let Some(c) = self.client.as_mut() else {
            return false;
        };
        let mut changed = false;
        for m in c.poll() {
            match m {
                Incoming::Exited => {
                    self.state = if self.state == State::Starting { State::Failed(format!("{} stopped during startup", self.name)) } else { State::Gone };
                    self.open.clear();
                    self.pending.clear();
                    events.push(Event::Message(format!("{} stopped", self.name)));
                }
                Incoming::Request { id, method, params } => {
                    let result = match method.as_str() {
                        "workspace/configuration" => Json::Arr(vec![Json::Null; params.get("items").and_then(Json::arr).map_or(1, <[Json]>::len)]),
                        "workspace/workspaceFolders" => vec![obj! { "uri" => path_to_uri(&self.root), "name" => self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() }].into(),
                        _ => Json::Null,
                    };
                    c.respond(&id, result);
                }
                Incoming::Notification { method, params } => match method.as_str() {
                    "textDocument/publishDiagnostics" => {
                        if let Some((path, list)) = parse::diagnostics(&params, self.utf16) {
                            if list.is_empty() {
                                diags.remove(&path);
                            } else {
                                diags.insert(path, list);
                            }
                            changed = true;
                        }
                    }
                    "$/progress" => {
                        if let Some((token, text, done)) = parse::progress(&params) {
                            self.progress.retain(|(t, _)| *t != token);
                            if !done {
                                self.progress.push((token, text));
                            }
                        }
                    }
                    "window/showMessage" => {
                        if params.get("type").and_then(Json::num).unwrap_or(4.0) <= 2.0 {
                            events.push(Event::Message(format!("{}: {}", self.name, params.field_str("message"))));
                        }
                    }
                    "experimental/serverStatus" => {
                        let health = params.field_str("health");
                        self.health = (health != "ok").then(|| format!("{}: {}", health, params.field_str("message")));
                    }
                    _ => {}
                },
                Incoming::Response { id, result, error } => {
                    let Some((p, params, tries)) = self.pending.remove(&id) else {
                        continue;
                    };
                    // "Content modified": the server moved on under the request; ask again.
                    if error.as_deref().is_some_and(|e| e.contains("content modified")) && tries < 3 {
                        let method = match &p {
                            Pending::Init => None,
                            Pending::Hover(..) => Some("textDocument/hover"),
                            Pending::Definition => Some("textDocument/definition"),
                            Pending::Completion(..) => Some("textDocument/completion"),
                        };
                        if let Some(m) = method {
                            let id = c.request(m, params.clone());
                            self.pending.insert(id, (p, params, tries + 1));
                            continue;
                        }
                    }
                    match p {
                        Pending::Init => {
                            if let Some(e) = error {
                                self.state = State::Failed(format!("{}: {e}", self.name));
                                continue;
                            }
                            self.utf16 = result.as_ref().is_none_or(parse::wants_utf16);
                            c.notify("initialized", obj! {});
                            self.state = State::Ready;
                        }
                        Pending::Hover(path, pos) => {
                            if let Some(text) = result.as_ref().and_then(parse::hover_text) {
                                events.push(Event::Hover { path, pos, text });
                            }
                        }
                        Pending::Definition => {
                            if let Some((path, line, col, end_line, end_col)) = result.as_ref().and_then(parse::definition) {
                                events.push(Event::Definition { path, line, col, end_line, end_col, utf16: self.utf16 });
                            } else {
                                events.push(Event::Message("No definition found".into()));
                            }
                        }
                        Pending::Completion(path, pos) => {
                            let items = result.as_ref().map(parse::completions).unwrap_or_default();
                            events.push(Event::Completion { path, pos, items });
                        }
                    }
                }
            }
        }
        changed
    }
}
