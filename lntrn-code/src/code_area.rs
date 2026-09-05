//! The Code editor of one area: its file tabs, the find bar when it is
//! open here, and the document view.

use lntrn_ui::{AreaCx, Key, Ui};

use crate::app::{App, TabState};
use crate::commands;
use crate::editor::find::draw_find_bar;
use crate::editor::tabs::{TabItem, draw_tabs};
use crate::editor::lsp_ui::LspOut;
use crate::editor::view::{DiagMark, ViewOpts, draw_doc};
use crate::lsp::pos::from_units;
use lntrn_ui::Action;

impl App {
    pub fn draw_code(&mut self, ui: &mut Ui, cx: &mut AreaCx<TabState>) -> bool {
        let (area, active) = (cx.area, cx.active);
        let mut close_id = None;
        let st = &mut *cx.state;
        st.docs.retain(|id| self.docs.iter().any(|d| d.id == *id));
        if st.docs.is_empty() {
            return self.draw_welcome(ui, cx);
        }
        st.current = st.current.min(st.docs.len() - 1);
        let items: Vec<TabItem> = st.docs.iter().filter_map(|id| self.docs.iter().find(|d| d.id == *id)).map(|d| TabItem { label: &d.title, dirty: d.is_dirty() }).collect();
        let tabs = draw_tabs(ui, &items, st.current);
        let editor_focus = ui.id("code");
        if let Some(i) = tabs.select {
            st.current = i;
            ui.state.focus = Some(editor_focus);
            ui.state.request_rebuild = true;
        }
        if let Some(i) = tabs.close {
            close_id = Some(st.docs[i]);
        }
        let doc_id = st.docs[st.current];
        if active || self.focus_doc.is_none() {
            self.focus_doc = Some(doc_id);
            self.focus_area = Some(area);
        }
        let finder_here = self.finder.open && self.focus_area == Some(area) && self.focus_doc == Some(doc_id);
        let marks = self.diag_marks(doc_id);
        let gutter = self.gutter_marks(doc_id, ui.state.now);
        if let Some(d) = self.docs.iter().find(|d| d.id == doc_id) {
            self.lsp_ui.utf16 = self.lsp.utf16(d.lang());
        }
        let App { finder, docs, settings, last_editor_focus, lsp_ui, .. } = self;
        let Some(doc) = docs.iter_mut().find(|d| d.id == doc_id) else {
            return false;
        };
        let mut changed = false;
        if finder_here {
            if ui.state.take_key(|k| k.key == Key::Escape && k.mods.is_empty()).is_some() {
                finder.close();
                ui.state.focus = Some(editor_focus);
                ui.state.request_rebuild = true;
            } else {
                let out = draw_find_bar(ui, finder, doc);
                changed |= out.changed;
                if out.closed {
                    ui.state.focus = Some(editor_focus);
                }
            }
        }
        if doc.disk_changed || doc.disk_missing {
            let mut reload = false;
            let mut keep = false;
            ui.row(|ui| {
                let msg = if doc.disk_missing { "This file was deleted on disk." } else { "This file changed on disk; you have unsaved edits." };
                ui.label_dim(msg);
                if !doc.disk_missing && ui.button("Reload").clicked {
                    reload = true;
                }
                if ui.button("Keep mine").clicked {
                    keep = true;
                }
            });
            if reload
                && let Some(p) = doc.path.clone()
                && let Ok(bytes) = std::fs::read(&p)
            {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                doc.replace_all(&text, ui.state.now);
                changed = true;
            }
            if keep {
                doc.disk_changed = false;
                doc.disk_missing = false;
                ui.state.request_rebuild = true;
            }
        }
        let (matches, current) = if finder_here && finder.open {
            finder.refresh(doc);
            (finder.matches.as_slice(), finder.current)
        } else {
            (&[][..], None)
        };
        let out = draw_doc(ui, doc, settings, ViewOpts { area_active: active, matches, current_match: current, diags: &marks, git: &gutter, lsp: lsp_ui });
        changed |= out.changed;
        if out.focused {
            *last_editor_focus = Some(editor_focus);
        }
        let asked = out.lsp;
        if asked != LspOut::default()
            && let Some(d) = self.docs.iter().find(|d| d.id == doc_id)
        {
            if let Some(p) = asked.hover {
                self.lsp.hover(d, p);
            }
            if let Some(p) = asked.definition {
                self.lsp.definition(d, p);
            }
            if let Some((p, trigger)) = asked.complete {
                self.lsp.complete(d, p, trigger);
            }
        }
        if out.clicked {
            self.focus_doc = Some(doc_id);
            self.focus_area = Some(area);
        }
        if let Some(id) = close_id {
            self.focus_doc = Some(id);
            self.focus_area = Some(area);
            self.run_action(&Action::new(commands::CLOSE_TAB), &mut cx.host());
        }
        changed
    }

    /// The problems of a document as marks with byte columns.
    fn diag_marks(&self, doc_id: crate::doc::DocId) -> Vec<DiagMark> {
        let Some(doc) = self.docs.iter().find(|d| d.id == doc_id) else {
            return Vec::new();
        };
        let Some(path) = doc.path.as_deref() else {
            return Vec::new();
        };
        let n = doc.buffer.line_count();
        let mut out = Vec::new();
        for p in self.problems() {
            if p.path.as_deref() != Some(path) {
                continue;
            }
            let (line, col, end) = match p.span {
                Some(s) => {
                    let l = s.line.min(n.saturating_sub(1));
                    let text = doc.line(l);
                    (l, from_units(text, s.col, s.utf16), (s.end_line == s.line).then(|| from_units(text, s.end_col, s.utf16)))
                }
                None => {
                    let l = p.line.saturating_sub(1).min(n.saturating_sub(1));
                    let text = doc.line(l);
                    (l, text.char_indices().nth(p.col.saturating_sub(1)).map(|(b, _)| b).unwrap_or(text.len()), None)
                }
            };
            out.push(DiagMark { line, col, end, severity: p.severity, message: p.message });
        }
        out
    }

    fn draw_welcome(&mut self, ui: &mut Ui, cx: &mut AreaCx<TabState>) -> bool {
        let mut action: Option<&str> = None;
        ui.row(|ui| {
            if ui.button("New File").clicked {
                action = Some(commands::NEW);
            }
            if ui.button("Open File…").clicked {
                action = Some(commands::OPEN);
            }
            if ui.button("Open Folder…").clicked {
                action = Some(commands::OPEN_FOLDER);
            }
        });
        if let Some(id) = action {
            self.focus_area = Some(cx.area);
            self.run_action(&Action::new(id), &mut cx.host());
        }
        false
    }
}
