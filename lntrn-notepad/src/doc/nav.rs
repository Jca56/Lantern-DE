//! Getting about in a document: a step by character or by word, the word
//! or paragraph at a place, and every place some text is found.

use super::{Doc, Pos};

/// What a character counts as when moving by word.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Other,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' || c == '\'' || c == '\u{2019}' {
        Class::Word
    } else {
        Class::Other
    }
}

impl Doc {
    /// One character back, into the paragraph before at a paragraph's
    /// start.
    pub fn left(&self, p: Pos) -> Pos {
        let text = &self.para(p.para).text;
        match text[..p.byte].char_indices().next_back() {
            Some((i, _)) => Pos::new(p.para, i),
            None if p.para > 0 => Pos::new(p.para - 1, self.para(p.para - 1).text.len()),
            None => p,
        }
    }

    pub fn right(&self, p: Pos) -> Pos {
        let text = &self.para(p.para).text;
        match text[p.byte..].chars().next() {
            Some(c) => Pos::new(p.para, p.byte + c.len_utf8()),
            None if p.para + 1 < self.paras.len() => Pos::new(p.para + 1, 0),
            None => p,
        }
    }

    /// To the start of the word before: back over any blanks, then over
    /// the word (or the run of punctuation) they followed.
    pub fn word_left(&self, p: Pos) -> Pos {
        let text = &self.para(p.para).text;
        if p.byte == 0 {
            return self.left(p);
        }
        let mut chars = text[..p.byte].char_indices().rev().peekable();
        let mut at = p.byte;
        while let Some((i, _)) = chars.next_if(|(_, c)| class(*c) == Class::Space) {
            at = i;
        }
        if let Some(kind) = chars.peek().map(|(_, c)| class(*c)) {
            while let Some((i, _)) = chars.next_if(|(_, c)| class(*c) == kind) {
                at = i;
            }
        }
        Pos::new(p.para, at)
    }

    /// To the end of the word after.
    pub fn word_right(&self, p: Pos) -> Pos {
        let text = &self.para(p.para).text;
        if p.byte >= text.len() {
            return self.right(p);
        }
        let mut chars = text[p.byte..].char_indices().peekable();
        while chars.next_if(|(_, c)| class(*c) == Class::Space).is_some() {}
        if let Some(kind) = chars.peek().map(|(_, c)| class(*c)) {
            while chars.next_if(|(_, c)| class(*c) == kind).is_some() {}
        }
        Pos::new(p.para, chars.peek().map_or(text.len(), |(i, _)| p.byte + i))
    }

    /// The word at a place, as a double click picks it: the run of
    /// characters of one kind around it, a word before the caret counting
    /// when there is none after.
    pub fn word_at(&self, p: Pos) -> (Pos, Pos) {
        let text = &self.para(p.para).text;
        let after = text[p.byte..].chars().next().map(class);
        let before = text[..p.byte].chars().next_back().map(class);
        let kind = match (after, before) {
            (Some(Class::Word), _) | (Some(_), None) => after,
            (_, Some(Class::Word)) => before,
            (Some(a), _) => Some(a),
            (None, b) => b,
        };
        let Some(kind) = kind else { return (p, p) };
        let start = text[..p.byte].char_indices().rev().take_while(|(_, c)| class(*c) == kind).last().map_or(p.byte, |(i, _)| i);
        let end = text[p.byte..].char_indices().take_while(|(_, c)| class(*c) == kind).last().map_or(p.byte, |(i, c)| p.byte + i + c.len_utf8());
        (Pos::new(p.para, start), Pos::new(p.para, end))
    }

    /// The whole paragraph at a place, as a triple click picks it.
    pub fn para_at(&self, p: Pos) -> (Pos, Pos) {
        (Pos::new(p.para, 0), Pos::new(p.para, self.para(p.para).text.len()))
    }
}

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Find {
    pub text: String,
    /// Capitals count.
    pub exact_case: bool,
    /// Only where it stands as a word of its own.
    pub whole_word: bool,
}

