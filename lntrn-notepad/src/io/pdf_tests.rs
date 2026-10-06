//! The PDF writer, read back. There is no PDF tool to ask, so a small
//! reader here takes a file apart as a viewer does: from the end, through
//! the cross-reference table to every object, down the page tree, and
//! through each font back to the text a page shows. It refuses what a
//! viewer could not rely on: an offset that is not an object's start, a
//! `/Length` that is not its stream's, a stream that does not inflate
//! whole, a number that is not one, a font whose parts disagree.

use std::collections::HashMap;

use lntrn_image::Image;
use lntrn_image::inflate::inflate_limit;
use lntrn_text::{TextEngine, TextStyle};

use super::fonts::EM;
use super::{A4, Pdf};

/// An object of a PDF. A string is its bytes, however it was written; a
/// word is an operator, `obj`, `stream`, or what ends an array or a
/// dictionary.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Obj {
    Num(f64),
    Name(String),
    Text(Vec<u8>),
    List(Vec<Obj>),
    Dict(Vec<(String, Obj)>),
    Ref(usize),
    Word(String),
}

impl Obj {
    fn get(&self, key: &str) -> Option<&Obj> {
        let Obj::Dict(entries) = self else { return None };
        entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub(super) fn num(&self) -> f64 {
        let Obj::Num(n) = self else { panic!("{self:?} is not a number") };
        *n
    }
    fn name(&self) -> &str {
        let Obj::Name(name) = self else { panic!("{self:?} is not a name") };
        name
    }
    pub(super) fn list(&self) -> &[Obj] {
        let Obj::List(list) = self else { panic!("{self:?} is not an array") };
        list
    }
    /// Whether it is this word, or this name.
    fn is(&self, word: &str) -> bool {
        matches!(self, Obj::Word(w) | Obj::Name(w) if w == word)
    }
}

/// PDF syntax, an object or a word at a time.
struct Lex<'a> {
    b: &'a [u8],
    at: usize,
}

/// Everything in a stream of PDF syntax.
fn lex(b: &[u8]) -> Vec<Obj> {
    Lex { b, at: 0 }.until("")
}

