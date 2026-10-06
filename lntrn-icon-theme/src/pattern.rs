//! The regular expressions an icon theme's rules are written in, matched
//! by backtracking: anchors, `.`, classes (ranges, negation, `\d \w \s`),
//! groups with alternation, `? * + {n} {n,m}`, and `\b`. Enough for every
//! rule in the Atom Material tables; not a general engine.
//!
//! A table tries hundreds of patterns on every name, so two things are
//! known about a pattern before any of that: where a match can start, and
//! (`last_chars`) what a name has to end on for it to match at all.

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Char(char),
    Any,
    Class { negated: bool, items: Vec<ClassItem> },
    Start,
    End,
    WordBoundary,
    Group(Vec<Vec<Node>>),
    Repeat { node: Box<Node>, min: usize, max: Option<usize> },
}

#[derive(Clone, Debug, PartialEq)]
enum ClassItem {
    Range(char, char),
    Digit,
    Word,
    Space,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    alts: Vec<Vec<Node>>,
    /// Case folded on both sides when set.
    ignore_case: bool,
}

struct Parser<'a> {
    chars: Vec<char>,
    i: usize,
    _src: &'a str,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn alternation(&mut self) -> Option<Vec<Vec<Node>>> {
        let mut alts = vec![self.sequence()?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.sequence()?);
        }
        Some(alts)
    }

    fn sequence(&mut self) -> Option<Vec<Node>> {
        let mut seq = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            let node = self.quantified(atom)?;
            seq.push(node);
        }
        Some(seq)
    }

    fn quantified(&mut self, atom: Node) -> Option<Node> {
        let (min, max) = match self.peek() {
            Some('?') => (0, Some(1)),
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('{') => {
                let close = self.chars[self.i..].iter().position(|&c| c == '}')? + self.i;
                let inner: String = self.chars[self.i + 1..close].iter().collect();
                self.i = close;
                let (a, b) = match inner.split_once(',') {
                    Some((a, b)) => (a.trim().parse().ok()?, if b.trim().is_empty() { None } else { Some(b.trim().parse().ok()?) }),
                    None => {
                        let n = inner.trim().parse().ok()?;
                        (n, Some(n))
                    }
                };
                (a, b)
            }
            _ => return Some(atom),
        };
        self.i += 1;
        // A lazy marker changes nothing for a full-string match.
        if self.peek() == Some('?') {
            self.i += 1;
        }
        Some(Node::Repeat { node: Box::new(atom), min, max })
    }

    fn escape(&mut self) -> Option<Node> {
        let c = self.peek()?;
        self.i += 1;
        Some(match c {
            'd' => Node::Class { negated: false, items: vec![ClassItem::Digit] },
            'D' => Node::Class { negated: true, items: vec![ClassItem::Digit] },
            'w' => Node::Class { negated: false, items: vec![ClassItem::Word] },
            'W' => Node::Class { negated: true, items: vec![ClassItem::Word] },
            's' => Node::Class { negated: false, items: vec![ClassItem::Space] },
            'S' => Node::Class { negated: true, items: vec![ClassItem::Space] },
            'b' => Node::WordBoundary,
            'n' => Node::Char('\n'),
            't' => Node::Char('\t'),
            other => Node::Char(other),
        })
    }

    fn atom(&mut self) -> Option<Node> {
        let c = self.peek()?;
        self.i += 1;
        Some(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '\\' => self.escape()?,
            '(' => {
                // `(?:` and `(?i)` markers: the first is a plain group.
                if self.peek() == Some('?') {
                    self.i += 1;
                    match self.peek() {
                        Some(':') => self.i += 1,
                        Some('i') => {
                            self.i += 1;
                            if self.peek() == Some(')') {
                                self.i += 1;
                                return Some(Node::Group(vec![Vec::new()]));
                            }
                        }
                        _ => {}
                    }
                }
                let alts = self.alternation()?;
                if self.peek() != Some(')') {
                    return None;
                }
                self.i += 1;
                Node::Group(alts)
            }
            '[' => {
                let negated = self.peek() == Some('^');
                if negated {
                    self.i += 1;
                }
                let mut items = Vec::new();
                let mut first = true;
                loop {
                    let c = self.peek()?;
                    if c == ']' && !first {
                        self.i += 1;
                        break;
                    }
                    first = false;
                    self.i += 1;
                    let lo = if c == '\\' {
                        let e = self.peek()?;
                        self.i += 1;
                        match e {
                            'd' => {
                                items.push(ClassItem::Digit);
                                continue;
                            }
                            'w' => {
                                items.push(ClassItem::Word);
                                continue;
                            }
                            's' => {
                                items.push(ClassItem::Space);
                                continue;
                            }
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        }
                    } else {
                        c
                    };
                    if self.peek() == Some('-') && self.chars.get(self.i + 1).is_some_and(|&n| n != ']') {
                        self.i += 1;
                        let mut hi = self.peek()?;
                        self.i += 1;
                        if hi == '\\' {
                            hi = self.peek()?;
                            self.i += 1;
                        }
                        items.push(ClassItem::Range(lo, hi));
                    } else {
                        items.push(ClassItem::Range(lo, lo));
                    }
                }
                Node::Class { negated, items }
            }
            other => Node::Char(other),
        })
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// What a piece of a pattern can end on: the last character it takes
/// (`chars: None` is any character), and whether it can take none.
struct Tail {
    chars: Option<Vec<char>>,
    empty: bool,
}

