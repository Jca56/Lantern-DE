//! What each editor kind draws in its header and body: the dispatch the
//! shell calls through `Host`.

use lntrn_ui::{Action, AreaCx, ShellRequest, Ui, prefs};

use crate::app::{APP_ID, App, Editor, TabState};
use crate::commands;
use crate::diff_view::{draw_diff, draw_diff_header};
use crate::files::draw_files;
use crate::preview::draw_preview;
use crate::problems::{ProblemRow, draw_problems};
use crate::term::draw_terminal;

impl App {
    pub fn draw_editor_header(&mut self, editor: Editor, ui: &mut Ui, cx: &mut AreaCx<TabState>) {
        match editor {
            Editor::Terminal => {
                if let Some(t) = cx.state.term.and_then(|id| self.terminals.iter().find(|t| t.id == id)) {
                    let title = t.title();
                    ui.label_dim(&title);
                }
            }
            Editor::Diff => {
                if let Some(d) = cx.state.diff.and_then(|id| self.diffs.iter().find(|d| d.id == id)) {
                    let out = draw_diff_header(ui, d);
                    let id = d.id;
                    if out.accept || out.reject {
                        self.pending_diff_resolve.push((id, out.accept));
                        cx.rebuild();
                    }
                }
            }
            _ => {}
        }
    }

    pub fn draw_editor_body(&mut self, editor: Editor, ui: &mut Ui, cx: &mut AreaCx<TabState>) -> bool {
        match editor {
            Editor::Code => self.draw_code(ui, cx),
            Editor::Files => {
                let selected = self.focus_doc().and_then(|d| d.path.clone());
                let out = draw_files(ui, self.project.as_mut(), selected.as_deref());
                if let Some(p) = out.open {
                    self.pending_paths.push(p);
                    cx.rebuild();
                }
                if out.open_folder {
                    self.run_action(&Action::new(commands::OPEN_FOLDER), &mut cx.host());
                }
                if let Some((path, is_dir, at)) = out.context {
                    cx.request(ShellRequest::ContextMenu(Box::new(crate::actions::file_menu(&path, is_dir, at))));
                }
                false
            }
            Editor::Terminal => {
                let id = match cx.state.term.filter(|id| self.terminals.iter().any(|t| t.id == *id)) {
                    Some(id) => id,
                    None => {
                        let id = self.new_terminal(None);
                        cx.state.term = Some(id);
                        id
                    }
                };
                let active = cx.active;
                let settings = &self.settings;
                if let Some(t) = self.terminals.iter_mut().find(|t| t.id == id) {
                    let out = draw_terminal(ui, t, settings, active);
                    if out.focused {
                        self.last_editor_focus = Some(ui.id("term"));
                    }
                    if let Some((path, line, col)) = out.open {
                        self.pending_paths.push(path.clone());
                        self.pending_goto = Some((path, line, col));
                        cx.rebuild();
                    }
                }
                false
            }
            Editor::Problems => {
                let rows: Vec<ProblemRow> = self
                    .diagnostics()
                    .map(|d| {
                        let shown = match (&d.resolved, &self.project) {
                            (Some(p), Some(pr)) if p.starts_with(&pr.root) => pr.relative(p),
                            _ => d.path.clone(),
                        };
                        ProblemRow { severity: d.severity, place: format!("{shown}:{}:{}", d.line, d.col), message: d.message.clone(), openable: d.resolved.is_some() }
                    })
                    .collect();
                let out = draw_problems(ui, &rows);
                let target = out.open.and_then(|i| self.diagnostics().nth(i)).and_then(|d| d.resolved.clone().map(|p| (p, d.line, d.col)));
                if let Some((p, line, col)) = target {
                    self.pending_paths.push(p.clone());
                    self.pending_goto = Some((p, Some(line), Some(col)));
                    cx.rebuild();
                }
                if out.clear {
                    for t in &mut self.terminals {
                        t.diags.clear();
                    }
                    cx.rebuild();
                }
                false
            }
            Editor::Preview => {
                let doc = self.focus_doc();
                draw_preview(ui, doc);
                false
            }
            Editor::Diff => {
                match cx.state.diff.and_then(|id| self.diffs.iter().find(|d| d.id == id)) {
                    Some(d) => {
                        if cx.active || self.focus_diff.is_none() {
                            self.focus_diff = Some(d.id);
                        }
                        draw_diff(ui, d, &self.settings);
                    }
                    None => {
                        ui.heading("Diff");
                        ui.label_dim("Claude Code shows the edits it proposes here. Run `claude` in the terminal.");
                    }
                }
                false
            }
            Editor::Preferences => {
                let mut tab = self.prefs_tab;
                ui.tabs(&mut tab, &["Editor", "Shell"]);
                self.prefs_tab = tab;
                if tab == 0 {
                    let mut changed = false;
                    ui.scroll_area("editor-prefs", None, |ui| {
                        ui.heading("Editor");
                        ui.label_dim("The code font takes effect after a restart.");
                        changed = ui.props_panel(&mut self.settings);
                    });
                    if changed {
                        self.settings.save(APP_ID);
                    }
                    changed
                } else {
                    prefs::draw(ui, cx.prefs)
                }
            }
            Editor::Keys => {
                ui.heading("Key Bindings");
                ui.label_dim("Click a key to rebind it; type an action id beside it.");
                ui.scroll_area("keys", None, |ui| {
                    ui.keymap_editor("keys", &mut self.keys);
                });
                false
            }
        }
    }
}