impl<'a> Lex<'a> {
    /// Step over the bytes `keep` holds for, and hand them back.
    fn run(&mut self, keep: impl Fn(u8) -> bool) -> &'a [u8] {
        let (b, start) = (self.b, self.at);
        while b.get(self.at).is_some_and(|c| keep(*c)) {
            self.at += 1;
        }
        &b[start..self.at]
    }

    /// The objects up to the word `end` (what ends an array or a
    /// dictionary), or to the end of the bytes.
    fn until(&mut self, end: &str) -> Vec<Obj> {
        std::iter::from_fn(|| self.next().filter(|o| !matches!(o, Obj::Word(w) if w == end))).collect()
    }

    fn next(&mut self) -> Option<Obj> {
        self.run(|c| b" \t\r\n\x0c\0".contains(&c));
        let (b, start) = (self.b, self.at);
        let regular = |c: u8| !b" \t\r\n\x0c\0()<>[]{}/%".contains(&c);
        self.at += 1;
        Some(match *b.get(start)? {
            b'/' => Obj::Name(String::from_utf8_lossy(self.run(regular)).into_owned()),
            b'[' => Obj::List(self.until("]")),
            b']' => Obj::Word("]".into()),
            b'<' if b.get(self.at) == Some(&b'<') => {
                self.at += 1;
                let mut parts = self.until(">>").into_iter();
                Obj::Dict(std::iter::from_fn(|| Some((parts.next()?.name().to_owned(), parts.next().expect("a key's value")))).collect())
            }
            b'>' => {
                assert_eq!(self.run(|c| c == b'>'), b">", "a > that ends nothing");
                Obj::Word(">>".into())
            }
            b'<' => {
                let hex = self.run(|c| c != b'>');
                self.at += 1;
                assert!(hex.len().is_multiple_of(2) && hex.iter().all(u8::is_ascii_hexdigit), "<{}> is not a hex string", String::from_utf8_lossy(hex));
                Obj::Text(hex.chunks(2).map(|pair| u8::from_str_radix(&String::from_utf8_lossy(pair), 16).expect("hex")).collect())
            }
            b'(' => {
                let (mut depth, mut out) = (1, Vec::new());
                loop {
                    self.at += 1;
                    match b[self.at - 1] {
                        b'\\' => self.at += 1,
                        b')' if depth == 1 => break Obj::Text(out),
                        c => depth += i32::from(c == b'(') - i32::from(c == b')'),
                    }
                    out.push(b[self.at - 1]);
                }
            }
            _ => {
                self.at = start;
                let word = String::from_utf8_lossy(self.run(regular)).into_owned();
                assert!(!word.is_empty(), "nothing readable at byte {start}");
                if !word.starts_with(|c: char| c.is_ascii_digit() || "+-.".contains(c)) {
                    return Some(Obj::Word(word));
                }
                // What the writer promises of a number: digits, at most
                // three more after a point, nothing else.
                let digits = word.strip_prefix('-').unwrap_or(&word);
                let (whole, part) = digits.split_once('.').unwrap_or((digits, "0"));
                let all_digits = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
                assert!((1..=10).contains(&whole.len()) && (1..=3).contains(&part.len()) && all_digits(whole) && all_digits(part), "{word:?} is not a number a PDF takes");
                // `12 0 R` points at object 12.
                if whole == word && b[self.at..].starts_with(b" 0 R") && b.get(self.at + 4).is_none_or(|c| !regular(*c)) {
                    self.at += 4;
                    return Some(Obj::Ref(word.parse().expect("an object's number")));
                }
                Obj::Num(word.parse().expect("a number"))
            }
        })
    }
}

/// A file read back: every object by its number, with its stream's bytes
/// (inflated) when it is one.
pub(super) struct Read {
    objects: HashMap<usize, (Obj, Option<Vec<u8>>)>,
    trailer: Obj,
}

/// Read a file from its end, as a viewer does, and check every font in it.
pub(super) fn read(file: &[u8]) -> Read {
    assert!(file.starts_with(b"%PDF-1.7\n%") && file.ends_with(b"\n%%EOF\n"), "the first and last lines");
    let mark = file.windows(10).rposition(|w| w == b"startxref\n").expect("startxref");
    let table: usize = String::from_utf8_lossy(&file[mark + 10..file.len() - 7]).parse().expect("the table's offset");
    let mut lex = Lex { b: file, at: table };
    assert_eq!((lex.next(), lex.next()), (Some(Obj::Word("xref".into())), Some(Obj::Num(0.0))), "startxref points at the table");
    let count = lex.next().expect("the table's size").num() as usize;
    // An entry is twenty bytes, whatever it says.
    let (entries, rest) = file[lex.at + 1..].split_at(20 * count);
    assert!(entries.starts_with(b"0000000000 65535 f \n") && rest.starts_with(b"trailer\n"), "the table's first entry and its end");
    let mut objects = HashMap::new();
    for (number, entry) in entries.chunks(20).enumerate().skip(1) {
        let entry = String::from_utf8_lossy(entry);
        let offset: usize = entry[..10].parse().expect("an offset");
        let head = format!("{number} 0 obj\n");
        assert!(entry.ends_with(" 00000 n \n") && file[offset - 1] == b'\n' && file[offset..].starts_with(head.as_bytes()), "object {number} does not start at {entry:?}");
        let mut lex = Lex { b: file, at: offset + head.len() };
        let (value, end) = (lex.next().expect("an object"), lex.next().expect("an object's end"));
        let stream = (!end.is("endobj")).then(|| {
            let len = value.get("Length").expect("a stream's /Length").num() as usize;
            let (data, after) = file[lex.at + 1..].split_at(len);
            assert!(end.is("stream") && file[lex.at] == b'\n' && after.starts_with(b"\nendstream\nendobj\n"), "object {number}: /Length is not its stream's length");
            assert!(value.get("Filter").is_some_and(|filter| filter.is("FlateDecode")));
            let plain = inflate_limit(data, 1 << 28).unwrap_or_else(|| panic!("object {number} does not inflate"));
            // The stream ends with the Adler-32 of what it holds.
            let (a, b) = plain.iter().fold((1u32, 0u32), |(a, b), x| ((a + u32::from(*x)) % 65521, (a + u32::from(*x) + b) % 65521));
            assert_eq!(data[len - 4..], (b << 16 | a).to_be_bytes(), "object {number} is cut short");
            plain
        });
        objects.insert(number, (value, stream));
    }
    let trailer = Lex { b: rest, at: 8 }.next().expect("the trailer");
    assert_eq!(trailer.get("Size"), Some(&Obj::Num(count as f64)));
    let read = Read { objects, trailer };
    for (font, _) in read.objects.values().filter(|(o, _)| o.get("Subtype").is_some_and(|kind| kind.is("Type3"))) {
        read.check_font(font);
    }
    read
}

