//! What the app does for Claude Code: answering the bridge's tool calls,
//! showing proposed edits as diffs to accept or reject, and telling the
//! CLI what is selected.

use std::path::{Path, PathBuf};

use lntrn_app::Waker;
use lntrn_core::log_warn;
use lntrn_ui::{Shell, ShellRequest};

use crate::app::{App, Editor};
use crate::bridge::{Bridge, ClientId, Incoming, tools};
use crate::buffer::{Pos, Range};
use crate::diff_view::{DiffDoc, DiffId};
use crate::json::Json;
use crate::obj;
use crate::term::diag::Severity;

/// A selection to make once a file the CLI asked for is open.
pub struct PendingSelect {
    pub path: PathBuf,
    pub start_text: String,
    pub end_text: String,
    pub to_line_end: bool,
}

/// Selected text is sent whole up to this size.
const SELECTION_CAP: usize = 1_000_000;

fn file_url(p: &Path) -> String {
    format!("file://{}", p.display())
}

fn char_col(line: &str, byte: usize) -> usize {
    line[..byte.min(line.len())].chars().count()
}

impl App {
    fn abs_path(&self, p: &str) -> PathBuf {
        let path = PathBuf::from(p);
        if path.is_absolute() { path } else { self.base_dir().join(path) }
    }

    pub fn ide_start(&mut self, waker: Waker) {
        match Bridge::start(self.project.as_ref().map(|p| p.root.as_path()), Some(waker)) {
            Ok(b) => self.bridge = Some(b),
            Err(e) => log_warn!("ide bridge: not started: {e}"),
        }
    }

    /// The lock file lists the project and every folder a terminal is in,
    /// so `claude` started in any of them is at home here.
    pub fn ide_sync_roots(&mut self) {
        let mut roots: Vec<PathBuf> = self.project.iter().map(|p| p.root.clone()).collect();
        for t in &self.terminals {
            if let Some(cwd) = t.cwd_now()
                && !roots.contains(&cwd)
            {
                roots.push(cwd);
            }
        }
        if let Some(b) = self.bridge.as_mut() {
            b.set_roots(roots);
        }
    }

