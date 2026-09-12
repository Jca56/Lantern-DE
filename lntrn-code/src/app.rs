//! The app as the shell sees it: its editors, what each draws, its menus
//! and palette, and its documents, terminals and project. Requests that
//! change the layout (open a file, show the terminal) are queued here and
//! applied once the rebuild is over ([`crate::pending`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_ui::{AreaId, KeyConfig, Shell, ShellRequest, WidgetId};

use crate::buffer::Pos;
use crate::bridge::Bridge;
use crate::commands;
use crate::diff_view::{DiffDoc, DiffId};
use crate::doc::{Doc, DocId};
use crate::ide::PendingSelect;
use crate::editor::find::Finder;
use crate::files::{Project, Tree, home};
use crate::git::Git;
use crate::git::gutter::LineMark;
use crate::lsp::{CodeAction, Lsp};
use crate::editor::lsp_ui::LspUi;
use crate::search::Search;
use crate::session::Session;
use crate::settings::Settings;
use crate::term::Terminal;
use crate::watch::Watcher;

pub const APP_ID: &str = "lntrn-code";

pub use crate::model::{ClipOp, EDITORS, Editor, Goto, TabState};

pub struct App {
    pub keys: KeyConfig,
    pub settings: Settings,
    pub docs: Vec<Doc>,
    next_doc: u64,
    untitled: usize,
    /// The folder the app works in: git, the language servers, search and
    /// the Claude bridge hang off it. Set on purpose, never by browsing.
    pub project: Option<Project>,
    /// The Files panel's tree, looking wherever the user took it.
    pub tree: Tree,
    /// The document the tree last revealed (its folders opened).
    pub last_revealed: Option<DocId>,
    pub terminals: Vec<Terminal>,
    pub(crate) next_term: u64,
    pub finder: Finder,
    pub search: Search,
    /// The language servers and their popups in the code view.
    pub lsp: Lsp,
    pub lsp_ui: LspUi,
    /// The fixes on offer while their menu is up.
    pub code_actions: Vec<CodeAction>,
    /// Saves waiting on a format from the server, and when to stop waiting.
    pub pending_saves: Vec<(DocId, std::time::Instant)>,
    /// The project's repository, when it is in one.
    pub git: Option<Git>,
    /// Gutter marks per document: `(last edit, HEAD, marks)`.
    pub(crate) git_marks: HashMap<DocId, (f64, String, Vec<LineMark>)>,
    /// A file whose diff against HEAD was asked for.
    pub pending_git_diff: Option<PathBuf>,
    /// The document actions act on: the current one of the last active
    /// Code area.
    pub focus_doc: Option<DocId>,
    pub focus_area: Option<AreaId>,
    pub last_editor_focus: Option<WidgetId>,
    /// The terminal last right-clicked: what its menu acts on.
    pub context_term: Option<crate::term::TermId>,
    /// The terminal area that is active, so the frame one becomes active
    /// (and takes the keyboard) is told apart from every frame after.
    pub term_active_area: Option<AreaId>,
    /// File and folder icons from the theme on disk.
    pub icons: crate::icons::IconTheme,
    /// The changed file last right-clicked in the Git editor: its path,
    /// its path in the repository, whether it was in the staged list,
    /// whether it is untracked.
    pub context_change: Option<(PathBuf, String, bool, bool)>,
    /// The tab last right-clicked and the tabs beside it in its area.
    pub context_tab: Option<(DocId, Vec<DocId>)>,
    /// The document and caret line the Preview last scrolled to.
    pub preview_follow: Option<(DocId, usize)>,
    /// A terminal paste asked for: the shell reads the clipboard next frame.
    pub pending_clipboard_wanted: bool,
    pub prefs_tab: usize,
    /// Text typed into the open dialog's field.
    pub dialog_text: String,
    // ---- applied after the rebuild, with the screen in hand ----
    pub pending_paths: Vec<PathBuf>,
    pub pending_docs: Vec<DocId>,
    /// Put the caret somewhere in a file once it is open: a path clicked
    /// in the terminal, a problem, a search hit.
    pub pending_goto: Option<(PathBuf, Goto)>,
    pub pending_folder: Option<PathBuf>,
    pub pending_show: Vec<Editor>,
    pub pending_new_terminal: Option<Option<PathBuf>>,
    pub pending_close: Vec<DocId>,
    pub pending_cycle: Option<i32>,
    pub pending_clip: Option<ClipOp>,
    pub pending_focus: Option<WidgetId>,
    /// A file dragged out of the tree and let go: what, where, when. A
    /// terminal drawn under the point types its path; else it expires.
    pub pending_drop: Option<(PathBuf, lntrn_math::Vec2, f64)>,
    pub(crate) paste_armed: bool,
    pub(crate) popup_was_open: bool,
    pub session_dirty: bool,
    /// Quit was confirmed in the unsaved-changes dialog: the next close
    /// goes straight through instead of asking again.
    pub(crate) quit_confirmed: bool,
    pub(crate) session: Session,
    /// The loop's waker, for the terminals' reader threads.
    pub(crate) waker: Option<Waker>,
    // ---- Claude Code ----
    pub bridge: Option<Bridge>,
    pub ide_connected: usize,
    pub diffs: Vec<DiffDoc>,
    pub next_diff: u64,
    /// The diff shown in the active area, what Accept and Reject act on.
    pub focus_diff: Option<DiffId>,
    pub pending_show_diff: Option<DiffId>,
    pub pending_diff_resolve: Vec<(DiffId, bool)>,
    pub pending_select: Option<PendingSelect>,
    pub last_selection_sent: String,
    pub pending_ide_send: bool,
    /// Accepted diffs the CLI should write: `(path, text, deadline)`.
    pub pending_writes: Vec<(PathBuf, String, std::time::Instant)>,
    started: std::time::Instant,
    watcher: Option<Watcher>,
}

