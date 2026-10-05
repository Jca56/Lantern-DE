//! Headless runs on Lantern UI's test harness, no window and no GPU,
//! against a real repo made for the purpose: the app is clicked and typed
//! at the way a hand would, the worker runs real git, and what git then
//! says is checked.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering};

use lntrn_kit::{look, probe};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, Shell, WidgetId};

use crate::app::{App, Editor};
use crate::github::RemoteRepo;
use crate::state::{Tab, View};

/// Point every folder the app reads at a scratch tree, and give git a
/// name to commit under that is nobody's: nothing here touches the real
/// home, the real config or the real git identity.
fn sandbox() -> PathBuf {
    static ONCE: Once = Once::new();
    let root = std::env::temp_dir().join(format!("lntrn-git-smoke-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Projects")).unwrap();
        // SAFETY: set once, before anything here reads them or starts a
        // thread that does.
        unsafe {
            std::env::set_var("HOME", &root);
            std::env::set_var("LANTERN_HOME", root.join("lantern"));
            std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
            for who in ["AUTHOR", "COMMITTER"] {
                std::env::set_var(format!("GIT_{who}_NAME"), "Smoke Test");
                std::env::set_var(format!("GIT_{who}_EMAIL"), "smoke@example.invalid");
            }
        }
    });
    root
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(repo).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A repo of its own for one test: two commits on `main`, one more on
/// `side`, then `main` checked out with one file changed and one new.
fn fixture() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let repo = sandbox().join("Projects").join(format!("demo-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("notes.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn answer() -> u32 {\n    42\n}\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "First"]);
    git(&repo, &["checkout", "-q", "-b", "side"]);
    std::fs::write(repo.join("side.txt"), "from the side\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "Side work"]);
    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(repo.join("notes.txt"), "one\n2\nthree\n").unwrap();
    std::fs::write(repo.join("new.txt"), "brand new\n").unwrap();
    repo
}

struct Rig {
    h: Harness,
    shell: Shell<App>,
    app: App,
}

impl Rig {
    /// The app in a window `width` × `height` logical pixels at `scale`,
    /// with `repo` open.
    fn at(repo: Option<PathBuf>, width: f64, height: f64, scale: f64) -> Rig {
        sandbox();
        probe::watch();
        let mut h = Harness::new(width * scale, height * scale);
        h.scale = scale;
        let mut shell = Shell::new(Editor::Git);
        shell.prefs.theme = look::theme(look::GOLD);
        let mut rig = Rig { h, shell, app: App::new(repo) };
        rig.settle();
        rig
    }

    fn open(repo: PathBuf) -> Rig {
        Rig::at(Some(repo), 1152.0, 720.0, 1.0)
    }

