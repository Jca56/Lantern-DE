//! What each editor kind draws in its header and body: the dispatch the
//! shell calls through `Host`.

use lntrn_props::Value;
use lntrn_ui::{Action, Dialog, AreaCx, ShellRequest, Ui, prefs};

use crate::app::{APP_ID, App, Editor, Goto, TabState};
use crate::diff_view::{draw_diff, draw_diff_header};
use crate::files::{FilesCx, draw_files};
use crate::git::view::draw_git;
use crate::preview::draw_preview;
use crate::problems::{ProblemRow, draw_problems};
use crate::search::view::draw_search;
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
                // The file being edited shows in the tree when it changes.
                if self.focus_doc != self.last_revealed {
                    self.last_revealed = self.focus_doc;
                    if let Some(p) = self.focus_doc().and_then(|d| d.path.clone()) {
                        if p.starts_with(&self.tree.root) {
                            self.tree.reveal = Some(p.clone());
                        }
                        self.tree.selected = Some(p);
                    }
                }
                let counts = self.problem_counts();
                let App { icons, tree, git: git_state, settings, project: proj, .. } = self;
                let git = git_state.as_ref().map(|g| (g, &settings.git));
                let project = proj.as_ref().map(|p| p.root.as_path());
                let cxf = FilesCx { icons, git, colors: &settings.colors, problems: &counts, project };
                let out = draw_files(ui, tree, cxf);
                if let Some(p) = out.open {
                    self.pending_paths.push(p);
                    cx.rebuild();
                }
                if let Some(d) = out.set_project {
                    self.pending_folder = Some(d);
                    cx.rebuild();
                }
                if let Some(at) = out.menu_at {
                    cx.request(ShellRequest::MenuAt("files".to_owned(), at));
                }
                // A drop asks first: a slip of the mouse must not move a folder.
                if let Some((from, to_dir)) = out.moved {
                    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let what = if from.is_dir() { "the folder" } else { "the file" };
                    let dest = to_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| to_dir.display().to_string());
                    let action = Action::new(crate::commands::MOVE_PATH).with("from", Value::Str(from.display().to_string())).with("to", Value::Str(to_dir.display().to_string()));
                    cx.request(ShellRequest::Dialog(Dialog::confirm("Move", &format!("Move {what} **{name}** into **{dest}**?"), "Move", action)));
                }
                if let Some((path, name)) = out.renamed {
                    let msg = self.rename_path(&path, &name);
                    cx.host().toast(&msg);
                    cx.rebuild();
                }
                if let Some((path, is_dir)) = out.created {
                    let msg = self.create_path(&path, is_dir);
                    cx.host().toast(&msg);
                    cx.rebuild();
                }
                if let Some(at) = out.context {
                    cx.request(ShellRequest::MenuAt("files-context".to_owned(), at));
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
                        self.pending_goto = Some((path, Goto::Printed { line, col }));
                        cx.rebuild();
                    }
                    if let Some(url) = out.open_url {
                        open_url(&url);
                        cx.host().toast(&format!("Opening {url}"));
                    }
                    if let Some(at) = out.context {
                        self.context_term = Some(id);
                        cx.request(ShellRequest::MenuAt("terminal".to_owned(), at));
                    }
                }
                false
            }
            Editor::Problems => {
                let all = self.problems();
                let rows: Vec<ProblemRow> = all.iter().map(|p| ProblemRow { severity: p.severity, place: format!("{}:{}:{} · {}", p.shown, p.line, p.col, p.source), message: p.message.clone(), openable: p.path.is_some() }).collect();
                let out = draw_problems(ui, &rows);
                let target = out.open.and_then(|i| all.get(i)).and_then(|p| p.path.clone().map(|path| (path, p.line, p.col)));
                if let Some((p, line, col)) = target {
                    self.pending_paths.push(p.clone());
                    self.pending_goto = Some((p, Goto::Printed { line: Some(line), col: Some(col) }));
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
            Editor::Git => {
                let Some(g) = self.git.as_mut() else {
                    ui.heading("Git");
                    ui.label_dim("The open folder is not in a git repository.");
                    return false;
                };
                let out = draw_git(ui, g, &self.settings.git);
                if out.refresh {
                    g.request_status();
                }
                for args in out.run {
                    g.run(args);
                }
                if out.push {
                    g.push();
                }
                if out.pull {
                    g.pull();
                }
                if out.fetch {
                    g.fetch();
                }
                if let Some(b) = out.checkout {
                    g.checkout(&b);
                }
                if out.more_log {
                    g.more_log();
                }
                if let Some((hash, short, rel)) = out.commit_diff {
                    g.request_commit_diff(&hash, &short, &rel);
                }
                if out.new_branch {
                    self.run_action(&Action::new(crate::commands::GIT_BRANCH_NEW), &mut cx.host());
                }
                if let Some((c, staged, at)) = out.context {
                    self.context_change = Some((c.path.clone(), c.rel.clone(), staged, c.status.untracked()));
                    cx.request(ShellRequest::MenuAt("git-file".to_owned(), at));
                }
                if let Some(p) = out.open {
                    self.pending_paths.push(p);
                    cx.rebuild();
                }
                if let Some(p) = out.diff {
                    self.pending_git_diff = Some(p);
                    cx.rebuild();
                }
                false
            }
            Editor::Search => {
                let out = draw_search(ui, &mut self.search, self.project.as_ref());
                if out.run {
                    self.run_search();
                    cx.rebuild();
                }
                if let Some((path, line, col, len)) = out.open {
                    self.pending_paths.push(path.clone());
                    self.pending_goto = Some((path, Goto::Span { line, col, len }));
                    cx.rebuild();
                }
                false
            }
            Editor::Preview => {
                let App { docs, focus_doc, preview_follow, .. } = self;
                let doc = focus_doc.and_then(|id| docs.iter().find(|d| d.id == id));
                draw_preview(ui, doc, preview_follow);
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

/// Hand a web address to the desktop's browser, without waiting on it.
fn open_url(url: &str) {
    use std::process::{Command, Stdio};
    match Command::new("xdg-open").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => lntrn_core::log_warn!("open url: {e}"),
    }
}
