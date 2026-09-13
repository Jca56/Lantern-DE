//! The app as the shell sees it: the editors it hosts, their labels,
//! the title, status bar and menus, the palette, key bindings, and what
//! a closing window or dropped files mean.

use std::path::PathBuf;

use lntrn_props::Value;
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, AreaId, Dialog, Host, HostCx, KeyItem, KeyPress, Menu, ShellRequest, Ui, actions};

use crate::app::{App, EDITORS, Editor, TabState};
use crate::commands;
use crate::problems::Severity;
use crate::text_util::cell_of_byte;

impl Host for App {
    type Editor = Editor;
    type AreaState = TabState;

    fn editors(&self) -> &[Editor] {
        &EDITORS
    }

    /// Saved layouts keep naming the editor "Code".
    fn editor_id(&self, editor: Editor) -> String {
        match editor {
            Editor::Code => "Code".to_owned(),
            e => self.editor_label(e).to_owned(),
        }
    }

    fn editor_label(&self, editor: Editor) -> &str {
        match editor {
            Editor::Code => "Editor",
            Editor::Files => "Files",
            Editor::Terminal => "Terminal",
            Editor::Preview => "Preview",
            Editor::Preferences => "Preferences",
            Editor::Keys => "Key Bindings",
            Editor::Diff => "Diff",
            Editor::Problems => "Problems",
            Editor::Search => "Search",
            Editor::Git => "Git",
        }
    }

    /// A terminal tab wears the dot while its bell waits to be seen.
    fn tab_attention(&self, editor: Editor, state: &TabState) -> bool {
        editor == Editor::Terminal && state.term.is_some_and(|id| self.terminals.iter().any(|t| t.id == id && t.attention))
    }

    /// The project name alone: the open file already shows in its tab.
    fn title(&self) -> String {
        self.project.as_ref().map(|p| p.name()).unwrap_or_else(|| "lntrn-code".to_owned())
    }

    fn status(&self) -> String {
        let Some(d) = self.focus_doc() else {
            return self.attention_status();
        };
        let bell = match self.attention_status() {
            s if s.is_empty() => s,
            s => format!(" · {s}"),
        };
        let col = cell_of_byte(d.line(d.cursor.line), d.tab(), d.cursor.col) + 1;
        let sel = if d.has_selection() { format!(" · {} selected", d.selected_text().chars().count()) } else { String::new() };
        let claude = match self.ide_connected {
            0 => String::new(),
            1 => " · Claude ✓".to_owned(),
            n => format!(" · Claude ×{n} ✓"),
        };
        let branch = self.git.as_ref().filter(|g| !g.branch.is_empty()).map(|g| format!("⎇ {} · ", g.branch)).unwrap_or_default();
        let all = self.problems();
        let (errors, warnings) = (all.iter().filter(|p| p.severity == Severity::Error).count(), all.iter().filter(|p| p.severity == Severity::Warning).count());
        let problems = match (errors, warnings) {
            (0, 0) => String::new(),
            (e, 0) => format!(" · {e} error{}", if e == 1 { "" } else { "s" }),
            (0, w) => format!(" · {w} warning{}", if w == 1 { "" } else { "s" }),
            (e, w) => format!(" · {e} error{}, {w} warning{}", if e == 1 { "" } else { "s" }, if w == 1 { "" } else { "s" }),
        };
        let server = self.lsp.status().map(|s| format!(" · {s}")).unwrap_or_default();
        // Prose is measured in words.
        let words = if d.lang() == crate::syntax::Language::Markdown { format!(" · {} words", d.buffer.lines().iter().map(|l| l.split_whitespace().count()).sum::<usize>()) } else { String::new() };
        // Unix line endings are the norm; only the other kind is worth a word.
        let ending = if d.buffer.ending.label() == "LF" { String::new() } else { format!(" · {}", d.buffer.ending.label()) };
        format!("{branch}Ln {}, Col {col}{sel} · {}{words}{ending}{claude}{problems}{server}{bell}", d.cursor.line + 1, d.lang().name())
    }