impl App {
    pub fn new(settings: Settings, session: Session, args: Vec<PathBuf>) -> Self {
        let mut app = Self {
            keys: commands::keymap(),
            settings,
            docs: Vec::new(),
            next_doc: 1,
            untitled: 0,
            project: None,
            tree: Tree::new(session.root.clone().unwrap_or_else(home)),
            last_revealed: None,
            terminals: Vec::new(),
            next_term: 1,
            finder: Finder::default(),
            search: Search::default(),
            lsp: Lsp::default(),
            lsp_ui: LspUi::default(),
            code_actions: Vec::new(),
            pending_saves: Vec::new(),
            git: None,
            git_marks: HashMap::new(),
            pending_git_diff: None,
            focus_doc: None,
            focus_area: None,
            last_editor_focus: None,
            context_term: None,
            term_active_area: None,
            context_tab: None,
            context_change: None,
            icons: crate::icons::IconTheme::load(),
            preview_follow: None,
            pending_clipboard_wanted: false,
            prefs_tab: 0,
            dialog_text: String::new(),
            pending_paths: Vec::new(),
            pending_docs: Vec::new(),
            pending_goto: None,
            pending_folder: None,
            pending_show: Vec::new(),
            pending_new_terminal: None,
            pending_close: Vec::new(),
            pending_cycle: None,
            pending_clip: None,
            pending_focus: None,
            pending_drop: None,
            paste_armed: false,
            popup_was_open: false,
            session_dirty: false,
            quit_confirmed: false,
            session: session.clone(),
            waker: None,
            bridge: None,
            ide_connected: 0,
            diffs: Vec::new(),
            next_diff: 1,
            focus_diff: None,
            pending_show_diff: None,
            pending_diff_resolve: Vec::new(),
            pending_select: None,
            last_selection_sent: String::new(),
            pending_ide_send: false,
            pending_writes: Vec::new(),
            started: std::time::Instant::now(),
            watcher: None,
        };
        if args.is_empty() {
            app.pending_folder = session.root.clone();
            app.pending_paths = session.open.iter().map(|(p, _, _)| p.clone()).collect();
        } else {
            for a in args {
                let a = std::path::absolute(&a).unwrap_or(a);
                if a.is_dir() {
                    app.pending_folder = Some(a);
                } else {
                    app.pending_paths.push(a);
                }
            }
        }
        app
    }

    pub fn doc(&self, id: DocId) -> Option<&Doc> {
        self.docs.iter().find(|d| d.id == id)
    }

    pub fn doc_mut(&mut self, id: DocId) -> Option<&mut Doc> {
        self.docs.iter_mut().find(|d| d.id == id)
    }

    pub fn focus_doc(&self) -> Option<&Doc> {
        self.focus_doc.and_then(|id| self.doc(id))
    }

    pub fn focus_doc_mut(&mut self) -> Option<&mut Doc> {
        let id = self.focus_doc?;
        self.doc_mut(id)
    }

