//! Writing Markdown: a paragraph a line, behind the marks of what it is,
//! with a blank line between blocks; the items of a list and the rows of
//! a table stay together, and lines of code go in one fence.
//!
//! A paragraph's runs are written as markers that nest: where two
//! stretches overlap, the inner one is closed and opened again around
//! the end of the outer. Markdown cannot say all that a paragraph can
//! have. A marker cannot open before a space or close after one, so bold,
//! italic and struck are not kept on the space at the ends of a stretch.
//! Nor can one open at punctuation in the middle of a word, or close
//! after it there: such a stretch gives up that end of itself, and is
//! written plain if nothing of it is left. What a paragraph is written as
//! is read back before it is kept, so nothing written ever reads as other
//! text than it was.

use lntrn_core::encoding::base64_encode;
use lntrn_image::Format;

use super::inline::{self, BOLD, Gather, ITALIC, Run, STRIKE, UNDER, address, defines, dress, kept, tag};
use super::{MONO, heading, marker, rule};
use crate::doc::{Doc, Font, List, Para, ParaAttrs, Spans, Style};

/// A stretch of a paragraph dressed one way: bytes `start..end` of its
/// text.
struct Seg {
    start: usize,
    end: usize,
    on: [bool; 4],
    code: bool,
    /// Nothing but space, which a marker can neither open before nor
    /// close after.
    space: bool,
}

/// What a paragraph is written as, in order: text and code (bytes of the
/// paragraph's text), and the markers that open and close around them.
#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Text(usize, usize),
    Code(usize, usize),
    Open(usize),
    Close(usize),
}

/// A paragraph as stretches, each all space or with none in it (code is
/// never space: its backticks are what a marker meets). The space at a
/// paragraph's ends that Markdown does not keep is left out.
fn segs(text: &str, spans: &Spans, mono: Font) -> Vec<Seg> {
    let (from, to) = kept(text, spans, mono);
    let mut out: Vec<Seg> = Vec::new();
    for run in spans.runs(from, to) {
        let (on, code) = ([run.attrs.bold, run.attrs.italic, run.attrs.strike, run.attrs.underline], run.attrs.font == Some(mono));
        let mut at = run.start;
        while at < run.end {
            let rest = &text[at..run.end];
            let space = !code && rest.starts_with(char::is_whitespace);
            let len = if code { rest.len() } else { rest.find(|c: char| c.is_whitespace() != space).unwrap_or(rest.len()) };
            // Runs that differ only in what Markdown cannot say are one.
            match out.last_mut() {
                Some(last) if (last.on, last.code, last.space) == (on, code, space) => last.end = at + len,
                _ => out.push(Seg { start: at, end: at + len, on, code, space }),
            }
            at += len;
        }
    }
    out
}

/// How far what is opened at the first of `segs` goes before it has to
/// close: to the last word it is on, with no space between that it is
/// not on (an underline, which is kept on space, to the last stretch).
fn reach(segs: &[Seg], dress: usize) -> usize {
    let on = segs.iter().take_while(|seg| seg.on[dress]);
    on.enumerate().filter(|(_, seg)| dress == UNDER || !seg.space).last().map_or(0, |(i, _)| i + 1)
}