    /// The status goes along the bottom, not beside the title (U036).
    fn status_bar(&self) -> bool {
        true
    }

    fn opaque_bars(&self) -> bool {
        self.settings.opaque_bars
    }

    fn title_menus(&self) -> &[(&str, &str)] {
        commands::title_menus()
    }

    fn menu(&self, name: &str) -> Option<Menu> {
        commands::menu(self, name)
    }

    fn palette(&self, query: &str) -> Vec<(String, String)> {
        // The palette is asked with `&self`; the file list is built from a
        // clone of the project's cache when it exists.
        let q = query.to_lowercase();
        let mut out: Vec<(String, String)> = commands::PALETTE.iter().filter(|(id, label)| q.is_empty() || label.to_lowercase().contains(&q) || id.contains(&q)).map(|(id, label)| ((*id).to_owned(), (*label).to_owned())).collect();
        if !q.is_empty() && let Some(p) = &self.project {
            for path in p.search_cached(query, 12) {
                out.push((format!("{}{}", commands::OPEN_PREFIX, path.display()), format!("Open {}", p.relative(&path))));
            }
        }
        out
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action)
    }

    /// The side panels draw at the Panel Scale setting, so a thin tree
    /// fits beside big code.
    /// The code view and the terminal paint their bodies edge to edge
    /// themselves, one layer, so translucency does not stack (U037).
    fn paints_body(&self, editor: Editor) -> bool {
        matches!(editor, Editor::Code | Editor::Terminal)
    }

    fn editor_scale(&self, editor: Editor) -> f64 {
        match editor {
            Editor::Files | Editor::Git | Editor::Search | Editor::Problems => self.settings.panel_scale,
            _ => 1.0,
        }
    }

    fn draw_header(&mut self, editor: Editor, ui: &mut Ui, cx: &mut AreaCx<TabState>) {
        self.draw_editor_header(editor, ui, cx);
    }

    fn draw_body(&mut self, editor: Editor, ui: &mut Ui, cx: &mut AreaCx<TabState>) -> bool {
        self.draw_editor_body(editor, ui, cx)
    }

    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        self.run_action(action, cx);
    }

    fn draw_item(&mut self, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
        match key {
            "text" | "line" => {
                let label = if key == "line" { "Line" } else { "Name" };
                ui.labelled(label, |ui| {
                    if ui.state.focus.is_none() {
                        let id = ui.id("field");
                        ui.state.focus = Some(id);
                        let te = ui.state.text_edit(id);
                        te.anchor = 0;
                        te.cursor = self.dialog_text.len();
                    }
                    if ui.text_field("field", &mut self.dialog_text).committed {
                        cx.request(ShellRequest::DialogDefault);
                    }
                });
            }
            _ => {}
        }
        false
    }

    fn key(&self, press: KeyPress, _editor: Option<Editor>) -> Option<Action> {
        self.keys.resolve(&[CTX_WINDOW], &press.to_event(), |_| true).map(KeyItem::action)
    }

    /// The main window closing with unsaved files asks first; Quit in
    /// that dialog goes straight through.
    fn close_requested(&mut self, main: bool, cx: &mut HostCx) -> bool {
        let dirty = self.docs.iter().filter(|d| d.is_dirty()).count();
        if !main || dirty == 0 || self.quit_confirmed {
            return true;
        }
        let body = format!("{dirty} file{} ha{} unsaved changes. Quit anyway?", if dirty == 1 { "" } else { "s" }, if dirty == 1 { "s" } else { "ve" });
        cx.request(ShellRequest::Dialog(Dialog::confirm("Unsaved changes", &body, "Quit", Action::new(actions::QUIT).with("force", Value::Bool(true)))));
        false
    }

    fn dropped(&mut self, paths: &[PathBuf], _area: Option<AreaId>, _editor: Option<Editor>, cx: &mut HostCx) {
        for p in paths {
            if p.is_dir() {
                self.pending_folder = Some(p.clone());
            } else {
                self.pending_paths.push(p.clone());
            }
        }
        cx.rebuild();
    }
}