/// A class of more characters than this is not listed out.
const TAIL_MAX: u32 = 128;

fn either(a: Option<Vec<char>>, b: Option<Vec<char>>) -> Option<Vec<char>> {
    let (mut a, b) = (a?, b?);
    a.extend(b);
    Some(a)
}

fn node_tail(node: &Node) -> Tail {
    match node {
        Node::Char(c) => Tail { chars: Some(vec![*c]), empty: false },
        Node::Any | Node::Class { negated: true, .. } => Tail { chars: None, empty: false },
        Node::Class { negated: false, items } => {
            let mut chars = Vec::new();
            for item in items {
                match item {
                    ClassItem::Range(lo, hi) if (*hi as u32).saturating_sub(*lo as u32) < TAIL_MAX => chars.extend(*lo..=*hi),
                    ClassItem::Digit => chars.extend('0'..='9'),
                    _ => return Tail { chars: None, empty: false },
                }
            }
            Tail { chars: Some(chars), empty: false }
        }
        Node::Start | Node::End | Node::WordBoundary => Tail { chars: Some(Vec::new()), empty: true },
        Node::Group(alts) => {
            let mut all = Tail { chars: Some(Vec::new()), empty: false };
            for seq in alts {
                let tail = seq_tail(seq);
                all.chars = either(all.chars, tail.chars);
                all.empty |= tail.empty;
            }
            all
        }
        Node::Repeat { max: Some(0), .. } => Tail { chars: Some(Vec::new()), empty: true },
        Node::Repeat { node, min, .. } => {
            let tail = node_tail(node);
            Tail { chars: tail.chars, empty: tail.empty || *min == 0 }
        }
    }
}

/// From the end backwards: whatever can take nothing lets the piece
/// before it be the one a match ends on.
fn seq_tail(seq: &[Node]) -> Tail {
    let mut chars = Some(Vec::new());
    for node in seq.iter().rev() {
        let tail = node_tail(node);
        chars = either(chars, tail.chars);
        if !tail.empty {
            return Tail { chars, empty: false };
        }
    }
    Tail { chars, empty: true }
}

impl Pattern {
    pub fn new(src: &str) -> Option<Self> {
        let (src, ignore_case) = match src.strip_prefix("(?i)") {
            Some(rest) => (rest, true),
            None => (src, false),
        };
        let mut p = Parser { chars: src.chars().collect(), i: 0, _src: src };
        let alts = p.alternation()?;
        if p.i != p.chars.len() {
            return None;
        }
        Some(Self { alts, ignore_case })
    }

