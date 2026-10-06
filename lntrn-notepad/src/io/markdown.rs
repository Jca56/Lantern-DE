//! Markdown in and out, for what a document has that Markdown can say.
//!
//! ```text
//! # A title                  `##` to `####` are the three headings
//! A title                    underlined with `===`; with `---` it is
//! =======                    the first heading
//! > a quote                  its lines are one paragraph
//! - a bullet                 `*` and `+` are bullets too
//!     1. a numbered item     further in than the item over it: a level down
//! - [ ] a box to tick        `- [x]` is one that is
//! **bold** *italic* ~~struck~~ <u>underlined</u> `code`
//! ![](picture.png)           alone on its line: a picture
//! ```
//!
//! - Lines that follow one another are one paragraph, joined by a space.
//!   A blank line parts paragraphs, and so does a line ended by two
//!   spaces or a backslash. Space at a paragraph's ends is not kept.
//! - Code is text in the mono font. A fenced block is a paragraph a line,
//!   each wholly in that font, with nothing in it read as markup.
//! - A picture's address is a `data:` one or a file on this machine.
//!   Nothing is ever fetched.
//! - Bold, italic and struck open and close where CommonMark says they
//!   do, so a file reads here as it does anywhere else.
//! - Whatever else Markdown has (links, tables, HTML, rules, footnotes)
//!   stays the text it was written as, a table's rows and a rule on lines
//!   of their own. A marker that closes nothing is text too: no file is
//!   ever refused.
//!
//! Reading what is inside a paragraph is in `markdown_inline.rs`, and
//! writing in `markdown_write.rs`.

use std::borrow::Cow;
use std::io::Read;
use std::path::Path;

use lntrn_core::encoding::{base64_decode, percent_decode};

use crate::doc::{Align, Doc, Font, List, MAX_LEVEL, Para, ParaAttrs, Pictures, Placed, Style, TextAttrs};

#[path = "markdown_inline.rs"]
mod inline;
#[path = "markdown_write.rs"]
mod write;

/// The family code is in.
const MONO: &str = "JetBrains Mono";
/// The most of a file that is read as a picture.
const MAX_PICTURE: u64 = 64 << 20;

/// Read Markdown. `base` is the folder the file is in, for pictures named
/// by a relative path.
pub fn read(content: &str, base: Option<&Path>) -> Doc {
    // The character a picture stands as means nothing in a file.
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    let content = if content.contains('\u{FFFC}') { Cow::Owned(content.replace('\u{FFFC}', "")) } else { Cow::Borrowed(content) };
    let mut reader = Reader { paras: Vec::new(), pictures: Pictures::default(), open: None, indents: Vec::new(), front: false, base, mono: Font::named(MONO) };
    let mut lines = content.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line));
    let mut first = true;
    while let Some(line) = lines.next() {
        // A file that starts with a rule starts with front matter.
        let fronted = std::mem::take(&mut first) && line.trim_end() == "---";
        let Some((mark, len, cols)) = fence(line) else {
            reader.line(line);
            reader.front |= fronted;
            continue;
        };
        // A fence that is never closed runs to the end of the file.
        reader.close();
        reader.indents.clear();
        for code in lines.by_ref() {
            if trim(code).len() >= len && trim(code).chars().all(|c| c == mark) {
                break;
            }
            let mut para = Para::plain(outdent(code, cols));
            para.spans.apply(0, para.text.len(), |a| a.font = Some(reader.mono));
            reader.paras.push(para);
        }
    }
    reader.close();
    let mut doc = Doc::from_paras(reader.paras);
    doc.pictures = reader.pictures;
    doc
}

/// A document as Markdown. Markdown has no empty paragraph, so the empty
/// ones are not written, but for those between the lines of a block of
/// code.
pub fn write(doc: &Doc) -> String {
    write::document(doc)
}

/// Whether Markdown can hold everything the document has (so the app can
/// warn before saving): no sizes, colours or highlights, no font but the
/// mono one, nothing aligned or spaced, no picture at a width of its own,
/// and no paragraph set in by a tab or spaces, which a Markdown file
/// would not keep.
pub fn holds(doc: &Doc) -> bool {
    let mono = Font::named(MONO);
    let run = |a: &TextAttrs| a.size.is_none() && a.color.is_none() && a.highlight.is_none() && a.font.is_none_or(|font| font == mono);
    let para = |p: &Para| (p.attrs.align, p.attrs.space_before, p.attrs.space_after, p.attrs.first_indent) == (Align::Left, 0.0, 0.0, 0.0) && p.picture.is_none_or(|placed| placed.width == 0.0);
    // Underlined or code, the space a paragraph starts with is kept.
    let edge = |p: &Para| !p.text.starts_with([' ', '\t']) || p.spans.at(0).underline || p.spans.at(0).font == Some(mono);
    doc.paras.iter().all(|p| para(p) && edge(p) && p.spans.list().iter().all(|s| run(&s.attrs)))
}