impl Read {
    /// A value, followed when it points at an object.
    pub(super) fn of<'a>(&'a self, value: &'a Obj) -> &'a Obj {
        let Obj::Ref(n) = value else { return value };
        &self.objects.get(n).unwrap_or_else(|| panic!("no object {n}")).0
    }
    pub(super) fn key<'a>(&'a self, dict: &'a Obj, key: &str) -> &'a Obj {
        self.of(dict.get(key).unwrap_or_else(|| panic!("no /{key} in {dict:?}")))
    }
    fn stream(&self, dict: &Obj, key: &str) -> &[u8] {
        let Some(Obj::Ref(n)) = dict.get(key) else { panic!("/{key} of {dict:?} is not an object of its own") };
        self.objects[n].1.as_deref().unwrap_or_else(|| panic!("/{key} is not a stream"))
    }
    fn count(&self, subtype: &str) -> usize {
        self.objects.values().filter(|(o, _)| o.get("Subtype").is_some_and(|kind| kind.is(subtype))).count()
    }
    fn title(&self) -> Option<&Obj> {
        self.key(&self.trailer, "Info").get("Title")
    }

    /// The pages in order, from the trailer down the tree.
    pub(super) fn pages(&self) -> Vec<&Obj> {
        let root = self.key(&self.trailer, "Root");
        let tree = self.key(root, "Pages");
        let kids: Vec<&Obj> = self.key(tree, "Kids").list().iter().map(|kid| self.of(kid)).collect();
        assert!(self.key(root, "Type").is("Catalog") && self.key(tree, "Type").is("Pages") && self.key(tree, "Count").num() == kids.len() as f64);
        for page in &kids {
            assert!(self.key(page, "Type").is("Page") && page.get("Parent") == root.get("Pages"));
            let words: Vec<Obj> = lex(self.stream(page, "Contents")).into_iter().filter(|o| matches!(o, Obj::Word(_))).collect();
            assert!(words[0].is("cm") && words.iter().all(|w| ["cm", "rg", "RG", "w", "re", "f", "m", "l", "S", "q", "Q", "Do", "BT", "ET", "Tf", "Tm", "TJ"].iter().any(|op| w.is(op))), "{words:?}");
        }
        kids
    }

    /// A page's size, from a `/MediaBox` that starts at the origin.
    fn size(&self, page: &Obj) -> (f64, f64) {
        let b: Vec<f64> = self.key(page, "MediaBox").list().iter().map(Obj::num).collect();
        assert_eq!((b.len(), b[0], b[1]), (4, 0.0, 0.0));
        (b[2], b[3])
    }

