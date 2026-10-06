//! A paragraph's runs: the stretches of its text that have attributes of
//! their own. They are kept in order, apart, and never plain or empty;
//! whatever lies between them is plain text.

use super::attrs::TextAttrs;

/// Bytes `start..end` of a paragraph's text, and what they have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub attrs: TextAttrs,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spans(Vec<Span>);

impl Spans {
    pub fn list(&self) -> &[Span] {
        &self.0
    }

    /// What the character at `byte` has.
    pub fn at(&self, byte: usize) -> TextAttrs {
        self.0.iter().find(|s| s.start <= byte && byte < s.end).map_or_else(TextAttrs::default, |s| s.attrs)
    }

    /// Every stretch of `start..end` in order, the plain ones included.
    pub fn runs(&self, start: usize, end: usize) -> Vec<Span> {
        let mut out = Vec::new();
        if start >= end {
            return out;
        }
        let mut at = start;
        for s in self.0.iter().filter(|s| s.end > start && s.start < end) {
            let (a, b) = (s.start.max(start), s.end.min(end));
            if a > at {
                out.push(Span { start: at, end: a, attrs: TextAttrs::default() });
            }
            out.push(Span { start: a, end: b, attrs: s.attrs });
            at = b;
        }
        if at < end {
            out.push(Span { start: at, end, attrs: TextAttrs::default() });
        }
        out
    }

    /// What all of `start..end` has in common; nothing, for an empty
    /// range.
    pub fn common(&self, start: usize, end: usize) -> TextAttrs {
        self.runs(start, end).into_iter().map(|s| s.attrs).reduce(TextAttrs::common).unwrap_or_default()
    }

    /// Change what `start..end` has, each stretch from what it had.
    pub fn apply(&mut self, start: usize, end: usize, change: impl Fn(&mut TextAttrs)) {
        if start >= end {
            return;
        }
        let mut all = self.runs(start, end);
        for s in &mut all {
            change(&mut s.attrs);
        }
        // What the spans have outside the range stays as it is.
        for s in &self.0 {
            if s.start < start {
                all.push(Span { end: s.end.min(start), ..*s });
            }
            if s.end > end {
                all.push(Span { start: s.start.max(end), ..*s });
            }
        }
        self.0 = all;
        self.tidy();
    }

    /// Cut at `byte`: what is after it comes back, counted from zero.
    pub fn split_off(&mut self, byte: usize) -> Spans {
        let after = self.0.iter().filter(|s| s.end > byte).map(|s| Span { start: s.start.max(byte) - byte, end: s.end - byte, attrs: s.attrs }).collect();
        self.0.retain_mut(|s| {
            s.end = s.end.min(byte);
            s.start < s.end
        });
        Spans(after)
    }

    /// Join `other` on after `len` bytes of this paragraph's own text.
    pub fn append(&mut self, other: Spans, len: usize) {
        self.0.extend(other.0.into_iter().map(|s| Span { start: s.start + len, end: s.end + len, attrs: s.attrs }));
        self.tidy();
    }

    /// `len` bytes went in at `byte`, with `attrs`. A run they went into
    /// the middle of is parted around them: the new text has exactly what
    /// it was given.
    pub fn insert(&mut self, byte: usize, len: usize, attrs: TextAttrs) {
        if len == 0 {
            return;
        }
        if let Some(i) = self.0.iter().position(|s| s.start < byte && byte < s.end) {
            let rest = Span { start: byte, ..self.0[i] };
            self.0[i].end = byte;
            self.0.push(rest);
        }
        for s in self.0.iter_mut().filter(|s| s.start >= byte) {
            (s.start, s.end) = (s.start + len, s.end + len);
        }
        self.0.push(Span { start: byte, end: byte + len, attrs });
        self.tidy();
    }

