//! What changed in a file since its HEAD copy, as marks for the gutter:
//! lines added, lines modified, and places where lines were deleted.

use crate::bridge::diff::{Kind, diff_lines};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Added,
    Modified,
    /// Lines vanished before `line`.
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineMark {
    pub kind: MarkKind,
    /// 0-based first line in the current text.
    pub line: usize,
    /// Lines spanned (0 for a deletion).
    pub len: usize,
}

/// The marks for `new` against `old` (the HEAD text). A hunk with lines
/// both removed and added is a modification of the lines it added.
pub fn marks(old: &str, new: &[String]) -> Vec<LineMark> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_refs: Vec<&str> = new.iter().map(String::as_str).collect();
    let rows = diff_lines(&old_lines, &new_refs);
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if rows[i].kind == Kind::Same {
            i += 1;
            continue;
        }
        let start = i;
        while i < rows.len() && rows[i].kind != Kind::Same {
            i += 1;
        }
        let hunk = &rows[start..i];
        let removed = hunk.iter().filter(|r| r.kind == Kind::Removed).count();
        let added: Vec<usize> = hunk.iter().filter_map(|r| (r.kind == Kind::Added).then_some(r.new).flatten()).collect();
        match (removed, added.first()) {
            (0, Some(&first)) => out.push(LineMark { kind: MarkKind::Added, line: first, len: added.len() }),
            (_, Some(&first)) => out.push(LineMark { kind: MarkKind::Modified, line: first, len: added.len() }),
            (_, None) => {
                // Only removals: the deletion sits before the next kept line.
                let line = rows[i..].iter().find_map(|r| r.new).unwrap_or(new.len());
                out.push(LineMark { kind: MarkKind::Deleted, line, len: 0 });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    #[test]
    fn kinds_of_change() {
        let old = "a\nb\nc\nd\ne\n";
        assert!(marks(old, &lines(old)).is_empty());
        let m = marks(old, &lines("a\nb\nX\nY\nc\nd\ne\n"));
        assert_eq!(m, vec![LineMark { kind: MarkKind::Added, line: 2, len: 2 }]);
        let m = marks(old, &lines("a\nB\nc\nd\ne\n"));
        assert_eq!(m, vec![LineMark { kind: MarkKind::Modified, line: 1, len: 1 }]);
        let m = marks(old, &lines("a\nb\ne\n"));
        assert_eq!(m, vec![LineMark { kind: MarkKind::Deleted, line: 2, len: 0 }]);
        let m = marks(old, &lines("a\nb\nc\nd\n"));
        assert_eq!(m, vec![LineMark { kind: MarkKind::Deleted, line: 4, len: 0 }], "deleted at the end");
        let m = marks("", &lines("new\nfile\n"));
        assert_eq!(m, vec![LineMark { kind: MarkKind::Added, line: 0, len: 2 }]);
    }
}