/// Without the spaces and tabs at its ends.
fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t'])
}

/// How far in a line starts, in columns (a tab goes to the next fourth),
/// and the line from there.
fn indent(line: &str) -> (usize, &str) {
    let mut cols = 0;
    for (i, c) in line.char_indices() {
        match c {
            ' ' => cols += 1,
            '\t' => cols = cols / 4 * 4 + 4,
            _ => return (cols, &line[i..]),
        }
    }
    (cols, "")
}

/// A line of code without the first `cols` columns of the space it
/// starts with: as far in as its fence was.
fn outdent(line: &str, cols: usize) -> &str {
    let mut at = 0;
    for (i, c) in line.char_indices() {
        if at >= cols || !matches!(c, ' ' | '\t') {
            return &line[i..];
        }
        at += if c == '\t' { 4 - at % 4 } else { 1 };
    }
    ""
}

/// A code fence opening on `line`: its character, how many of it, and
/// how far in it is.
fn fence(line: &str) -> Option<(char, usize, usize)> {
    let (cols, rest) = indent(line);
    let mark = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = rest.chars().take_while(|c| *c == mark).count();
    (len >= 3 && !(mark == '`' && rest[len..].contains('`'))).then_some((mark, len, cols))
}

/// Whether a line is a rule: three or more of `-`, `*` or `_`, and
/// nothing else but space.
fn rule(s: &str) -> bool {
    ['-', '*', '_'].into_iter().any(|mark| s.chars().filter(|c| *c == mark).count() >= 3 && s.chars().all(|c| c == mark || c == ' ' || c == '\t'))
}

/// The heading a line makes of the paragraph over it by underlining it:
/// all `=` a title, all `-` the first of the headings, as `#` and `##`
/// are.
fn underline(s: &str) -> Option<Style> {
    let marks = trim(s);
    [('=', Style::Title), ('-', Style::Heading1)].into_iter().find(|(mark, _)| !marks.is_empty() && marks.chars().all(|c| c == *mark)).map(|(_, style)| style)
}

/// A list item's mark at the start of `s`: the kind of item, the number
/// a numbered one has, and the item's text.
fn marker(s: &str) -> Option<(List, u32, &str)> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let (kind, number, rest) = match s.as_bytes().first()? {
        b'-' | b'*' | b'+' => (List::Bullet, 1, &s[1..]),
        _ if (1..=9).contains(&digits) && matches!(s.as_bytes().get(digits), Some(b'.' | b')')) => (List::Number, s[..digits].parse().unwrap_or(1), &s[digits + 1..]),
        _ => return None,
    };
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let rest = trim(rest);
    for (boxed, ticked) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(text) = rest.strip_prefix(boxed).filter(|text| kind == List::Bullet && (text.is_empty() || text.starts_with([' ', '\t']))) {
            return Some((List::Check(ticked), number, trim(text)));
        }
    }
    Some((kind, number, rest))
}

/// A heading's mark at the start of `s`: its style and its text, a run
/// of `#` that closes it taken off.
fn heading(s: &str) -> Option<(Style, &str)> {
    let hashes = s.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) || !(s.len() == hashes || s[hashes..].starts_with([' ', '\t'])) {
        return None;
    }
    let text = trim(&s[hashes..]);
    let open = text.trim_end_matches('#');
    let text = if open.is_empty() || open.ends_with([' ', '\t']) { trim(open) } else { text };
    Some(([Style::Title, Style::Heading1, Style::Heading2, Style::Heading3][hashes.min(4) - 1], text))
}

/// A paragraph being read: its lines so far, joined, and whether a `>`
/// began it.
struct Open {
    attrs: ParaAttrs,
    raw: String,
    quoted: bool,
}

struct Reader<'a> {
    paras: Vec<Para>,
    pictures: Pictures,
    open: Option<Open>,
    /// How far in the items of the list being read start, one for each
    /// level down to the last item's.
    indents: Vec<usize>,
    /// Reading front matter: what is between a first line of `---` and
    /// the next. Its lines are text, and none of them a heading.
    front: bool,
    base: Option<&'a Path>,
    mono: Font,
}