    /// Bytes `start..end` were taken out.
    pub fn remove(&mut self, start: usize, end: usize) {
        let len = end - start;
        let moved = |byte: usize| if byte <= start { byte } else { byte.max(end) - len };
        for s in &mut self.0 {
            (s.start, s.end) = (moved(s.start), moved(s.end));
        }
        self.tidy();
    }

    /// Back in order, apart, with nothing plain or empty, and neighbours
    /// that have the same joined.
    fn tidy(&mut self) {
        self.0.retain(|s| s.start < s.end && !s.attrs.is_default());
        self.0.sort_by_key(|s| s.start);
        let mut out: Vec<Span> = Vec::with_capacity(self.0.len());
        for s in self.0.drain(..) {
            match out.last_mut() {
                Some(last) if last.end == s.start && last.attrs == s.attrs => last.end = s.end,
                _ => out.push(s),
            }
        }
        self.0 = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold(a: &mut TextAttrs) {
        a.bold = true;
    }

    fn shape(s: &Spans) -> Vec<(usize, usize, bool, bool)> {
        s.list().iter().map(|s| (s.start, s.end, s.attrs.bold, s.attrs.italic)).collect()
    }

    #[test]
    fn attributes_are_applied_over_what_is_there() {
        let mut s = Spans::default();
        s.apply(2, 6, bold);
        s.apply(4, 9, |a| a.italic = true);
        assert_eq!(shape(&s), [(2, 4, true, false), (4, 6, true, true), (6, 9, false, true)]);
        assert_eq!((s.at(1).bold, s.at(2).bold, s.at(5).italic, s.at(9).italic), (false, true, true, false));
        // Taking bold off the middle leaves its ends; runs that come to
        // have the same are one run.
        s.apply(3, 5, |a| a.bold = false);
        assert_eq!(shape(&s), [(2, 3, true, false), (4, 5, false, true), (5, 6, true, true), (6, 9, false, true)]);
        s.apply(0, 20, |a| *a = TextAttrs { bold: true, ..TextAttrs::default() });
        assert_eq!(shape(&s), [(0, 20, true, false)]);
        s.apply(0, 20, |a| a.bold = false);
        assert!(s.list().is_empty(), "nothing plain is kept");
        // What a range has in common, and its stretches with the plain
        // ones between.
        s.apply(0, 4, bold);
        s.apply(2, 8, |a| a.size = Some(30.0));
        assert_eq!((s.common(0, 4).bold, s.common(0, 6).bold, s.common(2, 8).size, s.common(0, 8).size), (true, false, Some(30.0), None));
        assert_eq!(s.common(3, 3), TextAttrs::default());
        assert_eq!(s.runs(1, 10).iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(1, 2), (2, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn runs_follow_the_text_as_it_is_typed_cut_parted_and_joined() {
        let mut s = Spans::default();
        s.apply(2, 8, bold);
        // Plain text typed into a bold run parts it; bold text typed at
        // its end joins it; text before it moves it along.
        s.insert(4, 3, TextAttrs::default());
        assert_eq!(shape(&s), [(2, 4, true, false), (7, 11, true, false)]);
        s.insert(11, 2, TextAttrs { bold: true, ..TextAttrs::default() });
        s.insert(0, 1, TextAttrs::default());
        assert_eq!(shape(&s), [(3, 5, true, false), (8, 14, true, false)]);
        // A cut through a run's end shortens it; one that swallows a run
        // takes it; what is after moves back.
        s.remove(4, 9);
        assert_eq!(shape(&s), [(3, 9, true, false)], "the two ends meet and are one run");
        s.remove(0, 3);
        s.remove(4, 20);
        assert_eq!(shape(&s), [(0, 4, true, false)]);
        // Parted in two and put back together.
        s.apply(2, 6, |a| a.italic = true);
        let mut left = s.clone();
        let right = left.split_off(3);
        assert_eq!((shape(&left), shape(&right)), (vec![(0, 2, true, false), (2, 3, true, true)], vec![(0, 1, true, true), (1, 3, false, true)]));
        left.append(right, 3);
        assert_eq!(left, s);
    }
}