    /// The fonts or pictures a page lists (`kind` is `Font` or `XObject`),
    /// each under its name.
    pub(super) fn listed<'a>(&'a self, page: &'a Obj, kind: &str) -> &'a [(String, Obj)] {
        match self.key(page, "Resources").get(kind) {
            Some(Obj::Dict(entries)) => entries,
            _ => &[],
        }
    }
    fn names(&self, page: &Obj, kind: &str) -> String {
        self.listed(page, kind).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(" ")
    }
    pub(super) fn content(&self, page: &Obj) -> String {
        String::from_utf8(self.stream(page, "Contents").to_vec()).expect("a page's content is ASCII")
    }

    /// A font's codes back to text, from its `/ToUnicode`.
    pub(super) fn unicode(&self, font: &Obj) -> HashMap<u8, String> {
        let words = lex(self.stream(font, "ToUnicode"));
        let mut map = HashMap::new();
        for pair in words.split(|w| w.is("endbfchar")).filter_map(|part| part.split(|w| w.is("beginbfchar")).nth(1)).flat_map(|block| block.chunks(2)) {
            let [Obj::Text(code), Obj::Text(to)] = pair else { panic!("{pair:?} in a font's map") };
            let units: Vec<u16> = to.chunks(2).map(|u| u16::from_be_bytes([u[0], u[1]])).collect();
            assert!(code.len() == 1 && to.len().is_multiple_of(2) && map.insert(code[0], String::from_utf16(&units).expect("UTF-16")).is_none(), "code {code:?}");
        }
        map
    }

