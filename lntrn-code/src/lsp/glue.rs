//! What the app does with the language servers: documents kept in step
//! after every rebuild, their answers put on screen (or applied), the
//! requests the actions make, and every problem from every source as
//! one list.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lntrn_props::Value;
use lntrn_ui::{Action, ContextMenu, Item, Shell, ShellRequest};

use crate::app::{App, Editor, Goto};
use crate::buffer::{Pos, Range};
use crate::commands::CODE_ACTION_PICK;
use crate::doc::DocId;
use crate::editor::input::word_range;
use crate::editor::lsp_ui::{Completion, Hover, SignaturePopup, word_start};
use crate::lsp::edits::apply_edits;
use crate::lsp::pos::from_units;
use crate::lsp::{Ask, Event, Loc};
use crate::problems::{LspSpan, Problem};
use crate::search::{FileHits, Hit, Query, preview_of};

impl App {
    /// After every rebuild: documents to the servers, answers back.
    pub(crate) fn lsp_pump(&mut self, shell: &mut Shell<Self>) -> bool {
        self.lsp.sync(&self.docs);
        let now = shell.state.now;
        let (mut again, events) = self.lsp.poll();
        for e in events {
            again = true;
            match e {
                Event::Rename(edit) => {
                    let done = self.apply_workspace_edit(edit, now);
                    shell.request(self, ShellRequest::Toast(done.summary("Renamed:")));
                }
                Event::ApplyEdit(edit) => {
                    let done = self.apply_workspace_edit(edit, now);
                    shell.request(self, ShellRequest::Toast(done.summary("Applied")));
                }
                Event::References { name, locs, utf16 } => self.show_references(&name, locs, utf16),
                Event::CodeActions { path, actions } => {
                    if actions.is_empty() {
                        shell.request(self, ShellRequest::Toast("No code actions here".into()));
                    } else if self.focus_doc().and_then(|d| d.path.as_deref()) == Some(&path) {
                        let at = self.lsp_ui.caret_screen.unwrap_or(shell.state.pointer);
                        let items: Vec<Item> = actions.iter().enumerate().map(|(i, a)| Item::action(&a.title, Action::new(CODE_ACTION_PICK).with("i", Value::I64(i as i64)))).collect();
                        self.code_actions = actions;
                        shell.request(self, ShellRequest::ContextMenu(Box::new(ContextMenu::new("Code Actions", at).tab("Fixes", items))));
                    }
                }
                Event::Signature { path, pos, help } => {
                    let doc = self.docs.iter().find(|d| d.path.as_deref() == Some(&path)).filter(|d| d.cursor.line == pos.line);
                    match (help, doc) {
                        (Some(help), Some(d)) => {
                            self.lsp_ui.sig_cursor = Some(d.cursor);
                            self.lsp_ui.signature = Some(SignaturePopup { doc: d.id, line: pos.line, help });
                        }
                        _ => self.lsp_ui.signature = None,
                    }
                }
                Event::Formatted { path, edits, utf16, then_save } => {
                    if let Some(i) = self.doc_by_path(&path) {
                        if !edits.is_empty() {
                            apply_edits(&mut self.docs[i], &edits, utf16, now);
                        }
                        if then_save {
                            let id = self.docs[i].id;
                            self.pending_saves.retain(|(d, _)| *d != id);
                            let msg = self.save_index(i, now).unwrap_or_else(|e| e);
                            shell.request(self, ShellRequest::Toast(msg));
                        }
                    }
                }
                Event::Hover { path, pos, text } => {
                    if let Some((doc, asked, anchor)) = self.lsp_ui.asked
                        && asked == pos
                        && self.docs.iter().any(|d| d.id == doc && d.path.as_deref() == Some(&path))
                    {
                        self.lsp_ui.hover = Some(Hover { doc, pos, lines: text.lines().map(str::to_owned).collect(), anchor });
                        self.lsp_ui.asked = None;
                    }
                }
                Event::Definition { path, line, col, end_line, end_col, utf16 } => {
                    let end_col = if end_line == line { end_col } else { col };
                    self.pending_paths.push(path.clone());
                    self.pending_goto = Some((path, Goto::Units { line, col, end_col, utf16 }));
                }
                Event::Completion { path, pos, items } => {
                    if let Some(d) = self.docs.iter().find(|d| d.path.as_deref() == Some(&path))
                        && d.cursor.line == pos.line
                        && d.cursor.col >= pos.col
                        && !items.is_empty()
                    {
                        let anchor = Pos::new(pos.line, word_start(d.line(pos.line), pos.col));
                        self.lsp_ui.completion = Some(Completion { doc: d.id, anchor, items, selected: 0 });
                        match self.lsp_ui.filtered(d).first().copied() {
                            Some(i) => {
                                if let Some(c) = self.lsp_ui.completion.as_mut() {
                                    c.selected = i;
                                }
                            }
                            None => self.lsp_ui.completion = None,
                        }
                    }
                }
                Event::Message(m) => {
                    shell.request(self, ShellRequest::Toast(m));
                }
            }
        }
        again
    }

