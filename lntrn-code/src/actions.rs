//! What each action does: the file operations, the editing commands that
//! reach the focused document from a menu or key, the dialogs, and the
//! context menu of a file in the tree.

use std::path::{Path, PathBuf};

use lntrn_props::Value;
use lntrn_ui::{Action, Dialog, HostCx, ShellRequest};

use crate::app::{App, ClipOp, Editor};
use crate::buffer::Pos;
use crate::commands::*;
use crate::editor::{input, ops};
use crate::lsp::Ask;
use crate::syntax::Language;

fn arg_str(action: &Action, name: &str) -> Option<String> {
    match action.arg(name) {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn arg_i64(action: &Action, name: &str) -> Option<i64> {
    match action.arg(name) {
        Some(Value::I64(n)) => Some(*n),
        _ => None,
    }
}

/// `name copy.ext`, then `name copy 2.ext` and on: the first that is free.
fn duplicate_name(p: &Path) -> Option<PathBuf> {
    let dir = p.parent()?;
    let stem = p.file_stem()?.to_string_lossy();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..1000).map(|n| dir.join(if n == 1 { format!("{stem} copy{ext}") } else { format!("{stem} copy {n}{ext}") })).find(|c| !c.exists())
}

impl App {
    fn dialog_with_field(&mut self, title: &str, body: &str, verb: &str, action: &str, key: &str, cx: &mut HostCx) {
        let dialog = Dialog::new(title, body).button("Cancel", None).button(verb, Some(Action::new(action))).default_button(1).content(key);
        cx.request(ShellRequest::Dialog(dialog));
    }

    /// Write document `i` to its file. Returns a line for a toast, or
    /// what went wrong.
    pub(crate) fn save_index(&mut self, i: usize, now: f64) -> Result<String, String> {
        let trim = self.settings.trim_on_save;
        let doc = &mut self.docs[i];
        match doc.save(trim, now) {
            Ok(()) => {
                let msg = format!("Saved {}", doc.title);
                self.session_dirty = true;
                Ok(msg)
            }
            Err(e) => Err(format!("Could not save: {e}")),
        }
    }

    /// Save the focused document: after the server formats it when that
    /// is on (with a deadline, so a silent server never blocks a save).
    fn save_doc(&mut self, cx: &mut HostCx) {
        let now = cx_now();
        let Some(i) = self.focus_doc.and_then(|id| self.docs.iter().position(|d| d.id == id)) else {
            return;
        };
        if self.docs[i].path.is_none() {
            self.run_action(&Action::new(SAVE_AS), cx);
            return;
        }
        if self.settings.format_on_save && self.lsp.serves(self.docs[i].lang()) {
            let id = self.docs[i].id;
            self.lsp_ask(Ask::Format { then_save: true });
            self.pending_saves.retain(|(d, _)| *d != id);
            self.pending_saves.push((id, std::time::Instant::now() + std::time::Duration::from_secs(2)));
            return;
        }
        match self.save_index(i, now) {
            Ok(msg) => cx.toast(&msg),
            Err(e) => cx.request(ShellRequest::Dialog(Dialog::notice("Could not save", &e))),
        }
    }

    pub fn run_action(&mut self, action: &Action, cx: &mut HostCx) {
        let path = || arg_str(action, "path").map(PathBuf::from).unwrap_or_default();
        let now = cx_now();
        match action.id.as_str() {
            NEW => {
                let id = self.new_untitled();
                self.pending_docs.push(id);
            }
            // Open File… and Open Folder… type a path into the tree's path
            // bar; a folder typed for the latter becomes the project.
            OPEN => {
                self.tree.edit_path = Some(false);
                self.pending_show.push(Editor::Files);
            }
            OPENED => {
                let p = path();
                if p.is_dir() {
                    self.pending_folder = Some(p);
                } else {
                    self.pending_paths.push(p);
                }
            }
            OPEN_FOLDER => {
                self.tree.edit_path = Some(true);
                self.pending_show.push(Editor::Files);
            }
            GO => {
                self.tree.go(path());
                self.pending_show.push(Editor::Files);
            }
            SET_PROJECT => self.pending_folder = Some(path()),
            TOGGLE_HIDDEN => {
                self.tree.show_hidden = !self.tree.show_hidden;
                self.tree.refresh();
            }
            REFRESH_TREE => self.refresh_tree(),
            SAVE => self.save_doc(cx),
            SAVE_AS => {
                if let Some(d) = self.focus_doc() {
                    let suggest = d.path.clone().unwrap_or_else(|| self.base_dir().join(&d.title));
                    cx.request(ShellRequest::PathDialog { action: Action::new(SAVED_AS), save: true, suggest: suggest.display().to_string() });
                }
            }
            SAVED_AS => {
                let p = path();
                if let Some(doc) = self.focus_doc_mut() {
                    doc.set_path(p);
                }
                self.save_doc(cx);
                self.refresh_tree();
            }
            CLOSE_TAB => {
                if let Some(d) = self.focus_doc() {
                    let id = d.id;
                    if d.is_dirty() {
                        let dialog = Dialog::new("Unsaved changes", &format!("**{}** has unsaved changes.", d.title))
                            .button("Cancel", None)
                            .button("Don't Save", Some(Action::new(CLOSE_FORCE).with("doc", Value::I64(id.0 as i64))))
                            .button("Save", Some(Action::new(SAVE_CLOSE).with("doc", Value::I64(id.0 as i64))))
                            .default_button(2);
                        cx.request(ShellRequest::Dialog(dialog));
                    } else {
                        self.pending_close.push(id);
                    }
                }
            }
            CLOSE_FORCE => {
                if let Some(n) = arg_i64(action, "doc") {
                    self.pending_close.push(crate::doc::DocId(n as u64));
                }
            }
            SAVE_CLOSE => {
                if let Some(n) = arg_i64(action, "doc") {
                    let id = crate::doc::DocId(n as u64);
                    self.focus_doc = Some(id);
                    self.save_doc(cx);
                    if self.doc(id).is_some_and(|d| !d.is_dirty()) {
                        self.pending_close.push(id);
                    }
                }
            }
            // Closing the main window asks about unsaved work on the way.
            QUIT => cx.request(ShellRequest::CloseWindow),
            UNDO => {
                if let Some(d) = self.focus_doc_mut() {
                    d.undo(now);
                }
            }
            REDO => {
                if let Some(d) = self.focus_doc_mut() {
                    d.redo(now);
                }
            }
            CUT => self.pending_clip = Some(ClipOp::Copy { cut: true }),
            COPY => self.pending_clip = Some(ClipOp::Copy { cut: false }),
            PASTE => self.pending_clip = Some(ClipOp::Paste),
            SELECT_ALL => {
                if let Some(d) = self.focus_doc_mut() {
                    d.select_all();
                }
            }
            FIND | REPLACE => {
                let replace = action.id == REPLACE;
                let App { finder, docs, focus_doc, .. } = self;
                if let Some(d) = focus_doc.and_then(|id| docs.iter().find(|d| d.id == id)) {
                    finder.show(d, replace);
                }
            }
            FIND_NEXT | FIND_PREV => {
                let forward = action.id == FIND_NEXT;
                let App { finder, docs, focus_doc, .. } = self;
                if let Some(d) = focus_doc.and_then(|id| docs.iter_mut().find(|d| d.id == id)) {
                    if !finder.open {
                        finder.show(d, false);
                    }
                    finder.step(d, forward);
                }
            }
            GOTO_LINE => {
                if let Some((line, n)) = self.focus_doc().map(|d| (d.cursor.line + 1, d.buffer.line_count())) {
                    self.dialog_text = line.to_string();
                    self.dialog_with_field("Go to Line", &format!("1 to {n}"), "Go", GOTO_LINE_GO, "line", cx);
                }
            }
            GOTO_LINE_GO => {
                let line = self.dialog_text.trim().parse::<usize>().unwrap_or(1).max(1) - 1;
                if let Some(d) = self.focus_doc_mut() {
                    d.set_cursor(Pos::new(line, 0), false);
                }
                if let Some(a) = self.focus_area {
                    self.pending_focus = Some(crate::editor::editor_id(a));
                }
            }
            TOGGLE_COMMENT => {
                if let Some(d) = self.focus_doc_mut() {
                    ops::toggle_comment(d, now);
                }
            }
            DUPLICATE_LINE => {
                if let Some(d) = self.focus_doc_mut() {
                    ops::duplicate_lines(d, now);
                }
            }
            DELETE_LINE => {
                if let Some(d) = self.focus_doc_mut() {
                    ops::delete_lines(d, now);
                }
            }
            MOVE_LINE_UP | MOVE_LINE_DOWN => {
                let down = action.id == MOVE_LINE_DOWN;
                if let Some(d) = self.focus_doc_mut() {
                    ops::move_lines(d, down, now);
                }
            }
            FOLD | UNFOLD | FOLD_ALL | UNFOLD_ALL => {
                let which = action.id.as_str();
                if let Some(d) = self.focus_doc_mut() {
                    let line = d.cursor.line;
                    match which {
                        FOLD => d.fold_at(line),
                        UNFOLD => d.unfold_here(line),
                        FOLD_ALL => d.fold_all(),
                        _ => d.unfold_all(),
                    }
                }
            }
            ZOOM_IN | ZOOM_OUT | ZOOM_RESET => {
                let size = match action.id.as_str() {
                    ZOOM_IN => self.settings.font_size + 1.0,
                    ZOOM_OUT => self.settings.font_size - 1.0,
                    _ => crate::settings::Settings::default().font_size,
                };
                self.settings.font_size = size.clamp(8.0, 64.0);
                self.settings.save(crate::app::APP_ID);
            }
            TOGGLE_WRAP => {
                self.settings.wrap_prose = !self.settings.wrap_prose;
                self.settings.save(crate::app::APP_ID);
            }
            NEXT_FILE => self.pending_cycle = Some(1),
            PREV_FILE => self.pending_cycle = Some(-1),
            RENAME_SYMBOL => {
                if let Some(d) = self.focus_doc() {
                    let r = input::word_range(d, d.cursor);
                    self.dialog_text = d.line(d.cursor.line)[r.start.col..r.end.col].to_owned();
                    self.dialog_with_field("Rename Symbol", "Everywhere the language server knows of it.", "Rename", RENAME_SYMBOL_GO, "text", cx);
                }
            }
            RENAME_SYMBOL_GO => {
                let name = self.dialog_text.trim().to_owned();
                if !name.is_empty() {
                    self.lsp_ask(Ask::Rename(name));
                }
                if let Some(a) = self.focus_area {
                    self.pending_focus = Some(crate::editor::editor_id(a));
                }
            }
            GOTO_DEF => self.lsp_ask(Ask::Definition),
            REFERENCES => self.lsp_ask(Ask::References),
            CODE_ACTIONS => self.lsp_ask(Ask::CodeActions),
            SIGNATURE => self.lsp_ask(Ask::Signature),
            FORMAT => self.lsp_ask(Ask::Format { then_save: false }),
            CODE_ACTION_PICK => {
                if let Some(i) = arg_i64(action, "i")
                    && let Some(msg) = self.pick_code_action(i as usize, now)
                {
                    cx.toast(&msg);
                }
            }
            SET_LANG => {
                if let Some(i) = arg_i64(action, "lang").and_then(|i| Language::ALL.get(i as usize).copied())
                    && let Some(d) = self.focus_doc_mut()
                {
                    d.set_lang(i);
                }
            }
            SHOW_FILES => self.pending_show.push(Editor::Files),
            SHOW_TERMINAL => self.pending_show.push(Editor::Terminal),
            SHOW_PROBLEMS => self.pending_show.push(Editor::Problems),
            SHOW_GIT => self.pending_show.push(Editor::Git),
            SHOW_SEARCH => {
                // A one-line selection becomes the query, like the find bar.
                if let Some(d) = self.focus_doc()
                    && d.has_selection()
                {
                    let sel = d.selected_text();
                    if !sel.is_empty() && !sel.contains('\n') {
                        self.search.query = sel;
                        self.search.run_at = Some(0.0);
                    }
                }
                self.search.want_focus = true;
                self.pending_show.push(Editor::Search);
            }
            NEW_TERMINAL => self.pending_new_terminal = Some(None),
            SHOW_PREVIEW => self.pending_show.push(Editor::Preview),
            SHOW_PREFS => self.pending_show.push(Editor::Preferences),
            SHOW_KEYS => self.pending_show.push(Editor::Keys),
            ABOUT => cx.request(ShellRequest::Dialog(Dialog::notice(
                "lntrn-code",
                "The Lantern DE code editor, on **Lantern UI 2**.\n\nRust, `wgpu` and `winit`; everything else is ours: the text engine, the widgets, the syntax highlighting, the terminal.\n\n- Ctrl+P: command palette and quick open\n- Ctrl+B: files · Ctrl+`: terminal\n- Ctrl+F / Ctrl+H: find and replace\n- Split any area from the ⋮ menu in its header",
            ))),
            // The name of a new entry, or a new name, is typed in the tree
            // itself; without a folder given, the new entry goes where the
            // last click was.
            FILE_NEW | FOLDER_NEW => {
                let dir = if arg_str(action, "path").is_some() { path() } else { self.tree.target_dir() };
                self.tree.start_create(&dir, action.id == FOLDER_NEW);
                self.pending_show.push(Editor::Files);
            }
            RENAME => {
                self.tree.start_rename(&path());
                self.pending_show.push(Editor::Files);
            }
            DELETE_ASK => {
                let p = path();
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let what = if p.is_dir() { "folder and everything in it" } else { "file" };
                cx.request(ShellRequest::Dialog(Dialog::confirm("Delete", &format!("Delete the {what} **{name}**? This cannot be undone."), "Delete", Action::new(DELETE).with("path", Value::Str(p.display().to_string())))));
            }
            DELETE => {
                let p = path();
                let result = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
                match result {
                    Ok(()) => {
                        let gone: Vec<_> = self.docs.iter().filter(|d| d.path.as_ref().is_some_and(|dp| dp == &p || dp.starts_with(&p))).map(|d| d.id).collect();
                        self.pending_close.extend(gone);
                    }
                    Err(e) => cx.request(ShellRequest::Dialog(Dialog::notice("Could not delete", &e.to_string()))),
                }
                self.refresh_tree();
            }
            COPY_PATH => {
                self.pending_clip = Some(ClipOp::Set(path().display().to_string()));
                cx.toast("Path copied");
            }
            COPY_REL_PATH => {
                let p = path();
                let base = self.project.as_ref().map(|pr| pr.root.clone()).unwrap_or_else(|| self.tree.root.clone());
                let rel = p.strip_prefix(&base).unwrap_or(&p);
                self.pending_clip = Some(ClipOp::Set(rel.display().to_string()));
                cx.toast("Relative path copied");
            }
            DUPLICATE => {
                let p = path();
                match duplicate_name(&p) {
                    Some(to) => match std::fs::copy(&p, &to) {
                        Ok(_) => {
                            self.refresh_tree();
                            self.tree.reveal = Some(to.clone());
                            self.tree.start_rename(&to);
                            self.pending_show.push(Editor::Files);
                        }
                        Err(e) => cx.request(ShellRequest::Dialog(Dialog::notice("Could not duplicate", &e.to_string()))),
                    },
                    None => cx.toast("Could not find a free name"),
                }
            }
            TERMINAL_HERE => self.pending_new_terminal = Some(Some(path())),
            TAB_CLOSE => {
                if let Some((id, _)) = &self.context_tab {
                    self.focus_doc = Some(*id);
                    self.run_action(&Action::new(CLOSE_TAB), cx);
                }
            }
            TAB_CLOSE_OTHERS | TAB_CLOSE_ALL | TAB_CLOSE_SAVED => {
                let Some((keep, all)) = self.context_tab.clone() else {
                    return;
                };
                let mut kept_dirty = 0;
                for id in all {
                    if action.id == TAB_CLOSE_OTHERS && id == keep {
                        continue;
                    }
                    if self.doc(id).is_some_and(|d| d.is_dirty()) {
                        kept_dirty += 1;
                        continue;
                    }
                    self.pending_close.push(id);
                }
                if kept_dirty > 0 {
                    cx.toast(&format!("{kept_dirty} unsaved file{} left open", if kept_dirty == 1 { "" } else { "s" }));
                }
            }
            TAB_REVEAL => {
                if let Some(p) = self.context_tab.as_ref().and_then(|(id, _)| self.doc(*id)).and_then(|d| d.path.clone()) {
                    if !p.starts_with(&self.tree.root)
                        && let Some(dir) = p.parent()
                    {
                        self.tree.go(dir.to_path_buf());
                    }
                    self.tree.reveal = Some(p);
                    self.pending_show.push(Editor::Files);
                }
            }
            TERM_COPY | TERM_PASTE | TERM_CLEAR | TERM_RESTART => {
                let Some(t) = self.context_term.and_then(|id| self.terminals.iter_mut().find(|t| t.id == id)) else {
                    return;
                };
                match action.id.as_str() {
                    TERM_COPY => {
                        if let Some(text) = t.selection_text() {
                            self.pending_clip = Some(ClipOp::Set(text));
                        }
                    }
                    TERM_PASTE => {
                        t.paste_pending = true;
                        self.pending_clipboard_wanted = true;
                    }
                    TERM_CLEAR => t.clear(),
                    _ => t.respawn(),
                }
            }
            IDE_ACCEPT | IDE_REJECT => {
                if let Some(id) = self.focus_diff {
                    self.pending_diff_resolve.push((id, action.id == IDE_ACCEPT));
                }
            }
            IDE_SEND => self.pending_ide_send = true,
            other => {
                if let Some(p) = other.strip_prefix(OPEN_PREFIX) {
                    self.pending_paths.push(PathBuf::from(p));
                } else {
                    cx.toast(&format!("Unknown action {other}"));
                }
            }
        }
        cx.rebuild();
    }
}

/// The frame clock is not in `HostCx`; actions stamp edits with wall time
/// since the app started, which is what the widgets use too.
fn cx_now() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}