/// The stretches as text between markers that nest. Before each stretch,
/// what it is without is closed, with whatever was opened inside that;
/// then what it has is opened, the longest-lasting outermost.
///
/// Bold, italic and struck change only at the edge of words: where space
/// follows, what does not go on through all of it into the next word is
/// closed before it, and nothing opens until the space is over. An
/// underline is a tag, which space does not trouble, so it changes
/// exactly where it does.
fn tokens(segs: &[Seg]) -> Vec<Tok> {
    let (mut out, mut open): (Vec<Tok>, Vec<usize>) = (Vec::new(), Vec::new());
    for (i, seg) in segs.iter().enumerate() {
        let ahead = &segs[i..];
        let through = |dress: usize| ahead.iter().take_while(|s| s.space).all(|s| s.on[dress]) && ahead.iter().find(|s| !s.space).is_some_and(|s| s.on[dress]);
        // An underline that ends somewhere in the space ahead takes what
        // is open inside it along, and that can only close here.
        let mut doomed = false;
        let keep = open.iter().position(|&dress| match (seg.space, dress) {
            (false, _) => !seg.on[dress],
            (true, UNDER) => {
                doomed |= !through(UNDER);
                !seg.on[UNDER]
            }
            (true, _) => doomed || !through(dress),
        });
        out.extend(open.drain(keep.unwrap_or(open.len())..).rev().map(Tok::Close));
        // Of those that last as long, the tag goes outermost and the stars
        // innermost: a star reads as a marker only next to the text, or
        // to another star.
        let mut new: Vec<usize> = [UNDER, STRIKE, BOLD, ITALIC].into_iter().filter(|dress| seg.on[*dress] && !open.contains(dress) && (*dress == UNDER || !seg.space)).collect();
        new.sort_by_key(|dress| std::cmp::Reverse(reach(ahead, *dress)));
        out.extend(new.iter().map(|dress| Tok::Open(*dress)));
        open.extend(new);
        match (out.last_mut(), seg.code) {
            (_, true) => out.push(Tok::Code(seg.start, seg.end)),
            (Some(Tok::Text(_, end)), false) => *end = seg.end,
            _ => out.push(Tok::Text(seg.start, seg.end)),
        }
    }
    out.extend(open.into_iter().rev().map(Tok::Close));
    out
}

/// Put text in with a backslash before whatever would read as markup,
/// and before nothing else: a `*` with space on both sides is a `*`, and
/// an underscore inside a word an underscore. `before` and `after` say
/// whether a marker is right beside the text.
fn escape(out: &mut String, text: &str, before: bool, after: bool) {
    let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    // How far the text is an address, written as it is; and how far the
    // run of markers being written goes, and whether it is escaped.
    let (mut raw, mut run) = (0, (0, false));
    for (i, c) in text.char_indices() {
        let next = text[i + c.len_utf8()..].chars().next();
        if i >= run.0 && matches!(c, '*' | '_' | '~') {
            let end = text.len() - text[i..].trim_start_matches(c).len();
            let (left, right) = (text[..i].chars().next_back(), text[end..].chars().next());
            let lone = if c == '~' { end - i == 1 } else { (space(left) && space(right)) || (c == '_' && word(left) && word(right)) };
            run = (end, !lone || (before && i == 0) || (after && end == text.len()));
        }
        let mark = i >= raw
            && match c {
                '`' => true,
                '\\' => next.is_none_or(|next| next.is_ascii_punctuation()),
                '<' => tag(&text[i..]).is_some(),
                // A link's address is written as it is, for nothing in it
                // is read as markup: a definition's to the end, when
                // there is nothing but text to the end, and a link's to
                // its `)`. A `](` that starts none must not be read as
                // starting one.
                ']' if !before && defines(text, i) => {
                    raw = if after { raw } else { text.len() };
                    after
                }
                ']' if next == Some('(') => {
                    raw = address(&text[i + 2..]).map_or(raw, |len| i + len + 3);
                    raw <= i
                }
                '*' | '_' | '~' => run.1,
                _ => false,
            };
        if mark {
            out.push('\\');
        }
        out.push(c);
    }
}

/// Put code in, between as many backticks as it takes to have more than
/// any run of them in it, with a space to keep a backtick of its own off
/// them (and one to match a space at both its ends, which would be taken
/// for that).
fn code(out: &mut String, text: &str) {
    let ticks = "`".repeat(text.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1);
    let pad = text.starts_with('`') || text.ends_with('`') || (text.starts_with(' ') && text.ends_with(' ') && !text.trim_matches(' ').is_empty());
    let pad = if pad { " " } else { "" };
    out.push_str(&[ticks.as_str(), pad, text, pad, ticks.as_str()].concat());
}