    /// A font's parts agree: codes 1 to n, each with a name, a width, a
    /// drawing that says that width first, and text to copy as; and the
    /// font's box is the box around its drawings.
    fn check_font(&self, font: &Obj) {
        let widths: Vec<f64> = self.key(font, "Widths").list().iter().map(Obj::num).collect();
        let (n, map, drawings) = (widths.len(), self.unicode(font), self.key(font, "CharProcs"));
        assert_eq!((self.key(font, "FirstChar").num(), self.key(font, "LastChar").num(), map.len(), (1..=255).contains(&n)), (1.0, n as f64, n, true));
        assert!((1..=n).all(|code| map.contains_key(&(code as u8))) && matches!(drawings, Obj::Dict(d) if d.len() == n), "a code with no text, or no drawing");
        assert_eq!(self.key(font, "FontMatrix").list(), [0.001, 0.0, 0.0, 0.001, 0.0, 0.0].map(Obj::Num));
        let names = self.key(self.key(font, "Encoding"), "Differences").list();
        assert_eq!((names.len(), &names[0]), (n + 1, &Obj::Num(1.0)));
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for (name, width) in names[1..].iter().zip(&widths) {
            let ops = lex(self.stream(drawings, name.name()));
            assert!(ops[..2] == [Obj::Num(*width), Obj::Num(0.0)] && ops[2].is("d0"), "a glyph says its width first");
            assert!(ops.len() == 3 || (ops[5].is("m") && ops[ops.len() - 1].is("f")), "an outline moves first and is filled last");
            let mut points = Vec::new();
            for op in &ops[3..] {
                match op {
                    Obj::Word(op) => assert!([("m", 2), ("l", 2), ("c", 6), ("f", 0)].contains(&(op.as_str(), points.len())), "{op} after {points:?}"),
                    number => points.push(number.num()),
                }
                for p in points.drain(..if matches!(op, Obj::Word(_)) { points.len() } else { 0 }).collect::<Vec<_>>().chunks(2) {
                    (lo, hi) = ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]);
                }
            }
        }
        let around = if lo[0] > hi[0] { [0.0; 4] } else { [lo[0].floor(), lo[1].floor(), hi[0].ceil(), hi[1].ceil()] };
        assert_eq!(self.key(font, "FontBBox").list(), around.map(Obj::Num), "the box around the glyphs");
    }

    /// What a page shows as text, a run at a time: its strings through
    /// the `/ToUnicode` of the font current then; where each glyph's pen
    /// is from the run's start, in units, as a viewer adds them up; and
    /// where every stroke of every glyph's drawing ends, from there too.
    pub(super) fn runs(&self, page: &Obj) -> Vec<(String, Vec<f64>, Vec<[f64; 2]>)> {
        let mut runs: Vec<(String, Vec<f64>, Vec<[f64; 2]>)> = Vec::new();
        let (mut map, mut widths, mut inks, mut pen) = (HashMap::new(), Vec::new(), Vec::new(), 0.0);
        // An operator comes after what it works on.
        let words = lex(self.stream(page, "Contents"));
        for (i, op) in words.iter().enumerate() {
            if op.is("BT") {
                runs.push((String::new(), Vec::new(), Vec::new()));
                pen = 0.0;
            } else if op.is("Tf") {
                let font = self.of(&self.listed(page, "Font").iter().find(|(name, _)| name == words[i - 2].name()).expect("a font the page lists").1);
                let ink = |name: &Obj| {
                    let ops = lex(self.stream(self.key(font, "CharProcs"), name.name()));
                    (2..ops.len()).filter(|i| ["m", "l", "c"].iter().any(|stroke| ops[*i].is(stroke))).map(|i| [ops[i - 2].num(), ops[i - 1].num()]).collect::<Vec<_>>()
                };
                inks = self.key(self.key(font, "Encoding"), "Differences").list()[1..].iter().map(ink).collect();
                (map, widths) = (self.unicode(font), self.key(font, "Widths").list().iter().map(Obj::num).collect());
            } else if op.is("TJ") {
                let run = runs.last_mut().expect("text is shown inside BT");
                for piece in words[i - 1].list() {
                    let Obj::Text(codes) = piece else {
                        pen -= piece.num();
                        continue;
                    };
                    for code in codes.iter().map(|code| *code as usize - 1) {
                        run.0.push_str(&map[&(code as u8 + 1)]);
                        run.1.push(pen);
                        run.2.extend(inks[code].iter().map(|p| [p[0] + pen, p[1]]));
                        pen += widths[code];
                    }
                }
            }
        }
        runs
    }
    pub(super) fn text(&self, page: &Obj) -> String {
        self.runs(page).into_iter().map(|run| run.0).collect::<Vec<_>>().join("|")
    }
}

/// The machine's fonts. With none the engine draws nothing, and every
/// test here would pass having checked nothing.
pub(super) fn engine() -> TextEngine {
    let engine = TextEngine::new("Inter", "JetBrains Mono");
    assert!(engine.face_count() > 0, "no fonts on this machine: the PDF writer's text cannot be tested without any");
    engine
}

/// The engine's pen positions of a line, at the size glyphs are made at.
pub(super) fn pens(e: &mut TextEngine, s: &str, style: &TextStyle) -> Vec<(u32, f32)> {
    let mut out = Vec::new();
    e.advances(s, &TextStyle { size: EM, ..style.clone() }, &mut out);
    out
}

/// A title that is not ASCII, as a file holds it: UTF-16 behind its mark.
fn utf16(s: &str) -> Obj {
    Obj::Text([0xFEFF].into_iter().chain(s.encode_utf16()).flat_map(u16::to_be_bytes).collect())
}

