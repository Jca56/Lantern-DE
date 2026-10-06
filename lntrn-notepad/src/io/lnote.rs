//! The `.lnote` format: a few lines saying how the text is dressed, then
//! the text itself, untouched.
//!
//! ```text
//! lnote 2
//! para 0 style=title align=center
//! para 3 check checked level=1
//! para 5 picture=9f3a6c0d2b1e4f77 width=320
//! span 0 0 12 bold size=32
//! span 4 4 9 italic font=Lora color=cc3a34 bg=fff0c0
//! picture 9f3a6c0d2b1e4f77 iVBORw0KGgo...
//! text
//! <the paragraphs, one a line, exactly as they are>
//! ```
//!
//! - `para <paragraph>` then any of `bullet`, `number`, `check`,
//!   `checked`, `level=N`, `style=title|h1|h2|h3|quote`,
//!   `align=center|right|justify`, `before=F`, `after=F`, `indent=F`,
//!   `picture=ID` and `width=F`. Only paragraphs with something to say
//!   have one.
//! - `span <paragraph> <start> <end>` (bytes of that paragraph) then any
//!   of `bold`, `italic`, `underline`, `strike`, `size=F`, `font=Name`,
//!   `color=RRGGBB`, `bg=RRGGBB`.
//! - `picture <id> <base64>`: a picture file, whole. Its paragraph's text
//!   is the one character U+FFFC.
//! - Everything after the `text` line is the text. Nothing in it is
//!   escaped, so the format can't mangle what was written.
//!
//! A record or word that isn't known is skipped, so files from a newer
//! Notepad still open, and version 1 files (bullets, alignment and spans
//! only) are read as they always were. A header that makes no sense
//! means the file is read as plain text: none is ever refused.

use lntrn_core::encoding::{base64_decode, base64_encode};

use crate::doc::{Align, Doc, Font, List, MAX_LEVEL, OBJECT, Para, ParaAttrs, Placed, Style, TextAttrs};

/// The fonts Notepad used to come with, as `(family, name in a file)`:
/// version 1 files name them by the font file's name, and they still are,
/// so an older Notepad reads them too.
const BUNDLED: [(&str, &str); 20] = [
    ("Roboto", "Roboto"),
    ("Open Sans", "OpenSans"),
    ("Lato", "Lato"),
    ("Montserrat", "Montserrat"),
    ("Poppins", "Poppins"),
    ("Inter", "Inter"),
    ("Nunito", "Nunito"),
    ("Raleway", "Raleway"),
    ("Work Sans", "WorkSans"),
    ("DM Sans", "DMSans"),
    ("Rubik", "Rubik"),
    ("Quicksand", "Quicksand"),
    ("Oswald", "Oswald"),
    ("Source Sans 3", "SourceSans3"),
    ("PT Sans", "PTSans"),
    ("Lora", "Lora"),
    ("Merriweather", "Merriweather"),
    ("Playfair Display", "PlayfairDisplay"),
    ("Bitter", "Bitter"),
    ("JetBrains Mono", "JetBrainsMono"),
];

/// A family as one word of a record: a bundled one by its file's name,
/// any other with what a record can't hold written as `%XX`.
fn font_word(family: &str) -> String {
    match BUNDLED.iter().find(|(name, _)| *name == family) {
        Some((_, file)) => (*file).to_owned(),
        None => family.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect(),
    }
}

