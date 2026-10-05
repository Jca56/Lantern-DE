//! What the app knows: the repo that is open and what has been read of
//! it, and what the forms hold while they are filled in.

use std::path::{Path, PathBuf};

use crate::diff::Diff;
use crate::git::{self, Branch, FileStatus, RepoStatus};
use crate::github::RemoteRepo;
use crate::history::Graph;
use crate::worker::{Cmd, Link};

pub const APP_ID: &str = "lntrn-git";
/// How many commits the history tab asks for.
pub const HISTORY_COMMITS: usize = 300;

/// What fills the window beside the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Nothing is open yet.
    Welcome,
    Repo,
    NewRepo,
    Clone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Changes,
    Branches,
    History,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Changes, Tab::Branches, Tab::History];
}

/// The file whose changes are showing: its path, and whether it is the
/// staged side of it.
pub type Picked = (String, bool);

/// The open repo.
pub struct Repo {
    pub path: PathBuf,
    pub status: Option<RepoStatus>,
    /// Counts the statuses that have arrived: a diff belongs to the
    /// status it was read under.
    generation: u64,
    pub branches: Vec<Branch>,
    pub graph: Option<Graph>,
    /// The history showing is older than the last thing done.
    pub history_stale: bool,
    pub tab: Tab,
    pub picked: Option<Picked>,
    /// The diff that is showing, and what it is of.
    diff: Option<(Picked, u64, Diff)>,
    /// The diff that has been asked for and not yet come.
    asked: Option<(Picked, u64)>,
    /// The commit message being written.
    pub message: String,
    /// What last went wrong, until it is dismissed or something works.
    pub error: Option<String>,
}

impl Repo {
    pub fn new(path: PathBuf) -> Repo {
        Repo { path, status: None, generation: 0, branches: Vec::new(), graph: None, history_stale: true, tab: Tab::Changes, picked: None, diff: None, asked: None, message: String::new(), error: None }
    }

    pub fn name(&self) -> String {
        git::repo_name(&self.path)
    }

    /// The staged files, or the ones that aren't.
    pub fn files(&self, staged: bool) -> impl Iterator<Item = &FileStatus> {
        self.status.iter().flat_map(|s| s.files.iter()).filter(move |f| f.staged == staged)
    }

    pub fn count(&self, staged: bool) -> usize {
        self.files(staged).count()
    }

    fn find(&self, picked: &Picked) -> Option<&FileStatus> {
        self.files(picked.1).find(|f| f.path == picked.0)
    }

    /// The file that is picked.
    pub fn picked_file(&self) -> Option<&FileStatus> {
        self.picked.as_ref().and_then(|p| self.find(p))
    }

    /// A new status has come. The pick stays on its file: where it is if
    /// it is still there, on its other side if it was just staged or
    /// unstaged, else on the first file there is.
    pub fn on_status(&mut self, status: RepoStatus) {
        self.status = Some(status);
        self.generation += 1;
        let still = self.picked.take().and_then(|p| {
            let other = (p.0.clone(), !p.1);
            [p, other].into_iter().find(|side| self.find(side).is_some())
        });
        self.picked = still.or_else(|| self.files(false).chain(self.files(true)).next().map(|f| (f.path.clone(), f.staged)));
        if self.picked.is_none() {
            self.diff = None;
        }
    }

    /// The picked file's diff, if one has been read. It may be from
    /// before the last status: it shows until the one asked for since
    /// replaces it, so a refresh doesn't blank the pane.
    pub fn diff(&self) -> Option<&Diff> {
        let (of, _, diff) = self.diff.as_ref()?;
        (Some(of) == self.picked.as_ref()).then_some(diff)
    }

    /// Ask for the picked file's diff unless the one here is current or
    /// one is on its way.
    pub fn want_diff(&mut self, link: &Link) {
        let Some(file) = self.picked_file().cloned() else { return };
        let want = ((file.path.clone(), file.staged), self.generation);
        let current = self.diff.as_ref().is_some_and(|(of, generation, _)| *of == want.0 && *generation == want.1);
        if current || self.asked.as_ref() == Some(&want) {
            return;
        }
        link.send(Cmd::Diff { file, tag: want.1 });
        self.asked = Some(want);
    }

    /// A diff has come: kept when it is still the one wanted.
    pub fn on_diff(&mut self, file: &FileStatus, tag: u64, diff: Diff) {
        let of = (file.path.clone(), file.staged);
        if Some(&of) == self.picked.as_ref() && tag == self.generation {
            self.diff = Some((of, tag, diff));
        }
    }

    /// The branch that is checked out, as the branches tab knows it.
    pub fn current_branch(&self) -> Option<usize> {
        self.branches.iter().position(|b| b.is_current)
    }
}

/// The new-repository form.
pub struct NewRepo {
    pub name: String,
    pub parent: String,
    pub github: bool,
    pub private: bool,
    pub creating: bool,
    pub error: Option<String>,
}

