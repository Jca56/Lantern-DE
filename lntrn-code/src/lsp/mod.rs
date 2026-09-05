//! Language servers for the project: one per language that has one on
//! the machine ([`server`]), spoken to over stdio ([`client`],
//! [`framing`]) in our own JSON, with their diagnostics kept by file and
//! their answers (hover, definition, completion, rename, references,
//! code actions, signature help, formatting) handed to the app as
//! events. Columns go through [`pos`] because servers count UTF-16 units;
//! what they want changed is applied through [`edits`].

pub mod client;
pub mod edits;
pub mod framing;
mod glue;
mod parse;
pub mod pos;
mod server;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lntrn_app::Waker;

use self::server::{Server, State};
use crate::buffer::{Pos, Range};
use crate::doc::Doc;
use crate::json::Json;
use crate::problems::{LspSpan, Problem, Severity};
use crate::syntax::Language;

#[derive(Clone, Debug, PartialEq)]
pub struct LspDiag {
    pub span: LspSpan,
    pub severity: Severity,
    pub message: String,
    pub source: String,
    /// As the server sent it, handed back when asking for its fixes.
    pub raw: Json,
}

/// What a workspace edit does to one file, in the server's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Edits(PathBuf, Vec<TextEdit>),
    Create(PathBuf),
    Rename(PathBuf, PathBuf),
    Delete(PathBuf),
}

/// Changes across files, as a rename or a code action makes them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceEdit {
    pub changes: Vec<Change>,
    /// The columns are UTF-16 units.
    pub utf16: bool,
}

/// A place in a file, in the server's columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loc {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

/// A fix or refactoring the server offers.
#[derive(Clone, Debug)]
pub struct CodeAction {
    pub title: String,
    pub preferred: bool,
    pub edit: Option<WorkspaceEdit>,
    /// A command to run on the server instead: name and arguments.
    pub command: Option<(String, Json)>,
}

/// What an action asks of the server for the focused document.
pub enum Ask {
    Rename(String),
    References,
    CodeActions,
    Signature,
    Format { then_save: bool },
}

/// The signature being typed: its label, the active parameter's byte
/// range in it, its first line of documentation, and which of how many
/// overloads it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureHelp {
    pub label: String,
    pub active: Option<(usize, usize)>,
    pub doc: Option<String>,
    pub index: usize,
    pub count: usize,
}

/// A replacement in the server's columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
    /// LSP `CompletionItemKind` (2 method, 3 function, 5 field, 6 variable, 7 class, 22 struct...).
    pub kind: u32,
    /// What to type when there is no edit.
    pub insert: String,
    pub edit: Option<TextEdit>,
    /// Edits that come with it (an import, say).
    pub extra: Vec<TextEdit>,
    pub filter: String,
    pub sort: String,
}

pub enum Event {
    Hover { path: PathBuf, pos: Pos, text: String },
    Definition { path: PathBuf, line: usize, col: usize, end_line: usize, end_col: usize, utf16: bool },
    Completion { path: PathBuf, pos: Pos, items: Vec<CompletionItem> },
    Rename(WorkspaceEdit),
    References { name: String, locs: Vec<Loc>, utf16: bool },
    CodeActions { path: PathBuf, actions: Vec<CodeAction> },
    Signature { path: PathBuf, pos: Pos, help: Option<SignatureHelp> },
    Formatted { path: PathBuf, edits: Vec<TextEdit>, utf16: bool, then_save: bool },
    /// The server asks for changes of its own (after a command ran).
    ApplyEdit(WorkspaceEdit),
    /// Something to tell the user.
    Message(String),
}

#[derive(Default)]
pub struct Lsp {
    root: Option<PathBuf>,
    servers: Vec<Server>,
    pub diags: HashMap<PathBuf, Vec<LspDiag>>,
    /// Bumped when diagnostics change.
    pub version: u64,
    waker: Option<Waker>,
}