    /// The focused document's selection in the CLI's words.
    fn selection_json(&self) -> Json {
        let Some(d) = self.focus_doc().filter(|d| d.path.is_some()) else {
            return obj! { "success" => false, "message" => "No active editor" };
        };
        let path = d.path.as_deref().unwrap_or(Path::new(""));
        let sel = d.selection();
        let mut text = d.selected_text();
        if text.len() > SELECTION_CAP {
            let mut cut = SELECTION_CAP;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        let pos = |p: Pos| obj! { "line" => p.line, "character" => char_col(d.line(p.line), p.col) };
        obj! {
            "success" => true,
            "text" => text,
            "filePath" => path.display().to_string(),
            "fileUrl" => file_url(path),
            "selection" => obj! { "start" => pos(sel.start), "end" => pos(sel.end), "isEmpty" => sel.is_empty() },
        }
    }

    /// After every rebuild: tool calls in, the selection out.
    pub fn ide_pump(&mut self, shell: &mut Shell<Self>) -> bool {
        let Some(bridge) = self.bridge.take() else {
            return false;
        };
        let mut again = false;
        for m in bridge.poll() {
            again = true;
            match m {
                Incoming::Connected => {
                    self.ide_connected = bridge.connected();
                    self.last_selection_sent.clear();
                    shell.request(self, ShellRequest::Toast("Claude Code connected".into()));
                }
                Incoming::Disconnected => {
                    self.ide_connected = bridge.connected();
                    shell.request(self, ShellRequest::Toast("Claude Code disconnected".into()));
                }
                Incoming::Call { client, id, name, args } => self.ide_call(&bridge, client, &id, &name, &args),
            }
        }
        if self.ide_connected > 0 {
            let sel = self.selection_json();
            if sel.get("success").and_then(Json::bool) == Some(true) {
                let key = sel.to_text();
                if key != self.last_selection_sent {
                    self.last_selection_sent = key;
                    let params = Json::Obj(match sel {
                        Json::Obj(pairs) => pairs.into_iter().filter(|(k, _)| k != "success").collect(),
                        _ => Vec::new(),
                    });
                    bridge.notify("selection_changed", params);
                }
            }
        }
        self.bridge = Some(bridge);
        again
    }

    fn ide_call(&mut self, bridge: &Bridge, client: ClientId, id: &Json, name: &str, args: &Json) {
        let now = self.clock();
        let result = match name {
            "openFile" => {
                let path = self.abs_path(args.field_str("filePath"));
                let front = args.get("makeFrontmost").and_then(Json::bool).unwrap_or(true);
                self.pending_paths.push(path.clone());
                let start = args.field_str("startText");
                if !start.is_empty() {
                    self.pending_select = Some(PendingSelect { path: path.clone(), start_text: start.to_owned(), end_text: args.field_str("endText").to_owned(), to_line_end: args.get("selectToEndOfLine").and_then(Json::bool).unwrap_or(false) });
                }
                if front { tools::text(&format!("Opened file: {}", path.display())) } else { tools::json_text(&obj! { "success" => true, "filePath" => path.display().to_string() }) }
            }
            "openDiff" => {
                self.open_diff(client, id.clone(), args);
                return;
            }
            "getCurrentSelection" | "getLatestSelection" => tools::json_text(&self.selection_json()),
            "getOpenEditors" => {
                let tabs: Vec<Json> = self
                    .docs
                    .iter()
                    .filter_map(|d| {
                        let p = d.path.as_ref()?;
                        Some(obj! { "uri" => file_url(p), "isActive" => self.focus_doc == Some(d.id), "label" => d.title.as_str(), "languageId" => d.lang().name().to_lowercase(), "isDirty" => d.is_dirty() })
                    })
                    .collect();
                tools::json_text(&obj! { "tabs" => tabs })
            }
            "getWorkspaceFolders" => {
                let folders: Vec<Json> = bridge.roots().iter().map(|r| obj! { "name" => r.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), "uri" => file_url(r), "path" => r.display().to_string() }).collect();
                let root = bridge.roots().first().map(|r| r.display().to_string());
                tools::json_text(&obj! { "success" => true, "folders" => folders, "rootPath" => root })
            }
            "getDiagnostics" => {
                let only = args.get("uri").and_then(Json::str).map(|u| PathBuf::from(u.strip_prefix("file://").unwrap_or(u)));
                tools::json_text(&self.diagnostics_json(only.as_deref()))
            }
            "checkDocumentDirty" => {
                let path = self.abs_path(args.field_str("filePath"));
                match self.doc_by_path(&path) {
                    Some(i) => tools::json_text(&obj! { "success" => true, "filePath" => path.display().to_string(), "isDirty" => self.docs[i].is_dirty(), "isUntitled" => false }),
                    None => tools::json_text(&obj! { "success" => false, "message" => format!("Document not open: {}", path.display()) }),
                }
            }
            "saveDocument" => {
                let path = self.abs_path(args.field_str("filePath"));
                let trim = self.settings.trim_on_save;
                match self.doc_by_path(&path) {
                    Some(i) => match self.docs[i].save(trim, now) {
                        Ok(()) => tools::json_text(&obj! { "success" => true, "filePath" => path.display().to_string(), "saved" => true, "message" => "Document saved successfully" }),
                        Err(e) => tools::json_text(&obj! { "success" => false, "message" => e.to_string() }),
                    },
                    None => tools::json_text(&obj! { "success" => false, "message" => format!("Document not open: {}", path.display()) }),
                }
            }
            "close_tab" => {
                let tab = args.field_str("tab_name");
                if let Some(d) = self.diffs.iter().find(|d| d.tab_name == tab) {
                    self.pending_diff_resolve.push((d.id, false));
                }
                tools::text("TAB_CLOSED")
            }
            "closeAllDiffTabs" => {
                let n = self.diffs.len();
                let ids: Vec<DiffId> = self.diffs.iter().map(|d| d.id).collect();
                self.pending_diff_resolve.extend(ids.into_iter().map(|i| (i, false)));
                tools::text(&format!("CLOSED_{n}_DIFF_TABS"))
            }
            other => tools::error(&format!("Unknown tool: {other}")),
        };
        bridge.respond(client, id, result);
    }

