//! The requests queued during a rebuild and applied after it, when the
//! screen can be touched: files into the Code area, editors brought on
//! screen or split off, terminals made and reaped, tabs closed, the
//! caret sent to a line, the clipboard worked, focus handed back.

use std::collections::HashSet;
use std::path::PathBuf;

use lntrn_ui::{AreaId, Axis, Shell, ShellRequest};

use crate::app::{App, ClipOp, Editor};
use crate::bridge::Bridge;
use crate::buffer::Pos;
use crate::editor::editor_id;
use crate::term::{TermId, Terminal};

impl App {
    /// The Code area files open into: the active one if it is one, else
    /// the first on screen, else a Code tab added to the active area.
    fn code_area(&self, shell: &mut Shell<Self>) -> AreaId {
        if let Some(a) = shell.screen.target(Editor::Code) {
            return a;
        }
        let a = shell.screen.active.or_else(|| shell.screen.area_ids().next()).unwrap_or(0);
        shell.screen.add_tab(a, Editor::Code);
        a
    }

    /// Bring an editor on screen: focus the area that has it, or split
    /// one off for it (Files on the left, Terminal below, the rest to the
    /// right of the focused area).
    fn show_editor(&mut self, shell: &mut Shell<Self>, editor: Editor) -> Option<AreaId> {
        if let Some(a) = shell.screen.target(editor) {
            shell.screen.active = Some(a);
            return Some(a);
        }
        let base = shell.screen.active.or_else(|| shell.screen.area_ids().next())?;
        let area = match editor {
            Editor::Files => {
                let new = shell.screen.split(base, Axis::Horizontal, 0.78, editor)?;
                shell.screen.swap(base, new);
                base
            }
            Editor::Terminal => shell.screen.split(base, Axis::Vertical, 0.65, editor)?,
            // Problems sit beside the terminal whose output they came from.
            Editor::Problems => match shell.screen.target(Editor::Terminal) {
                Some(a) => {
                    shell.screen.add_tab(a, editor);
                    a
                }
                None => shell.screen.split(base, Axis::Vertical, 0.65, editor)?,
            },
            _ => shell.screen.split(base, Axis::Horizontal, 0.5, editor)?,
        };
        shell.screen.active = Some(area);
        Some(area)
    }

