//! Privileged file operations through sudo.
//!
//! What may need root is described as a [`PendingPrivOp`]: exact paths, each
//! a separate argument of a fixed command, nothing spliced into a shell
//! string. [`run`] executes it with `sudo -n` (the cached ticket, never a
//! prompt) or, given a password, with `sudo -S`. It blocks for as long as
//! the commands take, so it is only ever called from the worker thread in
//! `priv_ops.rs`, which also owns the dialog around it: the question before
//! a permanent delete, the password field, the "working" state.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod scripts;
mod tools;

use scripts::{copy_argv, move_argv};
#[cfg(test)]
use scripts::MOVE_REPLACE;

/// One privileged copy or move, to the exact place the user chose (a Keep
/// Both name included). The command never lands it anywhere else.
#[derive(Clone, Debug)]
pub struct PrivItem {
    pub src: PathBuf,
    pub target: PathBuf,
    /// Set when the user answered the name conflict with Replace: the slot
    /// in the home Trash the existing item is moved to before the new one
    /// takes its name. `None` means the target must not exist.
    pub old_to_trash: Option<crate::trash::Trashed>,
}

impl PrivItem {
    /// From a resolved paste item. A Replace claims a place in the Trash
    /// for the old item first; if that is not possible the item is refused
    /// with the reason, never overwritten.
    pub fn from_op(item: &crate::ops::OpItem) -> Result<Self, crate::ops::OpFailure> {
        let exists = std::fs::symlink_metadata(&item.target).is_ok();
        // Replacing a folder a drive is mounted on (or in) would move what
        // is on the drive to the Trash: `mv` copies across the mount and
        // then empties it. Nothing gets a Trash slot for that.
        if item.replace && exists && crate::mount_guard::holds_mount(&item.target) {
            return Err(crate::ops::OpFailure {
                path: item.target.clone(),
                reason: format!("not replaced: {}", crate::mount_guard::REASON),
            });
        }
        let old_to_trash = if item.replace && exists {
            match crate::trash::reserve_in_home(&item.target) {
                Ok(slot) => Some(slot),
                Err(e) => {
                    return Err(crate::ops::OpFailure {
                        path: item.target.clone(),
                        reason: format!(
                            "not replaced: the existing item can't be moved to the Trash ({})",
                            e.reason()
                        ),
                    })
                }
            }
        } else {
            None
        };
        Ok(Self {
            src: item.src.clone(),
            target: item.target.clone(),
            old_to_trash,
        })
    }
}

