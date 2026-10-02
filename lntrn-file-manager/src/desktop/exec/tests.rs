use super::*;
use std::os::unix::ffi::OsStringExt;

fn run(exec: &str, files: &[&str]) -> Vec<Vec<String>> {
    run_as(exec, files, &Entry::default())
}

fn run_as(exec: &str, files: &[&str], entry: &Entry<'_>) -> Vec<Vec<String>> {
    let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
    parse(exec)
        .expect("a valid Exec line")
        .commands(&files, entry)
        .into_iter()
        .map(|argv| {
            argv.into_iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        })
        .collect()
}

fn one(exec: &str, files: &[&str]) -> Vec<String> {
    let mut commands = run(exec, files);
    assert_eq!(commands.len(), 1, "{exec}: {commands:?}");
    commands.remove(0)
}

#[test]
fn a_quoted_program_path_with_a_space_stays_one_argument() {
    assert_eq!(
        one(r#""/opt/My App/app" %U"#, &["/home/a/my file.txt"]),
        ["/opt/My App/app", "/home/a/my file.txt"]
    );
    assert_eq!(
        one(
            r#""/home/alva/Ember Nights/standalone/Ember Nights Game/run.sh""#,
            &[]
        ),
        ["/home/alva/Ember Nights/standalone/Ember Nights Game/run.sh"]
    );
}

#[test]
fn quoted_arguments_keep_their_blanks_and_lose_their_quotes() {
    assert_eq!(
        one(
            r#"env "WINEPREFIX=/home/alva/.wine" wine start /ProgIDOpen "chm.file" %f"#,
            &["/d/help file.chm"]
        ),
        [
            "env",
            "WINEPREFIX=/home/alva/.wine",
            "wine",
            "start",
            "/ProgIDOpen",
            "chm.file",
            "/d/help file.chm"
        ]
    );
    // An empty quoted argument is an argument.
    assert_eq!(one(r#"prog "" x"#, &[]), ["prog", "", "x"]);
    // Quotes in the middle of a word join up.
    assert_eq!(
        one(r#"prog --name="two words""#, &[]),
        ["prog", "--name=two words"]
    );
}

#[test]
fn a_field_code_inside_an_argument_is_filled_in_place() {
    assert_eq!(
        one("viewer --file=%f --fit", &["/p/a b.png"]),
        ["viewer", "--file=/p/a b.png", "--fit"]
    );
    // The file is put where the code stands, not at the end.
    assert_eq!(
        one("player %f --then quit", &["/m/x.mp3"]),
        ["player", "/m/x.mp3", "--then", "quit"]
    );
}

/// A file's name never becomes part of a script's text: there is no way to
/// quote it that is safe for every script (`"%f"` inside `sh -c '...'` still
/// expands `$(...)`). Such a line is refused; the caller says the launcher
/// has a command Fox cannot read.
#[test]
fn a_file_code_mixed_into_a_quoted_command_is_refused() {
    for exec in [
        r#"sh -c "viewer %f""#,
        r#"sh -c 'viewer "%f" >/dev/null'"#,
        r#"sh -c "viewer \"%f\"""#,
        "sh -c 'viewer %F | less'",
        r#"sh -c "open %u""#,
        r#"wrapper "--entry %k""#,
        r#"player "%f %f""#,
        // Glued to a quoted section, or to an escaped blank: the same
        // script text by another spelling.
        r#"sh -c "viewer "%f"#,
        "sh -c 'viewer '%f",
        r"sh -c viewer\ %f",
        r#"sh -c %f" | less""#,
    ] {
        assert!(parse(exec).is_err(), "{exec} must not be run");
    }
}

/// A code that is a quoted section all by itself is just that code: the
/// quotes of `viewer "%f"` are the launcher author's habit, not text.
#[test]
fn a_code_quoted_on_its_own_is_the_plain_code() {
    let hostile = "/p/a $(touch PWNED) `b`'s.txt";
    assert_eq!(one(r#"viewer "%f""#, &[hostile]), ["viewer", hostile]);
    assert_eq!(one("viewer '%f'", &[hostile]), ["viewer", hostile]);
    assert_eq!(
        one(r#"viewer --file="%f" -x"#, &["/p/plain name.txt"]),
        ["viewer", "--file=/p/plain name.txt", "-x"]
    );
    assert_eq!(one(r#"viewer "%F""#, &["/a b", "/c"]), ["viewer", "/a b", "/c"]);
    // Quoted text in ANOTHER argument is none of the code's business.
    assert_eq!(
        one(r#""/opt/My App/run" --title "My App" %f"#, &[hostile]),
        ["/opt/My App/run", "--title", "My App", hostile]
    );
    // The entry's own name in a script is the launcher's own text.
    let entry = Entry {
        name: "My App",
        ..Entry::default()
    };
    assert_eq!(
        run_as(r#"sh -c "echo %c""#, &[], &entry),
        [["sh", "-c", "echo My App"]]
    );
}

#[test]
fn flatpak_file_forwarding_keeps_the_files_between_its_markers() {
    let exec = "/usr/bin/flatpak run --branch=stable --command=gimp \
                --file-forwarding org.gimp.GIMP @@u %U @@";
    assert_eq!(
        one(exec, &["/a/one.png", "/a/two.png"]),
        [
            "/usr/bin/flatpak",
            "run",
            "--branch=stable",
            "--command=gimp",
            "--file-forwarding",
            "org.gimp.GIMP",
            "@@u",
            "/a/one.png",
            "/a/two.png",
            "@@"
        ]
    );
}

#[test]
fn a_list_code_takes_every_file_and_a_single_code_runs_once_per_file() {
    assert_eq!(
        run("edit %F", &["/a", "/b", "/c"]),
        [["edit", "/a", "/b", "/c"]]
    );
    assert_eq!(
        run("edit %f", &["/a", "/b"]),
        [vec!["edit", "/a"], vec!["edit", "/b"]]
    );
    assert_eq!(run("open %u", &["/a", "/b"]).len(), 2);
    // No file at all: the codes simply go.
    assert_eq!(run("edit %F", &[]), [["edit"]]);
    assert_eq!(run("edit --new %f", &[]), [["edit", "--new"]]);
    assert_eq!(run("edit --file=%f", &[]), [["edit", "--file="]]);
}

#[test]
fn an_entry_without_a_file_code_still_gets_the_file() {
    assert_eq!(
        run("myeditor --wait", &["/a", "/b"]),
        [
            vec!["myeditor", "--wait", "/a"],
            vec!["myeditor", "--wait", "/b"]
        ]
    );
    assert_eq!(run("myeditor", &[]), [["myeditor"]]);
}

#[test]
fn icon_name_and_desktop_file_codes() {
    let entry = Entry {
        name: "My Editor",
        icon: Some("my-editor"),
        desktop_file: Some(Path::new("/usr/share/applications/my.desktop")),
    };
    assert_eq!(
        run_as("ed %i --title %c --from %k %f", &["/x"], &entry),
        [[
            "ed",
            "--icon",
            "my-editor",
            "--title",
            "My Editor",
            "--from",
            "/usr/share/applications/my.desktop",
            "/x"
        ]]
    );
    // No icon: `%i` is nothing at all, not an empty argument.
    let plain = Entry {
        name: "Ed",
        ..Entry::default()
    };
    assert_eq!(run_as("ed %i %f", &["/x"], &plain), [["ed", "/x"]]);
}

#[test]
fn percent_signs_and_deprecated_codes() {
    assert_eq!(one("calc 100%% %f", &["/x"]), ["calc", "100%", "/x"]);
    assert_eq!(one("old %d %D %n %N %v %m %f", &["/x"]), ["old", "/x"]);
    assert!(parse("prog %x").is_err());
    assert!(parse("prog %").is_err());
}

#[test]
fn escapes_are_undone_in_the_order_the_specification_gives() {
    // Value escapes first: `\\s` is a blank, which then separates.
    assert_eq!(one(r"prog a\sb", &[]), ["prog", "a", "b"]);
    // Then the quoting rule: `\\\\` in the file is one backslash in a
    // quoted argument, `\\$` a dollar sign, `\"` a quote.
    assert_eq!(
        one(
            r#"prog "C:\\\\dir" "cost \\$5" "say \"hi\"" "tick \`x\`""#,
            &[]
        ),
        ["prog", r"C:\dir", "cost $5", r#"say "hi""#, "tick `x`"]
    );
    // Outside quotes a backslash protects the next character.
    assert_eq!(one(r"prog two\\ words", &[]), ["prog", "two words"]);
}

#[test]
fn lines_that_cannot_be_run_are_refused() {
    assert!(parse("").is_err());
    assert!(parse("   ").is_err());
    assert!(parse(r#"prog "never closed"#).is_err());
    assert!(parse("prog 'never closed").is_err());
}

#[test]
fn file_names_are_passed_byte_for_byte() {
    let odd = PathBuf::from(OsString::from_vec(b"/p/caf\xe9 \"q\" %f;rm -rf ~".to_vec()));
    let commands = parse("view %f")
        .unwrap()
        .commands(std::slice::from_ref(&odd), &Entry::default());
    assert_eq!(
        commands,
        [vec![OsString::from("view"), odd.into_os_string()]]
    );
}