/// The tokens as Markdown, and where in it each one starts. Italic is
/// `*`; with `under` it is `_`, which keeps it from running together
/// with the `**` of bold, wherever no letter is right outside it (or
/// outside the bold it is right inside, whose stars a letter and an
/// underscore would leave unreadable between them).
fn render(text: &str, toks: &[Tok], under: bool) -> (String, Vec<usize>) {
    let letter = |tok: Option<&Tok>, last: bool| {
        let Some(Tok::Text(a, b)) = tok else { return false };
        (if last { text[*a..*b].chars().next_back() } else { text[*a..*b].chars().next() }).is_some_and(char::is_alphanumeric)
    };
    let (mut out, mut at, mut star) = (String::new(), Vec::with_capacity(toks.len()), true);
    for (k, tok) in toks.iter().enumerate() {
        at.push(out.len());
        match *tok {
            Tok::Text(a, b) => escape(&mut out, &text[a..b], k > 0, k + 1 < toks.len()),
            Tok::Code(a, b) => code(&mut out, &text[a..b]),
            Tok::Open(ITALIC) => {
                let close = toks[k..].iter().position(|t| *t == Tok::Close(ITALIC)).map_or(k, |n| k + n);
                let (before, after) = (toks[..k].iter().rev().find(|t| **t != Tok::Open(BOLD)), toks[close + 1..].iter().find(|t| **t != Tok::Close(BOLD)));
                star = !under || letter(before, true) || letter(after, false);
                out.push(if star { '*' } else { '_' });
            }
            Tok::Close(ITALIC) => out.push(if star { '*' } else { '_' }),
            Tok::Open(dress) => out.push_str(["**", "", "~~", "<u>"][dress]),
            Tok::Close(dress) => out.push_str(["**", "", "~~", "</u>"][dress]),
        }
    }
    (out, at)
}

/// What the tokens say: the text, and how it is dressed.
fn meaning(text: &str, toks: &[Tok], mono: Font) -> (String, Spans) {
    let (mut out, mut spans, mut on) = (String::new(), Gather::default(), [false; 4]);
    for tok in toks {
        match *tok {
            Tok::Open(dress) => on[dress] = true,
            Tok::Close(dress) => on[dress] = false,
            Tok::Text(a, b) | Tok::Code(a, b) => {
                out.push_str(&text[a..b]);
                spans.add(out.len() - (b - a), out.len(), dress(on, matches!(tok, Tok::Code(..)).then_some(mono)));
            }
        }
    }
    (out, spans.spans())
}