impl Reader<'_> {
    /// Take in a line that is not code: one more of the paragraph being
    /// read, or the start of another.
    fn line(&mut self, line: &str) {
        let (mut cols, mut rest) = indent(line);
        if self.front {
            self.front = !matches!(line.trim_end(), "---" | "...");
        } else if let Some(style) = underline(rest).filter(|_| cols < 4)
            && let Some(open) = self.open.as_mut().filter(|open| open.attrs == ParaAttrs::default())
        {
            // A plain paragraph underlined: the older way to write a
            // heading.
            open.attrs.style = style;
            return self.close();
        }
        let (mut attrs, mut quoted) = (ParaAttrs::default(), false);
        while let Some(after) = rest.strip_prefix('>') {
            (quoted, (cols, rest)) = (true, indent(after));
        }
        // A numbered item breaks into a paragraph only as number one, and
        // an empty item never does: a year that ends a sentence at the
        // start of a line is no list.
        let loose = self.open.is_some() && self.indents.is_empty();
        let item = marker(rest).filter(|(_, number, text)| !rule(rest) && !(loose && (*number != 1 || text.is_empty())));
        if let Some((kind, _, text)) = item {
            (attrs.list, attrs.level, rest) = (kind, self.level(cols), text);
            if let Some(after) = rest.strip_prefix('>').filter(|_| !quoted) {
                (quoted, rest) = (true, trim(after));
            }
        }
        let head = heading(rest);
        let (style, text) = head.unwrap_or((if quoted { Style::Quote } else { Style::Body }, trim(rest)));
        attrs.style = style;
        // An odd backslash at a line's end is a break; an even number of
        // them is backslashes.
        let slash = line.bytes().rev().take_while(|b| *b == b'\\').count() % 2 == 1;
        let hard = head.is_none() && (slash || line.ends_with("  "));
        let text = if hard && slash { trim(text.strip_suffix('\\').unwrap_or(text)) } else { text };
        let plain = item.is_none() && head.is_none();
        if plain && text.is_empty() {
            return self.close();
        }
        // What is never folded into the paragraph before it: a picture, a
        // heading, a rule, a table's row, and a definition where no
        // paragraph is being read.
        let picture = self.picture(text);
        let alone = picture.is_some() || head.is_some() || rule(text) || text.starts_with('|') || (self.open.is_none() && text.starts_with('[') && text.contains("]:"));
        match self.open.as_mut().filter(|open| plain && !alone && (open.quoted || !quoted)) {
            Some(open) => {
                if !open.raw.is_empty() {
                    open.raw.push(' ');
                }
                open.raw.push_str(text);
            }
            None => {
                self.close();
                // A list goes on past a paragraph that is as far in as
                // its items' text: one more paragraph of an item.
                if item.is_none() && self.indents.first().is_none_or(|first| cols < first + 2) {
                    self.indents.clear();
                }
                match picture {
                    Some(id) => self.paras.push(Para { attrs, ..Para::of_picture(Placed { id, width: 0.0 }) }),
                    None => self.open = Some(Open { attrs, raw: text.to_owned(), quoted }),
                }
            }
        }
        if alone || hard {
            self.close();
        }
    }

    /// The level of a list item that starts `cols` in: one deeper than
    /// the item it is under, which is the last one two columns or more
    /// to its left. The first item of a list is at level 0 wherever it
    /// starts.
    fn level(&mut self, cols: usize) -> u8 {
        while self.indents.last().is_some_and(|top| *top > cols) {
            self.indents.pop();
        }
        if self.indents.last().is_none_or(|top| cols >= top + 2) {
            self.indents.push(cols);
        }
        (self.indents.len() - 1).min(usize::from(MAX_LEVEL)) as u8
    }

    /// End the paragraph being read.
    fn close(&mut self) {
        if let Some(open) = self.open.take() {
            let inline::Inline { mut text, mut spans, .. } = inline::read(&open.raw, self.mono);
            // Space that a tag with nothing in it left at an end.
            let (from, to) = inline::kept(&text, &spans, self.mono);
            spans.remove(to, text.len());
            spans.remove(0, from);
            text.replace_range(to.., "");
            text.replace_range(..from, "");
            self.paras.push(Para { text, spans, attrs: open.attrs, picture: None });
        }
    }

    /// The picture a line shows, when the line is one image and nothing
    /// else, and its address is a `data:` one or a file that is here.
    fn picture(&mut self, line: &str) -> Option<u64> {
        let (_, inner) = line.strip_prefix("![")?.strip_suffix(')')?.split_once("](")?;
        let source = inline::target(inner)?;
        let bytes = match source.strip_prefix("data:") {
            Some(data) => data.split_once(";base64,").and_then(|(_, data)| base64_decode(data))?,
            None => {
                let name = source.strip_prefix("file://").unwrap_or(source);
                if name.contains("://") {
                    return None;
                }
                [name.to_owned(), percent_decode(name, false)].iter().find_map(|name| self.file(name))?
            }
        };
        self.pictures.add(bytes).ok()
    }

    /// What is in the file called `name`, when it is a file (not a
    /// device or a pipe, which may never end) of no more than a picture's
    /// size. A relative name is from the document's folder.
    fn file(&self, name: &str) -> Option<Vec<u8>> {
        let path = if Path::new(name).is_absolute() { Path::new(name).to_owned() } else { self.base?.join(name) };
        if !std::fs::metadata(&path).ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path).ok()?.take(MAX_PICTURE + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MAX_PICTURE).then_some(bytes)
    }
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