/// Where new work goes unless told otherwise: `~/Projects`.
pub fn projects_dir() -> String {
    Path::new(&std::env::var("HOME").unwrap_or_default()).join("Projects").display().to_string()
}

impl Default for NewRepo {
    fn default() -> Self {
        Self { name: String::new(), parent: projects_dir(), github: false, private: true, creating: false, error: None }
    }
}

/// The clone page: the user's repos on GitHub and where one goes.
pub struct CloneForm {
    pub repos: Vec<RemoteRepo>,
    /// GitHub has been asked and has answered (or failed to).
    pub asked: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// The folder a clone gets its own folder inside.
    pub dest: String,
    /// The name of the repo being cloned right now.
    pub cloning: Option<String>,
}

impl Default for CloneForm {
    fn default() -> Self {
        Self { repos: Vec::new(), asked: false, loading: false, error: None, dest: projects_dir(), cloning: None }
    }
}

/// What the dialogs hold while they are open.
pub struct Dialogs {
    pub branch_name: String,
    pub branch_push: bool,
    /// Indices into the open repo's branches.
    pub merge_from: usize,
    pub merge_into: usize,
    /// The file a "discard?" is being asked about.
    pub discard: Option<FileStatus>,
}

impl Default for Dialogs {
    fn default() -> Self {
        Self { branch_name: String::new(), branch_push: true, merge_from: 0, merge_into: 0, discard: None }
    }
}

/// Where the last repo opened is remembered between runs.
fn last_repo_file() -> Option<PathBuf> {
    lntrn_sys::dirs::app_dir(lntrn_sys::dirs::lantern_config(), APP_ID).map(|d| d.join("last-repo"))
}

/// The repo that was open when the app last ran, if it is still one.
pub fn last_repo() -> Option<PathBuf> {
    let path = PathBuf::from(std::fs::read_to_string(last_repo_file()?).ok()?.trim());
    git::is_repo(&path).then_some(path)
}

pub fn remember_repo(path: &Path) {
    if let Some(file) = last_repo_file() {
        let _ = std::fs::write(file, path.display().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileState;

    fn file(path: &str, staged: bool) -> FileStatus {
        FileStatus { path: path.into(), status: FileState::Modified, staged, is_submodule: false }
    }

    fn status(files: Vec<FileStatus>) -> RepoStatus {
        RepoStatus { branch: "main".into(), files, ahead: 0, behind: 0 }
    }

    #[test]
    fn the_pick_follows_its_file() {
        let mut repo = Repo::new(PathBuf::from("/r"));
        // The first status picks the first unstaged file.
        repo.on_status(status(vec![file("a.rs", true), file("b.rs", false), file("c.rs", false)]));
        assert_eq!(repo.picked, Some(("b.rs".into(), false)));
        assert_eq!((repo.count(true), repo.count(false)), (1, 2));
        // Staged: the pick goes with it to the other card.
        repo.on_status(status(vec![file("a.rs", true), file("b.rs", true), file("c.rs", false)]));
        assert_eq!(repo.picked, Some(("b.rs".into(), true)));
        // Committed: the pick moves to what is left.
        repo.on_status(status(vec![file("c.rs", false)]));
        assert_eq!(repo.picked, Some(("c.rs".into(), false)));
        repo.on_status(status(Vec::new()));
        assert_eq!(repo.picked, None);
    }

    #[test]
    fn a_diff_is_kept_only_while_it_is_the_one_wanted() {
        let mut repo = Repo::new(PathBuf::from("/r"));
        repo.on_status(status(vec![file("a.rs", false), file("b.rs", false)]));
        let lines = Diff { added: 3, ..Diff::default() };
        // One for a file that isn't picked is dropped.
        repo.on_diff(&file("b.rs", false), 1, lines.clone());
        assert!(repo.diff().is_none());
        repo.on_diff(&file("a.rs", false), 1, lines.clone());
        assert_eq!(repo.diff().map(|d| d.added), Some(3));
        // Picking another file hides it.
        repo.picked = Some(("b.rs".into(), false));
        assert!(repo.diff().is_none());
        repo.picked = Some(("a.rs".into(), false));
        assert!(repo.diff().is_some());
        // A new status leaves it showing until a newer one replaces it;
        // an answer to the old question arriving late is dropped.
        repo.on_status(status(vec![file("a.rs", false), file("b.rs", false)]));
        assert_eq!(repo.diff().map(|d| d.added), Some(3));
        repo.on_diff(&file("a.rs", false), 1, Diff { added: 9, ..Diff::default() });
        assert_eq!(repo.diff().map(|d| d.added), Some(3));
        repo.on_diff(&file("a.rs", false), 2, Diff { added: 5, ..Diff::default() });
        assert_eq!(repo.diff().map(|d| d.added), Some(5));
    }
}
