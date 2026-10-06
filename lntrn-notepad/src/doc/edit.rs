//! Changing a document: text typed and pasted in, ranges cut out,
//! paragraphs parted and joined, runs and paragraphs dressed. Every
//! change says where the caret belongs afterwards.
//!
//! A picture's paragraph is never glued to text: what is typed at one
//! goes into a paragraph of its own beside it, and a cut that would join
//! one to text leaves the two apart.

use std::sync::Arc;

use super::{Doc, Para, ParaAttrs, Pos, TextAttrs};

fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b { (a, b) } else { (b, a) }
}

impl Doc {
    /// Where text meant for `at` goes: there, unless that is a picture's
    /// paragraph, when an empty one is made for it before the picture or
    /// after.
    fn beside_picture(&mut self, at: Pos) -> Pos {
        if !self.para(at.para).is_picture() {
            return at;
        }
        let i = if at.byte == 0 { at.para } else { at.para + 1 };
        self.paras.insert(i, Arc::new(Para::default()));
        Pos::new(i, 0)
    }

    /// What typing at `at` should look like: as the character before
    /// it, or the one after at the start of a paragraph.
    pub fn typing_attrs(&self, at: Pos) -> TextAttrs {
        let p = self.para(at.para);
        match p.text[..at.byte.min(p.text.len())].char_indices().next_back() {
            Some((before, _)) => p.spans.at(before),
            None => p.spans.at(0),
        }
    }