    /// The problems the terminals found, per file, in the IDE's shape:
    /// `[{uri, diagnostics: [{message, severity, range, source}]}]`.
    pub fn diagnostics_json(&self, only: Option<&Path>) -> Json {
        let mut files: Vec<(PathBuf, Vec<Json>)> = Vec::new();
        for d in self.diagnostics() {
            let Some(p) = &d.resolved else {
                continue;
            };
            if only.is_some_and(|o| o != p) {
                continue;
            }
            let severity = match d.severity {
                Severity::Error => "Error",
                Severity::Warning => "Warning",
            };
            let pos = obj! { "line" => d.line.saturating_sub(1), "character" => d.col.saturating_sub(1) };
            let entry = obj! { "message" => d.message.as_str(), "severity" => severity, "range" => obj! { "start" => pos.clone(), "end" => pos }, "source" => "terminal" };
            match files.iter_mut().find(|(f, _)| f == p) {
                Some((_, list)) => list.push(entry),
                None => files.push((p.clone(), vec![entry])),
            }
        }
        Json::Arr(files.into_iter().map(|(p, list)| obj! { "uri" => file_url(&p), "diagnostics" => list }).collect())
    }

    /// A proposed edit: shown as a diff; the CLI hears back when the user
    /// decides.
    fn open_diff(&mut self, client: ClientId, id: Json, args: &Json) {
        let old_path = self.abs_path(args.field_str("old_file_path"));
        let new_path = self.abs_path(args.field_str("new_file_path"));
        let new_text = args.field_str("new_file_contents").to_owned();
        let tab = args.field_str("tab_name").to_owned();
        let old_text = self.doc_by_path(&old_path).map(|i| self.docs[i].buffer.to_text()).or_else(|| std::fs::read_to_string(&old_path).ok()).unwrap_or_default();
        let did = DiffId(self.next_diff);
        self.next_diff += 1;
        self.diffs.push(DiffDoc::new(did, &tab, &new_path, &old_text, new_text, Some((client, id))));
        self.pending_show_diff = Some(did);
    }

