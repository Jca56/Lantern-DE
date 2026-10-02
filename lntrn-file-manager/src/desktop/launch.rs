//! Starting an application from its desktop entry.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
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
            match in_terminal(&argv, work_dir.as_deref()) {
                Ok(cmd) => cmd,
                Err(e) => {
                    return Err(format!(
                        "\u{201c}{}\u{201d} could not be started in a terminal: {e}.",
                        app.name
                    ))
                }
            }
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
//
// lntrn-terminal takes no command to run: it starts `$SHELL`, with no
// arguments. So the command is written into a small script and the
// terminal is started with SHELL pointing at it. The script puts the real
// shell back into the environment first, and every run of it but the first
// (a new tab in that terminal window) simply becomes that shell.
//
// When lntrn-terminal learns an `-e`, this goes and the arguments are
// passed directly.

/// `value` as one word of an `sh` script.
fn sh_word(value: &OsStr) -> Vec<u8> {
    let mut out = vec![b'\''];
    for &b in value.as_bytes() {
        if b == b'\'' {
            out.extend_from_slice(b"'\\''");
        } else {
            out.push(b);
        }
    }
    out.push(b'\'');
    out
}

/// The script that runs `argv` once and is the user's shell afterwards.
fn terminal_script(argv: &[OsString], shell: &OsStr, work_dir: Option<&Path>) -> Vec<u8> {
    let mut script = Vec::new();
    script.extend_from_slice(
        b"#!/bin/sh\n\
          # Written by the Lantern file manager to run one terminal application.\n\
          SHELL=",
    );
    script.extend(sh_word(shell));
    script.extend_from_slice(
        // mkdir: of two tabs opening at the same moment exactly one gets
        // to run the command.
        b"\nexport SHELL\n\
          if ! mkdir \"$0.ran\" 2>/dev/null; then\n\
          \texec \"$SHELL\"\n\
          fi\n",
    );
    if let Some(dir) = work_dir {
        script.extend_from_slice(b"cd ");
        script.extend(sh_word(dir.as_os_str()));
        script.extend_from_slice(b" 2>/dev/null\n");
    }
    script.extend_from_slice(b"exec");
    for arg in argv {
        script.push(b' ');
        script.extend(sh_word(arg));
    }
    script.push(b'\n');
    script
}

/// Where the scripts live: Lantern's own runtime folder, on the disk the
/// desktop's binaries run from (a per-user tmpfs may be mounted noexec).
fn script_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".lantern/run/fox-launch")
}

/// Scripts older than this belong to terminals long closed.
const SCRIPT_KEEP: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

fn sweep_old_scripts(dir: &Path) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > SCRIPT_KEEP);
        if old {
            // A script, or the marker folder it left next to itself.
            let path = entry.path();
            let _ = std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path));
        }
    }
}

/// The command that runs `argv` in a terminal window.
fn in_terminal(argv: &[OsString], work_dir: Option<&Path>) -> std::io::Result<Command> {
    use std::io::Write;
    static SERIAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    let dir = script_dir();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    sweep_old_scripts(&dir);
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("run-{}-{stamp}-{serial}.sh", std::process::id()));
    // create_new: never write through something already at that name.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&path)?;
    file.write_all(&terminal_script(argv, &shell, work_dir))?;
    drop(file);

    let mut cmd = Command::new(TERMINAL);
    cmd.env("SHELL", &path);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_script_runs_the_command_once_and_is_a_shell_after_that() {
        let argv: Vec<OsString> = ["nvim", "--", "/home/a/it's here/notes.txt"]
            .iter()
            .map(OsString::from)
            .collect();
        let script = terminal_script(
            &argv,
            OsStr::new("/bin/zsh"),
            Some(Path::new("/home/a/it's here")),
        );
        let text = String::from_utf8(script).unwrap();
        assert!(text.starts_with("#!/bin/sh\n"));
        assert!(text.contains("\nSHELL='/bin/zsh'\nexport SHELL\n"));
        assert!(text.contains("if ! mkdir \"$0.ran\" 2>/dev/null; then\n\texec \"$SHELL\"\nfi\n"));
        assert!(text.contains("\ncd '/home/a/it'\\''s here' 2>/dev/null\n"));
        assert!(text.ends_with("\nexec 'nvim' '--' '/home/a/it'\\''s here/notes.txt'\n"));
    }

    #[test]
    fn the_script_does_what_it_says_when_sh_runs_it() {
        let dir = std::env::temp_dir().join(format!("fox-term-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out put.txt");
        // The "application": writes its arguments, one per line.
        let argv: Vec<OsString> = vec![
            "sh".into(),
            "-c".into(),
            "printf '%s\\n' \"$@\" > \"$0\"".into(),
            out.clone().into_os_string(),
            "a b".into(),
            "$HOME `x` 'q'".into(),
        ];
        let script = dir.join("run.sh");
        std::fs::write(
            &script,
            terminal_script(&argv, OsStr::new("/bin/true"), Some(&dir)),
        )
        .unwrap();
        let run = |script: &Path| {
            Command::new("/bin/sh")
                .arg(script)
                .status()
                .is_ok_and(|s| s.success())
        };
        assert!(run(&script));
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "a b\n$HOME `x` 'q'\n",
            "arguments arrive untouched by the shell"
        );
        // The second run (a new tab) is the shell, not the command again.
        std::fs::remove_file(&out).unwrap();
        assert!(run(&script));
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(&dir);
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