    fn next_doc_id(&mut self) -> DocId {
        let id = DocId(self.next_doc);
        self.next_doc += 1;
        id
    }

    /// A fresh empty document.
    pub fn new_untitled(&mut self) -> DocId {
        let id = self.next_doc_id();
        self.untitled += 1;
        self.docs.push(Doc::untitled(id, self.untitled, self.settings.tab()));
        id
    }

    /// The document for `path`, read now if it is not open yet.
    pub fn load_doc(&mut self, path: &Path) -> Result<DocId, String> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(d) = self.docs.iter().find(|d| d.path.as_deref() == Some(path.as_path())) {
            return Ok(d.id);
        }
        let id = self.next_doc_id();
        let mut doc = Doc::open(id, &path, self.settings.tab()).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some((l, c)) = self.session.caret(&path) {
            doc.set_cursor(Pos::new(l, c), false);
        }
        self.docs.push(doc);
        self.session_dirty = true;
        Ok(id)
    }

    /// Make `root` the project; the tree shows it too.
    pub fn set_project(&mut self, root: PathBuf) {
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        self.tree.go(root.clone());
        if self.project.as_ref().is_some_and(|p| p.root == root) {
            return;
        }
        self.git = Git::find(&root, self.waker.clone());
        self.git_marks.clear();
        self.lsp.set_root(Some(root.clone()));
        self.lsp_ui.close_all();
        self.session.remember(&root);
        self.project = Some(Project::new(root));
        self.session_dirty = true;
    }

    /// Something on disk changed under our hands: the tree reads its
    /// folders again and the quick-open list is rebuilt.
    pub(crate) fn refresh_tree(&mut self) {
        self.tree.refresh();
        self.icons.forget_names();
        if let Some(p) = self.project.as_mut() {
            p.refresh();
        }
    }

    /// Seconds since the app started: the same clock the widgets stamp
    /// edits with.
    pub fn clock(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// The document at `path`, open or not (its canonical form too).
    pub(crate) fn doc_by_path(&self, path: &Path) -> Option<usize> {
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.docs.iter().position(|d| d.path.as_deref().is_some_and(|p| p == path || p == canon))
    }

    /// Run the project search for the query as it is now: over the
    /// project's files, with unsaved documents as they are in the editor.
    pub(crate) fn run_search(&mut self) {
        let files = match self.project.as_mut() {
            Some(p) => p.files().to_vec(),
            None => Vec::new(),
        };
        let overrides: Vec<(PathBuf, String)> = self.docs.iter().filter(|d| d.is_dirty()).filter_map(|d| Some((d.path.clone()?, d.buffer.to_text()))).collect();
        self.search.start(files, overrides, self.waker.clone());
    }

    /// The folder new things go in: the project, else home.
    pub fn base_dir(&self) -> PathBuf {
        self.project.as_ref().map(|p| p.root.clone()).or_else(|| std::env::var_os("HOME").map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("/"))
    }

    fn save_session(&mut self) {
        self.session = Session {
            root: self.project.as_ref().map(|p| p.root.clone()),
            open: self.docs.iter().filter_map(|d| d.path.as_ref().map(|p| (p.clone(), d.cursor.line, d.cursor.col))).collect(),
            recent: self.session.recent.clone(),
        };
        self.session.save(APP_ID);
    }

    /// Keep the watches on the open files' folders and the listed project
    /// folders, then act on what changed: folders are re-read, clean
    /// files reloaded, dirty ones flagged.
    fn watch_pump(&mut self, shell: &mut Shell<Self>) -> bool {
        let Some(mut watcher) = self.watcher.take() else {
            return false;
        };
        let mut wanted: HashSet<PathBuf> = self.docs.iter().filter_map(|d| d.path.as_ref()?.parent().map(Path::to_path_buf)).collect();
        wanted.extend(self.tree.listed_dirs().map(Path::to_path_buf));
        // Commits and staging from a terminal show up as writes in .git.
        let git_dir = self.git.as_ref().map(|g| g.root.join(".git")).filter(|d| d.is_dir());
        if let Some(d) = &git_dir {
            wanted.insert(d.clone());
        }
        // The desktop's settings: the window follows its opacity live (U037).
        let config_dir = lantern_config_dir().filter(|d| d.is_dir());
        if let Some(d) = &config_dir {
            wanted.insert(d.clone());
        }
        watcher.retain(|p| wanted.contains(p));
        for dir in &wanted {
            watcher.watch(dir);
        }
        let mut again = false;
        let now = shell.state.now;
        for c in watcher.poll() {
            again = true;
            if config_dir.as_deref() == Some(c.dir.as_path()) {
                if c.name.as_deref() == Some("lantern.toml") {
                    shell.opacity = window_opacity();
                }
                continue;
            }
            let in_git = git_dir.as_deref() == Some(c.dir.as_path());
            // Lock files come and go with every git command; they say nothing.
            if in_git && c.name.as_deref().is_some_and(|n| n.ends_with(".lock")) {
                continue;
            }
            if let Some(g) = self.git.as_mut() {
                g.mark_dirty(now);
            }
            if in_git {
                continue;
            }
            if c.is_listing() {
                self.tree.invalidate(&c.dir);
                if let Some(p) = self.project.as_mut()
                    && c.dir.starts_with(&p.root)
                {
                    p.refresh();
                }
            }
            let hits: Vec<usize> = self.docs.iter().enumerate().filter(|(_, d)| d.path.as_ref().is_some_and(|p| p.parent() == Some(c.dir.as_path()) && (c.name.is_none() || p.file_name().and_then(|n| n.to_str()) == c.name.as_deref()))).map(|(i, _)| i).collect();
            for i in hits {
                let path = self.docs[i].path.clone().unwrap_or_default();
                if c.is_removal() && !path.exists() {
                    self.docs[i].disk_missing = true;
                    continue;
                }
                if !c.is_write() {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&bytes);
                let doc = &mut self.docs[i];
                if text == doc.buffer.to_text() {
                    doc.disk_missing = false;
                    continue;
                }
                if doc.is_dirty() {
                    doc.disk_changed = true;
                } else {
                    doc.replace_all(&text, now);
                    let msg = format!("Reloaded {}", doc.title);
                    shell.request(self, ShellRequest::Toast(msg));
                }
            }
        }
        self.watcher = Some(watcher);
        again
    }

}