/// A privileged action, waiting for its turn, its confirmation or its
/// password, or running.
#[derive(Clone, Debug)]
pub enum PendingPrivOp {
    /// `rm -rf <paths>` — permanent delete. Never run without the user
    /// having said yes to these very paths (`asks_first`).
    Remove(Vec<PathBuf>),
    /// `mv -T <src> <target>` for each item.
    Move { items: Vec<PrivItem> },
    /// `cp -rT <src> <target>` for each item.
    Copy { items: Vec<PrivItem> },
    /// Rename in place. `case_only`: the new name differs in letter case
    /// only and the drive ignores case, so "the target exists" is the item
    /// itself.
    Rename { item: PrivItem, case_only: bool },
    /// `mkdir <path>` + optional color sidecar.
    NewFolder {
        path: PathBuf,
        color: Option<&'static str>,
    },
    /// `touch <path>`.
    NewFile(PathBuf),
    /// Pack `members` (names inside `dir`) into the new `archive`.
    Archive {
        dir: PathBuf,
        archive: PathBuf,
        members: Vec<OsString>,
    },
    /// Unpack each archive into the new folder next to it: (archive, folder).
    Extract(Vec<(PathBuf, PathBuf)>),
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
            PendingPrivOp::Move { items } => describe_items("Move", items),
            PendingPrivOp::Copy { items } => describe_items("Copy", items),
            PendingPrivOp::Rename { item, .. } => {
                let mut text = format!(
                    "Rename \u{201C}{}\u{201D} to \u{201C}{}\u{201D}",
                    name_of(&item.src),
                    name_of(&item.target)
                );
                if item.old_to_trash.is_some() {
                    text.push_str(" (the existing one goes to the Trash)");
                }
                text
            }
            PendingPrivOp::NewFolder { path, .. } => format!(
                "Create folder in \u{201C}{}\u{201D}",
                name_of(path.parent().unwrap_or(path))
            ),
            PendingPrivOp::NewFile(path) => format!(
                "Create file in \u{201C}{}\u{201D}",
                name_of(path.parent().unwrap_or(path))
            ),
            PendingPrivOp::Archive {
                archive, members, ..
            } => match members.as_slice() {
                [one] => format!(
                    "Compress \u{201C}{}\u{201D} into \u{201C}{}\u{201D}",
                    one.to_string_lossy(),
                    name_of(archive)
                ),
                _ => format!(
                    "Compress {} items into \u{201C}{}\u{201D}",
                    members.len(),
                    name_of(archive)
                ),
            },
            PendingPrivOp::Extract(items) => match items.as_slice() {
                [(archive, into)] => format!(
                    "Extract \u{201C}{}\u{201D} into \u{201C}{}\u{201D}",
                    name_of(archive),
                    name_of(into)
                ),
                _ => format!("Extract {} archives", items.len()),
            },
        }
    }

    /// A permanent delete is never run on the strength of a cached sudo
    /// ticket alone: the user is shown what will be deleted and has to say
    /// yes first, every time.
    pub fn asks_first(&self) -> bool {
        matches!(self, PendingPrivOp::Remove(_))
    }

    /// The folders this op makes or changes things in. Unless root mode is
    /// visibly on for every one of them, the op is asked about before it
    /// runs (priv_ops.rs): a cached sudo ticket alone is not the user
    /// saying "as administrator".
    pub fn folders(&self) -> Vec<&Path> {
        match self {
            PendingPrivOp::Remove(paths) => paths.iter().map(|p| parent(p)).collect(),
            PendingPrivOp::Move { items } | PendingPrivOp::Copy { items } => {
                items.iter().map(|i| parent(&i.target)).collect()
            }
            PendingPrivOp::Rename { item, .. } => vec![parent(&item.src), parent(&item.target)],
            PendingPrivOp::NewFolder { path, .. } | PendingPrivOp::NewFile(path) => {
                vec![parent(path)]
            }
            PendingPrivOp::Archive { dir, archive, .. } => vec![dir, parent(archive)],
            PendingPrivOp::Extract(items) => items.iter().map(|(_, into)| parent(into)).collect(),
        }
    }

    /// The folders a move takes its items out of. Taking an item out of a
    /// folder changes that folder too; it only needs asking about where
    /// the user could not have done it themselves (priv_ops.rs).
    pub fn source_folders(&self) -> Vec<&Path> {
        match self {
            PendingPrivOp::Move { items } => items.iter().map(|i| parent(&i.src)).collect(),
            _ => Vec::new(),
        }
    }

    /// This op without the items that involve a mounted device, and those
    /// items with the reason: a privileged move or rename of a folder a
    /// drive is mounted on (or in) ends in a copy of the whole device and
    /// an `rm` of it; replacing one moves the device's contents to the
    /// Trash; deleting one empties it. `None`: nothing is left to run.
    /// Trash slots claimed for the items taken out are given back.
    pub fn without_mounts(self) -> (Option<Self>, Vec<crate::ops::OpFailure>) {
        use crate::mount_guard::{holds_mount, REASON};
        let fail = |path: &Path| crate::ops::OpFailure {
            path: path.to_path_buf(),
            reason: REASON.to_string(),
        };
        let mut refused = Vec::new();
        // The path of `item` a mount stands in the way of, if any.
        let blocked = |item: &PrivItem, moves_src: bool| -> Option<PathBuf> {
            if moves_src && holds_mount(&item.src) {
                Some(item.src.clone())
            } else if item.old_to_trash.is_some() && holds_mount(&item.target) {
                Some(item.target.clone())
            } else {
                None
            }
        };
        let mut sift = |items: Vec<PrivItem>, moves_src: bool| -> Vec<PrivItem> {
            items
                .into_iter()
                .filter(|item| match blocked(item, moves_src) {
                    Some(path) => {
                        if let Some(slot) = &item.old_to_trash {
                            crate::trash::release(slot);
                        }
                        refused.push(fail(&path));
                        false
                    }
                    None => true,
                })
                .collect()
        };
        let op = match self {
            PendingPrivOp::Move { items } => {
                let items = sift(items, true);
                (!items.is_empty()).then_some(PendingPrivOp::Move { items })
            }
            PendingPrivOp::Copy { items } => {
                let items = sift(items, false);
                (!items.is_empty()).then_some(PendingPrivOp::Copy { items })
            }
            PendingPrivOp::Rename { item, case_only } => sift(vec![item], true)
                .pop()
                .map(|item| PendingPrivOp::Rename { item, case_only }),
            PendingPrivOp::Remove(paths) => {
                let (mounted, plain): (Vec<PathBuf>, Vec<PathBuf>) =
                    paths.into_iter().partition(|p| holds_mount(p));
                refused.extend(mounted.iter().map(|p| fail(p)));
                (!plain.is_empty()).then_some(PendingPrivOp::Remove(plain))
            }
            other => Some(other),
        };
        (op, refused)
    }

    /// What a permanent delete removes, for the question that names it.
    pub fn doomed(&self) -> &[PathBuf] {
        match self {
            PendingPrivOp::Remove(paths) => paths,
            _ => &[],
        }
    }

    /// Call once the op will not be run (again): give back the Trash slots
    /// claimed for Replace items whose old item never went there.
    pub fn settle(&self) {
        let items: &[PrivItem] = match self {
            PendingPrivOp::Move { items } | PendingPrivOp::Copy { items } => items,
            PendingPrivOp::Rename { item, .. } => std::slice::from_ref(item),
            _ => &[],
        };
        for slot in items.iter().filter_map(|i| i.old_to_trash.as_ref()) {
            crate::trash::release(slot);
        }
    }
}