impl Lsp {
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// The project changed: servers start over for the new root.
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if self.root == root {
            return;
        }
        self.root = root;
        self.servers.clear();
        self.diags.clear();
        self.version += 1;
    }

    fn server_mut(&mut self, lang: Language) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.lang == lang)
    }

    pub fn utf16(&self, lang: Language) -> bool {
        self.servers.iter().find(|s| s.lang == lang).is_none_or(|s| s.utf16)
    }

    /// Whether `lang` has a running server.
    pub fn serves(&self, lang: Language) -> bool {
        self.servers.iter().any(|s| s.lang == lang && s.serving())
    }

    /// Start servers for the languages on screen and keep their
    /// documents in step.
    pub fn sync(&mut self, docs: &[Doc]) {
        let Some(root) = self.root.clone() else {
            return;
        };
        for d in docs {
            let lang = d.lang();
            if d.path.is_some()
                && server::has_server(lang)
                && !self.servers.iter().any(|s| s.lang == lang)
                && let Some(s) = Server::spawn(lang, &root, self.waker.clone())
            {
                self.servers.push(s);
            }
        }
        for s in &mut self.servers {
            s.sync(docs);
        }
    }

    pub fn hover(&mut self, doc: &Doc, pos: Pos) {
        if let (Some(path), Some(s)) = (doc.path.clone(), self.server_mut(doc.lang())) {
            s.hover(&path, doc.line(pos.line), pos);
        }
    }

    pub fn definition(&mut self, doc: &Doc, pos: Pos) {
        if let (Some(path), Some(s)) = (doc.path.clone(), self.server_mut(doc.lang())) {
            s.definition(&path, doc.line(pos.line), pos);
        }
    }

    pub fn complete(&mut self, doc: &Doc, pos: Pos, trigger: Option<char>) {
        if let (Some(path), Some(s)) = (doc.path.clone(), self.server_mut(doc.lang())) {
            s.complete(&path, doc.line(pos.line), pos, trigger);
        }
    }

    pub fn rename(&mut self, doc: &Doc, pos: Pos, new_name: &str) {
        if let (Some(path), Some(s)) = (doc.path.clone(), self.server_mut(doc.lang())) {
            s.rename(&path, doc.line(pos.line), pos, new_name);
        }
    }

    /// Every use of the symbol at `pos`; `name` labels the answer.
    pub fn references(&mut self, doc: &Doc, pos: Pos, name: &str) {
        if let (Some(path), Some(s)) = (doc.path.clone(), self.server_mut(doc.lang())) {
            s.references(&path, doc.line(pos.line), pos, name);
        }
    }

    /// The fixes for `range`, with the diagnostics there handed back so
    /// the server can attach their quick fixes.
    pub fn code_actions(&mut self, doc: &Doc, range: Range) {
        let Some(path) = doc.path.clone() else {
            return;
        };
        let raws: Vec<Json> = self.diags.get(&path).map(|list| list.iter().filter(|d| d.span.line <= range.end.line && d.span.end_line >= range.start.line).map(|d| d.raw.clone()).collect()).unwrap_or_default();
        if let Some(s) = self.server_mut(doc.lang()) {
            s.code_actions(&path, (range.start, doc.line(range.start.line)), (range.end, doc.line(range.end.line)), raws);
        }
    }

    pub fn signature(&mut self, doc: &Doc, pos: Pos, trigger: Option<char>, retrigger: bool) {
        if let (Some(path), Some(s)) = (doc.path.clone(), self.server_mut(doc.lang())) {
            s.signature(&path, doc.line(pos.line), pos, trigger, retrigger);
        }
    }

    /// Format the whole document; `then_save` saves it once the edits are in.
    pub fn format(&mut self, doc: &Doc, tab: usize, spaces: bool, then_save: bool) {
        if let (Some(path), Some(s)) = (doc.path.clone(), self.server_mut(doc.lang())) {
            s.format(&path, tab, spaces, then_save);
        }
    }

    /// Run a server command (a code action without an edit of its own).
    pub fn execute(&mut self, lang: Language, command: &str, args: Json) {
        if let Some(s) = self.server_mut(lang) {
            s.execute(command, args);
        }
    }

    /// Take in what the servers sent. Returns whether diagnostics
    /// changed, and the answers to hand on.
    pub fn poll(&mut self) -> (bool, Vec<Event>) {
        let mut events = Vec::new();
        let mut changed = false;
        for s in &mut self.servers {
            changed |= s.pump(&mut self.diags, &mut events);
        }
        if changed {
            self.version += 1;
        }
        (changed, events)
    }

    /// A line for the status bar: what a server is doing, or what went
    /// wrong.
    pub fn status(&self) -> Option<String> {
        for s in &self.servers {
            match &s.state {
                State::Starting => return Some(format!("{} starting", s.name)),
                State::Failed(e) => return Some(e.clone()),
                State::Gone => return Some(format!("{} stopped", s.name)),
                State::Ready => {
                    if let Some(p) = s.progress() {
                        return Some(format!("{} {p}", s.name));
                    }
                    if let Some(h) = &s.health {
                        return Some(format!("{} {h}", s.name));
                    }
                }
            }
        }
        None
    }

    /// The servers' diagnostics as problems; `char_col` turns a span into
    /// a 1-based character column when the file's text is at hand.
    pub fn problems(&self, shown: impl Fn(&Path) -> String, char_col: impl Fn(&Path, &LspSpan) -> Option<usize>) -> Vec<Problem> {
        let mut out = Vec::new();
        for (path, list) in &self.diags {
            for d in list {
                let col = char_col(path, &d.span).unwrap_or(d.span.col + 1);
                out.push(Problem { severity: d.severity, message: d.message.clone(), source: d.source.clone(), path: Some(path.clone()), shown: shown(path), line: d.span.line + 1, col, span: Some(d.span) });
            }
        }
        out.sort_by(|a, b| a.shown.cmp(&b.shown).then(a.line.cmp(&b.line)).then(a.col.cmp(&b.col)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::DocId;

    /// rust-analyzer on a throwaway crate: the handshake, a diagnostic
    /// for a type error, a hover and a definition.
    #[test]
    #[ignore = "needs rust-analyzer on the path and a few seconds"]
    fn rust_analyzer_round_trip() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-lsp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        let text = "fn helper() -> u32 { 1 }\nfn main() {\n    let x: u32 = \"a\";\n    let y = helper();\n}\n";
        std::fs::write(dir.join("src/main.rs"), text).unwrap();
        let path = std::fs::canonicalize(dir.join("src/main.rs")).unwrap();
        let doc = Doc::from_text(DocId(1), Some(path.clone()), text, 4);
        let mut lsp = Lsp::default();
        lsp.set_root(Some(std::fs::canonicalize(&dir).unwrap()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut hovered = None;
        let mut defined = None;
        let mut asked = false;
        let mut last_print = std::time::Instant::now();
        loop {
            if last_print.elapsed().as_secs() >= 5 {
                last_print = std::time::Instant::now();
                eprintln!("[test] status={:?} servers={} diags={:?}", lsp.status(), lsp.servers.len(), lsp.diags.keys().collect::<Vec<_>>());
            }
            assert!(std::time::Instant::now() < deadline, "timed out; status {:?}", lsp.status());
            lsp.sync(std::slice::from_ref(&doc));
            let (_, events) = lsp.poll();
            for e in events {
                match e {
                    Event::Hover { text, .. } => hovered = Some(text),
                    Event::Definition { path, line, .. } => defined = Some((path, line)),
                    _ => {}
                }
            }
            let has_error = lsp.diags.get(&path).is_some_and(|l| l.iter().any(|d| d.severity == Severity::Error && d.span.line == 2));
            if has_error && !asked && lsp.serves(Language::Rust) {
                lsp.hover(&doc, Pos::new(1, 3));
                lsp.definition(&doc, Pos::new(3, 13));
                asked = true;
            }
            if hovered.is_some() && defined.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(hovered.unwrap().contains("fn main"), "hover names the function");
        assert_eq!(defined.unwrap(), (path.clone(), 0), "helper is defined on the first line");
        let problems = lsp.problems(|p| p.display().to_string(), |_, _| None);
        assert!(problems.iter().any(|p| p.line == 3 && p.severity == Severity::Error), "{problems:?}");
        // Round two: references, formatting, signature help, code
        // actions and a rename, all in flight at once.
        lsp.references(&doc, Pos::new(0, 3), "helper");
        lsp.format(&doc, 4, true, false);
        lsp.signature(&doc, Pos::new(3, 19), Some('('), false);
        lsp.code_actions(&doc, Range::new(Pos::new(2, 4), Pos::new(2, 4)));
        lsp.rename(&doc, Pos::new(0, 3), "helper2");
        let (mut refs, mut formatted, mut signature, mut actions, mut renamed) = (None, None, None, None, None);
        while refs.is_none() || formatted.is_none() || signature.is_none() || actions.is_none() || renamed.is_none() {
            assert!(std::time::Instant::now() < deadline, "round two timed out: refs {} fmt {} sig {} act {} ren {}", refs.is_some(), formatted.is_some(), signature.is_some(), actions.is_some(), renamed.is_some());
            lsp.sync(std::slice::from_ref(&doc));
            for e in lsp.poll().1 {
                match e {
                    Event::References { locs, .. } => refs = Some(locs),
                    Event::Formatted { edits, .. } => formatted = Some(edits),
                    Event::Signature { help, .. } => signature = Some(help),
                    Event::CodeActions { actions: a, .. } => actions = Some(a),
                    Event::Rename(edit) => renamed = Some(edit),
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(refs.unwrap().len(), 2, "the definition and the call");
        assert!(!formatted.unwrap().is_empty(), "rustfmt spreads the one-line body");
        assert!(signature.unwrap().is_some_and(|s| s.label.contains("helper")), "the call's signature");
        let edit = renamed.unwrap();
        let edits: usize = edit.changes.iter().map(|c| if let Change::Edits(_, e) = c { e.len() } else { 0 }).sum();
        assert_eq!(edits, 2, "both mentions renamed: {edit:?}");
        let _ = actions.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
