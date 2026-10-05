//! What changed in one file, as lines to show: `git diff` read into
//! added, removed and unchanged lines with their line numbers. Blocking,
//! like the rest of the git layer.

use std::path::Path;

use crate::git::{FileState, FileStatus, git};

/// The most lines a diff shows; past it the rest is counted, not listed.
const MAX_LINES: usize = 5000;
/// The most characters of a line that are kept.
const MAX_WIDTH: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// `@@ -1,4 +1,6 @@`: where the lines after it sit in the file.
    Hunk,
    Added,
    Removed,
    Same,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Its number in the file: the new file's, or the old one's for a
    /// line that was removed. None for a hunk header.
    pub number: Option<u32>,
    pub text: String,
}

/// One file's changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<Line>,
    pub added: usize,
    pub removed: usize,
    /// Said in place of lines, or under them: the file is binary, is a
    /// folder, has more lines than are shown.
    pub note: Option<String>,
}

impl Diff {
    fn note(text: impl Into<String>) -> Diff {
        Diff { note: Some(text.into()), ..Diff::default() }
    }

    /// The longest line, in characters.
    pub fn width(&self) -> usize {
        self.lines.iter().map(|l| l.text.chars().count()).max().unwrap_or(0)
    }
}

/// A line as it is shown: tabs as four spaces, no carriage return, cut
/// at [`MAX_WIDTH`].
fn shown(text: &str) -> String {
    text.trim_end_matches('\r').replace('\t', "    ").chars().take(MAX_WIDTH).collect()
}

/// The numbers a hunk header starts its lines at: `(old, new)`.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let start = |s: &str| s[1..].split(',').next().and_then(|n| n.parse::<u32>().ok());
    let old = parts.next().filter(|p| p.starts_with('-')).and_then(start)?;
    let new = parts.next().filter(|p| p.starts_with('+')).and_then(start)?;
    Some((old, new))
}

