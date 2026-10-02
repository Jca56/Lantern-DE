//! Finding the repository a folder belongs to and reading its branch,
//! without starting git.
//!
//! The same walk git does: up from the folder, to the first one that holds
//! a `.git` (a directory, or a file naming one: a linked work tree or a
//! submodule), stopping at a mount boundary. Because Fox does this walk
//! itself and then tells git exactly which repository to use, git never
//! goes looking on its own: it cannot pick up a bare repository planted in
//! the folder, and the config it will read is the one checked here.

use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use super::config;

/// Nothing git keeps in the files read here is anywhere near this long.
const MAX_FILE: u64 = 1024 * 1024;
/// A branch name is shown in a chip; an absurd one is cut.
const MAX_BRANCH_CHARS: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Repo {
    /// The top of the work tree.
    pub root: PathBuf,
    /// Where this work tree's HEAD and index live.
    pub git_dir: PathBuf,
    /// Where the config, objects and refs live. The same as `git_dir`
    /// except for a linked work tree.
    pub common_dir: PathBuf,
}

/// What HEAD points at.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Head {
    Branch(String),
    /// A commit, not a branch: its id.
    Detached(String),
    /// A valid HEAD that does not say (the reftable format keeps the real
    /// one elsewhere).
    Unreadable,
}

impl Head {
    /// The text for the branch chip.
    pub(super) fn label(&self) -> Option<String> {
        match self {
            Head::Branch(name) => Some(display_name(name)),
            Head::Detached(id) => Some(id.chars().take(7).collect()),
            Head::Unreadable => None,
        }
    }
}

/// A name from a repository, made fit to draw: it is somebody else's text.
pub(super) fn display_name(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= MAX_BRANCH_CHARS {
        return clean;
    }
    let mut cut: String = clean.chars().take(MAX_BRANCH_CHARS - 1).collect();
    cut.push('\u{2026}');
    cut
}

/// Read a small regular file. Never waits: a FIFO or a device node named
/// `config` in an unpacked archive must not hang the scan thread.
fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// `base/rel` with `.` and `..` folded away, the way git resolves the
/// paths it stores (no symlink is followed for a `..`).
fn join_lexical(base: &Path, rel: &Path) -> PathBuf {
    let mut out = if rel.is_absolute() {
        PathBuf::new()
    } else {
        base.to_path_buf()
    };
    for part in rel.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn parse_head(text: &str) -> Option<Head> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    if let Some(target) = line.strip_prefix("ref:") {
        let target = target.trim();
        return match target.strip_prefix("refs/heads/") {
            Some(".invalid") => Some(Head::Unreadable),
            Some(branch) if !branch.is_empty() => Some(Head::Branch(branch.to_string())),
            Some(_) => None,
            // Some other ref (a remote branch, a tag): show which.
            None => target
                .strip_prefix("refs/")
                .filter(|rest| !rest.is_empty())
                .map(|rest| Head::Branch(rest.to_string())),
        };
    }
    let id = line.trim();
    let is_id = matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit());
    is_id.then(|| Head::Detached(id.to_string()))
}