impl Find {
    /// Every place it is in `doc`, in order. Nothing is found across
    /// paragraphs.
    pub fn all(&self, doc: &Doc) -> Vec<(Pos, Pos)> {
        if self.text.is_empty() {
            return Vec::new();
        }
        let fold = |s: &str| if self.exact_case { s.to_owned() } else { s.to_lowercase() };
        let needle = fold(&self.text);
        let mut out = Vec::new();
        for (i, para) in doc.paras.iter().enumerate() {
            // Lowering a character can change how many bytes it takes:
            // the text is searched folded, and each match is counted back
            // into the text as it is.
            let folded = fold(&para.text);
            let same = folded.len() == para.text.len();
            let mut from = 0;
            while let Some(hit) = folded[from..].find(&needle).map(|h| h + from) {
                let (start, end) = if same { (hit, hit + needle.len()) } else { (unfold(&para.text, &folded[..hit]), unfold(&para.text, &folded[..hit + needle.len()])) };
                let edge = |c: Option<char>| c.is_none_or(|c| class(c) != Class::Word);
                if !self.whole_word || (edge(para.text[..start].chars().next_back()) && edge(para.text[end..].chars().next())) {
                    out.push((Pos::new(i, start), Pos::new(i, end)));
                }
                from = hit + needle.len().max(1);
            }
        }
        out
    }
}

/// How many bytes of `text` lower to the folded text `upto`.
fn unfold(text: &str, upto: &str) -> usize {
    let mut folded = 0;
    for (i, c) in text.char_indices() {
        if folded >= upto.len() {
            return i;
        }
        folded += c.to_lowercase().map(char::len_utf8).sum::<usize>();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_by_character_and_by_word() {
        let doc = Doc::from_text("héllo,  big_world!\nnext");
        let at = |byte| Pos::new(0, byte);
        // é is two bytes: a step is a character, not a byte.
        assert_eq!((doc.right(at(1)), doc.left(at(3)), doc.left(at(0)), doc.right(doc.end())), (at(3), at(1), at(0), doc.end()));
        assert_eq!((doc.right(at(19)), doc.left(Pos::new(1, 0))), (Pos::new(1, 0), at(19)));
        // Words, punctuation as words of its own, blanks skipped.
        let rights: Vec<usize> = std::iter::successors(Some(at(0)), |p| (p.para == 0).then(|| doc.word_right(*p))).map(|p| p.byte).collect();
        assert_eq!(rights, [0, 6, 7, 18, 19, 0]);
        let lefts: Vec<usize> = std::iter::successors(Some(at(19)), |p| (p.byte > 0).then(|| doc.word_left(*p))).map(|p| p.byte).collect();
        assert_eq!(lefts, [19, 18, 9, 6, 0]);
        assert_eq!(doc.word_left(Pos::new(1, 0)), at(19));
        // A double click takes the word under it, or the one it is at
        // the end of; a triple, the paragraph.
        assert_eq!(doc.word_at(at(11)), (at(9), at(18)));
        assert_eq!(doc.word_at(at(6)), (at(0), at(6)), "at a word's end, that word");
        assert_eq!(doc.word_at(at(8)), (at(7), at(9)), "between blanks, the blanks");
        assert_eq!(doc.word_at(at(19)), (at(18), at(19)));
        assert_eq!(doc.para_at(at(4)), (at(0), at(19)));
        assert_eq!(Doc::default().word_at(Pos::default()), (Pos::default(), Pos::default()));
    }

    #[test]
    fn text_is_found_wherever_it_is() {
        let doc = Doc::from_text("The cat sat. Concatenate THE CAT\ncat İstanbul cat");
        let find = |text: &str, exact_case, whole_word| Find { text: text.into(), exact_case, whole_word }.all(&doc).into_iter().map(|(a, b)| (a.para, a.byte, b.byte)).collect::<Vec<_>>();
        assert_eq!(find("cat", false, false), [(0, 4, 7), (0, 16, 19), (0, 29, 32), (1, 0, 3), (1, 14, 17)]);
        assert_eq!(find("cat", false, true), [(0, 4, 7), (0, 29, 32), (1, 0, 3), (1, 14, 17)]);
        assert_eq!(find("CAT", true, false), [(0, 29, 32)]);
        assert_eq!(find("the cat", false, false), [(0, 0, 7), (0, 25, 32)]);
        // İ lowers to more bytes than it has: matches after it still land
        // on the text as it is.
        assert_eq!(&doc.para(1).text[14..17], "cat");
        assert_eq!(find("", false, false), []);
        assert_eq!(find("aa", false, false), [], "no match is not a hang");
    }
}
