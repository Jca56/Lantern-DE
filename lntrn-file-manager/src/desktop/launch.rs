//! Starting an application from its desktop entry.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{exec, DesktopApp};

const LAUNCH_LOG_MAX: u64 = 4 * 1024 * 1024;

/// The terminal a `Terminal=true` entry is run in.
const TERMINAL: &str = "lntrn-terminal";

/// Open `files` with `app`: its Exec line filled in by the rules of the
/// Desktop Entry specification (desktop/exec.rs), in a terminal when the
/// entry asks for one. An app that takes a list of files gets them all in
/// one go; any other is started once per file.
///
/// `Err` is a sentence for the user: the entry cannot be run at all (a
/// broken Exec line, a program that is not installed). The processes are
/// started and reaped on threads of their own; what they write to stderr
/// goes to `~/.lantern/log/fox-launch.log`.
pub fn launch_app(app: &DesktopApp, files: &[PathBuf]) -> Result<(), String> {
    let template = exec::parse(&app.exec).map_err(|why| {
        format!(
            "\u{201c}{}\u{201d} cannot be started: its launcher ({}) has a command Fox cannot read: {why}.",
            app.name,
            app.file.display()
        )
    })?;
    let entry = exec::Entry {
        name: &app.name,
        icon: app.icon.as_deref(),
        desktop_file: Some(&app.file),
    };
    let commands = template.commands(files, &entry);
    if commands.is_empty() {
        return Err(format!(
            "\u{201c}{}\u{201d} cannot be started: its launcher ({}) names no program.",
            app.name,
            app.file.display()
        ));
    }
    // Looked up here, where a miss can still be reported: the spawn itself
    // happens off this thread.
    for argv in &commands {
        if find_program(&argv[0]).is_none() {
            return Err(format!(
                "\u{201c}{}\u{201d} cannot be started: the program {} was not found.",
                app.name,
                Path::new(&argv[0]).display()
            ));
        }
    }
    if app.terminal && find_program(OsStr::new(TERMINAL)).is_none() {
        return Err(format!(
            "\u{201c}{}\u{201d} runs in a terminal, and {TERMINAL} was not found.",
            app.name
        ));
    }

    // The folder the entry asks to be started in, if it names one. (Not
    // the folder of the file: a program sitting in a folder on a USB stick
    // keeps the stick from being ejected long after it closed the file.)
    let work_dir = app
        .work_dir
        .as_ref()
        .map(PathBuf::from)
        // On a phone or a share even "is it there?" can hang.
        .filter(|dir| !crate::fs::is_slow_path(dir) && dir.is_dir());

    for argv in commands {
        let mut cmd = if app.terminal {
            in_terminal(&argv)
        } else {
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..]);
            cmd
        };
        if let Some(dir) = &work_dir {
            cmd.current_dir(dir);
        }
        spawn_logged(cmd);
    }
    Ok(())
}

/// The executable `program` names: as given when it holds a slash, else
/// the first match on `$PATH`.
fn find_program(program: &OsStr) -> Option<PathBuf> {
    let runnable = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.as_bytes().contains(&b'/') {
        let path = PathBuf::from(program);
        return runnable(&path).then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|candidate| runnable(candidate))
}

/// Start `cmd` detached, its stderr appended to the launch log, and reap
/// it when it exits.
fn spawn_logged(mut cmd: Command) {
    std::thread::spawn(move || {
        let log_path = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".lantern/log/fox-launch.log"))
            .unwrap_or_else(|_| PathBuf::from("/tmp/fox-launch.log"));
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Every launched app's stderr lands here; keep one old generation.
        if std::fs::metadata(&log_path).is_ok_and(|m| m.len() > LAUNCH_LOG_MAX) {
            let _ = std::fs::rename(&log_path, log_path.with_extension("log.1"));
        }
        let stderr_dest = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null());
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr_dest);
        // Detach process group so the child outlives the parent and doesn't
        // receive the parent's signals (SIGHUP on close, etc.).
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        match cmd.spawn() {
            // Reap the child from this thread (it's already dedicated to the
            // launch): without a wait() every closed or killed app lingers as
            // a <defunct> row under Fox until Fox itself exits.
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(e) => eprintln!("[fox] failed to spawn {:?}: {e}", cmd.get_program()),
        }
    });
}

// ── Terminal=true ───────────────────────────────────────────────────────────

/// The command that runs `argv` in a terminal window: `lntrn-terminal -e`
/// takes the program and its arguments as they are (no shell reads them)
/// and runs them in the window's own tab, in the working directory the
/// terminal is started with.
fn in_terminal(argv: &[OsString]) -> Command {
    let mut cmd = Command::new(TERMINAL);
    cmd.arg("-e").args(argv);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_app_is_handed_to_the_terminal_argument_by_argument() {
        let argv: Vec<OsString> = ["nvim", "--", "/home/a/it's $(here)/notes.txt"]
            .iter()
            .map(OsString::from)
            .collect();
        let cmd = in_terminal(&argv);
        assert_eq!(cmd.get_program(), TERMINAL);
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            ["-e", "nvim", "--", "/home/a/it's $(here)/notes.txt"].map(OsStr::new)
        );
    }

    #[test]
    fn a_program_is_found_by_path_or_on_the_search_path() {
        assert!(find_program(OsStr::new("/bin/sh")).is_some());
        assert!(find_program(OsStr::new("sh")).is_some());
        assert!(find_program(OsStr::new("/nonexistent-fox-test/prog")).is_none());
        assert!(find_program(OsStr::new("no-such-program-fox-test")).is_none());
        // A folder is not a program.
        assert!(find_program(OsStr::new("/bin")).is_none());
    }

    #[test]
    fn an_entry_that_cannot_run_is_reported_not_started() {
        let app = |exec: &str| DesktopApp {
            name: "Thing".into(),
            exec: exec.into(),
            desktop_id: "thing".into(),
            terminal: false,
            icon: None,
            work_dir: None,
            file: PathBuf::from("/usr/share/applications/thing.desktop"),
        };
        let file = [PathBuf::from("/tmp/x.txt")];
        let err = launch_app(&app("no-such-program-fox-test %f"), &file).unwrap_err();
        assert!(err.contains("Thing") && err.contains("not found"), "{err}");
        let err = launch_app(&app(r#""/opt/My App/app %U"#), &file).unwrap_err();
        assert!(err.contains("cannot read"), "{err}");
        let err = launch_app(&app("prog %z"), &file).unwrap_err();
        assert!(err.contains("%z"), "{err}");
    }
}
