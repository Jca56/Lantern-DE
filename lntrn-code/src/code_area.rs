//! The Code editor of one area: its file tabs, the find bar when it is
//! open here, and the document view.

use lntrn_ui::{AreaCx, Key, ShellRequest, Ui};

use crate::app::{App, TabState};
use crate::commands;
use crate::editor::find::draw_find_bar;
use crate::editor::tabs::{TabItem, draw_tabs};
use crate::editor::lsp_ui::LspOut;
use crate::editor::decor::DiagMark;
use crate::editor::view::{ViewOpts, draw_doc};
use crate::lsp::pos::from_units;
use lntrn_ui::Action;

impl App {
    pub fn draw_code(&mut self, ui: &mut Ui, cx: &mut AreaCx<TabState>) -> bool {
        let (area, active) = (cx.area, cx.active);
        let mut close_id = None;
        let st = &mut *cx.state;
        st.docs.retain(|id| self.docs.iter().any(|d| d.id == *id));
        if st.docs.is_empty() {
            return false;
        }
        st.current = st.current.min(st.docs.len() - 1);
        let icon_px = (ui.m.widget_h * 0.7).round().max(8.0) as u32;
        let root = self.tree.root.clone();
        let mut icons_of: Vec<Option<_>> = Vec::new();
        for id in st.docs.iter() {
            let path = self.docs.iter().find(|d| d.id == *id).and_then(|d| d.path.clone());
            icons_of.push(path.and_then(|p| self.icons.icon(&p, false, &root, icon_px)));
        }
        let items: Vec<TabItem> = st.docs.iter().zip(icons_of).filter_map(|(id, icon)| self.docs.iter().find(|d| d.id == *id).map(|d| TabItem { label: &d.title, dirty: d.is_dirty(), icon })).collect();
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
        if let Some((from, to)) = tabs.reorder {
            let current_id = st.docs[st.current];
            let moved = st.docs.remove(from);
            st.docs.insert(to.min(st.docs.len()), moved);
            st.current = st.docs.iter().position(|d| *d == current_id).unwrap_or(0);
            ui.state.request_rebuild = true;
        }
        let mut tab_menu_at = None;
        if let Some((i, at)) = tabs.context {
            self.context_tab = Some((st.docs[i], st.docs.clone()));
            tab_menu_at = Some(at);
        }
        let doc_id = st.docs[st.current];
        if let Some(at) = tab_menu_at {
            cx.request(ShellRequest::MenuAt("tab".to_owned(), at));
        }
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
        if out.zoom != 0 {
            self.settings.font_size = (self.settings.font_size + f64::from(out.zoom)).clamp(8.0, 64.0);
            self.settings.save(crate::app::APP_ID);
            ui.state.request_rebuild = true;
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
            if let Some((p, trigger, retrigger)) = asked.signature {
                self.lsp.signature(d, p, trigger, retrigger);
            }
        }
        if out.clicked {
            self.focus_doc = Some(doc_id);
            self.focus_area = Some(area);
        }
        if let Some(at) = out.context {
            self.focus_doc = Some(doc_id);
            self.focus_area = Some(area);
            cx.request(ShellRequest::MenuAt("editor-context".to_owned(), at));
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
}
