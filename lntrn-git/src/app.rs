//! The app as the shell sees it: one editor showing the repos down the
//! left and the open one beside them, its menu and palette, and what its
//! actions do. Git itself runs on the worker; this sends it commands and
//! takes in what comes back.

use std::path::{Path, PathBuf};

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_kit::desktop::{Desktop, Follow};
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, FILL, Host, HostCx, Key, KeyConfig, KeyItem, KeyPress, Menu, MenuItem, Modifiers, Shell, Trigger, Ui, actions};

use crate::dialogs;
use crate::git;
use crate::history::Graph;
use crate::sidebar;
use crate::state::{self, CloneForm, Dialogs, HISTORY_COMMITS, NewRepo, Repo, View};
use crate::views;
use crate::worker::{Cmd, Event, Link, first_line};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Git,
}

const EDITORS: [Editor; 1] = [Editor::Git];

pub struct App {
    pub link: Link,
    follow: Follow,
    keys: KeyConfig,
    /// The repos found under `~/Projects`.
    pub repos: Vec<PathBuf>,
    /// That search has come back.
    pub repos_found: bool,
    pub view: View,
    pub repo: Option<Repo>,
    /// The repos the open one is nested in when it is a submodule,
    /// outermost first.
    pub parents: Vec<PathBuf>,
    /// How many repos have been opened: what the worker says about an
    /// earlier one is dropped.
    opens: u64,
    /// The worker has something still to do, and what to call it.
    pub busy: bool,
    doing: &'static str,
    pub new_repo: NewRepo,
    pub clone: CloneForm,
    pub dialogs: Dialogs,
    was_focused: bool,
}

