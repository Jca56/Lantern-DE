//! Sudo password modal + privileged action runner.
//!
//! The flow: try a filesystem op directly. If it fails with permission denied,
//! try `sudo -n` (uses cached ticket — no prompt). If THAT fails, queue a
//! `PendingPrivOp` and pop the `SudoPrompt` modal so the user can type their
//! password. On submit, run with `sudo -S` (stdin password). sudo's own ~5min
//! ticket cache means subsequent ops within that window won't re-prompt.

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// A privileged action queued behind the password prompt.
#[derive(Clone, Debug)]
pub enum PendingPrivOp {
    /// `rm -rf <paths>` — permanent delete.
    Remove(Vec<PathBuf>),
    /// Wipe `~/.local/share/Trash/{files,info}`.
    EmptyTrash,
    /// `mv <src> <dest>` for each source (used by move + trash).
    Move {
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// `cp -r <src> <dest>` for each source.
    Copy {
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// `mkdir <path>` + optional color sidecar.
    NewFolder {
        path: PathBuf,
        color: Option<&'static str>,
    },
    /// `touch <path>`.
    NewFile(PathBuf),
}

impl PendingPrivOp {
    /// Short label for the modal so the user knows what they're authorizing.
    pub fn description(&self) -> String {
        match self {
            PendingPrivOp::Remove(paths) => {
                if paths.len() == 1 {
                    format!("Permanently delete \u{201C}{}\u{201D}", name_of(&paths[0]))
                } else {
                    format!("Permanently delete {} items", paths.len())
                }
            }
            PendingPrivOp::EmptyTrash => "Empty the Trash".into(),
            PendingPrivOp::Move { sources, dest } => {
                let plural = if sources.len() == 1 { "item" } else { "items" };
                format!(
                    "Move {} {} to \u{201C}{}\u{201D}",
                    sources.len(),
                    plural,
                    name_of(dest)
                )
            }
            PendingPrivOp::Copy { sources, dest } => {
                let plural = if sources.len() == 1 { "item" } else { "items" };
                format!(
                    "Copy {} {} to \u{201C}{}\u{201D}",
                    sources.len(),
                    plural,
                    name_of(dest)
                )
            }
            PendingPrivOp::NewFolder { path, .. } => format!(
                "Create folder in \u{201C}{}\u{201D}",
                name_of(path.parent().unwrap_or(path))
            ),
            PendingPrivOp::NewFile(path) => format!(
                "Create file in \u{201C}{}\u{201D}",
                name_of(path.parent().unwrap_or(path))
            ),
        }
    }
}

fn name_of(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

/// Result of a `sudo -n` (cached ticket) attempt.
pub enum CachedResult {
    /// Action ran successfully.
    Ok,
    /// No valid sudo ticket — caller should prompt for password.
    NeedPassword,
    /// Action ran but failed (non-auth failure).
    Failed(String),
}

/// Try to run a privileged op using sudo's cached ticket (`sudo -n`). Never
/// prompts. Returns `NeedPassword` if the ticket is missing/expired so the
/// caller knows to show the password modal.
pub fn try_cached(op: &PendingPrivOp) -> CachedResult {
    run_with_sudo(op, None)
}

/// Run a privileged op with `sudo -S`, feeding the password via stdin.
/// Returns `Err(message)` on failure (caller surfaces in modal).
pub fn run_with_password(op: &PendingPrivOp, password: &str) -> Result<(), String> {
    match run_with_sudo(op, Some(password)) {
        CachedResult::Ok => Ok(()),
        CachedResult::NeedPassword => Err("Incorrect password".into()),
        CachedResult::Failed(msg) => Err(msg),
    }
}

fn run_with_sudo(op: &PendingPrivOp, password: Option<&str>) -> CachedResult {
    let argv = build_argv(op);
    if argv.is_empty() {
        return CachedResult::Ok;
    }
    for cmd_argv in argv {
        let mut cmd = Command::new("sudo");
        // -S = read password from stdin (only consumed when no cached ticket).
        // -n = non-interactive: fail instead of prompting.
        // -p "" = empty prompt so sudo doesn't emit "[sudo] password:" noise.
        if password.is_none() {
            cmd.arg("-n");
        } else {
            cmd.arg("-S").arg("-p").arg("");
        }
        cmd.arg("--");
        for a in &cmd_argv {
            cmd.arg(a);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return CachedResult::Failed(format!("Failed to spawn sudo: {e}")),
        };
        if let Some(pw) = password {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = writeln!(stdin, "{pw}");
            }
        }
        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return CachedResult::Failed(format!("sudo wait failed: {e}")),
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // sudo's "no cached ticket" error in -n mode contains "a password is required".
            let needs_password = stderr.contains("password is required")
                || stderr.contains("a terminal is required")
                || stderr.contains("askpass");
            if needs_password && password.is_none() {
                return CachedResult::NeedPassword;
            }
            let msg = if stderr.trim().is_empty() {
                format!(
                    "Command failed (exit {})",
                    output.status.code().unwrap_or(-1)
                )
            } else {
                stderr.trim().to_string()
            };
            return CachedResult::Failed(msg);
        }
    }
    CachedResult::Ok
}

/// Build the argv list for an op. Returns one argv per sub-command — most ops
/// are a single command, but Move/Copy emit one per source so a partial
/// failure in the middle doesn't lose the remaining sources' destinations.
fn build_argv(op: &PendingPrivOp) -> Vec<Vec<OsString>> {
    match op {
        PendingPrivOp::Remove(paths) => {
            let mut argv: Vec<OsString> = vec!["rm".into(), "-rf".into()];
            for p in paths {
                argv.push(p.as_os_str().to_owned());
            }
            vec![argv]
        }
        PendingPrivOp::EmptyTrash => {
            // Use sh -c so we can glob-delete the contents (not the dirs).
            let home = std::env::var("HOME").unwrap_or_default();
            let script = format!(
                "rm -rf -- {home}/.local/share/Trash/files/* {home}/.local/share/Trash/files/.[!.]* \
                 {home}/.local/share/Trash/info/* {home}/.local/share/Trash/info/.[!.]* 2>/dev/null; true"
            );
            vec![vec!["sh".into(), "-c".into(), script.into()]]
        }
        PendingPrivOp::Move { sources, dest } => sources
            .iter()
            .filter_map(|s| {
                let name = s.file_name()?;
                let target = dest.join(name);
                Some(vec![
                    "mv".into(),
                    s.as_os_str().to_owned(),
                    target.into_os_string(),
                ])
            })
            .collect(),
        PendingPrivOp::Copy { sources, dest } => sources
            .iter()
            .filter_map(|s| {
                let name = s.file_name()?;
                let target = dest.join(name);
                Some(vec![
                    "cp".into(),
                    "-r".into(),
                    "--".into(),
                    s.as_os_str().to_owned(),
                    target.into_os_string(),
                ])
            })
            .collect(),
        PendingPrivOp::NewFolder { path, color } => {
            let mut cmds = vec![vec![
                "mkdir".into(),
                "--".into(),
                path.as_os_str().to_owned(),
            ]];
            if let Some(c) = color {
                // Mirror icons::set_folder_color → user.lantern.folder_color xattr.
                cmds.push(vec![
                    "setfattr".into(),
                    "-n".into(),
                    "user.lantern.folder_color".into(),
                    "-v".into(),
                    OsString::from(*c),
                    path.as_os_str().to_owned(),
                ]);
            }
            cmds
        }
        PendingPrivOp::NewFile(path) => {
            vec![vec![
                "touch".into(),
                "--".into(),
                path.as_os_str().to_owned(),
            ]]
        }
    }
}