/// A paragraph's text and runs as Markdown. It is read back, and until
/// it reads as what was meant, what the markers that went wrong were on
/// is written plain and the rest nested afresh.
fn written(text: &str, spans: &Spans, mono: Font) -> String {
    // Runs that are not on the edges of characters cannot be believed:
    // the text goes without them rather than not at all.
    let sound = spans.list().iter().all(|s| s.start <= s.end && text.is_char_boundary(s.start.min(text.len())) && text.is_char_boundary(s.end.min(text.len())));
    let (mut spans, mut under) = (if sound { spans.clone() } else { Spans::default() }, false);
    loop {
        let toks = tokens(&segs(text, &spans, mono));
        let (raw, at) = render(text, &toks, under);
        let (read, meant) = (inline::read(&raw, mono), meaning(text, &toks, mono));
        if (&read.text, &read.spans) == (&meant.0, &meant.1) {
            return raw;
        }
        // Italic first tries the other character it has.
        if !under {
            under = true;
            continue;
        }
        // Every marker: where it is, what it dresses, whether it opens,
        // and the run of markers the reader found it in.
        let marker = |(k, tok): (usize, &Tok)| match *tok {
            Tok::Open(dress) => Some((k, dress, true)),
            Tok::Close(dress) => Some((k, dress, false)),
            _ => None,
        };
        let marks: Vec<(usize, usize, bool, Option<&Run>)> = toks.iter().enumerate().filter_map(marker).map(|(k, dress, opens)| (k, dress, opens, read.runs.iter().find(|run| (run.at..run.at + run.n * run.unit).contains(&at[k])))).collect();
        // The ones that went wrong are those that cannot open or close
        // where they are: a stretch that starts at a bracket in the
        // middle of a word, say. Each gives up the end of its stretch
        // that is in its way, a word or the punctuation before one at a
        // time. Failing any, markers paired other than as they were meant
        // to, and whole stretches go: the first star or tilde left over,
        // or all of them, or at the last the tags.
        let blocked: Vec<_> = marks.iter().filter(|(_, _, opens, run)| run.is_none_or(|run| if *opens { !run.open } else { !run.close })).collect();
        let stray = marks.iter().filter(|(_, dress, _, run)| *dress != UNDER && run.is_some_and(|run| run.left > 0)).take(1);
        let stars = marks.iter().filter(|(_, dress, ..)| *dress != UNDER);
        let whole = blocked.is_empty();
        let wrong = [blocked, stray.collect(), stars.collect(), marks.iter().collect()].into_iter().find(|wrong| !wrong.is_empty()).unwrap_or_default();
        let was = spans.clone();
        for &&(k, dress, opens, _) in &wrong {
            // From the marker to its other end, and the text between.
            let pair = if opens { &toks[k..k + toks[k..].iter().position(|t| *t == Tok::Close(dress)).unwrap_or(0)] } else { &toks[toks[..k].iter().rposition(|t| *t == Tok::Open(dress)).unwrap_or(k)..k] };
            let texts: Vec<(usize, usize, bool)> = pair.iter().filter_map(|t| if let Tok::Text(a, b) | Tok::Code(a, b) = t { Some((*a, *b, matches!(t, Tok::Code(..)))) } else { None }).collect();
            let (Some(&(from, first, code)), Some(&(last, to, coded))) = (texts.first(), texts.last()) else { continue };
            // The word, or what is not one, at an end of some text; code
            // goes as one.
            let word = |c: char| c.is_alphanumeric();
            let head = |s: &str| s.find(|c| Some(word(c)) != s.chars().next().map(word)).unwrap_or(s.len());
            let tail = |s: &str| s.rfind(|c| Some(word(c)) != s.chars().next_back().map(word)).map_or(0, |i| i + s[i..].chars().next().map_or(1, char::len_utf8));
            let (start, end) = match (whole, opens) {
                (true, _) => (from, to),
                (false, true) => (from, if code { first } else { from + head(&text[from..first]) }),
                (false, false) => (if coded { last } else { last + tail(&text[last..to]) }, to),
            };
            spans.apply(start, end, |a| *[&mut a.bold, &mut a.italic, &mut a.strike, &mut a.underline][dress] = false);
        }
        // With nothing left to write plain, this is as near as it gets.
        if spans == was {
            return raw;
        }
        under = false;
    }
}

/// A paragraph's Markdown with a backslash before what would be read as
/// the mark of a block and not as its text. `mark` is the list item's
/// mark the line has before it.
fn guard(mut raw: String, mark: &str, attrs: &ParaAttrs) -> String {
    let digits = raw.bytes().take_while(u8::is_ascii_digit).count();
    let hashes = raw.len() - raw.trim_end_matches('#').len();
    let at = match marker(&raw) {
        // A heading's text can start with anything, but `#`s at its end
        // would be taken for the ones that close a heading.
        _ if !matches!(attrs.style, Style::Body | Style::Quote) => (hashes > 0 && (hashes == raw.len() || raw[..raw.len() - hashes].ends_with([' ', '\t']))).then_some(raw.len() - hashes),
        // A rule reads as the text it is, unless an item's mark before it
        // would be read as part of it.
        _ if mark.is_empty() && rule(&raw) => None,
        _ if rule(&[mark, raw.as_str()].concat()) => Some(0),
        Some((List::Number, ..)) => Some(digits),
        Some(_) => Some(0),
        None if raw.starts_with('>') || heading(&raw).is_some() => Some(0),
        // A bullet whose text starts as a box does would be read as one.
        None if mark == "- " && marker(&["- ", raw.as_str()].concat()).is_some_and(|(kind, ..)| kind != List::Bullet) => Some(0),
        None => None,
    };
    if let Some(at) = at {
        raw.insert(at, '\\');
    }
    raw
}