    /// Put `text` in at `at`, dressed in `attrs`. Each newline in it
    /// starts a paragraph like the one it is in. Returns the place after
    /// it.
    pub fn insert_text(&mut self, at: Pos, text: &str, attrs: TextAttrs) -> Pos {
        if text.is_empty() {
            return at;
        }
        let at = self.beside_picture(at);
        let mut lines = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line));
        let first = lines.next().unwrap_or("");
        let rest: Vec<&str> = lines.collect();
        let p = self.para_mut(at.para);
        if rest.is_empty() {
            p.text.insert_str(at.byte, first);
            p.spans.insert(at.byte, first.len(), attrs);
            return Pos::new(at.para, at.byte + first.len());
        }
        // What was after the caret ends up after the last line.
        let (tail_text, tail_spans) = (p.text.split_off(at.byte), p.spans.split_off(at.byte));
        p.text.push_str(first);
        p.spans.insert(at.byte, first.len(), attrs);
        let para_attrs = ParaAttrs { list: p.attrs.list.next(), ..p.attrs };
        let mut new: Vec<Para> = rest.iter().map(|line| Para { text: (*line).to_owned(), attrs: para_attrs, ..Para::default() }).collect();
        for para in &mut new {
            para.spans.insert(0, para.text.len(), attrs);
        }
        let end = new.last().map_or(0, |last| last.text.len());
        if let Some(last) = new.last_mut() {
            last.text.push_str(&tail_text);
            last.spans.append(tail_spans, end);
        }
        let n = new.len();
        self.paras.splice(at.para + 1..at.para + 1, new.into_iter().map(Arc::new));
        Pos::new(at.para + n, end)
    }

    /// Part the paragraph at `at` in two, both as it was.
    fn part(&mut self, at: Pos) {
        let p = self.para_mut(at.para);
        let after = Para { text: p.text.split_off(at.byte), spans: p.spans.split_off(at.byte), attrs: p.attrs, picture: None };
        self.paras.insert(at.para + 1, Arc::new(after));
    }

    /// Enter at `at`: a new paragraph from there on. A list goes on with
    /// its next item; a heading ended is followed by body text. Returns
    /// the start of the new paragraph.
    pub fn split(&mut self, at: Pos) -> Pos {
        if self.para(at.para).is_picture() {
            // An empty line over the picture, or under it.
            self.paras.insert(if at.byte == 0 { at.para } else { at.para + 1 }, Arc::new(Para::default()));
            return Pos::new(at.para + 1, 0);
        }
        let at_end = at.byte == self.para(at.para).text.len();
        self.part(at);
        let next = self.para_mut(at.para + 1);
        next.attrs.list = next.attrs.list.next();
        if at_end {
            next.attrs.style = next.attrs.style.next();
        }
        Pos::new(at.para + 1, 0)
    }

    /// Take out what is between two places. Returns where it was.
    pub fn delete(&mut self, a: Pos, b: Pos) -> Pos {
        let (a, b) = ordered(a, b);
        if a == b {
            return a;
        }
        if a.para == b.para {
            let p = self.para_mut(a.para);
            p.text.replace_range(a.byte..b.byte, "");
            p.spans.remove(a.byte, b.byte);
            p.settle();
            return a;
        }
        // What is left of the last paragraph, and of the first.
        let mut tail = self.para(b.para).clone();
        tail.text.replace_range(..b.byte, "");
        tail.spans.remove(0, b.byte);
        tail.settle();
        let head = self.para_mut(a.para);
        let whole = head.text.len();
        head.text.truncate(a.byte);
        head.spans.remove(a.byte, whole);
        head.settle();
        if head.is_picture() || tail.is_picture() {
            // A picture stays a paragraph of its own. An end with nothing
            // left of it goes, so long as the other is still there.
            let (head_empty, tail_empty) = (head.text.is_empty(), tail.text.is_empty());
            *self.para_mut(b.para) = tail;
            let from = if head_empty { a.para } else { a.para + 1 };
            let to = if tail_empty && !head_empty { b.para + 1 } else { b.para };
            self.paras.drain(from..to);
            return if head_empty { Pos::new(a.para, 0) } else { a };
        }
        // They join: as the first was, unless nothing of it is left.
        if a.byte == 0 {
            head.attrs = tail.attrs;
        }
        head.text.push_str(&tail.text);
        head.spans.append(tail.spans, a.byte);
        self.paras.drain(a.para + 1..=b.para);
        a
    }

    /// What is between two places, as paragraphs: the first and last of
    /// them the parts that are inside.
    pub fn slice(&self, a: Pos, b: Pos) -> Vec<Para> {
        let (a, b) = ordered(a, b);
        let part = |i: usize| {
            let mut p = self.para(i).clone();
            let (from, to, len) = (if i == a.para { a.byte } else { 0 }, if i == b.para { b.byte } else { p.text.len() }, p.text.len());
            p.text.truncate(to);
            p.spans.remove(to, len);
            p.text.replace_range(..from, "");
            p.spans.remove(0, from);
            p.settle();
            p
        };
        (a.para..=b.para).map(part).collect()
    }

    /// Put paragraphs cut or copied from a document in at `at`: the
    /// first goes on the end of what is before the caret and the last
    /// before what is after it, each keeping how its text is dressed.
    /// Returns the place after them.
    pub fn insert_paras(&mut self, at: Pos, paras: &[Para]) -> Pos {
        let Some((first, more)) = paras.split_first() else { return at };
        let at = self.beside_picture(at);
        if more.is_empty() && !first.is_picture() {
            let p = self.para_mut(at.para);
            p.text.insert_str(at.byte, &first.text);
            p.spans.insert(at.byte, first.text.len(), TextAttrs::default());
            for s in first.spans.list() {
                p.spans.apply(at.byte + s.start, at.byte + s.end, |a| *a = s.attrs);
            }
            return Pos::new(at.para, at.byte + first.text.len());
        }
        self.part(at);
        // Whole paragraphs go in before the tail, which moves down.
        let (head, mut tail) = (at.para, at.para + 1);
        let mut end = Pos::new(tail, 0);
        for (i, para) in paras.iter().enumerate() {
            if i == 0 && !para.is_picture() {
                let h = self.para_mut(head);
                if h.text.is_empty() {
                    h.attrs = para.attrs;
                }
                let len = h.text.len();
                h.text.push_str(&para.text);
                h.spans.append(para.spans.clone(), len);
            } else if i + 1 == paras.len() && !para.is_picture() {
                let t = self.para_mut(tail);
                let mut joined = para.clone();
                if !t.text.is_empty() {
                    joined.attrs = t.attrs;
                }
                let len = joined.text.len();
                joined.text.push_str(&t.text);
                joined.spans.append(std::mem::take(&mut t.spans), len);
                *t = joined;
                end = Pos::new(tail, len);
            } else {
                self.paras.insert(tail, Arc::new(para.clone()));
                tail += 1;
                end = Pos::new(tail, 0);
            }
        }
        // Parting at a paragraph's start for a picture left an empty one
        // over it.
        if at.byte == 0 && first.is_picture() {
            self.paras.remove(head);
            end.para -= 1;
        }
        end
    }

    /// Change what the text between two places has.
    pub fn format(&mut self, a: Pos, b: Pos, change: impl Fn(&mut TextAttrs)) {
        let (a, b) = ordered(a, b);
        for i in a.para..=b.para {
            let (from, to) = (if i == a.para { a.byte } else { 0 }, if i == b.para { b.byte } else { self.para(i).text.len() });
            if from < to && !self.para(i).is_picture() {
                self.para_mut(i).spans.apply(from, to, &change);
            }
        }
    }

    /// What all the text between two places has in common. Paragraphs
    /// with nothing of theirs in the range have no say.
    pub fn common(&self, a: Pos, b: Pos) -> TextAttrs {
        let (a, b) = ordered(a, b);
        let of = |i: usize| {
            let p = self.para(i);
            let (from, to) = (if i == a.para { a.byte } else { 0 }, if i == b.para { b.byte } else { p.text.len() });
            (from < to && !p.is_picture()).then(|| p.spans.common(from, to))
        };
        (a.para..=b.para).filter_map(of).reduce(TextAttrs::common).unwrap_or_default()
    }

    /// Change what paragraphs `from..=to` have.
    pub fn set_paras(&mut self, from: usize, to: usize, change: impl Fn(&mut ParaAttrs)) {
        for i in from..=to.min(self.paras.len() - 1) {
            change(&mut self.para_mut(i).attrs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{List, Placed, Style};

    fn bold() -> TextAttrs {
        TextAttrs { bold: true, ..TextAttrs::default() }
    }

    fn texts(doc: &Doc) -> Vec<&str> {
        doc.paras.iter().map(|p| p.text.as_str()).collect()
    }

    fn picture() -> Para {
        Para::of_picture(Placed { id: 7, width: 0.0 })
    }

    #[test]
    fn text_goes_in_and_keeps_how_what_is_around_it_is_dressed() {
        let mut doc = Doc::from_text("hello world");
        doc.format(Pos::new(0, 6), Pos::new(0, 11), |a| a.bold = true);
        // One line into the middle.
        assert_eq!(doc.insert_text(Pos::new(0, 5), "XYZ", TextAttrs::default()), Pos::new(0, 8));
        assert_eq!(texts(&doc), ["helloXYZ world"]);
        assert!(doc.para(0).spans.at(9).bold && !doc.para(0).spans.at(6).bold);
        // Several: the tail goes after the last, still bold; the lines
        // are as they were typed.
        let end = doc.insert_text(Pos::new(0, 5), "AA\r\nBB\nCC", bold());
        assert_eq!((texts(&doc), end), (vec!["helloAA", "BB", "CCXYZ world"], Pos::new(2, 2)));
        assert!(doc.para(0).spans.at(5).bold && doc.para(1).spans.at(0).bold && doc.para(2).spans.at(1).bold);
        assert!(!doc.para(2).spans.at(2).bold && doc.para(2).spans.at(6).bold, "XYZ is plain, world is bold");
        // Typing takes after what is before it; at a paragraph's start,
        // what is after.
        assert!(doc.typing_attrs(Pos::new(0, 7)).bold && !doc.typing_attrs(Pos::new(0, 5)).bold);
        assert!(doc.typing_attrs(Pos::new(1, 0)).bold);
        // A list goes on in what is pasted; a ticked box starts unticked.
        let mut list = Doc::from_text("one");
        list.set_paras(0, 0, |a| a.list = List::Check(true));
        list.insert_text(Pos::new(0, 3), "\ntwo", TextAttrs::default());
        assert_eq!(list.paras.iter().map(|p| p.attrs.list).collect::<Vec<_>>(), [List::Check(true), List::Check(false)]);
    }

    #[test]
    fn enter_parts_a_paragraph_and_a_cut_joins_them() {
        let mut doc = Doc::from_text("Chapter One\nbody text");
        doc.set_paras(0, 0, |a| a.style = Style::Heading1);
        doc.set_paras(1, 1, |a| a.list = List::Number);
        // In the middle of a heading both halves are headings; at its
        // end what follows is body text. A list goes on.
        assert_eq!(doc.split(Pos::new(0, 7)), Pos::new(1, 0));
        assert_eq!(doc.split(Pos::new(1, 4)), Pos::new(2, 0));
        assert_eq!(doc.split(Pos::new(3, 4)), Pos::new(4, 0));
        assert_eq!(texts(&doc), ["Chapter", " One", "", "body", " text"]);
        assert_eq!(doc.paras.iter().map(|p| p.attrs.style).collect::<Vec<_>>(), [Style::Heading1, Style::Heading1, Style::Body, Style::Body, Style::Body]);
        assert_eq!((doc.number(3), doc.number(4), doc.para(2).attrs.list), (1, 2, List::None));
        // A cut across paragraphs joins its ends as the first was; from a
        // paragraph's very start, as the last was.
        assert_eq!(doc.delete(Pos::new(4, 2), Pos::new(0, 4)), Pos::new(0, 4));
        assert_eq!((texts(&doc), doc.para(0).attrs.style, doc.para(0).attrs.list), (vec!["Chapext"], Style::Heading1, List::None));
        let mut doc = Doc::from_text("gone\nkept");
        doc.set_paras(1, 1, |a| a.list = List::Bullet);
        doc.format(Pos::new(1, 2), Pos::new(1, 4), |a| a.bold = true);
        doc.delete(Pos::new(0, 0), Pos::new(1, 1));
        assert_eq!((texts(&doc), doc.para(0).attrs.list), (vec!["ept"], List::Bullet));
        assert!(!doc.para(0).spans.at(0).bold && doc.para(0).spans.at(1).bold);
        // Within one paragraph, and nothing at all.
        assert_eq!(doc.delete(Pos::new(0, 1), Pos::new(0, 2)), Pos::new(0, 1));
        assert_eq!(doc.delete(Pos::new(0, 1), Pos::new(0, 1)), Pos::new(0, 1));
        assert_eq!(texts(&doc), ["et"]);
    }

    #[test]
    fn what_is_copied_goes_back_in_dressed_as_it_was() {
        let mut doc = Doc::from_text("alpha\nbeta\ngamma");
        doc.set_paras(1, 1, |a| a.style = Style::Quote);
        doc.format(Pos::new(0, 3), Pos::new(2, 2), |a| a.italic = true);
        let cut = doc.slice(Pos::new(2, 2), Pos::new(0, 3));
        assert_eq!(cut.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["ha", "beta", "ga"]);
        assert_eq!((cut[1].attrs.style, cut[0].spans.common(0, 2).italic, doc.plain(Pos::new(0, 3), Pos::new(2, 2))), (Style::Quote, true, "ha\nbeta\nga".to_owned()));
        assert_eq!((doc.common(Pos::new(0, 3), Pos::new(2, 2)).italic, doc.common(Pos::new(0, 0), Pos::new(2, 2)).italic), (true, false));
        // One paragraph of it is text put into the line.
        let mut other = Doc::from_text("12");
        assert_eq!(other.insert_paras(Pos::new(0, 1), &cut[..1]), Pos::new(0, 3));
        assert_eq!(texts(&other), ["1ha2"]);
        assert!(other.para(0).spans.at(1).italic && !other.para(0).spans.at(3).italic);
        // Several part the line: the first on the end of its head, the
        // last before its tail, the ones between whole.
        assert_eq!(other.insert_paras(Pos::new(0, 2), &cut), Pos::new(2, 2));
        assert_eq!(texts(&other), ["1hha", "beta", "gaa2"]);
        assert_eq!(other.paras.iter().map(|p| p.attrs.style).collect::<Vec<_>>(), [Style::Body, Style::Quote, Style::Body]);
        // Into an empty paragraph they come as they were.
        let mut empty = Doc::default();
        assert_eq!(empty.insert_paras(Pos::default(), &cut[1..]), Pos::new(1, 2));
        assert_eq!((texts(&empty), empty.para(0).attrs.style), (vec!["beta", "ga"], Style::Quote));
    }

    #[test]
    fn a_picture_is_a_paragraph_of_its_own() {
        let mut doc = Doc::from_text("before\nafter");
        // Put in at a paragraph's start, in its middle, and at its end.
        assert_eq!(doc.insert_paras(Pos::new(1, 0), &[picture()]), Pos::new(2, 0));
        assert_eq!(texts(&doc), ["before", "\u{FFFC}", "after"]);
        assert_eq!(doc.insert_paras(Pos::new(0, 3), &[picture()]), Pos::new(2, 0));
        assert_eq!(texts(&doc), ["bef", "\u{FFFC}", "ore", "\u{FFFC}", "after"]);
        assert_eq!((doc.plain(Pos::default(), doc.end()), doc.count()), ("bef\n\nore\n\nafter".to_owned(), (3, 11)));
        // Typing at a picture goes beside it, and Enter makes a line
        // over it or under it.
        assert_eq!(doc.insert_text(Pos::new(1, 0), "x", TextAttrs::default()), Pos::new(1, 1));
        assert_eq!(doc.insert_text(Pos::new(2, 3), "y", TextAttrs::default()), Pos::new(3, 1));
        assert_eq!(texts(&doc), ["bef", "x", "\u{FFFC}", "y", "ore", "\u{FFFC}", "after"]);
        assert_eq!(doc.split(Pos::new(5, 0)), Pos::new(6, 0));
        assert_eq!(texts(&doc)[4..], ["ore", "", "\u{FFFC}", "after"]);
        // Backspace after a picture takes it, leaving a line to type on.
        assert_eq!(doc.delete(Pos::new(6, 0), Pos::new(6, 3)), Pos::new(6, 0));
        assert!(!doc.para(6).is_picture() && doc.para(6).text.is_empty());
        // A cut that ends at a picture leaves it whole and apart; one
        // that takes it takes all of it.
        let mut doc = Doc::from_paras(vec![Para::plain("text"), picture(), Para::plain("more")]);
        assert_eq!(doc.delete(Pos::new(0, 2), Pos::new(1, 0)), Pos::new(0, 2));
        assert_eq!(texts(&doc), ["te", "\u{FFFC}", "more"]);
        assert_eq!(doc.delete(Pos::new(1, 3), Pos::new(2, 2)), Pos::new(1, 3));
        assert_eq!(texts(&doc), ["te", "\u{FFFC}", "re"]);
        assert_eq!(doc.delete(Pos::new(0, 1), Pos::new(2, 1)), Pos::new(0, 1));
        assert_eq!((texts(&doc), doc.paras.iter().any(|p| p.is_picture())), (vec!["te"], false));
        let mut doc = Doc::from_paras(vec![picture(), Para::plain("more")]);
        assert_eq!(doc.delete(Pos::new(0, 0), Pos::new(1, 1)), Pos::new(0, 0));
        assert_eq!(texts(&doc), ["ore"]);
    }
}
