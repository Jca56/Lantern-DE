//! The text of a document: lines of UTF-8 without their line breaks, a
//! position as `(line, byte column)`, and one edit primitive
//! ([`Buffer::replace`]) every change goes through.

use std::cmp::Ordering;

/// A place in the text: a line, and a byte offset into it (always on a
/// character boundary once clamped).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

impl PartialOrd for Pos {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Pos {
    fn cmp(&self, o: &Self) -> Ordering {
        self.line.cmp(&o.line).then(self.col.cmp(&o.col))
    }
}

/// An ordered span of text: `start <= end`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub start: Pos,
    pub end: Pos,
}

impl Range {
    /// The span between two positions in either order.
    pub fn new(a: Pos, b: Pos) -> Self {
        if a <= b { Self { start: a, end: b } } else { Self { start: b, end: a } }
    }

    pub fn at(p: Pos) -> Self {
        Self { start: p, end: p }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Buffer {
    lines: Vec<String>,
    pub ending: LineEnding,
    /// The file ended with a line break (kept so a save changes nothing
    /// the user did not touch).
    pub trailing_newline: bool,
    version: u64,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    /// One empty line.
    pub fn new() -> Self {
        Self { lines: vec![String::new()], ending: LineEnding::Lf, trailing_newline: true, version: 1 }
    }

    /// Split `text` into lines, remembering its line-ending style.
    pub fn from_text(text: &str) -> Self {
        let ending = if text.contains("\r\n") { LineEnding::CrLf } else { LineEnding::Lf };
        let trailing_newline = text.ends_with('\n') || text.is_empty();
        let body = text.strip_suffix("\r\n").or_else(|| text.strip_suffix('\n')).unwrap_or(text);
        let lines = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned()).collect();
        Self { lines, ending, trailing_newline, version: 1 }
    }

    /// The text as it goes to disk.
    pub fn to_text(&self) -> String {
        let sep = self.ending.as_str();
        let mut out = self.lines.join(sep);
        if self.trailing_newline && !self.is_empty() {
            out.push_str(sep);
        }
        out
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The line's text (empty past the end).
    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Bumps on every change; equal versions mean equal text.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn end(&self) -> Pos {
        Pos::new(self.lines.len() - 1, self.lines[self.lines.len() - 1].len())
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// `p` moved onto a real character boundary inside the text.
    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        let s = &self.lines[line];
        let mut col = p.col.min(s.len());
        while !s.is_char_boundary(col) {
            col -= 1;
        }
        Pos::new(line, col)
    }

    pub fn text_in(&self, r: Range) -> String {
        let (a, b) = (self.clamp(r.start), self.clamp(r.end));
        if a.line == b.line {
            return self.lines[a.line][a.col..b.col].to_owned();
        }
        let mut out = String::from(&self.lines[a.line][a.col..]);
        for l in &self.lines[a.line + 1..b.line] {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
        out.push_str(&self.lines[b.line][..b.col]);
        out
    }

    /// Replace `r` with `text` (which may hold line breaks). Returns where
    /// the inserted text ends.
    pub fn replace(&mut self, r: Range, text: &str) -> Pos {
        let (a, b) = (self.clamp(r.start), self.clamp(r.end));
        let head = self.lines[a.line][..a.col].to_owned();
        let tail = self.lines[b.line][b.col..].to_owned();
        let mut new_lines: Vec<String> = Vec::new();
        let mut end = a;
        for (i, seg) in text.split('\n').enumerate() {
            let seg = seg.strip_suffix('\r').unwrap_or(seg);
            if i == 0 {
                new_lines.push(format!("{head}{seg}"));
                end = Pos::new(a.line, head.len() + seg.len());
            } else {
                new_lines.push(seg.to_owned());
                end = Pos::new(a.line + i, seg.len());
            }
        }
        new_lines.last_mut().expect("split yields at least one").push_str(&tail);
        self.lines.splice(a.line..=b.line, new_lines);
        self.version += 1;
        end
    }

    /// The character starting at `p`, if any (`None` at a line's end).
    pub fn char_at(&self, p: Pos) -> Option<char> {
        let p = self.clamp(p);
        self.lines[p.line][p.col..].chars().next()
    }

    /// The character before `p` on its line.
    pub fn char_before(&self, p: Pos) -> Option<char> {
        let p = self.clamp(p);
        self.lines[p.line][..p.col].chars().next_back()
    }

    /// One character forward, crossing line breaks.
    pub fn next_pos(&self, p: Pos) -> Pos {
        let p = self.clamp(p);
        let s = &self.lines[p.line];
        if p.col < s.len() {
            Pos::new(p.line, p.col + s[p.col..].chars().next().map_or(1, char::len_utf8))
        } else if p.line + 1 < self.lines.len() {
            Pos::new(p.line + 1, 0)
        } else {
            p
        }
    }

    /// One character back, crossing line breaks.
    pub fn prev_pos(&self, p: Pos) -> Pos {
        let p = self.clamp(p);
        if p.col > 0 {
            let s = &self.lines[p.line];
            Pos::new(p.line, p.col - s[..p.col].chars().next_back().map_or(1, char::len_utf8))
        } else if p.line > 0 {
            Pos::new(p.line - 1, self.lines[p.line - 1].len())
        } else {
            p
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_line_endings() {
        let b = Buffer::from_text("a\r\nb\r\n");
        assert_eq!(b.line_count(), 2);
        assert_eq!(b.ending, LineEnding::CrLf);
        assert_eq!(b.to_text(), "a\r\nb\r\n");
        let b = Buffer::from_text("a\nb");
        assert!(!b.trailing_newline);
        assert_eq!(b.to_text(), "a\nb");
        assert_eq!(Buffer::from_text("").to_text(), "");
        assert_eq!(Buffer::new().to_text(), "");
        assert_eq!(Buffer::from_text("\n").line_count(), 1);
        assert!(Buffer::from_text("\n").trailing_newline);
    }

    #[test]
    fn replace_across_lines() {
        let mut b = Buffer::from_text("hello\nworld\n!");
        let end = b.replace(Range::new(Pos::new(0, 2), Pos::new(1, 3)), "X\nYY");
        assert_eq!(b.lines(), &["heX", "YYld", "!"]);
        assert_eq!(end, Pos::new(1, 2));
        let end = b.replace(Range::at(Pos::new(2, 1)), "\n");
        assert_eq!(b.lines(), &["heX", "YYld", "!", ""]);
        assert_eq!(end, Pos::new(3, 0));
        assert_eq!(b.text_in(Range::new(Pos::new(0, 1), Pos::new(2, 1))), "eX\nYYld\n!");
        let v = b.version();
        b.replace(Range::new(Pos::new(0, 0), b.end()), "");
        assert!(b.is_empty() && b.version() > v);
    }

    #[test]
    fn stepping_and_clamping() {
        let b = Buffer::from_text("aé\nb");
        assert_eq!(b.next_pos(Pos::new(0, 1)), Pos::new(0, 3), "é is two bytes");
        assert_eq!(b.next_pos(Pos::new(0, 3)), Pos::new(1, 0));
        assert_eq!(b.prev_pos(Pos::new(1, 0)), Pos::new(0, 3));
        assert_eq!(b.prev_pos(Pos::new(0, 0)), Pos::new(0, 0));
        assert_eq!(b.clamp(Pos::new(0, 2)), Pos::new(0, 1), "inside é snaps back");
        assert_eq!(b.clamp(Pos::new(9, 9)), Pos::new(1, 1));
        assert_eq!(b.char_at(Pos::new(0, 1)), Some('é'));
        assert_eq!(b.char_before(Pos::new(1, 1)), Some('b'));
    }
}