/// `git_dir` is a repository as git judges one: a HEAD that reads as one,
/// and an object store and refs (its own, or its common directory's).
fn open_git_dir(root: &Path, git_dir: PathBuf) -> Option<Repo> {
    parse_head(&read_small(&git_dir.join("HEAD"))?)?;
    // A `commondir` that is there but cannot be read as a small text file
    // (a FIFO, megabytes of padding, a name that is not UTF-8) is not
    // "no commondir": git would still follow it, to a config nobody has
    // read. Such a folder is simply not a repository to Fox. (The scan also
    // tells git which common directory to use, see `git_status`, so what
    // is vetted here is what git reads whatever that file says.)
    let pointer = git_dir.join("commondir");
    let common_dir = match std::fs::symlink_metadata(&pointer) {
        Ok(_) => {
            let text = read_small(&pointer)?;
            join_lexical(&git_dir, Path::new(text.trim_end_matches(['\n', '\r'])))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => git_dir.clone(),
        Err(_) => return None,
    };
    if crate::fs::is_slow_path(&common_dir) {
        return None;
    }
    let is_dir = |p: PathBuf| std::fs::metadata(p).is_ok_and(|m| m.is_dir());
    if !is_dir(common_dir.join("objects")) || !is_dir(common_dir.join("refs")) {
        return None;
    }
    Some(Repo {
        root: root.to_path_buf(),
        git_dir,
        common_dir,
    })
}

/// The repository `dir` is in. `dir` is a real path (no symlinks), as git
/// would see its working directory.
pub(super) fn discover(dir: &Path) -> Option<Repo> {
    let mut device = std::fs::metadata(dir).ok()?.dev();
    for at in dir.ancestors() {
        // Never ask a phone or a network share on the way up.
        if crate::fs::is_slow_path(at) {
            return None;
        }
        // git stays on the filesystem it started on.
        let here = std::fs::metadata(at).ok()?.dev();
        if here != device {
            return None;
        }
        device = here;

        let dot_git = at.join(".git");
        let Ok(meta) = std::fs::metadata(&dot_git) else {
            continue;
        };
        let git_dir = if meta.is_dir() {
            Some(dot_git)
        } else {
            // "gitdir: <path>": a linked work tree or a submodule.
            read_small(&dot_git).and_then(|text| {
                let path = text.strip_prefix("gitdir:")?.trim();
                (!path.is_empty()).then(|| join_lexical(at, Path::new(path)))
            })
        };
        // A `.git` file may name a repository anywhere, a share that is
        // gone included: that one is not followed.
        let git_dir = git_dir.filter(|git_dir| !crate::fs::is_slow_path(git_dir));
        // A `.git` that is not a repository is passed by, as git does.
        if let Some(repo) = git_dir.and_then(|git_dir| open_git_dir(at, git_dir)) {
            return Some(repo);
        }
    }
    None
}

impl Repo {
    pub(super) fn head(&self) -> Option<Head> {
        parse_head(&read_small(&self.git_dir.join("HEAD"))?)
    }

    /// May `git status` be run for the folder `dir` of this repository?
    /// `Err` is the reason, for the log.
    ///
    /// The answer is yes only when nothing the repository holds can make
    /// git start a program: see config.rs. Whatever is unclear is a no.
    pub(super) fn scan_allowed(&self, dir: &Path) -> Result<(), String> {
        // git refuses a repository that belongs to another user unless told
        // otherwise; naming the repository outright (as the scan does)
        // would skip that check, so it is made here.
        let me = unsafe { libc::geteuid() };
        let owned = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.uid() == me);
        if !owned(&self.root) || !owned(&self.git_dir) || !owned(&self.common_dir) {
            return Err("it belongs to another user".into());
        }
        // Inside the repository's own storage there is nothing to badge.
        if dir.starts_with(&self.git_dir) || dir.starts_with(&self.common_dir) {
            return Err("this folder is inside .git".into());
        }

        let facts = self.inspect(&self.common_dir.join("config"))?;
        let mut worktrees = facts.worktrees;
        if facts.worktree_config {
            let extra = self.git_dir.join("config.worktree");
            if std::fs::symlink_metadata(&extra).is_ok() {
                // The rules are the same; whether it asks for itself again
                // does not matter.
                worktrees.extend(self.inspect(&extra)?.worktrees);
            }
        }
        // A submodule's config names its work tree. Anything but the folder
        // the `.git` was found in would make git look at files elsewhere.
        let real_root = std::fs::canonicalize(&self.root).map_err(|e| e.to_string())?;
        for worktree in worktrees {
            let named = join_lexical(&self.git_dir, Path::new(&worktree));
            if !std::fs::canonicalize(&named).is_ok_and(|real| real == real_root) {
                return Err("its config sets core.worktree to another folder".into());
            }
        }
        Ok(())
    }

    fn inspect(&self, config_file: &Path) -> Result<config::Facts, String> {
        match std::fs::symlink_metadata(config_file) {
            // No config at all is git's defaults.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(config::Facts::default()),
            _ => match read_small(config_file) {
                Some(text) => config::inspect(&text),
                None => Err("its config cannot be read".into()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fox-git-repo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// The least git accepts as a repository.
    fn make_git_dir(git_dir: &Path, head: &str, config: Option<&str>) {
        std::fs::create_dir_all(git_dir.join("objects")).unwrap();
        std::fs::create_dir_all(git_dir.join("refs")).unwrap();
        std::fs::write(git_dir.join("HEAD"), head).unwrap();
        if let Some(config) = config {
            std::fs::write(git_dir.join("config"), config).unwrap();
        }
    }

    #[test]
    fn head_is_read_without_git() {
        assert_eq!(
            parse_head("ref: refs/heads/main\n"),
            Some(Head::Branch("main".into()))
        );
        assert_eq!(
            parse_head("ref: refs/heads/feature/x\n"),
            Some(Head::Branch("feature/x".into()))
        );
        let id = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            parse_head(&format!("{id}\n")),
            Some(Head::Detached(id.into()))
        );
        assert_eq!(
            Head::Detached(id.into()).label().as_deref(),
            Some("0123456")
        );
        // reftable: the real HEAD is not in this file.
        assert_eq!(
            parse_head("ref: refs/heads/.invalid\n"),
            Some(Head::Unreadable)
        );
        assert_eq!(Head::Unreadable.label(), None);
        assert_eq!(
            parse_head("ref: refs/remotes/origin/main\n"),
            Some(Head::Branch("remotes/origin/main".into()))
        );
        assert_eq!(parse_head(""), None);
        assert_eq!(parse_head("garbage\n"), None);
        assert_eq!(parse_head("ref: refs/heads/\n"), None);
    }

    #[test]
    fn a_name_from_a_repository_is_made_fit_to_draw() {
        assert_eq!(display_name("main"), "main");
        assert_eq!(display_name("a\u{1b}[31mb\n"), "a[31mb");
        let long = "x".repeat(500);
        let shown = display_name(&long);
        assert_eq!(shown.chars().count(), MAX_BRANCH_CHARS);
        assert!(shown.ends_with('\u{2026}'));
    }

    #[test]
    fn the_repository_is_found_from_any_depth() {
        let top = scratch("depth");
        make_git_dir(&top.join("repo/.git"), "ref: refs/heads/main\n", None);
        std::fs::create_dir_all(top.join("repo/a/b")).unwrap();

        let repo = discover(&top.join("repo/a/b")).expect("found");
        assert_eq!(repo.root, top.join("repo"));
        assert_eq!(repo.git_dir, top.join("repo/.git"));
        assert_eq!(repo.common_dir, repo.git_dir);
        assert_eq!(repo.head(), Some(Head::Branch("main".into())));
        assert_eq!(
            discover(&top.join("repo")).map(|r| r.root),
            Some(top.join("repo"))
        );
        // No config at all: nothing in it can be dangerous.
        assert_eq!(repo.scan_allowed(&top.join("repo/a")), Ok(()));
        // Inside .git there is nothing to scan.
        assert!(repo.scan_allowed(&top.join("repo/.git/refs")).is_err());
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn a_dot_git_that_is_no_repository_is_passed_by() {
        let top = scratch("passby");
        make_git_dir(&top.join("outer/.git"), "ref: refs/heads/outer\n", None);
        // An empty `.git` folder, and one whose HEAD is nonsense.
        std::fs::create_dir_all(top.join("outer/empty/.git")).unwrap();
        make_git_dir(&top.join("outer/junk/.git"), "nonsense", None);
        for sub in ["outer/empty", "outer/junk"] {
            let repo = discover(&top.join(sub)).expect("the outer one");
            assert_eq!(repo.root, top.join("outer"), "{sub}");
        }
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn a_planted_bare_repository_is_not_what_gets_used() {
        // A folder that is itself a bare repository with a hostile config,
        // inside an ordinary one. git, left to look for itself, would use
        // the bare one. The walk here only ever takes a `.git`.
        let top = scratch("bare");
        make_git_dir(&top.join("repo/.git"), "ref: refs/heads/main\n", None);
        make_git_dir(
            &top.join("repo/evil"),
            "ref: refs/heads/x\n",
            Some("[core]\n\tfsmonitor = /tmp/evil\n"),
        );
        let repo = discover(&top.join("repo/evil")).expect("the outer one");
        assert_eq!(repo.git_dir, top.join("repo/.git"));
        assert_eq!(repo.scan_allowed(&top.join("repo/evil")), Ok(()));
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn a_linked_work_tree_reads_the_main_repositorys_config() {
        let top = scratch("worktree");
        let main = top.join("main/.git");
        make_git_dir(
            &main,
            "ref: refs/heads/main\n",
            Some("[core]\n\tbare = false\n"),
        );
        let linked = main.join("worktrees/wt");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        std::fs::write(linked.join("commondir"), "../..\n").unwrap();
        std::fs::create_dir_all(top.join("wt")).unwrap();
        std::fs::write(
            top.join("wt/.git"),
            format!("gitdir: {}\n", linked.display()),
        )
        .unwrap();

        let repo = discover(&top.join("wt")).expect("found");
        assert_eq!(repo.root, top.join("wt"));
        assert_eq!(repo.git_dir, linked);
        assert_eq!(repo.common_dir, main);
        assert_eq!(repo.head(), Some(Head::Branch("feature/x".into())));
        assert_eq!(repo.scan_allowed(&top.join("wt")), Ok(()));

        // The config that counts is the main repository's.
        std::fs::write(main.join("config"), "[filter \"x\"]\n\tclean = evil\n").unwrap();
        assert!(repo.scan_allowed(&top.join("wt")).is_err());

        // And the work tree's own, once the repository says it has one.
        std::fs::write(
            main.join("config"),
            "[extensions]\n\tworktreeConfig = true\n",
        )
        .unwrap();
        assert_eq!(repo.scan_allowed(&top.join("wt")), Ok(()));
        std::fs::write(
            linked.join("config.worktree"),
            "[core]\n\tfsmonitor = evil\n",
        )
        .unwrap();
        assert!(repo.scan_allowed(&top.join("wt")).is_err());
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn a_dangerous_or_unreadable_config_stops_the_scan_not_the_branch() {
        let top = scratch("config");
        let git_dir = top.join("repo/.git");
        make_git_dir(
            &git_dir,
            "ref: refs/heads/main\n",
            Some("[core]\n\tbare = false\n[include]\n\tpath = /tmp/evil\n"),
        );
        let repo = discover(&top.join("repo")).expect("found");
        assert!(repo.scan_allowed(&top.join("repo")).is_err());
        assert_eq!(repo.head().and_then(|h| h.label()).as_deref(), Some("main"));

        // Not a regular file (here: a folder named config).
        std::fs::remove_file(git_dir.join("config")).unwrap();
        std::fs::create_dir(git_dir.join("config")).unwrap();
        assert!(repo.scan_allowed(&top.join("repo")).is_err());
        // Not text.
        std::fs::remove_dir(git_dir.join("config")).unwrap();
        std::fs::write(git_dir.join("config"), [0xff, 0xfe, b'[']).unwrap();
        assert!(repo.scan_allowed(&top.join("repo")).is_err());
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn core_worktree_is_accepted_only_for_the_folder_itself() {
        // The layout of a submodule: `.git` is a file, the repository sits
        // in the parent's modules folder and names its work tree.
        let top = scratch("submodule");
        let module = top.join("super/.git/modules/sub");
        make_git_dir(&top.join("super/.git"), "ref: refs/heads/main\n", None);
        make_git_dir(
            &module,
            "ref: refs/heads/main\n",
            Some("[core]\n\tworktree = ../../../sub\n"),
        );
        std::fs::create_dir_all(top.join("super/sub")).unwrap();
        std::fs::write(top.join("super/sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();

        let repo = discover(&top.join("super/sub")).expect("found");
        assert_eq!(repo.root, top.join("super/sub"));
        assert_eq!(repo.git_dir, module);
        assert_eq!(repo.scan_allowed(&top.join("super/sub")), Ok(()));

        // Pointing anywhere else: git would be scanning that place.
        std::fs::write(module.join("config"), "[core]\n\tworktree = ../../..\n").unwrap();
        assert!(repo.scan_allowed(&top.join("super/sub")).is_err());
        std::fs::write(module.join("config"), "[core]\n\tworktree\n").unwrap();
        assert!(repo.scan_allowed(&top.join("super/sub")).is_err());
        let _ = std::fs::remove_dir_all(&top);
    }
}
