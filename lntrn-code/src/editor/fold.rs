//! Folding: the blocks a document can fold away (bracketed blocks read
//! off the tokens; indented blocks where a language has no brackets),
//! and the layout that maps document lines to rows on screen once some
//! are folded. The same pass gives every line its bracket depth, for
//! coloring brackets by nesting.

use std::collections::BTreeSet;

use crate::doc::Doc;
use crate::editor::wrap::Wrap;
use crate::syntax::{Language, TokenKind};
use crate::text_util::{bracket_pair, cell_width, indent_of};

/// Lines `start..=end`; folded, `start` stays and the rest hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
}

/// What one pass over the document found.
pub struct Scan {
    /// One per start line, the longest block starting there, sorted.
    pub regions: Vec<Region>,
    /// Bracket depth at the start of every line.
    pub depth: Vec<u16>,
}

fn by_indent(lang: Language) -> bool {
    matches!(lang, Language::Python | Language::Yaml | Language::Markdown | Language::Plain)
}

/// The foldable blocks of `doc` and the bracket depth of every line.
/// The tokens must be ready for the whole document (`Highlighter::ensure`).
pub fn scan(doc: &Doc) -> Scan {
    let n = doc.buffer.line_count();
    let mut ends: Vec<Option<usize>> = vec![None; n];
    let mut depth = vec![0u16; n];
    let tab = doc.tab();
    if by_indent(doc.lang()) {
        let indent = |l: usize| -> Option<usize> {
            let t = doc.line(l);
            if t.trim().is_empty() { None } else { Some(cell_width(indent_of(t), tab)) }
        };
        for (l, end) in ends.iter_mut().enumerate() {
            let Some(i) = indent(l) else {
                continue;
            };
            let mut last = None;
            let mut m = l + 1;
            while m < n {
                match indent(m) {
                    None => {}
                    Some(j) if j > i => last = Some(m),
                    _ => break,
                }
                m += 1;
            }
            *end = last;
        }
        // Markdown: a heading folds everything down to the next heading
        // of its level or above, or to a horizontal rule (`---`), which
        // ends a section without starting one.
        if doc.lang() == Language::Markdown {
            let level = |l: usize| -> Option<usize> {
                let t = doc.line(l).trim_start();
                (doc.highlight.tokens(l).first().is_some_and(|t| t.kind == TokenKind::Heading)).then(|| t.bytes().take_while(|&b| b == b'#').count())
            };
            let is_rule = |l: usize| -> bool {
                let t = doc.line(l).trim();
                let c = t.chars().next();
                matches!(c, Some('-' | '*' | '_')) && t.chars().filter(|&x| x != ' ').all(|x| Some(x) == c) && t.chars().filter(|&x| x != ' ').count() >= 3
            };
            for l in 0..n {
                let Some(lv) = level(l) else {
                    continue;
                };
                let next = (l + 1..n).find(|&m| is_rule(m) || level(m).is_some_and(|k| k <= lv)).unwrap_or(n);
                let mut end = next - 1;
                while end > l && doc.line(end).trim().is_empty() {
                    end -= 1;
                }
                if end > l {
                    ends[l] = Some(end);
                }
            }
        }
    } else {
        // Brackets outside strings and comments, matched across lines.
        let mut stack: Vec<usize> = Vec::new();
        let mut d = 0u16;
        for (l, dep) in depth.iter_mut().enumerate() {
            *dep = d;
            let text = doc.line(l);
            for t in doc.highlight.tokens(l) {
                if !matches!(t.kind, TokenKind::Punct | TokenKind::Operator) {
                    continue;
                }
                let (a, b) = (t.start as usize, (t.end as usize).min(text.len()));
                for c in text[a..b].chars() {
                    match bracket_pair(c) {
                        Some((_, _, true)) => {
                            stack.push(l);
                            d = d.saturating_add(1);
                        }
                        Some((_, _, false)) => {
                            d = d.saturating_sub(1);
                            if let Some(s) = stack.pop()
                                && s < l
                            {
                                ends[s] = Some(ends[s].map_or(l, |e| e.max(l)));
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    }
    let regions = ends.iter().enumerate().filter_map(|(s, e)| e.map(|end| Region { start: s, end })).collect();
    Scan { regions, depth }
}

/// Rows on screen: which line (and which wrapped row of it) each row
/// shows, and which row each line starts on (a hidden line is on its
/// fold header's row).
pub struct Layout {
    rows: Vec<(u32, u16)>,
    row_of: Vec<u32>,
    /// Rows each line takes: its wrapped rows, or 0 when hidden.
    nrows: Vec<u16>,
    hidden: Vec<bool>,
    /// The last hidden line of a folded header.
    ends: Vec<Option<u32>>,
}

impl Layout {
    pub fn build(n: usize, regions: &[Region], folded: &BTreeSet<usize>, wrap: &Wrap) -> Self {
        let n = n.max(1);
        let mut ends: Vec<Option<u32>> = vec![None; n];
        for r in regions {
            if folded.contains(&r.start) && r.start < n {
                ends[r.start] = Some(r.end.min(n - 1) as u32);
            }
        }
        let mut rows = Vec::with_capacity(n);
        let mut row_of = vec![0u32; n];
        let mut nrows = vec![0u16; n];
        let mut hidden = vec![false; n];
        let mut l = 0;
        while l < n {
            let row = rows.len() as u32;
            let k = wrap.rows_of(l).max(1);
            for s in 0..k {
                rows.push((l as u32, s as u16));
            }
            row_of[l] = row;
            nrows[l] = k as u16;
            match ends[l] {
                Some(e) => {
                    for h in l + 1..=e as usize {
                        hidden[h] = true;
                        row_of[h] = row;
                    }
                    l = e as usize + 1;
                }
                None => l += 1,
            }
        }
        Self { rows, row_of, nrows, hidden, ends }
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// The line on `row`, clamped to the last row.
    pub fn line_at(&self, row: usize) -> usize {
        self.rows[row.min(self.rows.len() - 1)].0 as usize
    }

    /// The line on `row` and which of its wrapped rows it is.
    pub fn seg_at(&self, row: usize) -> (usize, usize) {
        let (l, s) = self.rows[row.min(self.rows.len() - 1)];
        (l as usize, s as usize)
    }

    /// The first row of `line`.
    pub fn row_of(&self, line: usize) -> usize {
        self.row_of[line.min(self.row_of.len() - 1)] as usize
    }

    /// Rows `line` takes on screen (0 when folded away).
    pub fn rows_of(&self, line: usize) -> usize {
        self.nrows.get(line).copied().unwrap_or(1) as usize
    }

    pub fn hidden(&self, line: usize) -> bool {
        self.hidden.get(line).copied().unwrap_or(false)
    }

    /// The last line a folded header hides, when `line` is one.
    pub fn folded_end(&self, line: usize) -> Option<usize> {
        self.ends.get(line).copied().flatten().map(|e| e as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::DocId;
    use std::path::PathBuf;

    #[test]
    fn brackets_and_layout() {
        let text = "fn a() {\n    if x {\n        y();\n    }\n}\nfn b() {}\n";
        let mut doc = Doc::from_text(DocId(1), Some(PathBuf::from("/t/a.rs")), text, 4);
        doc.highlight.ensure(&doc.buffer, 6);
        let s = scan(&doc);
        assert_eq!(s.regions, vec![Region { start: 0, end: 4 }, Region { start: 1, end: 3 }], "one-line braces fold nothing");
        assert_eq!(&s.depth[..6], &[0, 1, 2, 2, 1, 0]);
        let folded: BTreeSet<usize> = [0].into_iter().collect();
        let l = Layout::build(7, &s.regions, &folded, &Wrap::none());
        assert_eq!(l.rows(), 3, "header, fn b, the last empty line");
        assert!(l.hidden(2) && !l.hidden(5));
        assert_eq!((l.row_of(3), l.row_of(5), l.line_at(1)), (0, 1, 5));
        assert_eq!(l.folded_end(0), Some(4));
        let both: BTreeSet<usize> = [0, 1].into_iter().collect();
        assert_eq!(Layout::build(7, &s.regions, &both, &Wrap::none()).rows(), 3, "a fold inside a fold hides nothing more");
    }

    #[test]
    fn indentation_blocks() {
        let text = "def a():\n    x = 1\n\n    y = 2\nprint(a)\n";
        let doc = Doc::from_text(DocId(2), Some(PathBuf::from("/t/a.py")), text, 4);
        let s = scan(&doc);
        assert_eq!(s.regions, vec![Region { start: 0, end: 3 }], "blank lines inside stay inside");
    }
}