fn font_from_word(word: &str) -> Option<Font> {
    if let Some((family, _)) = BUNDLED.iter().find(|(_, file)| *file == word) {
        return Some(Font::named(family));
    }
    let mut bytes = Vec::with_capacity(word.len());
    let mut rest = word.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        match (b, tail.get(..2).and_then(|hex| std::str::from_utf8(hex).ok()).and_then(|hex| u8::from_str_radix(hex, 16).ok())) {
            (b'%', Some(byte)) => {
                bytes.push(byte);
                rest = &tail[2..];
            }
            _ => {
                bytes.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8(bytes).ok().filter(|name| !name.is_empty()).map(|name| Font::named(&name))
}

fn para_record(i: usize, p: &Para) -> Option<String> {
    let a = p.attrs;
    if a == ParaAttrs::default() && p.picture.is_none() {
        return None;
    }
    let mut out = format!("para {i}");
    out.push_str(match a.list {
        List::None => "",
        List::Bullet => " bullet",
        List::Number => " number",
        List::Check(false) => " check",
        List::Check(true) => " check checked",
    });
    if a.level > 0 {
        out.push_str(&format!(" level={}", a.level));
    }
    if a.style != Style::Body {
        out.push_str(&format!(" style={}", a.style.word()));
    }
    out.push_str(match a.align {
        Align::Left => "",
        Align::Center => " align=center",
        Align::Right => " align=right",
        Align::Justify => " align=justify",
    });
    for (word, value) in [("before", a.space_before), ("after", a.space_after), ("indent", a.first_indent)] {
        if value != 0.0 {
            out.push_str(&format!(" {word}={value}"));
        }
    }
    if let Some(placed) = p.picture {
        out.push_str(&format!(" picture={:016x}", placed.id));
        if placed.width > 0.0 {
            out.push_str(&format!(" width={}", placed.width));
        }
    }
    Some(out)
}

fn span_record(i: usize, start: usize, end: usize, a: &TextAttrs) -> String {
    let mut out = format!("span {i} {start} {end}");
    for (on, word) in [(a.bold, " bold"), (a.italic, " italic"), (a.underline, " underline"), (a.strike, " strike")] {
        if on {
            out.push_str(word);
        }
    }
    if let Some(size) = a.size {
        out.push_str(&format!(" size={size}"));
    }
    if let Some(font) = a.font {
        out.push_str(&format!(" font={}", font_word(font.name())));
    }
    if let Some(color) = a.color {
        out.push_str(&format!(" color={color:06x}"));
    }
    if let Some(color) = a.highlight {
        out.push_str(&format!(" bg={color:06x}"));
    }
    out
}

/// A document as a `.lnote` file's text. Only the pictures that show
/// somewhere go in.
pub fn write(doc: &Doc) -> String {
    let mut out = String::from("lnote 2\n");
    let mut shown: Vec<u64> = Vec::new();
    for (i, p) in doc.paras.iter().enumerate() {
        if let Some(record) = para_record(i, p) {
            out.push_str(&record);
            out.push('\n');
        }
        for s in p.spans.list() {
            out.push_str(&span_record(i, s.start, s.end, &s.attrs));
            out.push('\n');
        }
        shown.extend(p.picture.map(|placed| placed.id).filter(|id| !shown.contains(id)));
    }
    for id in shown {
        if let Some(picture) = doc.pictures.get(id) {
            out.push_str(&format!("picture {id:016x} {}\n", base64_encode(&picture.bytes)));
        }
    }
    out.push_str("text\n");
    for (i, p) in doc.paras.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&p.text);
    }
    out
}

fn read_para<'a>(p: &mut Para, picture: &mut Option<(u64, f32)>, words: impl Iterator<Item = &'a str>) {
    let number = |v: &str| v.parse::<f32>().ok().filter(|f| f.is_finite());
    for word in words {
        let (key, value) = word.split_once('=').unwrap_or((word, ""));
        match key {
            "bullet" => p.attrs.list = List::Bullet,
            "number" => p.attrs.list = List::Number,
            "check" if p.attrs.list != List::Check(true) => p.attrs.list = List::Check(false),
            "checked" => p.attrs.list = List::Check(true),
            "level" => p.attrs.level = value.parse::<u8>().unwrap_or(0).min(MAX_LEVEL),
            "style" => p.attrs.style = Style::from_word(value).unwrap_or_default(),
            "align" => {
                p.attrs.align = match value {
                    "center" => Align::Center,
                    "right" => Align::Right,
                    "justify" => Align::Justify,
                    _ => Align::Left,
                }
            }
            "before" => p.attrs.space_before = number(value).unwrap_or(0.0),
            "after" => p.attrs.space_after = number(value).unwrap_or(0.0),
            "indent" => p.attrs.first_indent = number(value).unwrap_or(0.0),
            "picture" => *picture = u64::from_str_radix(value, 16).ok().map(|id| (id, picture.map_or(0.0, |(_, w)| w))),
            "width" => *picture = Some((picture.map_or(0, |(id, _)| id), number(value).unwrap_or(0.0).max(0.0))),
            _ => {}
        }
    }
}