impl App {
    /// The app with the worker started and the repo list asked for;
    /// `open` is the repo to start in.
    pub fn new(open: Option<PathBuf>) -> App {
        let mut keys = KeyConfig::default();
        keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(Key::Char('q'), Modifiers::CTRL), actions::QUIT));
        keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(Key::F(3), Modifiers::NONE), actions::PALETTE));
        keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(Key::F(5), Modifiers::NONE), "repo.refresh"));
        keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(Key::Char('r'), Modifiers::CTRL), "repo.refresh"));
        let mut app = App {
            link: Link::spawn(),
            follow: Follow::default(),
            keys,
            repos: Vec::new(),
            repos_found: false,
            view: View::Welcome,
            repo: None,
            parents: Vec::new(),
            opens: 0,
            busy: false,
            doing: "",
            new_repo: NewRepo::default(),
            clone: CloneForm::default(),
            dialogs: Dialogs::default(),
            was_focused: false,
        };
        app.send(Cmd::FindRepos, "");
        if let Some(path) = open {
            app.open(path);
        }
        app
    }

    /// The desktop's look as it was read at the start: its font and how
    /// see-through its windows are.
    pub fn desktop(&self) -> &Desktop {
        self.follow.desktop()
    }

    /// Hand the worker something to do; `doing` is what the title bar
    /// says meanwhile.
    pub fn send(&mut self, cmd: Cmd, doing: &'static str) {
        self.busy = true;
        if !doing.is_empty() {
            self.doing = doing;
        }
        self.link.send(cmd);
    }

    fn show(&mut self, path: PathBuf) {
        self.view = View::Repo;
        self.opens += 1;
        self.send(Cmd::Open(path.clone()), "");
        self.repo = Some(Repo::new(path));
    }

    /// Open a repo from the sidebar: it is nobody's submodule.
    pub fn open(&mut self, path: PathBuf) {
        self.parents.clear();
        state::remember_repo(&path);
        self.show(path);
    }

    /// Go into a submodule of the open repo.
    pub fn open_submodule(&mut self, path: PathBuf) {
        if let Some(repo) = &self.repo {
            self.parents.push(repo.path.clone());
        }
        self.show(path);
    }

    /// Leave a submodule for the repo it is in.
    pub fn back(&mut self) {
        if let Some(parent) = self.parents.pop() {
            self.show(parent);
        }
    }

    /// The repo the sidebar marks: the open one, or the outermost one it
    /// is nested in.
    pub fn root(&self) -> Option<&Path> {
        self.parents.first().map(PathBuf::as_path).or(self.repo.as_ref().filter(|_| self.view == View::Repo).map(|r| r.path.as_path()))
    }

    /// Take in what the worker has sent since the last frame.
    fn take_events(&mut self, cx: &mut AreaCx<()>) {
        for (opens, event) in self.link.take() {
            // What is said about a repo is for the one open when it was
            // asked: drop it if another has been opened since.
            let current = opens == self.opens;
            match (event, self.repo.as_mut().filter(|_| current)) {
                (Event::Repos(repos), _) => {
                    self.repos = repos;
                    self.repos_found = true;
                }
                (Event::Idle(_), _) => {}
                (Event::GitHubRepos(answer), _) => {
                    self.clone.loading = false;
                    self.clone.asked = true;
                    match answer {
                        Ok(repos) => (self.clone.repos, self.clone.error) = (repos, None),
                        Err(why) => self.clone.error = Some(why),
                    }
                }
                (Event::Cloned(result), _) => {
                    let name = self.clone.cloning.take().unwrap_or_default();
                    match result {
                        Ok(path) => {
                            cx.toast(&format!("Cloned {name}"));
                            self.link.send(Cmd::FindRepos);
                            self.open(path);
                        }
                        Err(why) => self.clone.error = Some(why),
                    }
                }
                (Event::Created(result), _) => {
                    self.new_repo.creating = false;
                    match result {
                        Ok((path, complaint)) => {
                            self.new_repo = NewRepo::default();
                            self.link.send(Cmd::FindRepos);
                            self.open(path);
                            if let (Some(repo), Some(why)) = (&mut self.repo, complaint) {
                                repo.error = Some(format!("The repository was made here, but not on GitHub: {why}"));
                            }
                        }
                        Err(why) => self.new_repo.error = Some(why),
                    }
                }
                (Event::Status(status), Some(repo)) => repo.on_status(status),
                (Event::Branches(branches), Some(repo)) => repo.branches = branches,
                (Event::History(commits), Some(repo)) => repo.graph = Some(Graph::new(commits)),
                (Event::Diff { file, tag, diff }, Some(repo)) => repo.on_diff(&file, tag, diff),
                (Event::Done(said), Some(repo)) => {
                    repo.error = None;
                    repo.history_stale = true;
                    let line = first_line(&said);
                    if !line.is_empty() {
                        cx.toast(line);
                    }
                }
                (Event::Failed(why), Some(repo)) => {
                    repo.error = Some(why);
                    repo.history_stale = true;
                }
                (_, None) => {}
            }
        }
        self.busy = self.link.busy();
        if !self.busy {
            self.doing = "";
        }
    }

    /// Read the open repo again: files, branches, and the history next
    /// time it shows.
    pub fn refresh(&mut self, doing: &'static str) {
        if let Some(repo) = &mut self.repo {
            repo.history_stale = true;
            self.send(Cmd::Refresh, doing);
        }
    }

    /// Show the new-repository or the clone page.
    pub fn go(&mut self, view: View) {
        self.view = view;
        if view == View::Clone && !self.clone.asked && !self.clone.loading {
            self.clone.loading = true;
            self.send(Cmd::GitHubRepos, "Asking GitHub…");
        }
    }

    /// Ask for the history when its tab shows and what is there is old.
    pub fn want_history(&mut self) {
        if let Some(repo) = &mut self.repo
            && repo.history_stale
        {
            repo.history_stale = false;
            self.send(Cmd::History(HISTORY_COMMITS), "");
        }
    }
}

impl Host for App {
    type Editor = Editor;
    type AreaState = ();

    fn editors(&self) -> &[Editor] {
        &EDITORS
    }

    fn editor_label(&self, _editor: Editor) -> &str {
        "Git"
    }

    fn title(&self) -> String {
        "Lantern Git".to_owned()
    }

    fn status(&self) -> String {
        self.doing.to_owned()
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        false
    }

    fn title_menus(&self) -> &[(&str, &str)] {
        &[("Git", "git")]
    }