#[test]
fn a_document_of_two_pages_holds_together() {
    let mut e = engine();
    let plain = TextStyle::new(12.0);
    let named = if e.has_family("Lora") { plain.clone().family("Lora") } else { plain.clone().italic() };
    let (clear, solid) = (Image::new(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 9, 9, 255]), Image::solid(3, 2, [10, 20, 30, 255]));
    let mut pdf = Pdf::new("Two (odd) pages");
    pdf.page(612.0, 792.0);
    pdf.rect(72.0, 72.0, 468.0, 2.5, 0xFAC800);
    pdf.line(72.0, 90.0, 540.0, 90.25, 0.75, 0x1C1B18);
    pdf.text(&mut e, "A heading", &plain.clone().bold(), 72.0, 120.0, 24.0, 0x1C1B18);
    pdf.text(&mut e, "Plain words under it.", &plain, 72.0, 150.0, 12.0, 0x1C1B18);
    pdf.image(&clear, 72.0, 200.0, 100.0, 50.0);
    pdf.image(&solid, 200.0, 200.0, 150.0, 100.0);
    pdf.page(A4.0, A4.1);
    pdf.text(&mut e, "Set in another family", &named, 72.0, 100.0, 14.0, 0x333333);
    pdf.text(&mut e, "let x = 1;", &plain.clone().mono(), 72.0, 130.0, 11.5, 0x706C64);
    let file = read(&pdf.finish());
    let pages = file.pages();
    assert_eq!((pages.len(), file.size(pages[0]), file.size(pages[1])), (2, (612.0, 792.0), (595.28, 841.89)));
    assert_eq!((file.text(pages[0]).as_str(), file.text(pages[1]).as_str()), ("A heading|Plain words under it.", "Set in another family|let x = 1;"));
    // A page lists what it uses and no more: four looks, a font each, two
    // to a page.
    assert_eq!((file.names(pages[0], "Font").as_str(), file.names(pages[1], "Font").as_str(), file.count("Type3")), ("F1 F2", "F3 F4", 4));
    assert_eq!((file.names(pages[0], "XObject").as_str(), file.names(pages[1], "XObject").as_str(), file.count("Image")), ("Im1 Im2", "", 3));
    // The shapes are on the first page as they were given, y down.
    let content = file.content(pages[0]);
    assert!(content.starts_with("1 0 0 -1 0 792 cm\n0.98 0.784 0 rg\n72 72 468 2.5 re\nf\n0.11 0.106 0.094 RG\n0.75 w\n72 90 m\n540 90.25 l\nS\nBT\n0.11 0.106 0.094 rg\n1 0 0 -1 72 120 Tm\n/F1 24 Tf\n[<"), "{content}");
    assert!(content.ends_with("ET\nq\n100 0 0 -50 72 250 cm\n/Im1 Do\nQ\nq\n150 0 0 -100 200 300 cm\n/Im2 Do\nQ\n"), "{content}");
    // A picture is its colours, and its alpha beside them only when some
    // of it shows through.
    let shown = self::Obj::Dict(file.listed(pages[0], "XObject").to_vec());
    let (clear, solid) = (file.key(&shown, "Im1"), file.key(&shown, "Im2"));
    let mask = file.key(clear, "SMask");
    assert_eq!((file.stream(&shown, "Im1"), file.stream(clear, "SMask"), solid.get("SMask")), (&[255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9][..], &[255, 128, 0, 255][..], None));
    assert!(clear.get("ColorSpace").is_some_and(|space| space.is("DeviceRGB")) && mask.get("ColorSpace").is_some_and(|space| space.is("DeviceGray")));
    assert_eq!([clear, mask, solid].map(|p| (file.key(p, "Width").num(), file.key(p, "Height").num(), file.key(p, "BitsPerComponent").num())), [(2.0, 2.0, 8.0), (2.0, 2.0, 8.0), (3.0, 2.0, 8.0)]);
    assert_eq!((file.title(), file.key(&file.trailer, "Info").get("Producer")), (Some(&Obj::Text(b"Two (odd) pages".to_vec())), Some(&Obj::Text(b"Lantern Notepad".to_vec()))));
}

