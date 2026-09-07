//! The thread that talks to the git binary: one request at a time,
//! each answered with replies and a wake of the app.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender};

use lntrn_app::Waker;

use super::{Blob, Commit, CommitDiff, CommitFile, FileStatus, Reply, Req, Snapshot};

/// A git command in `root`. Optional locks are off so a status never
/// rewrites the index: the app watches `.git`, and its own status
/// touching the index would ask for another status, forever.
fn command(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).env("GIT_OPTIONAL_LOCKS", "0");
    c
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = command(root).args(args).output().map_err(|e| format!("git: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() { Ok(text) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

/// Like [`git`], but a success reports what went to stderr too: push
/// and pull say what they did there.
fn git_chatty(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = command(root).args(args).output().map_err(|e| format!("git: {e}"))?;
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok(if text.is_empty() { err } else { text })
    } else {
        Err(err)
    }
}

/// `git log` with unit separators between the fields.
fn parse_log(raw: &str) -> Vec<Commit> {
    raw.lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            let hash = f.next()?.to_owned();
            let short = f.next()?.to_owned();
            let author = f.next()?.to_owned();
            let when = f.next()?.to_owned();
            let subject = f.next().unwrap_or("").to_owned();
            (hash.len() == 40).then_some(Commit { hash, short, author, when, subject })
        })
        .collect()
}

/// `git diff-tree --name-status`: `X\tpath` per line (renames `R100\told\tnew`).
fn parse_name_status(raw: &str) -> Vec<CommitFile> {
    raw.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let letter = f.next()?.chars().next()?;
            let rel = f.last()?.to_owned();
            Some(CommitFile { letter, rel })
        })
        .collect()
}

/// `git status --porcelain=v1 -z`: `XY path\0`, renames adding `\0old`.
pub(super) fn parse_status(raw: &[u8]) -> Vec<(String, FileStatus)> {
    let mut out = Vec::new();
    let mut parts = raw.split(|b| *b == 0).peekable();
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let index = entry[0] as char;
        let work = entry[1] as char;
        let rel = String::from_utf8_lossy(&entry[3..]).into_owned();
        if index == 'R' || index == 'C' || work == 'R' || work == 'C' {
            parts.next();
        }
        out.push((rel, FileStatus { index, work }));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn status_reply(root: &Path, log_limit: usize) -> Reply {
    let branch = match git(root, &["symbolic-ref", "--short", "-q", "HEAD"]) {
        Ok(b) if !b.trim().is_empty() => b.trim().to_owned(),
        _ => git(root, &["rev-parse", "--short", "HEAD"]).map(|h| format!("detached {}", h.trim())).unwrap_or_else(|_| "no commits".to_owned()),
    };
    let head = git(root, &["rev-parse", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_default();
    let upstream = git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok().map(|u| u.trim().to_owned()).filter(|u| !u.is_empty());
    let (ahead, behind) = match upstream.as_ref().and_then(|_| git(root, &["rev-list", "--left-right", "--count", "HEAD...@{u}"]).ok()) {
        Some(t) => {
            let mut n = t.split_whitespace().map(|x| x.parse::<usize>().unwrap_or(0));
            (n.next().unwrap_or(0), n.next().unwrap_or(0))
        }
        None => (0, 0),
    };
    let branches = git(root, &["branch", "--format=%(refname:short)"]).map(|b| b.lines().map(str::to_owned).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
    let n = log_limit.to_string();
    let log = git(root, &["log", "--format=%H%x1f%h%x1f%an%x1f%ar%x1f%s", "-n", &n]).map(|l| parse_log(&l)).unwrap_or_default();
    let out = command(root).args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]).output();
    let (changes, error) = match out {
        Ok(o) if o.status.success() => (parse_status(&o.stdout), None),
        Ok(o) => (Vec::new(), Some(String::from_utf8_lossy(&o.stderr).trim().to_owned())),
        Err(e) => (Vec::new(), Some(format!("git: {e}"))),
    };
    Reply::Status(Box::new(Snapshot { branch, head, upstream, ahead, behind, branches, log, changes, error }))
}

pub(super) fn worker(root: &Path, rx: &Receiver<Req>, tx: &Sender<Reply>, waker: Option<&Waker>) {
    while let Ok(req) = rx.recv() {
        let replies = match req {
            Req::Status { log } => vec![status_reply(root, log)],
            Req::Blob(path, rel) => {
                let head = git(root, &["rev-parse", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_default();
                let blob = match git(root, &["show", &format!("HEAD:{rel}")]) {
                    Ok(t) => Blob::Text(t),
                    Err(_) => Blob::Missing,
                };
                vec![Reply::Blob { path, head, blob }]
            }
            Req::Run(args, log) => {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let ran = match git_chatty(root, &refs) {
                    Ok(o) => Reply::Ran { ok: true, output: o },
                    Err(e) => Reply::Ran { ok: false, output: e },
                };
                vec![ran, status_reply(root, log)]
            }
            Req::CommitFiles(hash) => {
                let files = git(root, &["diff-tree", "--no-commit-id", "--name-status", "-r", "--root", "-M", &hash]).map(|t| parse_name_status(&t)).unwrap_or_default();
                vec![Reply::CommitFiles { hash, files }]
            }
            Req::CommitDiff { hash, short, rel } => {
                let old = git(root, &["show", &format!("{hash}^:{rel}")]).unwrap_or_default();
                let new = git(root, &["show", &format!("{hash}:{rel}")]).unwrap_or_default();
                vec![Reply::CommitDiff(Box::new(CommitDiff { short, rel, old, new }))]
            }
        };
        for r in replies {
            if tx.send(r).is_err() {
                return;
            }
        }
        if let Some(w) = waker {
            w.wake();
        }
    }
}