    /// Whether the pattern matches somewhere in `text` (anchors decide
    /// where, as usual).
    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = if self.ignore_case { text.to_lowercase().chars().collect() } else { text.chars().collect() };
        self.found(&chars)
    }

    /// `is_match(text)` for a caller that tries many patterns on one name
    /// and has split it into `chars` once.
    pub(crate) fn is_match_chars(&self, text: &str, chars: &[char]) -> bool {
        if self.ignore_case { self.is_match(text) } else { self.found(chars) }
    }

    fn found(&self, chars: &[char]) -> bool {
        self.alts.iter().any(|seq| {
            let from = |start: usize| self.seq(seq, 0, chars, start, &mut |_| true);
            match seq.first() {
                // `^…` matches from the start or not at all.
                Some(Node::Start) => from(0),
                // `.*…` matching from somewhere matches from the start of
                // that line too, the `.*` taking what lies between. Trying
                // every start made such a pattern, most of the table,
                // quadratic in the length of the name.
                Some(Node::Repeat { node, min: 0, max: None }) if **node == Node::Any => (0..=chars.len()).filter(|&i| i == 0 || chars[i - 1] == '\n').any(from),
                _ => (0..=chars.len()).any(from),
            }
        })
    }

    /// The characters a text has to end on for the pattern to match it,
    /// when the pattern says so: each alternative ends in `$` with
    /// something before it that takes a character. `None` when it may
    /// match a text ending on anything.
    pub(crate) fn last_chars(&self) -> Option<Vec<char>> {
        if self.ignore_case {
            return None;
        }
        let mut all = Vec::new();
        for seq in &self.alts {
            let Some((Node::End, body)) = seq.split_last() else { return None };
            let tail = seq_tail(body);
            if tail.empty {
                return None;
            }
            all.extend(tail.chars?);
        }
        all.sort_unstable();
        all.dedup();
        Some(all)
    }

    fn one(&self, node: &Node, chars: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        match node {
            Node::Char(c) => {
                let want = if self.ignore_case { c.to_lowercase().next().unwrap_or(*c) } else { *c };
                i < chars.len() && chars[i] == want && k(i + 1)
            }
            Node::Any => i < chars.len() && chars[i] != '\n' && k(i + 1),
            Node::Class { negated, items } => {
                if i >= chars.len() {
                    return false;
                }
                let c = chars[i];
                let hit = items.iter().any(|it| match it {
                    ClassItem::Range(lo, hi) => {
                        if self.ignore_case {
                            let (lo, hi) = (lo.to_lowercase().next().unwrap_or(*lo), hi.to_lowercase().next().unwrap_or(*hi));
                            (lo..=hi).contains(&c)
                        } else {
                            (*lo..=*hi).contains(&c)
                        }
                    }
                    ClassItem::Digit => c.is_ascii_digit(),
                    ClassItem::Word => is_word(c),
                    ClassItem::Space => c.is_whitespace(),
                });
                hit != *negated && k(i + 1)
            }
            Node::Start => i == 0 && k(i),
            Node::End => i == chars.len() && k(i),
            Node::WordBoundary => {
                let before = i > 0 && is_word(chars[i - 1]);
                let after = i < chars.len() && is_word(chars[i]);
                before != after && k(i)
            }
            Node::Group(alts) => alts.iter().any(|seq| self.seq(seq, 0, chars, i, k)),
            Node::Repeat { node, min, max } => self.repeat(node, *min, *max, 0, chars, i, k),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(&self, node: &Node, min: usize, max: Option<usize>, count: usize, chars: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        // Greedy: one more if allowed, else what follows.
        if max.is_none_or(|m| count < m) {
            let mut again = |j: usize| j != i && self.repeat(node, min, max, count + 1, chars, j, k);
            if self.one(node, chars, i, &mut again) {
                return true;
            }
        }
        count >= min && k(i)
    }

    fn seq(&self, seq: &[Node], at: usize, chars: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        match seq.get(at) {
            None => k(i),
            Some(node) => self.one(node, chars, i, &mut |j| self.seq(seq, at + 1, chars, j, k)),
        }
    }
}

#[cfg(test)]
impl Pattern {
    /// The plain way, every start tried: what `is_match` has to agree with.
    pub(crate) fn is_match_plain(&self, text: &str) -> bool {
        let chars: Vec<char> = if self.ignore_case { text.to_lowercase().chars().collect() } else { text.chars().collect() };
        (0..=chars.len()).any(|start| self.alts.iter().any(|seq| self.seq(seq, 0, &chars, start, &mut |_| true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, t: &str) -> bool {
        let pattern = Pattern::new(p).unwrap_or_else(|| panic!("parse {p}"));
        let hit = pattern.is_match(t);
        assert_eq!(hit, pattern.is_match_plain(t), "{p} on {t:?}");
        // A pattern that names its endings takes nothing that ends otherwise.
        if let (true, Some(last)) = (hit, pattern.last_chars()) {
            assert!(t.chars().last().is_some_and(|c| last.contains(&c)), "{p} took {t:?}, which ends on none of {last:?}");
        }
        hit
    }

    fn last(p: &str) -> Option<String> {
        Pattern::new(p).unwrap().last_chars().map(|chars| chars.into_iter().collect())
    }

    #[test]
    fn a_leading_dot_star_is_tried_from_each_line_start_only() {
        assert!(m(r".*\.rs$", "a/very/long/path/to/main.rs") && !m(r".*\.rs$", "main.rsx"));
        assert!(m(r".*foo", "xxfoo") && m(r".*foo", "foo") && !m(r".*foo", "fo"));
        // `.` stops at a line break: the match has to start past it.
        assert!(m(r".*a$", "b\na") && m(r".*b$", "a\nb") && !m(r"^.*b$", "b\na"));
        assert!(m(r".*x.*y$", "1\n2x3y") && !m(r".*x.*y$", "1x\n3y"));
        assert!(m(r"^a|.*b$", "cb") && m(r"^a|.*b$", "ac") && !m(r"^a|.*b$", "ca"));
        assert!(m(r".*", "") && m(r".*$", "\n") && m(r"^$", "") && !m(r"^$", "a"));
    }

    #[test]
    fn a_pattern_knows_what_a_name_has_to_end_on() {
        assert_eq!(last(r".*\.rs$").as_deref(), Some("s"));
        assert_eq!(last(r".*\.(3fr|ari|cr[2w])$").as_deref(), Some("2irw"));
        assert_eq!(last(r"^(README|readme)(\.(md|txt))?$").as_deref(), Some("Edet"));
        assert_eq!(last(r".*\.h8(SX?|\d{3})?$").as_deref(), Some("0123456789SX"));
        assert_eq!(last(r"^[\._]?(src|sources?)$").as_deref(), Some("ces"));
        assert_eq!(last(r"^a$|^b$").as_deref(), Some("ab"));
        assert_eq!(last(r"foo\b$").as_deref(), Some("o"));
        assert_eq!(last(r"^ab{0}$").as_deref(), Some("a"));
        // Nothing to go by: no `$`, one hidden in a group, any character
        // last, nothing that has to be taken, case folded.
        for open in [r".*\.muse", r"^(ant\.xml$|\.ant)", r"^a$|b", r"^foo.*$", r"^foo[^.]$", r"^foo\w$", r"a?$", r"^$", r"(?i)^readme$", r""] {
            assert_eq!(last(open), None, "{open}");
        }
    }

    #[test]
    fn matches_the_shapes_the_rules_use() {
        assert!(m(r"^CODE_OF_CONDUCT\.(md|txt)$", "CODE_OF_CONDUCT.md"));
        assert!(!m(r"^CODE_OF_CONDUCT\.(md|txt)$", "CODE_OF_CONDUCT.rst"));
        assert!(m(r".*\.github/.*\.ya?ml$", ".github/workflows/ci.yml"));
        assert!(m(r"(main|workflow|ci|release|build|config)\.ya?ml$", "ci.yaml"));
        assert!(m(r"^[\._]?(addons?|addins)$", "_addons") && m(r"^[\._]?(addons?|addins)$", "addins"));
        assert!(m(r"^babel(\.[\w\-]+)?\.[cm]?[jt]s(on)?$", "babel.config.mjs"));
        assert!(m(r".*\.h8(SX?|\d{3})?$", "a.h8123") && !m(r".*\.h8(SX?|\d{3})?$", "a.h812"));
        assert!(m(r"^[a-zA-Z]{2}(_[a-zA-Z]{2})*\.(json)$", "en_US.json"));
        assert!(m(r"\.rs$", "src/main.rs") && !m(r"\.rs$", "main.rsx"));
        assert!(m(r"^cordova([^.]*\.|-(\d\.)+)[cm]?[jt]s$", "cordova-1.2.js"));
        assert!(m(r"\bfoo\b", "a foo b") && !m(r"\bfoo\b", "afoob"));
        assert!(m(r"(?i)^readme(\.md)?$", "README.MD"));
        assert!(m(r"^a+b$", "aaab") && !m(r"^a+b$", "b"));
        assert!(Pattern::new(r"(unclosed").is_none());
    }
}
