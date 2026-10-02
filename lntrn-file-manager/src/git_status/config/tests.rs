use super::*;

fn refused(config: &str) -> bool {
    inspect(config).is_err()
}

/// What `git init`, `git clone` and a first push leave behind.
const PLAIN: &str = "[core]\n\
    \trepositoryformatversion = 0\n\
    \tfilemode = true\n\
    \tbare = false\n\
    \tlogallrefupdates = true\n\
    [remote \"origin\"]\n\
    \turl = https://example.org/x.git\n\
    \tfetch = +refs/heads/*:refs/remotes/origin/*\n\
    [branch \"main\"]\n\
    \tremote = origin\n\
    \tmerge = refs/heads/main\n\
    [user]\n\
    \tname = A. Person ; a comment\n\
    \temail = a@example.org\n";

#[test]
fn an_ordinary_config_is_let_through() {
    assert_eq!(inspect(PLAIN), Ok(Facts::default()));
    assert_eq!(inspect(""), Ok(Facts::default()));
    assert_eq!(inspect("# nothing\n; at all\n"), Ok(Facts::default()));
    // Windows line ends, a byte order mark, old-style and odd spacing.
    let crlf = PLAIN.replace('\n', "\r\n");
    assert_eq!(inspect(&crlf), Ok(Facts::default()));
    assert_eq!(
        inspect("\u{feff}[core]\n bare=false\n[pull]rebase = true\n"),
        Ok(Facts::default())
    );
    assert!(!refused("[lfs]\n\trepositoryformatversion = 0\n"));
    assert!(!refused("[lfs \"https://x/info/lfs\"]\n\taccess = basic\n"));
    assert!(!refused(
        "[submodule \"lib\"]\n\turl = ../lib\n\tactive = true\n"
    ));
    assert!(!refused(
        "[remote]\n\tpushDefault = origin\n[rerere]\n\tenabled = true\n"
    ));
}

#[test]
fn a_filter_is_refused() {
    assert!(refused(
        "[filter \"x\"]\n\tclean = sh -c 'touch /tmp/pwned'\n"
    ));
    assert!(refused("[filter \"x\"]\n\tsmudge = cat\n"));
    assert!(refused(
        "[filter \"lfs\"]\n\tprocess = git-lfs filter-process\n"
    ));
    assert!(refused("[filter \"x\"]\n\trequired\n"));
    // The old dotted spelling of the same section.
    assert!(refused("[filter.x]\n\tclean = evil\n"));
    // On the section's own line, and in any case.
    assert!(refused("[FILTER \"x\"] CLEAN = evil\n"));
}

#[test]
fn an_include_is_refused() {
    assert!(refused("[include]\n\tpath = ../evil.cfg\n"));
    assert!(refused("[includeIf \"gitdir:/home/\"]\n\tpath = evil\n"));
    assert!(refused("[includeif \"onbranch:main\"]\n\tpath = evil\n"));
    // Hidden behind an ordinary start.
    assert!(refused(&format!("{PLAIN}[include]\n\tpath = x\n")));
}

#[test]
fn core_keys_that_name_a_program_or_a_file_are_refused() {
    for key in [
        "fsmonitor",
        "hooksPath",
        "sshCommand",
        "pager",
        "editor",
        "askPass",
        "gitProxy",
        "attributesFile",
        "excludesFile",
        "alternateRefsCommand",
    ] {
        assert!(refused(&format!("[core]\n\t{key} = /tmp/x\n")), "{key}");
    }
    // Also as a bare key, and with the value on a continued line.
    assert!(refused("[core]\n\tfsmonitor\n"));
    assert!(refused("[core]\n\tfsmonitor = \\\n /tmp/x\n"));
}

#[test]
fn diff_and_merge_drivers_and_tools_are_refused() {
    assert!(refused("[diff \"img\"]\n\ttextconv = exiftool\n"));
    assert!(refused("[diff \"img\"]\n\tcommand = evil\n"));
    assert!(refused("[diff]\n\texternal = evil\n"));
    assert!(refused("[diff]\n\ttool = evil\n"));
    assert!(refused("[merge \"ours\"]\n\tdriver = evil %O %A %B\n"));
    assert!(refused("[merge]\n\ttool = evil\n"));
    assert!(refused("[mergetool \"x\"]\n\tcmd = evil\n"));
    assert!(refused("[difftool \"x\"]\n\tcmd = evil\n"));
    // The plain settings of the same sections stay allowed.
    assert!(!refused(
        "[diff]\n\talgorithm = histogram\n[merge]\n\tff = only\n"
    ));
}

