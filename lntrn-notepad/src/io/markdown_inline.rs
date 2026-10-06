//! What is inside a paragraph of Markdown, read: the characters that are
//! text as they stand (an escaped one, code, a link's address), and the
//! runs of markers around the rest, paired as CommonMark pairs them. A
//! marker that pairs with nothing is text too.

use std::collections::HashMap;

use super::trim;
use crate::doc::{Font, Spans, TextAttrs};

/// What markers dress a run in, in the order they are counted.
pub(super) const BOLD: usize = 0;
pub(super) const ITALIC: usize = 1;
pub(super) const STRIKE: usize = 2;
pub(super) const UNDER: usize = 3;

/// What a paragraph's Markdown reads as: its text, how that is dressed,
/// and the runs of markers that were in it.
pub(super) struct Inline {
    pub text: String,
    pub spans: Spans,
    pub runs: Vec<Run>,
}

/// A run of one marker (`*`s, `_`s, a `~~`, a `<u>` or a `</u>`) and
/// what became of it.
#[derive(Clone, Copy)]
pub(super) struct Run {
    /// Where in the Markdown it starts.
    pub at: usize,
    /// The marker's character; `<` for a tag.
    ch: u8,
    /// Bytes to a marker, how many markers it had, and how many are
    /// still unpaired.
    pub unit: usize,
    pub n: usize,
    pub left: usize,
    /// Whether it is where a marker can open a stretch, and close one.
    pub open: bool,
    pub close: bool,
    /// How many stretches of each dress start after it and end before
    /// it.
    opens: [usize; 4],
    closes: [usize; 4],
}

enum Piece {
    Text { text: String, mono: bool },
    Run(Run),
}

/// A paragraph's runs, gathered in order into its spans. `Spans` puts
/// itself in order each time it is added to, so the runs are joined in
/// heaps that double: a paragraph of thousands of them is still quick.
#[derive(Default)]
pub(super) struct Gather(Vec<Spans>);

impl Gather {
    pub fn add(&mut self, start: usize, end: usize, attrs: TextAttrs) {
        if attrs.is_default() || start >= end {
            return;
        }
        let mut run = Spans::default();
        run.apply(start, end, |a| *a = attrs);
        self.0.push(run);
        while let [.., below, top] = &self.0[..] && below.list().len() <= top.list().len() {
            self.join();
        }
    }

    fn join(&mut self) {
        if let (Some(top), Some(below)) = (self.0.pop(), self.0.last_mut()) {
            below.append(top, 0);
        }
    }

    pub fn spans(mut self) -> Spans {
        while self.0.len() > 1 {
            self.join();
        }
        self.0.pop().unwrap_or_default()
    }
}

/// The part of a paragraph's text that Markdown keeps, as bytes from
/// and to: all but the space at its ends, unless that is underlined or
/// code, which a tag or backticks hold.
pub(super) fn kept(text: &str, spans: &Spans, mono: Font) -> (usize, usize) {
    let held = |(i, c): &(usize, char)| !matches!(c, ' ' | '\t') || spans.at(*i).underline || spans.at(*i).font == Some(mono);
    let from = text.char_indices().find(held).map_or(text.len(), |(i, _)| i);
    (from, text.char_indices().rfind(held).map_or(from, |(i, c)| i + c.len_utf8()))
}

/// What a run has, as text attributes.
pub(super) fn dress(on: [bool; 4], font: Option<Font>) -> TextAttrs {
    TextAttrs { bold: on[BOLD], italic: on[ITALIC], strike: on[STRIKE], underline: on[UNDER], font, ..TextAttrs::default() }
}

/// An underline's tag at the start of `s`: how long it is, and whether
/// it is the closing one.
pub(super) fn tag(s: &str) -> Option<(usize, bool)> {
    [("<u>", false), ("</u>", true)].into_iter().find(|(tag, _)| s.get(..tag.len()).is_some_and(|head| head.eq_ignore_ascii_case(tag))).map(|(tag, close)| (tag.len(), close))
}