/// A unified diff as lines. Everything before the first hunk (the file
/// names, the index line) is dropped.
pub fn parse(patch: &str) -> Diff {
    let mut diff = Diff::default();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    let mut skipped = 0usize;
    for line in patch.lines() {
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            return Diff::note("A binary file: there are no lines to show.");
        }
        let (kind, number, body) = if let Some((o, n)) = line.strip_prefix("@@").and_then(|_| hunk_starts(line)) {
            (old, new, in_hunk) = (o, n, true);
            (LineKind::Hunk, None, line)
        } else if !in_hunk || line.starts_with('\\') {
            // The header, or "\ No newline at end of file".
            continue;
        } else if let Some(body) = line.strip_prefix('+') {
            new += 1;
            diff.added += 1;
            (LineKind::Added, Some(new - 1), body)
        } else if let Some(body) = line.strip_prefix('-') {
            old += 1;
            diff.removed += 1;
            (LineKind::Removed, Some(old - 1), body)
        } else {
            (old, new) = (old + 1, new + 1);
            (LineKind::Same, Some(new - 1), line.strip_prefix(' ').unwrap_or(line))
        };
        if diff.lines.len() < MAX_LINES {
            diff.lines.push(Line { kind, number, text: shown(body) });
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 {
        diff.note = Some(format!("{skipped} more lines are not shown."));
    } else if diff.lines.is_empty() {
        diff.note = Some("Nothing changed inside the file (its mode or name did).".into());
    }
    diff
}

/// A file git has never seen, as a diff: every line of it added.
fn untracked(path: &Path) -> Diff {
    if path.is_dir() {
        return Diff::note("A new folder. Stage it to see the files inside.");
    }
    let Ok(bytes) = std::fs::read(path) else { return Diff::note("The file could not be read.") };
    if bytes.contains(&0) {
        return Diff::note("A binary file: there are no lines to show.");
    }
    let text = String::from_utf8_lossy(&bytes);
    let total = text.lines().count();
    let lines: Vec<Line> = text.lines().take(MAX_LINES).enumerate().map(|(i, l)| Line { kind: LineKind::Added, number: Some(i as u32 + 1), text: shown(l) }).collect();
    let note = match total {
        0 => Some("An empty file.".to_owned()),
        n if n > MAX_LINES => Some(format!("{} more lines are not shown.", n - MAX_LINES)),
        _ => None,
    };
    Diff { added: total, removed: 0, lines, note }
}

/// What changed in `file`: its staged changes against the last commit,
/// or its unstaged ones against what is staged.
pub fn of(repo: &Path, file: &FileStatus) -> Diff {
    if file.status == FileState::Untracked {
        return untracked(&repo.join(&file.path));
    }
    let mut cmd = git(repo);
    cmd.args(["diff", "--no-color", "--no-ext-diff"]);
    if file.staged {
        cmd.arg("--cached");
    }
    match cmd.args(["--", &file.path]).output() {
        Ok(output) => parse(&String::from_utf8_lossy(&output.stdout)),
        Err(e) => Diff::note(format!("git could not be run: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/src/app.rs b/src/app.rs\nindex 1a2b..3c4d 100644\n--- a/src/app.rs\n+++ b/src/app.rs\n@@ -10,4 +10,5 @@ fn main() {\n     let a = 1;\n-    let b = 2;\n+\tlet b = 3;\n+    let c = 4;\n     run(a);\n\\ No newline at end of file\n@@ -40,2 +41,1 @@\n-gone\n kept\n";

    #[test]
    fn a_patch_becomes_numbered_lines() {
        let d = parse(PATCH);
        assert_eq!((d.added, d.removed, d.note.as_deref()), (2, 2, None));
        let got: Vec<(LineKind, Option<u32>, &str)> = d.lines.iter().map(|l| (l.kind, l.number, l.text.as_str())).collect();
        use LineKind::*;
        assert_eq!(
            got,
            [
                (Hunk, None, "@@ -10,4 +10,5 @@ fn main() {"),
                (Same, Some(10), "    let a = 1;"),
                (Removed, Some(11), "    let b = 2;"),
                // A tab shows as four spaces.
                (Added, Some(11), "    let b = 3;"),
                (Added, Some(12), "    let c = 4;"),
                (Same, Some(13), "    run(a);"),
                (Hunk, None, "@@ -40,2 +41,1 @@"),
                (Removed, Some(40), "gone"),
                (Same, Some(41), "kept"),
            ]
        );
        assert_eq!(d.width(), "@@ -10,4 +10,5 @@ fn main() {".len());
    }

    #[test]
    fn what_has_no_lines_says_why() {
        assert!(parse("diff --git a/x.png b/x.png\nBinary files a/x.png and b/x.png differ\n").note.unwrap().contains("binary"));
        assert!(parse("diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n").note.unwrap().contains("mode"));
        // Past the cap the rest is counted.
        let long = format!("@@ -1,0 +1,{0} @@\n{1}", MAX_LINES + 7, "+x\n".repeat(MAX_LINES + 7));
        let d = parse(&long);
        assert_eq!((d.lines.len(), d.added), (MAX_LINES, MAX_LINES + 7));
        assert_eq!(d.note.as_deref(), Some("8 more lines are not shown."));
    }

    #[test]
    fn a_new_file_is_all_added() {
        let dir = std::env::temp_dir().join(format!("lntrn-git-diff-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        std::fs::write(dir.join("new.txt"), "one\ntwo\r\n").unwrap();
        std::fs::write(dir.join("blob.bin"), [1u8, 0, 2]).unwrap();
        let d = untracked(&dir.join("new.txt"));
        assert_eq!((d.added, d.lines.len(), d.lines[1].number, d.lines[1].text.as_str()), (2, 2, Some(2), "two"));
        assert!(untracked(&dir.join("blob.bin")).note.unwrap().contains("binary"));
        assert!(untracked(&dir.join("folder")).note.unwrap().contains("folder"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