#[test]
fn everything_else_that_names_a_command_is_refused() {
    for config in [
        "[alias]\n\tst = !evil\n",
        "[credential]\n\thelper = evil\n",
        "[sequence]\n\teditor = evil\n",
        "[gpg]\n\tprogram = evil\n",
        "[pager]\n\tstatus = evil\n",
        "[url \"ext::sh -c evil\"]\n\tinsteadOf = https://\n",
        "[protocol \"ext\"]\n\tallow = always\n",
        "[lfs \"extension.x\"]\n\tclean = evil\n",
        "[uploadpack]\n\tpackObjectsHook = evil\n",
        "[submodule \"lib\"]\n\tupdate = !evil\n",
        "[someSectionGitLearnsNextYear]\n\tcommand = evil\n",
    ] {
        assert!(refused(config), "{config}");
    }
}

#[test]
fn a_promisor_remote_is_refused() {
    // A status of a partial clone fetches what it lacks, through the
    // remote's own programs.
    assert!(refused("[extensions]\n\tpartialClone = origin\n"));
    assert!(refused("[remote \"origin\"]\n\tpromisor = true\n"));
    assert!(refused("[remote \"origin\"]\n\tuploadpack = evil\n"));
    assert!(refused("[remote \"origin\"]\n\tvcs = evil\n"));
    assert!(refused(
        "[remote \"origin\"]\n\tpartialclonefilter = blob:none\n"
    ));
}

#[test]
fn a_file_git_would_read_differently_is_refused() {
    // Nothing here is understood well enough to vouch for.
    assert!(refused("[core\n\tbare = false\n"));
    assert!(refused("[core]\n\tbare = \"unterminated\n"));
    assert!(refused("[core]\n\tbare = bad\\escape\n"));
    assert!(refused("bare = true\n"));
    assert!(refused("[core]\n\t= true\n"));
    assert!(refused("[core]\n\tbare false\n"));
    assert!(refused("[core \"x]\n\tbare = false\n"));
    assert!(refused(
        "[core]\n\tbare = false\r[filter \"x\"]\rclean = evil\n"
    ));
    assert!(refused("[core]\n\tbare = false\u{c}\n"));
    assert!(refused("[core]\n\tbare = fal\0se\n"));
}

#[test]
fn worktree_settings_are_handed_to_the_caller() {
    let facts = inspect("[core]\n\tworktree = ../../../sub\n").unwrap();
    assert_eq!(facts.worktrees, ["../../../sub"]);
    assert!(!facts.worktree_config);
    let facts = inspect("[extensions]\n\tworktreeConfig = true\n").unwrap();
    assert!(facts.worktree_config);
    // Said twice: the caller sees both.
    let facts = inspect("[core]\n\tworktree = a\n\tworktree = /b\n").unwrap();
    assert_eq!(facts.worktrees, ["a", "/b"]);
}

#[test]
fn values_are_read_the_way_git_reads_them() {
    let items = parse(
        "[a \"B c\"]\n\
         \tone = two words  # trailing\n\
         \ttwo = \"quoted ; text\" tail\n\
         \tthree = line\\\n\
         \t  continued\n\
         \tfour\n\
         [x.Y]\n\
         \tfive = \\\"q\\\" \\t.\n",
    )
    .unwrap();
    let got: Vec<(String, Option<String>, String, Option<String>)> = items
        .into_iter()
        .map(|i| (i.section, i.subsection, i.key, i.value))
        .collect();
    let s = |v: &str| Some(v.to_string());
    assert_eq!(
        got,
        [
            ("a".into(), s("B c"), "one".into(), s("two words")),
            ("a".into(), s("B c"), "two".into(), s("quoted ; text tail")),
            ("a".into(), s("B c"), "three".into(), s("line   continued")),
            ("a".into(), s("B c"), "four".into(), None),
            ("x".into(), s("y"), "five".into(), s("\"q\" \t.")),
        ]
    );
}