    pub(crate) fn apply_pending(&mut self, shell: &mut Shell<Self>) -> bool {
        let mut again = false;
        if let Some(folder) = self.pending_folder.take() {
            self.set_project(folder);
            again = true;
        }
        if let Some(p) = self.project.as_mut() {
            p.ensure_files();
        }
        for p in std::mem::take(&mut self.pending_paths) {
            match self.load_doc(&p) {
                Ok(id) => self.pending_docs.push(id),
                Err(e) => {
                    shell.request(self, ShellRequest::Toast(format!("Could not open {e}")));
                }
            }
        }
        if !self.pending_docs.is_empty() {
            let area = self.code_area(shell);
            let ids = std::mem::take(&mut self.pending_docs);
            if let Some(a) = shell.screen.area_mut(area) {
                let st = a.state_mut();
                for id in &ids {
                    if !st.docs.contains(id) {
                        st.docs.push(*id);
                    }
                    st.current = st.docs.iter().position(|d| d == id).unwrap_or(0);
                }
            }
            shell.screen.active = Some(area);
            self.focus_area = Some(area);
            self.focus_doc = ids.last().copied();
            self.pending_focus = Some(editor_id(area));
            again = true;
        }
        if self.pending_select.is_some() {
            self.apply_pending_select();
            again = true;
        }
        if let Some((path, line, col)) = self.pending_goto.take() {
            if let Some(i) = self.doc_by_path(&path) {
                let doc = &mut self.docs[i];
                let n = doc.buffer.line_count();
                let l = line.unwrap_or(1).saturating_sub(1).min(n.saturating_sub(1));
                let text = doc.line(l);
                // Compilers count columns in characters, from one.
                let byte = col.filter(|c| *c > 0).and_then(|c| text.char_indices().nth(c - 1).map(|(b, _)| b)).unwrap_or(0);
                doc.set_cursor(Pos::new(l, byte), false);
            }
            again = true;
        }
        if let Some(did) = self.pending_show_diff.take() {
            self.show_diff(shell, did);
            again = true;
        }
        if std::mem::take(&mut self.pending_ide_send) {
            self.ide_send_selection(shell);
            again = true;
        }
        for (did, accept) in std::mem::take(&mut self.pending_diff_resolve) {
            self.ide_resolve(shell, did, accept);
            again = true;
        }
        for e in std::mem::take(&mut self.pending_show) {
            self.show_editor(shell, e);
            again = true;
        }
        if let Some(cwd) = self.pending_new_terminal.take() {
            let area = match shell.screen.target(Editor::Terminal) {
                Some(a) => {
                    shell.screen.add_tab(a, Editor::Terminal);
                    shell.screen.active = Some(a);
                    Some(a)
                }
                None => self.show_editor(shell, Editor::Terminal),
            };
            if let Some(a) = area
                && let Some(ar) = shell.screen.area_mut(a)
            {
                let id = self.new_terminal(cwd);
                ar.state_mut().term = Some(id);
            }
            again = true;
        }
        for id in std::mem::take(&mut self.pending_close) {
            for a in shell.screen.area_ids().collect::<Vec<_>>() {
                if let Some(ar) = shell.screen.area_mut(a) {
                    for tab in &mut ar.tabs {
                        tab.state.docs.retain(|d| *d != id);
                        tab.state.current = tab.state.current.min(tab.state.docs.len().saturating_sub(1));
                    }
                }
            }
            self.docs.retain(|d| d.id != id);
            if self.focus_doc == Some(id) {
                self.focus_doc = None;
            }
            self.session_dirty = true;
            again = true;
        }
        if let Some(by) = self.pending_cycle.take()
            && let Some(a) = self.focus_area
            && let Some(ar) = shell.screen.area_mut(a)
        {
            let st = ar.state_mut();
            if !st.docs.is_empty() {
                st.current = (st.current as i64 + i64::from(by)).rem_euclid(st.docs.len() as i64) as usize;
            }
            shell.state.focus = Some(editor_id(a));
            again = true;
        }
        if let Some(op) = self.pending_clip.take() {
            let now = shell.state.now;
            match op {
                ClipOp::Set(s) => shell.state.set_clipboard(s),
                ClipOp::Copy { cut } => {
                    if let Some(doc) = self.focus_doc_mut() {
                        let text = if doc.has_selection() { doc.selected_text() } else { format!("{}\n", doc.line(doc.cursor.line)) };
                        shell.state.set_clipboard(text);
                        if cut && doc.has_selection() {
                            doc.delete(doc.selection(), now);
                        } else if cut {
                            crate::editor::ops::delete_lines(doc, now);
                        }
                    }
                }
                ClipOp::Paste => {
                    if self.paste_armed {
                        self.paste_armed = false;
                        let text = shell.state.clipboard.clone();
                        if let Some(doc) = self.focus_doc_mut()
                            && !text.is_empty()
                        {
                            doc.insert(&text, now);
                        }
                    } else {
                        self.paste_armed = true;
                        shell.state.clipboard_wanted = true;
                        self.pending_clip = Some(ClipOp::Paste);
                    }
                }
            }
            again = true;
        }
        if let Some(id) = self.pending_focus.take() {
            shell.state.focus = Some(id);
            shell.state.focus_visible = false;
        }
        // A popup just closed and took focus with it: the editor gets it back.
        let popup_open = shell.popup_open();
        if self.popup_was_open
            && !popup_open
            && shell.state.focus.is_none()
            && let Some(id) = self.last_editor_focus
        {
            shell.state.focus = Some(id);
            again = true;
        }
        self.popup_was_open = popup_open;
        again
    }

    pub(crate) fn new_terminal(&mut self, cwd: Option<PathBuf>) -> TermId {
        let id = TermId(self.next_term);
        self.next_term += 1;
        let cwd = cwd.or_else(|| Some(self.base_dir()));
        let scrollback = self.settings.scrollback.clamp(0, 1_000_000) as usize;
        let env = self.bridge.as_ref().map(Bridge::env).unwrap_or_default();
        self.terminals.push(Terminal::new(id, cwd, 80, 24, scrollback, self.waker.clone(), env));
        id
    }

    /// Terminals no area of the main window holds any more are closed.
    pub(crate) fn reap_terminals(&mut self, shell: &Shell<Self>) {
        let used: HashSet<TermId> = shell.screen.area_ids().filter_map(|a| shell.screen.area(a)).flat_map(|a| a.tabs.iter().filter_map(|t| t.state.term)).collect();
        self.terminals.retain(|t| used.contains(&t.id));
    }
}