#[test]
fn text_reads_back_through_its_fonts_as_it_was_written() {
    let mut e = engine();
    let plain = TextStyle::new(12.0);
    let serif = if e.has_family("Lora") { plain.clone().family("Lora") } else { plain.clone() };
    let lines = [("Plain ASCII: (parens) \\ and <angles> 0123456789!", &plain), ("héllo wörld, na\u{ef}ve cafe\u{301}", &plain), ("an office file, affixed", &serif), ("fi", &serif), ("", &plain), ("first line\nnot shown", &plain)];
    let mut pdf = Pdf::new("Titel: héllo \u{1F98A}");
    for (i, (line, style)) in lines.iter().enumerate() {
        pdf.text(&mut e, line, style, 50.0, 60.0 + 20.0 * i as f32, 12.0, 0);
    }
    let file = read(&pdf.finish());
    let page = file.pages()[0];
    assert_eq!(file.text(page), "Plain ASCII: (parens) \\ and <angles> 0123456789!|héllo wörld, na\u{ef}ve cafe\u{301}|an office file, affixed|fi|first line");
    // Whatever the engine makes one cluster of is one glyph: where this
    // family has an fi ligature, a single code copies as both letters.
    let ligature = pens(&mut e, "fi", &serif).len() == 2;
    let fi = file.runs(page).into_iter().find(|run| run.0 == "fi").expect("the run");
    assert_eq!(fi.1.len(), if ligature { 1 } else { 2 }, "fi is {} to the engine", if ligature { "one cluster" } else { "two" });
    // A letter and the mark on it are one glyph, however it was typed.
    assert!(file.unicode(file.of(&file.listed(page, "Font")[0].1)).values().any(|text| text == "e\u{301}"));
    assert_eq!(file.title(), Some(&utf16("Titel: héllo \u{1F98A}")));
}

#[test]
fn a_glyph_and_a_picture_are_kept_once_however_often_they_are_used() {
    let mut e = engine();
    let plain = TextStyle::new(12.0);
    let picture = Image::solid(4, 4, [200, 100, 50, 255]);
    let mut pdf = Pdf::new("Shared");
    pdf.text(&mut e, "aaaaaaaaaa", &plain, 50.0, 50.0, 12.0, 0);
    pdf.text(&mut e, "a a", &plain, 50.0, 70.0, 30.0, 0xFF0000);
    pdf.image(&picture, 50.0, 100.0, 40.0, 40.0);
    pdf.image(&picture.clone(), 100.0, 100.0, 80.0, 80.0);
    pdf.page(A4.0, A4.1);
    pdf.text(&mut e, "aa", &TextStyle::new(40.0), 50.0, 50.0, 9.0, 0);
    pdf.image(&picture, 0.0, 0.0, 10.0, 10.0);
    pdf.text(&mut e, "aa", &plain.clone().bold(), 50.0, 80.0, 9.0, 0);
    let file = read(&pdf.finish());
    let pages = file.pages();
    // One look at every size and on every page is one font: here of two
    // glyphs, the letter and the space. Bold is another look.
    let (first, second) = (file.listed(pages[0], "Font"), file.listed(pages[1], "Font"));
    assert_eq!((first.len(), second.len(), &first[0], file.count("Type3")), (1, 2, &second[0], 2));
    assert_eq!([&first[0], &second[1]].map(|font| file.key(file.of(&font.1), "LastChar").num()), [2.0, 1.0]);
    assert_eq!((file.text(pages[0]).as_str(), file.text(pages[1]).as_str()), ("aaaaaaaaaa|a a", "aa|aa"));
    let content = file.content(pages[0]);
    assert!(content.contains("[<01010101010101010101>] TJ") && content.contains("/F1 30 Tf\n[<010201>] TJ"), "{content}");
    // The picture is one object, drawn three times.
    assert_eq!((file.count("Image"), file.listed(pages[0], "XObject"), content.matches("/Im1 Do").count()), (1, file.listed(pages[1], "XObject"), 2));
}