/// The address in what an image has between its brackets or a
/// definition after its colon: in angle brackets or with no space in it,
/// and after it a title in quotes or brackets, or nothing.
pub(super) fn target(s: &str) -> Option<&str> {
    let (address, title) = match trim(s).strip_prefix('<') {
        Some(angled) => angled.split_once('>')?,
        None => trim(s).split_once([' ', '\t']).unwrap_or((trim(s), "")),
    };
    let title = trim(title);
    (title.is_empty() || (title.len() >= 2 && title.starts_with(['"', '\'', '(']) && title.ends_with(['"', '\'', ')']))).then_some(address)
}

/// Whether a paragraph's Markdown is a link's definition, `[name]:` and
/// an address, with the `]` at byte `at`. Its address is not read for
/// markup either.
pub(super) fn defines(raw: &str, at: usize) -> bool {
    raw.starts_with('[') && raw.find(']') == Some(at) && raw[at + 1..].starts_with(':') && target(&raw[at + 2..]).is_some_and(|address| !address.is_empty())
}

/// How long a link's address is: `s`, which follows `](`, up to the `)`
/// that closes it. Nothing in an address is markup.
pub(super) fn address(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' if depth == 32 => return None,
            b'(' => depth += 1,
            b')' if depth == 0 => return Some(i),
            b')' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Whether a run of markers with these characters on either side of it
/// can open a stretch, and close one: CommonMark's flanking rules. The
/// ends of the text count as space, and whatever is neither space nor a
/// letter or digit as punctuation.
fn flanks(ch: u8, before: Option<char>, after: Option<char>) -> (bool, bool) {
    let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
    let punct = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace() && !c.is_alphanumeric());
    let left = !space(after) && (!punct(after) || space(before) || punct(before));
    let right = !space(before) && (!punct(before) || space(after) || punct(after));
    // An underscore inside a word is an underscore.
    if ch == b'_' { (left && (!right || punct(before)), right && (!left || punct(after))) } else { (left, right) }
}

/// A paragraph's Markdown as text and runs of markers. Escapes, code and
/// links' addresses are settled here, since nothing in them is a marker.
fn pieces(raw: &str) -> Vec<Piece> {
    fn text(out: &mut Vec<Piece>, s: &str) {
        match out.last_mut() {
            Some(Piece::Text { text, mono: false }) => text.push_str(s),
            _ => out.push(Piece::Text { text: s.to_owned(), mono: false }),
        }
    }
    // Where the last run of each length of backticks starts, so that code
    // with no end is known for it without the paragraph being searched.
    let (mut last, mut at) = (HashMap::new(), 0);
    while let Some(start) = raw[at..].find('`').map(|found| at + found) {
        at = start + raw[start..].bytes().take_while(|b| *b == b'`').count();
        last.insert(at - start, start);
    }
    let (mut out, mut i) = (Vec::new(), 0);
    while i < raw.len() {
        let rest = &raw[i..];
        let plain = rest.bytes().position(|b| b"\\`]<*_~".contains(&b)).unwrap_or(rest.len());
        // How long the run of markers here is.
        let same = if b"`*_~".contains(&rest.as_bytes()[0]) { rest.bytes().take_while(|b| *b == rest.as_bytes()[0]).count() } else { 1 };
        let run = move |ch: u8, unit: usize, n: usize, (open, close): (bool, bool)| Piece::Run(Run { at: i, ch, unit, n, left: n, open, close, opens: [0; 4], closes: [0; 4] });
        i += match rest.as_bytes()[0] {
            _ if plain > 0 => {
                text(&mut out, &rest[..plain]);
                plain
            }
            b'\\' if rest.as_bytes().get(1).is_some_and(u8::is_ascii_punctuation) => {
                text(&mut out, &rest[1..2]);
                2
            }
            // Code runs to the next run of as many backticks; a space at
            // each end of it, there to keep a backtick off the fence, is
            // not part of it.
            b'`' => match last.get(&same).filter(|last| **last >= i + same).and_then(|_| rest[same..].match_indices('`').find(|(at, _)| !rest[same..at + same].ends_with('`') && rest[same + at..].bytes().take_while(|b| *b == b'`').count() == same)) {
                Some((len, _)) => {
                    let code = &rest[same..same + len];
                    let padded = code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.trim_matches(' ').is_empty();
                    out.push(Piece::Text { text: if padded { &code[1..len - 1] } else { code }.to_owned(), mono: true });
                    len + 2 * same
                }
                None => {
                    text(&mut out, &rest[..same]);
                    same
                }
            },
            b']' if defines(raw, i) => {
                text(&mut out, rest);
                rest.len()
            }
            b']' if rest[1..].starts_with('(') => {
                let len = address(&rest[2..]).map_or(1, |len| len + 3);
                text(&mut out, &rest[..len]);
                len
            }
            b'<' => match tag(rest) {
                Some((len, close)) => {
                    out.push(run(b'<', len, 1, (!close, close)));
                    len
                }
                None => {
                    text(&mut out, "<");
                    1
                }
            },
            b'~' if same != 2 => {
                text(&mut out, &rest[..same]);
                same
            }
            ch @ (b'*' | b'_' | b'~') => {
                let (unit, n) = if ch == b'~' { (2, 1) } else { (1, same) };
                out.push(run(ch, unit, n, flanks(ch, raw[..i].chars().next_back(), rest[same..].chars().next())));
                same
            }
            _ => {
                text(&mut out, &rest[..1]);
                1
            }
        };
    }
    out
}

