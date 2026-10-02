use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;

/// A scanner the test holds the leash of: every scan waits for `release`,
/// and reports how many ran at once.
struct Rig {
    git: GitStatus,
    release: mpsc::Sender<()>,
    started: Arc<AtomicUsize>,
    most_at_once: Arc<AtomicUsize>,
}

fn rig() -> Rig {
    let (release, gate) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let started = Arc::new(AtomicUsize::new(0));
    let running = Arc::new(AtomicUsize::new(0));
    let most_at_once = Arc::new(AtomicUsize::new(0));
    let (s, r, m) = (started.clone(), running.clone(), most_at_once.clone());
    let scanner: Scanner = Arc::new(move |dir: &Path| {
        s.fetch_add(1, Ordering::SeqCst);
        m.fetch_max(r.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
        let _ = gate.lock().unwrap().recv();
        r.fetch_sub(1, Ordering::SeqCst);
        // A repo rooted at /repo, the branch named after the folder.
        if dir.starts_with("/repo") {
            Scanned {
                root: Some(PathBuf::from("/repo")),
                branch: dir.file_name().map(|n| n.to_string_lossy().into_owned()),
                marks: HashMap::from([(dir.join("changed"), GitMark::Modified)]),
            }
        } else {
            Scanned::default()
        }
    });
    Rig {
        git: GitStatus::with_scanner(scanner),
        release,
        started,
        most_at_once,
    }
}

impl Rig {
    /// Which scan is in the air.
    fn flight_id(&self) -> Option<(PathBuf, Instant)> {
        self.git.flight.as_ref().map(|f| (f.dir.clone(), f.started))
    }

    /// Let the scan in flight finish and take its result in, then let any
    /// follow-up start (as the loop does once the cooldown is over).
    /// Returns what `poll` said about a redraw.
    fn land(&mut self) -> bool {
        let flying = self.flight_id();
        assert!(flying.is_some(), "no scan in flight");
        self.release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let now = Instant::now();
            let changed = self.git.poll(now);
            if self.flight_id() != flying {
                self.start_waiting();
                return changed;
            }
            assert!(Instant::now() < deadline, "the scan never landed");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The cooldown is over: a scan that was waiting starts.
    fn start_waiting(&mut self) {
        assert!(!self.git.poll(Instant::now() + MAX_COOLDOWN));
    }
}

#[test]
fn one_scan_at_a_time_and_one_more_is_remembered() {
    let mut rig = rig();
    rig.git.enter(Path::new("/repo/a"));
    // The folder changes eight times while git is still out.
    for _ in 0..8 {
        rig.git.refresh();
    }
    assert!(rig.land());
    assert_eq!(rig.git.branch(), Some("a"));
    // Exactly one follow-up, started when the first landed.
    assert!(rig.git.flight.is_some());
    assert!(
        !rig.land(),
        "the same result again changes nothing on screen"
    );
    assert!(rig.git.flight.is_none());
    assert_eq!(rig.started.load(Ordering::SeqCst), 2);
    assert_eq!(rig.most_at_once.load(Ordering::SeqCst), 1);
}

#[test]
fn a_result_for_a_folder_the_view_left_is_not_shown() {
    let mut rig = rig();
    rig.git.enter(Path::new("/repo/a"));
    rig.git.enter(Path::new("/elsewhere"));
    assert!(
        !rig.land(),
        "the scan of /repo/a lands after the view left it"
    );
    assert_eq!(rig.git.branch(), None);
    assert_eq!(rig.git.mark(Path::new("/repo/a/changed")), None);
    // The scan of the folder shown now started at once.
    assert!(!rig.land());
    assert!(!rig.git.in_repo());
    assert_eq!(rig.most_at_once.load(Ordering::SeqCst), 1);
}

#[test]
fn the_branch_stays_inside_a_work_tree_and_goes_outside_it() {
    let mut rig = rig();
    rig.git.enter(Path::new("/repo"));
    assert!(rig.land());
    assert_eq!(rig.git.branch(), Some("repo"));
    assert_eq!(
        rig.git.mark(Path::new("/repo/changed")),
        Some(GitMark::Modified)
    );

    // One level down: the chip stays while the scan is out, the badges of
    // the folder just left do not.
    rig.git.enter(Path::new("/repo/sub"));
    assert_eq!(rig.git.branch(), Some("repo"));
    assert_eq!(rig.git.mark(Path::new("/repo/changed")), None);
    assert!(rig.land());
    assert_eq!(rig.git.branch(), Some("sub"));

    // Out of the repository: gone before any scan says so.
    rig.git.enter(Path::new("/elsewhere"));
    assert_eq!(rig.git.branch(), None);
    assert!(!rig.git.in_repo());
    rig.land();
}

#[test]
fn clear_forgets_and_drops_the_result_still_in_the_air() {
    let mut rig = rig();
    rig.git.enter(Path::new("/repo"));
    assert!(rig.land());
    rig.git.refresh();
    rig.start_waiting();
    assert!(rig.git.clear(), "the chip went away: draw");
    assert!(!rig.git.clear());
    assert!(!rig.land());
    assert_eq!(rig.git.branch(), None);
    assert!(rig.git.flight.is_none(), "nothing is scanned after a clear");
    // Asking for a refresh of "no folder" does nothing.
    rig.git.refresh();
    assert!(rig.git.flight.is_none());
}

#[test]
fn a_follow_up_waits_as_long_as_the_scan_took() {
    let mut rig = rig();
    rig.git.enter(Path::new("/repo"));
    rig.git.refresh();
    // The scan takes a moment, so there is something to wait out.
    std::thread::sleep(Duration::from_millis(20));
    rig.release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let landed_at = loop {
        let now = Instant::now();
        rig.git.poll(now);
        if rig.git.flight.is_none() {
            break now;
        }
        assert!(Instant::now() < deadline, "the scan never landed");
        std::thread::sleep(Duration::from_millis(2));
    };
    // Landed, one more wanted, and not started yet.
    assert!(rig.git.wanted);
    let wake = rig
        .git
        .wake_at()
        .expect("the loop is told when to look again");
    assert!(wake >= landed_at + Duration::from_millis(20));
    assert!(wake <= landed_at + MAX_COOLDOWN);
    rig.git.poll(wake);
    assert!(rig.git.flight.is_some(), "it starts once the wait is over");
    assert_eq!(rig.git.wake_at(), None);
    rig.land();
    assert_eq!(rig.most_at_once.load(Ordering::SeqCst), 1);
}

#[test]
fn going_to_another_folder_does_not_wait_out_a_cooldown() {
    let mut rig = rig();
    rig.git.enter(Path::new("/repo"));
    std::thread::sleep(Duration::from_millis(20));
    rig.release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while rig.git.flight.is_some() {
        rig.git.poll(Instant::now());
        assert!(Instant::now() < deadline, "the scan never landed");
        std::thread::sleep(Duration::from_millis(2));
    }
    rig.git.enter(Path::new("/repo/sub"));
    assert!(
        rig.git.flight.is_some(),
        "the new folder is scanned at once"
    );
    rig.land();
}

#[test]
fn status_output_becomes_badges_on_the_entries_shown() {
    // The folder shown is /link/sub, a symlinked spelling of /repo/sub.
    let out = "## main...origin/main [ahead 1]\0 M sub/a.txt\0?? sub/deep/new.txt\0\
               ?? sub/deep/other.txt\0M  sub/deep/staged.rs\0!! sub/target/\0\
               ?? sub/ spaced\0 M elsewhere/x\0";
    let (branch, marks) = parse_status(
        out,
        Path::new("/repo"),
        Path::new("/repo/sub"),
        Path::new("/link/sub"),
    );
    assert_eq!(branch.as_deref(), Some("main"));
    assert_eq!(
        marks.get(Path::new("/link/sub/a.txt")),
        Some(&GitMark::Modified)
    );
    // A folder with both kinds below it shows the stronger one.
    assert_eq!(
        marks.get(Path::new("/link/sub/deep")),
        Some(&GitMark::Modified)
    );
    assert_eq!(
        marks.get(Path::new("/link/sub/ spaced")),
        Some(&GitMark::Untracked)
    );
    // Ignored files and paths outside the folder get nothing.
    assert_eq!(marks.len(), 3);
}

#[test]
fn the_header_line_names_the_branch() {
    assert_eq!(branch_from_header("main").as_deref(), Some("main"));
    assert_eq!(
        branch_from_header("feature/x...origin/feature/x [behind 2]").as_deref(),
        Some("feature/x")
    );
    assert_eq!(
        branch_from_header("No commits yet on trunk").as_deref(),
        Some("trunk")
    );
    assert_eq!(branch_from_header("HEAD (no branch)"), None);
    assert_eq!(branch_from_header(""), None);
}

/// Change a tracked file so that git has to read it to know whether it
/// changed: the same size as what was committed, another time stamp. (A
/// file whose size differs is "modified" to git without a look inside, so
/// its clean filter may or may not run, depending on timing.)
fn change_keeping_the_size(path: &Path, content: &str) {
    assert_eq!(
        std::fs::metadata(path).unwrap().len(),
        content.len() as u64,
        "the stand-in content has to be as long as the committed one"
    );
    std::fs::write(path, content).unwrap();
    let earlier = std::time::SystemTime::now() - Duration::from_secs(90);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(earlier)
        .unwrap();
}

/// End to end against the real git, when there is one: a repository whose
/// config names a filter gets its branch and no badges, and the filter is
/// not run; the same repository without it gets badges.
#[test]
fn a_repository_with_a_filter_is_never_handed_to_git() {
    let git = |dir: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.org"])
            .args(["-c", "init.defaultBranch=trunk"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let top = std::env::temp_dir().join(format!("fox-git-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(top.join("repo/sub")).unwrap();
    let top = top.canonicalize().unwrap();
    let repo = top.join("repo");
    if !git(&repo, &["init", "-q"]) {
        eprintln!("git is not installed: skipped");
        let _ = std::fs::remove_dir_all(&top);
        return;
    }
    std::fs::write(repo.join("sub/tracked.txt"), "one\n").unwrap();
    std::fs::write(repo.join(".gitattributes"), "* filter=mark\n").unwrap();
    assert!(git(&repo, &["add", "."]));
    assert!(git(&repo, &["commit", "-q", "-m", "first"]));
    std::fs::write(repo.join("sub/tracked.txt"), "two, and longer\n").unwrap();
    std::fs::write(repo.join("sub/new.txt"), "new\n").unwrap();

    // Harmless config: badges.
    let found = scan(&repo.join("sub"));
    assert_eq!(found.root.as_deref(), Some(repo.as_path()));
    assert_eq!(found.branch.as_deref(), Some("trunk"));
    assert_eq!(
        found.marks.get(&repo.join("sub/tracked.txt")),
        Some(&GitMark::Modified)
    );
    assert_eq!(
        found.marks.get(&repo.join("sub/new.txt")),
        Some(&GitMark::Untracked)
    );
    let from_top = scan(&repo);
    assert_eq!(
        from_top.marks.get(&repo.join("sub")),
        Some(&GitMark::Modified)
    );

    // The same repository with a clean filter that leaves a trace.
    let trace = top.join("filter-ran");
    let mut config = std::fs::read_to_string(repo.join(".git/config")).unwrap();
    config.push_str(&format!(
        "[filter \"mark\"]\n\tclean = \"touch '{}' && cat\"\n",
        trace.display()
    ));
    std::fs::write(repo.join(".git/config"), config).unwrap();
    // A tracked file git cannot judge by its size (the other one grew).
    change_keeping_the_size(&repo.join(".gitattributes"), "* filter=mark\n");
    // The trap works: git itself, asked directly, runs the filter. (This is
    // what a scan of this folder used to do.)
    assert!(git(&repo, &["status", "--porcelain"]));
    assert!(trace.exists(), "the test's filter is not one git runs");
    std::fs::remove_file(&trace).unwrap();

    let found = scan(&repo.join("sub"));
    assert_eq!(found.branch.as_deref(), Some("trunk"), "the chip stays");
    assert!(found.marks.is_empty(), "no badges");
    assert!(!trace.exists(), "the repository's filter was run");

    let _ = std::fs::remove_dir_all(&top);
}

/// A `commondir` crafted so that git and Fox disagree about where the
/// shared part of the repository (and so its config) is. Whatever Fox makes
/// of such a folder, git must never get to run the config Fox did not read.
#[test]
fn a_crafted_commondir_never_leads_git_to_an_unread_config() {
    let git = |dir: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.org"])
            .args(["-c", "init.defaultBranch=trunk"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let top = std::env::temp_dir().join(format!("fox-git-commondir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(top.join("repo")).unwrap();
    let top = top.canonicalize().unwrap();
    let repo = top.join("repo");
    if !git(&repo, &["init", "-q"]) {
        eprintln!("git is not installed: skipped");
        let _ = std::fs::remove_dir_all(&top);
        return;
    }
    std::fs::write(repo.join("tracked.txt"), "one\n").unwrap();
    std::fs::write(repo.join(".gitattributes"), "* filter=mark\n").unwrap();
    assert!(git(&repo, &["add", "."]));
    assert!(git(&repo, &["commit", "-q", "-m", "first"]));
    change_keeping_the_size(&repo.join("tracked.txt"), "two\n");

    // The other "common directory": a config with a filter that leaves a
    // trace, next to the harmless one in .git.
    let trace = top.join("filter-ran");
    let evil = repo.join("evil");
    std::fs::create_dir_all(evil.join("objects")).unwrap();
    std::fs::create_dir_all(evil.join("refs")).unwrap();
    std::fs::create_dir_all(evil.join("sub")).unwrap();
    std::fs::write(
        evil.join("config"),
        format!(
            "[core]\n\trepositoryformatversion = 0\n[filter \"mark\"]\n\tclean = \"touch '{}' && cat\"\n",
            trace.display()
        ),
    )
    .unwrap();

    let pointer = repo.join(".git/commondir");
    let attacks: [(&str, Box<dyn Fn()>); 2] = [
        (
            "too large to read, which is not the same as absent",
            Box::new(|| {
                let mut text = String::from("../evil");
                text.push_str(&"\n".repeat(1_200_000));
                std::fs::write(&pointer, text).unwrap();
            }),
        ),
        (
            "a link that makes `..` mean another folder",
            Box::new(|| {
                std::os::unix::fs::symlink("../evil/sub", repo.join(".git/s")).unwrap();
                std::fs::write(&pointer, "s/..\n").unwrap();
            }),
        ),
    ];
    for (what, plant) in attacks {
        plant();
        // The trap works: git, left to itself, follows the file to the
        // other config and runs its filter.
        let _ = git(&repo, &["status", "--porcelain"]);
        assert!(trace.exists(), "{what}: the trap does not spring for git itself");
        std::fs::remove_file(&trace).unwrap();

        let _ = scan(&repo);
        assert!(!trace.exists(), "{what}: git ran a config Fox never read");
        std::fs::remove_file(&pointer).unwrap();
    }
    let _ = std::fs::remove_dir_all(&top);
}