fn read_span<'a>(words: impl Iterator<Item = &'a str>) -> TextAttrs {
    let mut a = TextAttrs::default();
    for word in words {
        let (key, value) = word.split_once('=').unwrap_or((word, ""));
        match key {
            "bold" => a.bold = true,
            "italic" => a.italic = true,
            "underline" => a.underline = true,
            "strike" => a.strike = true,
            "size" => a.size = value.parse::<f32>().ok().filter(|f| f.is_finite() && *f > 0.0),
            "font" => a.font = font_from_word(value),
            "color" => a.color = u32::from_str_radix(value, 16).ok().map(|c| c & 0xFF_FFFF),
            "bg" => a.highlight = u32::from_str_radix(value, 16).ok().map(|c| c & 0xFF_FFFF),
            _ => {}
        }
    }
    a
}

/// Read a `.lnote` file's text. `None` when it does not start as one.
pub fn read(content: &str) -> Option<Doc> {
    let (first, mut rest) = content.split_once('\n')?;
    let mut header = first.split_whitespace();
    if header.next()? != "lnote" || header.next()?.parse::<u32>().is_err() {
        return None;
    }
    let mut records: Vec<&str> = Vec::new();
    let body = loop {
        let (line, tail) = rest.split_once('\n')?;
        if line == "text" {
            break tail;
        }
        records.push(line);
        rest = tail;
    };
    let mut paras: Vec<Para> = body.split('\n').map(Para::plain).collect();
    let mut doc = Doc::default();
    // The number a file calls a picture by, and the one it has here.
    let mut known: Vec<(u64, u64)> = Vec::new();
    let mut placed: Vec<(usize, u64, f32)> = Vec::new();
    for record in records {
        let mut words = record.split(' ').filter(|w| !w.is_empty());
        let kind = words.next();
        let mut index = || words.next().and_then(|w| w.parse::<usize>().ok());
        match kind {
            Some("para") => {
                let Some(p) = index().and_then(|i| paras.get_mut(i).map(|p| (i, p))) else { continue };
                let mut picture = None;
                read_para(p.1, &mut picture, words);
                placed.extend(picture.map(|(id, width)| (p.0, id, width)));
            }
            Some("span") => {
                let (Some(i), Some(start), Some(end)) = (index(), index(), index()) else { continue };
                let Some(p) = paras.get_mut(i) else { continue };
                // Offsets someone wrote by hand may be off: one that is
                // not on a character's edge is skipped, not believed.
                let end = end.min(p.text.len());
                if start < end && p.text.is_char_boundary(start) && p.text.is_char_boundary(end) {
                    let attrs = read_span(words);
                    p.spans.apply(start, end, |a| *a = attrs);
                }
            }
            Some("picture") => {
                let (Some(id), Some(data)) = (words.next().and_then(|w| u64::from_str_radix(w, 16).ok()), words.next().and_then(base64_decode)) else { continue };
                if let Ok(here) = doc.pictures.add(data) {
                    known.push((id, here));
                }
            }
            _ => {}
        }
    }
    for (i, id, width) in placed {
        let here = known.iter().find(|(file, _)| *file == id).map(|(_, here)| *here);
        if let (Some(id), true) = (here, paras[i].text == OBJECT) {
            paras[i].picture = Some(Placed { id, width });
        }
    }
    // The object character with no picture behind it shows as nothing.
    for p in paras.iter_mut().filter(|p| p.picture.is_none() && p.text.contains('\u{FFFC}')) {
        *p = Para { attrs: p.attrs, ..Para::plain(&p.text.replace('\u{FFFC}', "")) };
    }
    doc.paras = paras.into_iter().map(std::sync::Arc::new).collect();
    Some(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Pos;
    use lntrn_image::{Image, encode_png};

    #[test]
    fn everything_a_document_has_comes_back() {
        let mut doc = Doc::from_text("Big Centered Title\n\nbody with styled words\na bullet item\nthree\nfour\n\u{FFFC}\nend");
        doc.set_paras(0, 0, |a| (a.style, a.align, a.space_after) = (Style::Title, Align::Center, 8.0));
        doc.format(Pos::new(0, 0), Pos::new(0, 18), |a| (a.bold, a.size) = (true, Some(32.0)));
        doc.set_paras(2, 2, |a| (a.first_indent, a.align, a.space_before) = (24.0, Align::Justify, 2.5));
        doc.format(Pos::new(2, 10), Pos::new(2, 16), |a| *a = TextAttrs { italic: true, underline: true, strike: true, font: Some(Font::named("Lora")), color: Some(0xcc3a34), highlight: Some(0xfff0c0), ..TextAttrs::default() });
        doc.format(Pos::new(2, 0), Pos::new(2, 4), |a| a.font = Some(Font::named("Comic Neue 2")));
        doc.set_paras(3, 3, |a| a.list = List::Bullet);
        doc.set_paras(4, 4, |a| (a.list, a.level, a.align) = (List::Number, 2, Align::Right));
        doc.set_paras(5, 5, |a| (a.list, a.style) = (List::Check(true), Style::Quote));
        let id = doc.pictures.add(encode_png(&Image::solid(3, 2, [1, 2, 3, 255]))).unwrap();
        doc.pictures.add(encode_png(&Image::solid(1, 1, [9, 9, 9, 255]))).unwrap();
        doc.para_mut(6).picture = Some(Placed { id, width: 320.0 });
        let file = write(&doc);
        assert!(file.starts_with("lnote 2\npara 0 style=title align=center after=8\nspan 0 0 18 bold size=32\n"), "{file}");
        assert!(file.contains("\nspan 2 0 4 font=Comic%20Neue%202\nspan 2 10 16 italic underline strike font=Lora color=cc3a34 bg=fff0c0\n"), "{file}");
        assert!(file.contains("\npara 4 number level=2 align=right\npara 5 check checked style=quote\n"), "{file}");
        assert_eq!(file.matches("\npicture ").count(), 1, "a picture nothing shows is left behind");
        let back = read(&file).expect("an lnote");
        assert_eq!(back.paras, doc.paras);
        assert_eq!(back.pictures.get(id).map(|p| (p.image.width, p.image.height)), Some((3, 2)));
        assert_eq!(write(&back), file);
    }

    #[test]
    fn the_text_is_never_mangled_and_older_files_still_open() {
        // Text that looks like records stays text.
        let doc = Doc::from_text("text\nspan 0 0 4 bold\n- not a bullet here\n");
        let back = read(&write(&doc)).unwrap();
        assert_eq!((back.text(), back.paras.iter().all(|p| p.spans.list().is_empty() && p.attrs == ParaAttrs::default())), (doc.text(), true));
        // A version 1 file, as the old Notepad wrote it.
        let old = "lnote 1\npara 0 align=center after=8\nspan 0 0 5 bold size=32\npara 2 bullet indent=24\nspan 2 2 6 italic font=PlayfairDisplay color=cc3a34 bg=fff0c0\ntext\nTitle\n\na bullet\n";
        let doc = read(old).unwrap();
        assert_eq!(doc.paras.len(), 4, "the empty last line is a paragraph");
        assert_eq!((doc.para(0).attrs.align, doc.para(0).attrs.space_after, doc.para(0).spans.at(2).size), (Align::Center, 8.0, Some(32.0)));
        assert_eq!((doc.para(2).attrs.list, doc.para(2).attrs.first_indent), (List::Bullet, 24.0));
        let run = doc.para(2).spans.at(3);
        assert_eq!((run.italic, run.font.map(Font::name), run.color, run.highlight), (true, Some("Playfair Display"), Some(0xcc3a34), Some(0xfff0c0)));
        // What isn't known is skipped; offsets that can't be are too; a
        // picture that isn't there leaves no mark.
        let odd = "lnote 9\ncomment from the future\npara 0 bullet sparkle=yes\npara 99 bullet\nspan 0 1 2 bold\nspan 0 50 60 bold\nspan 99 0 1 bold\npara 1 picture=00000000000000aa width=50\npicture zz !!!\ntext\nhéllo\n\u{FFFC}";
        let doc = read(odd).unwrap();
        assert_eq!((doc.para(0).text.as_str(), doc.para(0).attrs.list, doc.para(0).spans.list().len()), ("héllo", List::Bullet, 0));
        assert!(doc.para(1).text.is_empty() && !doc.para(1).is_picture());
        // Not one of ours at all.
        assert!(read("not an lnote\njust text").is_none() && read("lnote 1\nno text marker").is_none() && read("").is_none());
    }
}