/// Pair the runs that open with the ones that close them, as CommonMark
/// does: a closer takes the nearest opener of its kind before it, two
/// markers of each for bold when both have two, else one for italic,
/// and whatever is between the pair can pair with nothing after.
fn pair(pieces: &mut [Piece]) {
    let mut open: Vec<usize> = Vec::new();
    // For each kind of closer, how much of `open` is known to have no
    // opener for it, so that no stretch of markers is searched twice.
    let mut floor = [0; 24];
    for at in 0..pieces.len() {
        let Piece::Run(mut closer) = pieces[at] else { continue };
        let kind = b"*_~<".iter().position(|c| *c == closer.ch).unwrap_or(0);
        while closer.close && closer.left > 0 {
            let key = kind * 6 + usize::from(closer.open) * 3 + closer.n % 3;
            // CommonMark's rule of three: when either run could do both,
            // lengths that add up to a multiple of three do not pair,
            // unless each is one itself. It is what reads `**a*b*c**`
            // right.
            let pairs = |o: &Run| o.ch == closer.ch && !((o.close || closer.open) && (o.n + closer.n).is_multiple_of(3) && !(o.n.is_multiple_of(3) && closer.n.is_multiple_of(3)));
            let Some(s) = (floor[key]..open.len()).rev().find(|s| matches!(&pieces[open[*s]], Piece::Run(o) if pairs(o))) else {
                floor[key] = open.len();
                break;
            };
            let Piece::Run(opener) = &mut pieces[open[s]] else { break };
            let (used, dress) = match closer.ch {
                b'*' | b'_' if opener.left >= 2 && closer.left >= 2 => (2, BOLD),
                b'*' | b'_' => (1, ITALIC),
                b'~' => (1, STRIKE),
                _ => (1, UNDER),
            };
            (opener.left, closer.left) = (opener.left - used, closer.left - used);
            (opener.opens[dress], closer.closes[dress]) = (opener.opens[dress] + 1, closer.closes[dress] + 1);
            open.truncate(if opener.left == 0 { s } else { s + 1 });
            floor.iter_mut().for_each(|f| *f = (*f).min(open.len()));
        }
        pieces[at] = Piece::Run(closer);
        if closer.open && closer.left > 0 {
            open.push(at);
        }
    }
}

/// Read a paragraph's Markdown.
pub(super) fn read(raw: &str, mono: Font) -> Inline {
    let mut pieces = pieces(raw);
    pair(&mut pieces);
    let (mut out, mut spans, mut runs, mut on) = (String::with_capacity(raw.len()), Gather::default(), Vec::new(), [0usize; 4]);
    for piece in &pieces {
        // The markers of a run that paired with nothing are text between
        // what it closes and what it opens.
        let (text, code, opens) = match piece {
            Piece::Text { text, mono } => (text.as_str(), *mono, [0; 4]),
            Piece::Run(run) => {
                on.iter_mut().zip(run.closes).for_each(|(on, closes)| *on -= closes);
                runs.push(*run);
                (&raw[run.at..run.at + run.left * run.unit], false, run.opens)
            }
        };
        out.push_str(text);
        spans.add(out.len() - text.len(), out.len(), dress(on.map(|n| n > 0), code.then_some(mono)));
        on.iter_mut().zip(opens).for_each(|(on, opens)| *on += opens);
    }
    Inline { text: out, spans: spans.spans(), runs }
}