fn parent(p: &Path) -> &Path {
    p.parent().unwrap_or(p)
}

fn describe_items(verb: &str, items: &[PrivItem]) -> String {
    let dest = items
        .first()
        .and_then(|i| i.target.parent())
        .map(name_of)
        .unwrap_or_default();
    let replacing = items.iter().filter(|i| i.old_to_trash.is_some()).count();
    let mut text = match items {
        [one] => format!(
            "{verb} \u{201C}{}\u{201D} to \u{201C}{dest}\u{201D}",
            name_of(&one.src)
        ),
        _ => format!("{verb} {} items to \u{201C}{dest}\u{201D}", items.len()),
    };
    if replacing > 0 {
        text.push_str(&format!(" ({replacing} replaced, old to Trash)"));
    }
    text
}

pub(crate) fn name_of(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

/// End the sudo ticket Fox's own runs left behind, so administrator rights
/// do not outlive the ROOT badge. Returns at once; needs no password. (Under
/// a NOPASSWD rule there is no ticket to end, which is why operations
/// outside root mode are asked about as well.)
pub fn drop_ticket() {
    let mut cmd = Command::new("sudo");
    cmd.arg("-k").stderr(Stdio::null());
    crate::desktop::spawn_reaped(cmd);
}

/// Why [`run`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Every command of the op was run (see `Report::errors` for the ones
    /// that failed).
    Finished,
    /// sudo has no valid ticket and no password was given: nothing more
    /// was run. Ask for the password and continue at `Report::next`.
    NeedPassword,
    /// The password given is not the right one. Nothing more was run.
    WrongPassword,
}

/// What a [`run`] did.
#[derive(Debug)]
pub struct Report {
    pub stop: Stop,
    /// The first command not run yet. An op can be several commands (one
    /// per item), and the ticket can run out between two of them: the next
    /// attempt continues here instead of repeating what is done.
    pub next: usize,
    /// One message per command that ran and failed.
    pub errors: Vec<String>,
}

/// What one sudo invocation came to.
enum Ran {
    Ok,
    NeedPassword,
    WrongPassword,
    Failed(String),
}

/// How sudo's own complaint reads (under `LC_ALL=C`) when it is about the
/// password rather than about the command it was asked to run.
fn classify_failure(stderr: &str, had_password: bool, code: Option<i32>) -> Ran {
    if had_password {
        // -S got a line it did not accept, then end of input.
        let rejected = stderr.contains("incorrect password attempt")
            || stderr.contains("Sorry, try again")
            || stderr.contains("no password was provided");
        if rejected {
            return Ran::WrongPassword;
        }
    } else {
        // -n: "sudo: a password is required".
        let wanted = stderr.contains("password is required")
            || stderr.contains("a terminal is required")
            || stderr.contains("askpass");
        if wanted {
            return Ran::NeedPassword;
        }
    }
    let msg = stderr.trim();
    Ran::Failed(if msg.is_empty() {
        format!("Command failed (exit {})", code.unwrap_or(-1))
    } else {
        msg.to_string()
    })
}