    fn menu(&self, name: &str) -> Option<Menu> {
        let open = self.repo.is_some() && self.view == View::Repo;
        (name == "git").then(|| {
            Menu::new(
                "Git",
                vec![
                    MenuItem::new("Refresh", Action::new("repo.refresh")).enabled(open),
                    MenuItem::new("Pull", Action::new("repo.pull")).enabled(open),
                    MenuItem::new("Push", Action::new("repo.push")).enabled(open),
                    MenuItem::separator(),
                    MenuItem::new("New Branch…", Action::new("branch.new")).enabled(open),
                    MenuItem::new("Merge…", Action::new("merge.open")).enabled(open),
                    MenuItem::separator(),
                    MenuItem::new("New Repository…", Action::new("view.new")),
                    MenuItem::new("Clone from GitHub…", Action::new("view.clone")),
                    MenuItem::separator(),
                    MenuItem::pref_toggle("Reduce Motion", "reduce_motion"),
                    MenuItem::new("Quit", Action::new(actions::QUIT)),
                ],
            )
        })
    }

    fn palette(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_lowercase();
        let repos = self.repos.iter().enumerate().map(|(i, p)| (format!("open.{i}"), format!("Open {}", git::repo_name(p))));
        let rest = [("repo.refresh", "Refresh"), ("repo.pull", "Pull"), ("repo.push", "Push"), ("branch.new", "New Branch…"), ("merge.open", "Merge…"), ("view.new", "New Repository…"), ("view.clone", "Clone from GitHub…"), (actions::QUIT, "Quit")];
        repos.chain(rest.into_iter().map(|(id, label)| (id.to_owned(), label.to_owned()))).filter(|(_, label)| label.to_lowercase().contains(&q)).collect()
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action)
    }

    fn draw_body(&mut self, _editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        self.take_events(cx);
        let width = ui.m.px(sidebar::WIDTH);
        ui.columns(&[width, FILL], |ui, col| match col {
            0 => sidebar::draw(self, ui),
            _ => views::draw(self, ui, cx),
        });
        false
    }

    fn draw_item(&mut self, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
        dialogs::draw(self, key, ui, cx)
    }

    fn dialog_ready(&self, action: &Action) -> bool {
        dialogs::ready(self, &action.id)
    }

    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        let path = || action.arg("path").and_then(|v| v.as_str()).map(str::to_owned);
        if let Some(i) = action.id.strip_prefix("open.").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(repo) = self.repos.get(i).cloned() {
                self.open(repo);
            }
            return;
        }
        match action.id.as_str() {
            "view.new" => self.go(View::NewRepo),
            "view.clone" => self.go(View::Clone),
            "newrepo.folder" => self.new_repo.parent = path().unwrap_or_default(),
            "clone.folder" => self.clone.dest = path().unwrap_or_default(),
            _ if self.repo.is_none() || self.view != View::Repo => {}
            "repo.refresh" => self.refresh("Refreshing…"),
            "repo.pull" => self.send(Cmd::Pull, "Pulling…"),
            "repo.push" => self.send(Cmd::Push, "Pushing…"),
            "branch.new" => cx.request(dialogs::new_branch(self)),
            "branch.create" => {
                let name = git::branch_name(&self.dialogs.branch_name);
                self.send(Cmd::CreateBranch { name, push: self.dialogs.branch_push }, "Making the branch…");
            }
            "merge.open" => cx.request(dialogs::merge(self)),
            "merge.run" => {
                let names = |i: usize| self.repo.as_ref().and_then(|r| r.branches.get(i)).map(|b| b.name.clone());
                if let (Some(source), Some(target)) = (names(self.dialogs.merge_from), names(self.dialogs.merge_into)) {
                    self.send(Cmd::Merge { source, target }, "Merging…");
                }
            }
            "file.discard" => {
                if let Some(file) = self.dialogs.discard.take() {
                    self.send(Cmd::Discard(file), "Discarding…");
                }
            }
            other => cx.toast(&format!("unknown action {other}")),
        }
    }

    fn key(&self, press: KeyPress, _editor: Option<Editor>) -> Option<Action> {
        self.keys.resolve(&[CTX_WINDOW], &press.to_event(), |_| true).map(KeyItem::action)
    }
}

impl AppHost for App {
    fn waker(&mut self, waker: Waker) {
        self.link.set_waker(waker);
    }

    fn after_rebuild(&mut self, _gpu: &Gpu, _images: &mut Images, shell: &mut Shell<Self>) -> bool {
        // Coming back to the window: files may have changed meanwhile.
        let focused = shell.window_focused;
        if focused && !self.was_focused && self.view == View::Repo && !self.busy {
            self.refresh("");
        }
        self.was_focused = focused;
        self.follow.apply(shell)
    }
}