impl AppHost for App {
    fn waker(&mut self, waker: Waker) {
        self.ide_start(waker.clone());
        self.lsp.set_waker(waker.clone());
        if let Some(p) = &self.project {
            self.git = Git::find(&p.root, Some(waker.clone()));
        }
        match Watcher::new(Some(waker.clone())) {
            Ok(w) => self.watcher = Some(w),
            Err(e) => lntrn_core::log_warn!("file watching off: {e}"),
        }
        self.waker = Some(waker);
    }

    fn after_rebuild(&mut self, gpu: &Gpu, images: &mut Images, shell: &mut Shell<Self>) -> bool {
        self.ide_sync_roots();
        let mut again = self.ide_pump(shell);
        again |= self.icons.upload(gpu, images);
        again |= self.ide_settle_writes(shell);
        again |= self.watch_pump(shell);
        again |= self.search.poll();
        again |= self.lsp_pump(shell);
        again |= self.settle_saves(shell);
        // Every terminal, shown or not: a hidden tab's output is read in
        // as it comes, so the program there never blocks on a full pipe.
        let now = shell.state.now;
        for t in &mut self.terminals {
            again |= t.pump(now);
        }
        again |= self.settle_bells(shell);
        if self.pending_drop.as_ref().is_some_and(|(_, _, at)| now - at > 0.5) {
            self.pending_drop = None;
        }
        again |= self.git_poll(shell);
        again |= self.apply_pending(shell);
        if shell.title.is_none() {
            self.reap_terminals(shell);
        }
        if std::mem::take(&mut self.session_dirty) {
            self.save_session();
        }
        again
    }
}

/// Where the desktop keeps its settings.
pub(crate) fn lantern_config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".lantern/config"))
}

/// The desktop's `[windows].background_opacity` from
/// `~/.lantern/config/lantern.toml`, or 1.0: the one setting every
/// Lantern window follows.
pub(crate) fn window_opacity() -> f64 {
    let Some(dir) = lantern_config_dir() else { return 1.0 };
    let Ok(text) = std::fs::read_to_string(dir.join("lantern.toml")) else { return 1.0 };
    let mut in_windows = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_windows = l == "[windows]";
        } else if in_windows
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == "background_opacity"
        {
            return v.trim().parse::<f64>().map_or(1.0, |o| o.clamp(0.05, 1.0));
        }
    }
    1.0
}
