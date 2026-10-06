//! Plain text: the paragraphs a line each, and nothing of how they were
//! dressed, but for the lists a text file can spell out: `- ` before a
//! bullet, `- [ ] ` and `- [x] ` before a box to tick.

use crate::doc::{Doc, List, Para, ParaAttrs};

/// Read text, a paragraph a line.
pub fn read(content: &str) -> Doc {
    let para = |line: &str| {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let (list, text) = if let Some(text) = line.strip_prefix("- [ ] ") {
            (List::Check(false), text)
        } else if let Some(text) = line.strip_prefix("- [x] ").or_else(|| line.strip_prefix("- [X] ")) {
            (List::Check(true), text)
        } else if let Some(text) = line.strip_prefix("- ") {
            (List::Bullet, text)
        } else {
            (List::None, line)
        };
        Para { attrs: ParaAttrs { list, ..ParaAttrs::default() }, ..Para::plain(text) }
    };
    // A file's last line ends with a newline: that is not one more
    // paragraph.
    let content = content.strip_suffix('\n').unwrap_or(content);
    Doc::from_paras(content.split('\n').map(para).collect())
}

fn mark(list: List) -> &'static str {
    match list {
        List::Bullet => "- ",
        List::Check(false) => "- [ ] ",
        List::Check(true) => "- [x] ",
        List::None | List::Number => "",
    }
}

pub fn write(doc: &Doc) -> String {
    let mut out = String::new();
    for (i, p) in doc.paras.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(mark(p.attrs.list));
        out.extend(p.text.chars().filter(|c| *c != '\u{FFFC}'));
    }
    out
}

/// Whether a text file can hold all a document has: nothing dressed, no
/// pictures, and lists only of the kinds it spells out.
pub fn holds(doc: &Doc) -> bool {
    doc.paras.iter().all(|p| p.spans.list().is_empty() && !p.is_picture() && p.attrs.list != List::Number && p.attrs == ParaAttrs { list: p.attrs.list, ..ParaAttrs::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Pos, Style};

    #[test]
    fn text_files_keep_their_lists_and_nothing_else() {
        let doc = read("title\r\n- first\n-\n- [ ] milk\n- [x] eggs\n");
        assert_eq!(doc.paras.iter().map(|p| (p.text.as_str(), p.attrs.list)).collect::<Vec<_>>(), [("title", List::None), ("first", List::Bullet), ("-", List::None), ("milk", List::Check(false)), ("eggs", List::Check(true))]);
        assert_eq!(write(&doc), "title\n- first\n-\n- [ ] milk\n- [x] eggs");
        assert!(holds(&doc));
        assert_eq!((read("").paras.len(), read("\n").paras.len(), read("a\n\n").paras.len()), (1, 1, 2));
        // What a text file can't hold.
        for change in [|d: &mut Doc| d.format(Pos::new(0, 0), Pos::new(0, 2), |a| a.bold = true), |d: &mut Doc| d.set_paras(0, 0, |a| a.style = Style::Heading1), |d: &mut Doc| d.set_paras(1, 1, |a| a.list = List::Number), |d: &mut Doc| d.set_paras(1, 1, |a| a.level = 1)] {
            let mut dressed = doc.clone();
            change(&mut dressed);
            assert!(!holds(&dressed));
        }
    }
}