    /// The user (or the CLI) decided: answer the CLI and take the diff off
    /// the screen. On accept the CLI writes the file itself once it hears
    /// `FILE_SAVED` (writing it first trips its own freshness check); the
    /// editor shows the new text at once, and if the file has not changed
    /// on disk shortly after, the editor writes it after all.
    pub fn ide_resolve(&mut self, shell: &mut Shell<Self>, did: DiffId, accept: bool) {
        let Some(i) = self.diffs.iter().position(|d| d.id == did) else {
            return;
        };
        let d = self.diffs.remove(i);
        let now = shell.state.now;
        let name = d.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let result = if accept {
            if let Some(j) = self.doc_by_path(&d.path) {
                self.docs[j].replace_all(&d.new_text, now);
            }
            self.pending_writes.push((d.path.clone(), d.new_text.clone(), std::time::Instant::now() + std::time::Duration::from_secs(2)));
            if let Some(w) = self.waker.clone() {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(2200));
                    w.wake();
                });
            }
            shell.request(self, ShellRequest::Toast(format!("Accepted {name}")));
            tools::texts(&["FILE_SAVED", &d.new_text])
        } else {
            tools::texts(&["DIFF_REJECTED", &d.tab_name])
        };
        if let (Some(b), Some((client, id))) = (&self.bridge, d.pending) {
            b.respond(client, &id, result);
        }
        self.close_diff_tab(shell, did);
        if self.focus_diff == Some(did) {
            self.focus_diff = None;
        }
    }

    /// Accepted diffs the CLI was to write: those still not on disk when
    /// their time is up are written here.
    pub fn ide_settle_writes(&mut self, shell: &mut Shell<Self>) -> bool {
        if self.pending_writes.is_empty() {
            return false;
        }
        let now = std::time::Instant::now();
        let mut again = false;
        let mut keep = Vec::new();
        for (path, text, due) in std::mem::take(&mut self.pending_writes) {
            let on_disk = std::fs::read(&path).map(|b| String::from_utf8_lossy(&b).into_owned()).ok();
            if on_disk.as_deref() == Some(text.as_str()) {
                again = true;
                continue;
            }
            if now < due {
                keep.push((path, text, due));
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let written = path.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|()| std::fs::write(&path, &text));
            match written {
                Ok(()) => {
                    if let Some(j) = self.doc_by_path(&path) {
                        self.docs[j].replace_all(&text, shell.state.now);
                    }
                    if let Some(p) = self.project.as_mut() {
                        p.refresh();
                    }
                    shell.request(self, ShellRequest::Toast(format!("Wrote {name}")));
                }
                Err(e) => {
                    shell.request(self, ShellRequest::Toast(format!("Could not write {name}: {e}")));
                }
            }
            again = true;
        }
        self.pending_writes = keep;
        again
    }

    /// Put a new diff on screen: in the Diff area if there is one, else as
    /// a tab beside the code.
    pub fn show_diff(&mut self, shell: &mut Shell<Self>, did: DiffId) {
        let area = match shell.screen.target(Editor::Diff) {
            Some(a) => a,
            None => {
                let a = self.focus_area.or(shell.screen.active).or_else(|| shell.screen.area_ids().next()).unwrap_or(0);
                shell.screen.add_tab(a, Editor::Diff);
                a
            }
        };
        if let Some(ar) = shell.screen.area_mut(area) {
            ar.state_mut().diff = Some(did);
        }
        shell.screen.active = Some(area);
        self.focus_diff = Some(did);
    }

    fn close_diff_tab(&mut self, shell: &mut Shell<Self>, did: DiffId) {
        for a in shell.screen.area_ids().collect::<Vec<_>>() {
            let Some(ar) = shell.screen.area_mut(a) else {
                continue;
            };
            let Some(i) = ar.tabs.iter().position(|t| t.state.diff == Some(did)) else {
                continue;
            };
            if ar.tabs.len() > 1 {
                ar.current = i;
                shell.screen.close_tab(a);
            } else {
                ar.tabs[i].state.diff = None;
                ar.tabs[i].editor = Editor::Code;
            }
        }
    }

    /// Tell the CLI about the selection as an `@file#L1-2` mention.
    pub fn ide_send_selection(&mut self, shell: &mut Shell<Self>) {
        let Some(d) = self.focus_doc().filter(|d| d.path.is_some()) else {
            shell.request(self, ShellRequest::Toast("No file is focused".into()));
            return;
        };
        let path = d.path.clone().unwrap_or_default();
        let sel = d.selection();
        let (ls, le) = if sel.is_empty() {
            (d.cursor.line, d.cursor.line)
        } else {
            (sel.start.line, if sel.end.col == 0 && sel.end.line > sel.start.line { sel.end.line - 1 } else { sel.end.line })
        };
        let Some(b) = &self.bridge else {
            return;
        };
        if b.connected() == 0 {
            shell.request(self, ShellRequest::Toast("Claude Code is not connected. Run `claude` in the terminal.".into()));
            return;
        }
        b.notify("at_mentioned", obj! { "filePath" => path.display().to_string(), "lineStart" => ls, "lineEnd" => le });
        let msg = format!("Sent {}:{}-{} to Claude", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), ls + 1, le + 1);
        shell.request(self, ShellRequest::Toast(msg));
    }

    /// A selection the CLI asked for with `openFile`, once the file is open.
    pub fn apply_pending_select(&mut self) {
        let Some(ps) = self.pending_select.take() else {
            return;
        };
        let Some(i) = self.doc_by_path(&ps.path) else {
            return;
        };
        let doc = &mut self.docs[i];
        let lines = doc.buffer.lines();
        let Some((sl, sc)) = lines.iter().enumerate().find_map(|(l, t)| t.find(&ps.start_text).map(|c| (l, c))) else {
            return;
        };
        let mut end = Pos::new(sl, sc + ps.start_text.len());
        if !ps.end_text.is_empty()
            && let Some((el, ec)) = lines.iter().enumerate().skip(sl).find_map(|(l, t)| {
                let from = if l == sl { sc } else { 0 };
                t[from..].find(&ps.end_text).map(|c| (l, from + c))
            })
        {
            end = Pos::new(el, ec + ps.end_text.len());
        }
        if ps.to_line_end {
            end.col = lines[end.line].len();
        }
        doc.select(Range::new(Pos::new(sl, sc), end));
    }
}