    /// A request for the focused document's caret (or selection).
    pub(crate) fn lsp_ask(&mut self, ask: Ask) {
        let Some(i) = self.focus_doc.and_then(|id| self.docs.iter().position(|d| d.id == id)) else {
            return;
        };
        let d = &self.docs[i];
        let cur = d.cursor;
        match ask {
            Ask::Rename(name) => self.lsp.rename(d, cur, &name),
            Ask::References => {
                let r = word_range(d, cur);
                let name = d.line(cur.line)[r.start.col..r.end.col].to_owned();
                self.lsp.references(d, cur, &name);
            }
            Ask::CodeActions => {
                let range = if d.has_selection() { d.selection() } else { Range::new(cur, cur) };
                self.lsp.code_actions(d, range);
            }
            Ask::Signature => self.lsp.signature(d, cur, None, false),
            Ask::Format { then_save } => {
                let (tab, spaces) = (self.settings.tab(), self.settings.insert_spaces);
                self.lsp.format(d, tab, spaces, then_save);
            }
        }
    }

    /// The code action picked from the menu: its edit made, or its
    /// command sent. Returns a line for a toast.
    pub(crate) fn pick_code_action(&mut self, i: usize, now: f64) -> Option<String> {
        let action = self.code_actions.get(i).cloned()?;
        self.code_actions.clear();
        if let Some(edit) = action.edit {
            return Some(self.apply_workspace_edit(edit, now).summary("Applied"));
        }
        let (command, args) = action.command?;
        let lang = self.focus_doc()?.lang();
        self.lsp.execute(lang, &command, args);
        Some(action.title)
    }

    /// References as search results: grouped by file, the line shown,
    /// the name highlighted, the Search editor brought up.
    fn show_references(&mut self, name: &str, locs: Vec<Loc>, utf16: bool) {
        let mut files: Vec<FileHits> = Vec::new();
        let mut read: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for l in locs {
            let text: String = match self.doc_by_path(&l.path) {
                Some(i) => self.docs[i].line(l.line.min(self.docs[i].buffer.line_count().saturating_sub(1))).to_owned(),
                None => {
                    let lines = read.entry(l.path.clone()).or_insert_with(|| std::fs::read_to_string(&l.path).map(|t| t.lines().map(str::to_owned).collect()).unwrap_or_default());
                    lines.get(l.line).cloned().unwrap_or_default()
                }
            };
            let col = from_units(&text, l.col, utf16);
            let end = if l.end_line == l.line { from_units(&text, l.end_col, utf16).max(col) } else { text.len() };
            let (preview, pcol) = preview_of(&text, col, end - col);
            let hit = Hit { line: l.line, col, len: end - col, preview, pcol };
            match files.iter_mut().find(|f| f.path == l.path) {
                Some(f) => f.hits.push(hit),
                None => files.push(FileHits { path: l.path, hits: vec![hit] }),
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let s = &mut self.search;
        s.total = files.iter().map(|f| f.hits.len()).sum();
        s.files_seen = files.len();
        s.results = files;
        s.query = name.to_owned();
        s.shown_for = Some(Query { text: name.to_owned(), match_case: s.match_case, whole_word: s.whole_word });
        s.heading = Some(format!("References to `{name}`"));
        s.running = false;
        s.capped = false;
        s.run_at = None;
        s.want_focus = false;
        s.collapsed.clear();
        self.pending_show.push(Editor::Search);
    }

    /// Saves waiting on a format that never came: done anyway once their
    /// time is up.
    pub(crate) fn settle_saves(&mut self, shell: &mut Shell<Self>) -> bool {
        if self.pending_saves.is_empty() {
            return false;
        }
        let now_i = std::time::Instant::now();
        let due: Vec<DocId> = self.pending_saves.iter().filter(|(_, at)| *at <= now_i).map(|(id, _)| *id).collect();
        if due.is_empty() {
            shell.state.request_redraw_after(0.5);
            return false;
        }
        self.pending_saves.retain(|(id, _)| !due.contains(id));
        for id in due {
            if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                let msg = match self.save_index(i, shell.state.now) {
                    Ok(m) => format!("{m} (not formatted: the server did not answer)"),
                    Err(e) => e,
                };
                shell.request(self, ShellRequest::Toast(msg));
            }
        }
        true
    }

    /// The path to show for a file: relative to the project when inside it.
    fn shown_path(&self, p: &Path) -> String {
        match &self.project {
            Some(pr) if p.starts_with(&pr.root) => pr.relative(p),
            _ => p.display().to_string(),
        }
    }

    /// The 1-based character column of a server span, from the open
    /// document's text.
    fn char_col(&self, path: &Path, s: &LspSpan) -> Option<usize> {
        let d = self.docs.iter().find(|d| d.path.as_deref() == Some(path))?;
        let line = d.line(s.line.min(d.buffer.line_count().saturating_sub(1)));
        let b = from_units(line, s.col, s.utf16);
        Some(line[..b].chars().count() + 1)
    }

    /// Every problem: what the terminals read off builds and what the
    /// servers report, a build's copy dropped when a server has the same.
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = self.lsp.problems(|p| self.shown_path(p), |p, s| self.char_col(p, s));
        for t in &self.terminals {
            for d in &t.diags.items {
                let dup = d.resolved.as_ref().is_some_and(|r| out.iter().any(|p| p.path.as_deref() == Some(r.as_path()) && p.line == d.line && p.message == d.message));
                if dup {
                    continue;
                }
                let shown = match &d.resolved {
                    Some(p) => self.shown_path(p),
                    None => d.path.clone(),
                };
                out.push(Problem { severity: d.severity, message: d.message.clone(), source: "terminal".into(), path: d.resolved.clone(), shown, line: d.line, col: d.col, span: None });
            }
        }
        out
    }
}