/// How deep a list item is written: one level under the item it is
/// nested in, however much deeper the document has it, and the first
/// item of a list at the edge. Markdown nests an item no other way.
/// `over` is the items this one could be under: their levels, and how
/// deep each was written.
fn depth(over: &mut Vec<(u8, usize)>, level: u8) -> usize {
    while over.last().is_some_and(|top| top.0 > level) {
        over.pop();
    }
    match over.last() {
        Some(top) if top.0 == level => top.1,
        top => {
            let depth = top.map_or(0, |top| top.1 + 1);
            over.push((level, depth));
            depth
        }
    }
}

/// What was written last, for what goes between it and the next block.
#[derive(PartialEq)]
enum After {
    Start,
    Item,
    Row,
    Other,
}

pub(super) fn document(doc: &Doc) -> String {
    let mono = Font::named(MONO);
    // A line of code: a plain paragraph wholly in the mono font and
    // nothing else.
    let code = |i: usize| {
        let p = doc.para(i);
        (p.attrs.style, p.attrs.list, p.picture) == (Style::Body, List::None, None) && !p.text.is_empty() && p.spans.runs(0, p.text.len()).iter().all(|run| run.attrs == dress([false; 4], Some(mono)))
    };
    let (mut out, mut after, mut over, mut i) = (String::new(), After::Start, Vec::new(), 0);
    while i < doc.paras.len() {
        let p = doc.para(i);
        let mark = match p.attrs.list {
            List::None => String::new(),
            List::Bullet => "- ".to_owned(),
            List::Number => format!("{}. ", doc.number(i)),
            List::Check(ticked) => format!("- [{}] ", if ticked { 'x' } else { ' ' }),
        };
        let block = if code(i) {
            // One fence holds the lines of code that follow one another,
            // and the empty paragraphs between them.
            let last = (i..doc.paras.len()).take_while(|k| code(*k) || *doc.para(*k) == Para::default()).filter(|k| code(*k)).last().unwrap_or(i);
            let lines: Vec<&str> = (i..=last).map(|k| doc.para(k).text.as_str()).collect();
            let ticks = "`".repeat(lines.iter().flat_map(|line| line.split(|c| c != '`')).map(str::len).max().unwrap_or(0).max(2) + 1);
            i = last;
            Some((After::Other, format!("{ticks}\n{}\n{ticks}", lines.join("\n"))))
        } else {
            // A picture goes in whole. One the document does not have is
            // not written, like a paragraph with nothing in it.
            let body = match p.picture {
                Some(placed) => doc.pictures.get(placed.id).map(|picture| format!("![](data:{};base64,{})", Format::sniff(&picture.bytes).map_or("image/png", Format::mime), base64_encode(&picture.bytes))),
                None => Some(written(&p.text, &p.spans, mono)).filter(|raw| !raw.is_empty()).map(|raw| guard(raw, &mark, &p.attrs)),
            };
            body.map(|body| {
                let (quote, head) = match p.attrs.style {
                    Style::Body => ("", ""),
                    Style::Quote => ("> ", ""),
                    Style::Title => ("", "# "),
                    Style::Heading1 => ("", "## "),
                    Style::Heading2 => ("", "### "),
                    Style::Heading3 => ("", "#### "),
                };
                // Four spaces a level, which every Markdown reader takes
                // for an item under the one before, whatever its mark.
                let line = [quote, &" ".repeat(if mark.is_empty() { 0 } else { 4 * depth(&mut over, p.attrs.level) }), &mark, head, &body].concat();
                (if !mark.is_empty() { After::Item } else if line.starts_with('|') { After::Row } else { After::Other }, line)
            })
        };
        match block {
            Some((kind, text)) => {
                out.push_str(if after == After::Start { "" } else if kind == after && kind != After::Other { "\n" } else { "\n\n" });
                out.push_str(&text);
                if kind != After::Item {
                    over.clear();
                }
                after = kind;
            }
            // An item that is not written leaves its list in one piece;
            // anything else parts what is around it.
            None if p.attrs.list == List::None && after != After::Start => after = After::Other,
            None => {}
        }
        i += 1;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}