    /// Frames until the worker has nothing left to do and the layout has
    /// stopped asking for another pass.
    fn settle(&mut self) {
        for _ in 0..1000 {
            let out = self.h.shell_settle(&mut self.shell, &mut self.app, 8);
            if !self.app.busy && !out.rebuild_again {
                // One more for what the last events asked for (a diff).
                self.h.shell_settle(&mut self.shell, &mut self.app, 8);
                if !self.app.busy {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the worker never went idle");
    }

    /// The id of something `path` deep in the window's one area.
    fn id(&self, path: impl Fn(WidgetId) -> WidgetId) -> Option<WidgetId> {
        (0..8).map(|area| path(WidgetId::ROOT.with_u64(area).with("body"))).find(|id| self.h.rect_of(*id).is_some())
    }

    fn rect(&self, what: &str, path: impl Fn(WidgetId) -> WidgetId) -> Rect {
        let id = self.id(path).unwrap_or_else(|| panic!("{what} is not on screen"));
        self.h.rect_of(id).unwrap()
    }

    fn click_at(&mut self, at: Vec2) {
        self.h.move_to(at);
        self.h.press();
        self.h.shell_frame(&mut self.shell, &mut self.app);
        self.h.release();
        self.settle();
    }

    fn click(&mut self, what: &str, path: impl Fn(WidgetId) -> WidgetId) {
        let rect = self.rect(what, path);
        self.click_at(rect.center());
    }

    fn key(&mut self, key: Key) {
        self.h.key(key);
        self.settle();
    }

    fn repo(&self) -> &crate::state::Repo {
        self.app.repo.as_ref().expect("a repo is open")
    }

    fn files(&self, staged: bool) -> Vec<String> {
        self.repo().files(staged).map(|f| format!("{} {}", f.status.letter(), f.path)).collect()
    }
}

/// Inside the open repo, beside the sidebar.
fn in_repo(body: WidgetId) -> WidgetId {
    body.with_index(1).with("repo")
}

/// A file's row in the staged or the unstaged card.
fn file(card: &'static str, i: usize) -> impl Fn(WidgetId) -> WidgetId {
    move |body| in_repo(body).with_index(0).with("files").with(card).with("file").with_index(i)
}

fn tab(i: usize) -> impl Fn(WidgetId) -> WidgetId {
    move |body| in_repo(body).with("tabs").with_index(i)
}

#[test]
fn staging_committing_and_discarding_by_hand() {
    let repo = fixture();
    let mut rig = Rig::open(repo.clone());

    // The repo is in the sidebar, and it opened on its changes with the
    // first one picked and its diff read.
    assert!(rig.app.repos.contains(&repo));
    assert_eq!((rig.files(true), rig.files(false)), (vec![], vec!["M notes.txt".to_owned(), "? new.txt".to_owned()]));
    assert_eq!(rig.repo().picked, Some(("notes.txt".into(), false)));
    let diff = rig.repo().diff().expect("its diff");
    assert_eq!((diff.added, diff.removed), (1, 1));
    assert!(diff.lines.iter().any(|l| l.text == "2") && diff.lines.iter().any(|l| l.text == "two"));

    // Picking the other file shows it instead: new, so every line added.
    rig.click("new.txt", file("unstaged", 1));
    assert_eq!(rig.repo().picked, Some(("new.txt".into(), false)));
    assert_eq!(rig.repo().diff().map(|d| (d.added, d.removed)), Some((1, 0)));
    rig.click("notes.txt", file("unstaged", 0));

    // The + on new.txt stages it; the pick stays where it was.
    rig.click("the + on new.txt", |b| file("unstaged", 1)(b).with("button"));
    assert_eq!((rig.files(true), rig.files(false)), (vec!["A new.txt".to_owned()], vec!["M notes.txt".to_owned()]));
    assert_eq!(rig.repo().picked, Some(("notes.txt".into(), false)));

    // Commit is dead until there is a message; then it commits what is
    // staged and nothing else.
    rig.click("Commit", |b| in_repo(b).with("commit"));
    assert_eq!(git(&repo, &["log", "-1", "--format=%s"]), "First", "no message, no commit");
    rig.click("the message field", |b| in_repo(b).with("message"));
    rig.h.type_text("Add the new file");
    rig.settle();
    assert_eq!(rig.repo().message, "Add the new file");
    rig.click("Commit", |b| in_repo(b).with("commit"));
    assert_eq!(git(&repo, &["log", "-1", "--format=%s"]), "Add the new file");
    assert_eq!(git(&repo, &["show", "--stat", "--format=", "HEAD"]).lines().next().map(|l| l.trim_start().starts_with("new.txt")), Some(true));
    assert!(rig.repo().message.is_empty());
    assert_eq!((rig.files(true), rig.files(false)), (vec![], vec!["M notes.txt".to_owned()]));
    assert!(rig.repo().error.is_none(), "{:?}", rig.repo().error);

    // Discard asks first, and Cancel leaves the file alone.
    let discard = |b: WidgetId| in_repo(b).with_index(1).with("diff").with("discard");
    rig.click("Discard…", discard);
    assert!(rig.app.dialogs.discard.is_some());
    rig.key(Key::Escape);
    assert_eq!(std::fs::read_to_string(repo.join("notes.txt")).unwrap(), "one\n2\nthree\n");
    // Confirmed, the change is gone and there is nothing left to commit.
    rig.click("Discard…", discard);
    rig.key(Key::Enter);
    assert_eq!(std::fs::read_to_string(repo.join("notes.txt")).unwrap(), "one\ntwo\nthree\n");
    assert_eq!((rig.files(true), rig.files(false)), (vec![], vec![]));
    assert_eq!(rig.repo().picked, None);
}

#[test]
fn branches_switch_and_merge_and_the_history_follows() {
    let repo = fixture();
    // The work tree must be clean to switch: put the changes away.
    git(&repo, &["stash", "-q", "-u"]);
    let mut rig = Rig::open(repo.clone());
    assert_eq!(rig.files(false), Vec::<String>::new());

    rig.click("the Branches tab", tab(1));
    assert_eq!(rig.repo().tab, Tab::Branches);
    let names: Vec<(&str, bool, u32, u32)> = rig.repo().branches.iter().map(|b| (b.name.as_str(), b.is_current, b.ahead, b.behind)).collect();
    assert_eq!(names, [("main", true, 0, 0), ("side", false, 1, 0)]);

    // Switch on the side branch's row checks it out.
    let list = |b: WidgetId| in_repo(b).with("branches").with("list");
    rig.click("Switch on side", move |b| list(b).with("switch").with_index(1));
    assert_eq!(git(&repo, &["branch", "--show-current"]), "side");
    assert_eq!(rig.repo().current_branch(), Some(1));
    rig.click("Switch on main", move |b| list(b).with("switch").with_index(0));
    assert_eq!(git(&repo, &["branch", "--show-current"]), "main");

    // The history shows both branches' commits, newest first.
    rig.click("the History tab", tab(2));
    let graph = rig.repo().graph.as_ref().expect("the history");
    let subjects: Vec<&str> = graph.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["Side work", "First"]);

    // Merge… offers side into main (the one checked out); Enter merges.
    rig.click("the Branches tab", tab(1));
    rig.click("Merge…", |b| in_repo(b).with("branches").with("merge"));
    assert_eq!((rig.app.dialogs.merge_from, rig.app.dialogs.merge_into), (1, 0));
    rig.key(Key::Enter);
    assert!(rig.repo().error.is_none(), "{:?}", rig.repo().error);
    assert!(repo.join("side.txt").exists(), "main has side's file");
    assert_eq!(rig.repo().branches[1].ahead, 0, "side has nothing main lacks");

    // New Branch… names one (spaces and all) and switches to it. It is
    // not pushed: there is no remote here.
    rig.click("New Branch…", |b| in_repo(b).with("branches").with("new"));
    rig.app.dialogs.branch_push = false;
    rig.h.type_text("my new idea");
    rig.settle();
    rig.key(Key::Enter);
    assert_eq!(git(&repo, &["branch", "--show-current"]), "my-new-idea");
}

#[test]
fn every_view_settles_at_every_size() {
    let repo = fixture();
    // The first is the smallest the window gets; the second is what the
    // desktop opens it at on a 1920 x 1200 screen.
    for (w, h, scale) in [(1100.0, 700.0, 1.0), (1152.0, 720.0, 1.0), (1920.0, 1160.0, 1.0), (1152.0, 720.0, 1.4)] {
        let mut rig = Rig::at(Some(repo.clone()), w, h, scale);
        // GitHub is not asked in a test: the clone page gets its list here.
        rig.app.clone.asked = true;
        rig.app.clone.repos = (0..30).map(|i| RemoteRepo { name: format!("repo-number-{i}"), full_name: format!("someone/repo-number-{i}"), description: "A description long enough that it has to be cut short to fit on its line".into(), clone_url: "https://example.invalid/x".into(), is_private: i % 2 == 0, is_fork: i % 3 == 0 }).collect();
        probe::take_clipped();
        for tab in Tab::ALL {
            rig.app.repo.as_mut().unwrap().tab = tab;
            rig.settle();
        }
        for view in [View::NewRepo, View::Clone, View::Welcome, View::Repo] {
            rig.app.go(view);
            rig.settle();
        }
        rig.app.repo.as_mut().unwrap().tab = Tab::Changes;
        rig.settle();
        let cut = probe::take_clipped();
        assert!(cut.is_empty(), "at {w}x{h} @{scale}: labels cut short: {cut:?}");

        // The changes tab's three parts sit side by side and stacked as
        // meant: the lists left of the diff, the commit bar under both,
        // all inside the window.
        let window = rig.h.window();
        let first = rig.rect("the first file", file("unstaged", 0));
        let strip = rig.rect("Discard… in the diff's strip", |b| in_repo(b).with_index(1).with("diff").with("discard"));
        let commit = rig.rect("Commit", |b| in_repo(b).with("commit"));
        assert!(first.max.x < strip.min.x, "{first:?} | {strip:?}");
        assert!(commit.min.y > first.max.y && commit.min.y > strip.max.y);
        assert!(window.contains_rect(&commit) && window.contains_rect(&strip), "{commit:?} {strip:?} in {window:?}");
        // A file's row keeps real room for its name beside its button.
        let button = rig.rect("the first file's button", |b| file("unstaged", 0)(b).with("button"));
        assert!(first.width() >= 250.0 * scale && first.max.x <= button.min.x, "{first:?} then {button:?}");
        // The header's buttons are in the window, and the tabs stop short
        // of the branch picker.
        let pull = rig.rect("Pull", |b| in_repo(b).with("pull"));
        let (history, branch) = (rig.rect("the History tab", tab(2)), rig.rect("the branch picker", |b| in_repo(b).with("branch")));
        assert!(window.contains_rect(&pull) && history.max.x < branch.min.x, "{pull:?}, {history:?} | {branch:?}");
        // The sidebar's rows are one width, the list's and the two under it.
        let listed = rig.rect("a repo's row", |b| b.with_index(0).with("sidebar").with("repos").with("repo-0"));
        let new = rig.rect("New Repository", |b| b.with_index(0).with("sidebar").with_index(0).with("new"));
        assert!((listed.min.x - new.min.x).abs() <= 1.0 && (listed.max.x - new.max.x).abs() <= 1.0, "{listed:?} vs {new:?}");
    }
}

#[test]
fn the_sidebar_opens_repos_and_the_pages() {
    let (one, two) = (fixture(), fixture());
    let mut rig = Rig::open(one.clone());
    assert_eq!(rig.app.root(), Some(one.as_path()));
    // The other repo's row opens it.
    let i = rig.app.repos.iter().position(|r| *r == two).expect("the second repo is listed");
    rig.click("the other repo", move |b| b.with_index(0).with("sidebar").with("repos").with(&format!("repo-{i}")));
    assert_eq!(rig.repo().path, two);
    assert_eq!(rig.files(false).len(), 2);

    rig.click("New Repository", |b| b.with_index(0).with("sidebar").with_index(0).with("new"));
    assert_eq!(rig.app.view, View::NewRepo);
    assert_eq!(rig.app.root(), None, "no repo is marked while a page shows");

    // Making one from the form: typed name, Enter.
    let name = |b: WidgetId| b.with_index(1).with("page").with_index(1).with("where").with("Name");
    rig.click("the name field", name);
    rig.h.type_text("made-here");
    rig.settle();
    rig.key(Key::Enter);
    let made = sandbox().join("Projects/made-here");
    assert_eq!(rig.app.view, View::Repo);
    assert_eq!(rig.repo().path, made);
    assert!(crate::git::is_repo(&made));
    assert!(rig.app.repos.contains(&made), "and it is in the sidebar");
}