#[test]
fn an_empty_document_is_one_blank_a4_page() {
    let file = read(&Pdf::new("").finish());
    let pages = file.pages();
    assert_eq!((pages.len(), file.size(pages[0]), file.content(pages[0]).as_str()), (1, (595.28, 841.89), "1 0 0 -1 0 841.89 cm\n"));
    assert_eq!((file.key(pages[0], "Resources"), file.objects.len(), file.title()), (&Obj::Dict(Vec::new()), 5, Some(&Obj::Text(Vec::new()))));
    // Drawing before any page was started starts an A4 one.
    let mut pdf = Pdf::new("No page");
    pdf.rect(10.0, 10.0, 20.0, 20.0, 0x102030);
    let file = read(&pdf.finish());
    assert_eq!(file.pages().iter().map(|p| file.size(p)).collect::<Vec<_>>(), [(595.28, 841.89)]);
}

#[test]
fn what_is_not_a_number_never_reaches_the_file() {
    let mut e = engine();
    let style = TextStyle::new(f32::NAN);
    let (nan, inf) = (f32::NAN, f32::INFINITY);
    let mut pdf = Pdf::new("Odd\tinputs\n");
    pdf.page(nan, -5.0);
    pdf.page(inf, 0.25);
    pdf.page(1.0e30, 3.0);
    pdf.rect(nan, inf, -1.0e30, 1.0e30, 0xFFFF_FFFF);
    pdf.rect(-0.0001, 0.0004, 1.23456, -7.0, 0);
    pdf.line(nan, -inf, -1.0e30, 1.0e-30, nan, 0x80);
    pdf.line(0.0, 0.0, 1.0, 1.0, -3.0, 0);
    assert!(pdf.text(&mut e, "wide", &style, nan, inf, 12.0, 0) > 0.0 && pdf.text(&mut e, "huge", &style, -inf, 1.0e30, 1.0e30, 0).is_finite());
    assert_eq!([nan, inf, 0.0, -4.0].map(|size| pdf.text(&mut e, "no size", &style, 0.0, 0.0, size, 0)), [0.0; 4]);
    // A picture over no room, and pictures that are none, are not drawn.
    let picture = Image::solid(2, 2, [1, 2, 3, 4]);
    for (w, h) in [(nan, 10.0), (10.0, 0.0), (-10.0, 10.0), (0.0001, 10.0)] {
        pdf.image(&picture, 0.0, 0.0, w, h);
    }
    for (width, height) in [(0, 5), (3, 3), (u32::MAX, u32::MAX)] {
        pdf.image(&Image { width, height, rgba: vec![0; 8] }, 0.0, 0.0, 10.0, 10.0);
    }
    pdf.image(&picture, nan, 5.0, 1.0e30, 1.0e30);
    // The reader takes no number with an exponent or more than three
    // decimals, and no word that is not an operator.
    let file = read(&pdf.finish());
    let pages = file.pages();
    assert_eq!(pages.iter().map(|p| file.size(p)).collect::<Vec<_>>(), [(595.28, 841.89), (595.28, 841.89), (14400.0, 3.0)]);
    assert_eq!((file.text(pages[2]).as_str(), file.count("Image"), file.title()), ("wide|huge", 2, Some(&utf16("Odd\tinputs\n"))));
    let content = file.content(pages[2]);
    assert!(content.starts_with("1 0 0 -1 0 3 cm\n1 1 1 rg\n0 0 -1000000000 1000000000 re\nf\n0 0 0 rg\n0 0 1.235 -7 re\nf\n0 0 0.502 RG\n0 w\n0 0 m\n-1000000000 0 l\nS\n0 0 0 RG\n0 w\n"), "{content}");
    assert!(content.contains("1 0 0 -1 0 0 Tm\n/F1 12 Tf") && content.contains("1 0 0 -1 0 1000000000 Tm\n/F1 1000000000 Tf") && content.ends_with("q\n1000000000 0 0 -1000000000 0 1000000000 cm\n/Im1 Do\nQ\n"), "{content}");
}