fn sudo_once(argv: &[OsString], password: Option<&str>) -> Ran {
    let mut cmd = Command::new("sudo");
    // `classify_failure` matches sudo's English wording; pin the locale so
    // it holds on a translated system.
    cmd.env("LC_ALL", "C");
    // -S = read password from stdin (only consumed when no cached ticket).
    // -n = non-interactive: fail instead of prompting.
    // -p "" = empty prompt so sudo doesn't emit "[sudo] password:" noise.
    if password.is_none() {
        cmd.arg("-n");
    } else {
        cmd.arg("-S").arg("-p").arg("");
    }
    cmd.arg("--");
    cmd.args(argv);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran::Failed(format!("Failed to spawn sudo: {e}")),
    };
    if let Some(pw) = password {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = writeln!(stdin, "{pw}");
        }
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return Ran::Failed(format!("sudo wait failed: {e}")),
    };
    if output.status.success() {
        return Ran::Ok;
    }
    classify_failure(
        &String::from_utf8_lossy(&output.stderr),
        password.is_some(),
        output.status.code(),
    )
}

/// Run the commands of `op` from number `from` on: with `sudo -n` (cached
/// ticket, never prompts) when `password` is `None`, else with `sudo -S`.
/// Blocks until they have ended; call it from a worker thread.
///
/// A command that fails does not stop the ones after it (each is one item
/// of the op). A password problem does: nothing further is attempted.
pub fn run(op: &PendingPrivOp, password: Option<&str>, from: usize) -> Report {
    run_each(&build_argv(op), from, |argv| sudo_once(argv, password))
}

fn run_each(
    commands: &[Vec<OsString>],
    from: usize,
    mut exec: impl FnMut(&[OsString]) -> Ran,
) -> Report {
    let mut errors = Vec::new();
    for (i, argv) in commands.iter().enumerate().skip(from) {
        let stop = match exec(argv) {
            Ran::Ok => continue,
            Ran::Failed(msg) => {
                errors.push(msg);
                continue;
            }
            Ran::NeedPassword => Stop::NeedPassword,
            Ran::WrongPassword => Stop::WrongPassword,
        };
        return Report {
            stop,
            next: i,
            errors,
        };
    }
    Report {
        stop: Stop::Finished,
        next: commands.len().max(from),
        errors,
    }
}

/// Build the argv list for an op. Returns one argv per sub-command — most ops
/// are a single command, but Move/Copy/Extract emit one per item so a failure
/// in the middle doesn't take the remaining items down with it.
fn build_argv(op: &PendingPrivOp) -> Vec<Vec<OsString>> {
    match op {
        PendingPrivOp::Remove(paths) => {
            // `--`: a name starting with '-' is a path, not an option.
            // `--one-file-system`: never into a drive, phone or share
            // mounted somewhere below (mount_guard.rs is asked before this
            // is built; this is the same rule held by rm itself). It also
            // stops at a btrfs subvolume nested in the folder: rm then
            // reports that it could not remove everything, and says where.
            let mut argv: Vec<OsString> = vec![
                "rm".into(),
                "-rf".into(),
                "--one-file-system".into(),
                "--".into(),
            ];
            for p in paths {
                argv.push(p.as_os_str().to_owned());
            }
            vec![argv]
        }
        PendingPrivOp::Move { items } => items.iter().map(move_argv).collect(),
        PendingPrivOp::Copy { items } => items.iter().map(copy_argv).collect(),
        PendingPrivOp::Rename { item, case_only } => vec![if *case_only {
            tools::case_rename_argv(&item.src, &item.target)
        } else {
            move_argv(item)
        }],
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
        PendingPrivOp::Archive {
            dir,
            archive,
            members,
        } => vec![tools::archive_argv(dir, archive, members)],
        PendingPrivOp::Extract(items) => items
            .iter()
            .map(|(archive, into)| tools::extract_argv(archive, into))
            .collect(),
    }
}

#[cfg(test)]
mod tests;
