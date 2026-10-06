//! PDF written from positioned text, shapes and pictures.
//!
//! The paging is the caller's: it starts a page and puts rectangles,
//! lines, runs of text and pictures on it, in points from the page's
//! top-left with y down, and `finish` hands back the file.
//!
//! - Text is real text. It is drawn with fonts made here from the text
//!   engine's own outlines (`pdf_fonts.rs`), so it looks as the engine
//!   draws it whatever fonts the reader's machine has, and it selects,
//!   searches and copies through each font's `/ToUnicode`.
//! - A picture is kept once however often it is drawn: RGB, with its
//!   alpha beside it as an `/SMask` only when it has any.
//! - A PDF page measures from its bottom-left with y up. Each page's
//!   content starts by turning that over, once, and everything after it
//!   is written as it was given.
//! - The file is PDF 1.7 with a classic cross-reference table: the
//!   catalog, the page tree, the document's information, each page and
//!   its content, then the pictures and the fonts. Every stream is
//!   deflated.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use lntrn_image::deflate::zlib;
use lntrn_image::{Compression, Image};
use lntrn_text::{TextEngine, TextStyle};

use fonts::{EM, Fonts};

#[path = "pdf_fonts.rs"]
mod fonts;
#[cfg(test)]
#[path = "pdf_tests.rs"]
mod tests;

/// A4 in points: the page a document has when it starts none.
const A4: (f32, f32) = (595.28, 841.89);

/// The longest a page's side may be, in points: 200 inches, past which
/// viewers stop.
const MAX_SIDE: f32 = 14_400.0;

/// The largest number written. Nothing past it is a place on any page,
/// and it still fits where a viewer keeps a whole number.
const MAX_NUMBER: f64 = 1.0e9;

/// How far a glyph may sit from where it belongs before the line is
/// adjusted for it, in thousandths of a unit of glyph space. Under it is
/// only what rounding widths to three decimals leaves, far too little to
/// see; the least a font kerns by is some twenty times as much.
const SLACK: i64 = 20;

/// A number in thousandths, as it is written: rounded, held within
/// [`MAX_NUMBER`], and 0 when it is no number at all.
fn milli(v: f64) -> i64 {
    if v.is_finite() { (v.clamp(-MAX_NUMBER, MAX_NUMBER) * 1000.0).round() as i64 } else { 0 }
}

/// A number as a PDF takes it: at most three decimals and no exponent.
struct N<T>(T);

impl<T: Copy + Into<f64>> fmt::Display for N<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let milli = milli(self.0.into());
        let (sign, whole, part) = (if milli < 0 { "-" } else { "" }, milli.abs() / 1000, milli.abs() % 1000);
        match part {
            0 => write!(f, "{sign}{whole}"),
            _ if part % 100 == 0 => write!(f, "{sign}{whole}.{}", part / 100),
            _ if part % 10 == 0 => write!(f, "{sign}{whole}.{:02}", part / 10),
            _ => write!(f, "{sign}{whole}.{part:03}"),
        }
    }
}

/// A colour 0xRRGGBB as the three numbers a colour operator takes.
fn ink(rgb: u32) -> String {
    let part = |shift: u32| N(f64::from((rgb >> shift) & 0xFF) / 255.0);
    format!("{} {} {}", part(16), part(8), part(0))
}

/// Bytes as a `/FlateDecode` stream holds them.
fn flate(data: &[u8]) -> Vec<u8> {
    zlib(data, Compression::Default)
}

/// UTF-16 in hex, high byte first: what a code of a font maps back to,
/// and a title that is not ASCII.
fn utf16_hex(units: impl Iterator<Item = u16>) -> String {
    units.map(|u| format!("{u:04X}")).collect()
}

/// The map from a font's codes back to text, as a `/ToUnicode` stream has
/// it. `entries` are lines of `<code> <UTF-16>`; a map takes them a
/// hundred at a time.
fn code_map(entries: &[String]) -> String {
    let mut map = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n");
    for block in entries.chunks(100) {
        map.push_str(&format!("{} beginbfchar\n{}endbfchar\n", block.len(), block.concat()));
    }
    map + "endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
}

/// Words as a PDF string: as they are when they are plain ASCII, else as
/// UTF-16 behind its byte order mark.
fn text_string(s: &str) -> String {
    if !s.bytes().all(|b| (0x20..0x7F).contains(&b)) {
        return format!("<FEFF{}>", utf16_hex(s.encode_utf16()));
    }
    let mut out = String::from("(");
    for c in s.chars() {
        if matches!(c, '(' | ')' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out + ")"
}

/// A picture as the file keeps it, already deflated: three bytes of
/// colour a pixel, and a byte of alpha a pixel when any of it shows
/// through.
struct Picture {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

/// The pictures of a document, each kept once.
#[derive(Default)]
struct Pictures {
    list: Vec<Picture>,
    /// A picture's size and a hash of its pixels, to its place in `list`.
    known: HashMap<(u32, u32, u64), usize>,
}

impl Pictures {
    /// Take a picture in, or find it again. `None` for one with no
    /// pixels, or fewer than its size says.
    fn add(&mut self, image: &Image) -> Option<usize> {
        let len = (image.width as usize).checked_mul(image.height as usize)?.checked_mul(4)?;
        let rgba = image.rgba.get(..len).filter(|pixels| !pixels.is_empty())?;
        let mut hash = DefaultHasher::new();
        rgba.hash(&mut hash);
        let key = (image.width, image.height, hash.finish());
        if let Some(&index) = self.known.get(&key) {
            return Some(index);
        }
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let alpha: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
        let alpha = alpha.iter().any(|a| *a < 255).then(|| flate(&alpha));
        self.list.push(Picture { width: image.width, height: image.height, rgb: flate(&rgb), alpha });
        self.known.insert(key, self.list.len() - 1);
        Some(self.list.len() - 1)
    }
}

/// The file as it grows. An object has its number before it is written,
/// so that others can point at it first.
struct File {
    bytes: Vec<u8>,
    /// Where each object starts, by its number. Number 0 is never an
    /// object: it heads the table's list of free ones.
    offsets: Vec<usize>,
}

impl File {
    /// The number of an object still to be written.
    fn number(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len() - 1
    }

    fn object(&mut self, number: usize, body: &str) {
        self.offsets[number] = self.bytes.len();
        self.bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }

    /// An object that is a stream. `dict` is what its dictionary says
    /// besides how the bytes are packed and how many they are.
    fn stream(&mut self, number: usize, dict: &str, deflated: &[u8]) {
        self.offsets[number] = self.bytes.len();
        self.bytes.extend_from_slice(format!("{number} 0 obj\n<< {dict}/Filter /FlateDecode /Length {} >>\nstream\n", deflated.len()).as_bytes());
        self.bytes.extend_from_slice(deflated);
        self.bytes.extend_from_slice(b"\nendstream\nendobj\n");
    }
}

struct Page {
    width: f32,
    height: f32,
    /// What draws it: the operators of its content stream.
    content: String,
    /// The fonts and the pictures it uses, by their place in the document.
    fonts: BTreeSet<usize>,
    pictures: BTreeSet<usize>,
}

/// A PDF as it is being written: its pages so far, and the fonts and
/// pictures they share.
pub struct Pdf {
    title: String,
    pages: Vec<Page>,
    fonts: Fonts,
    pictures: Pictures,
}

impl Pdf {
    pub fn new(title: &str) -> Pdf {
        Pdf { title: title.to_owned(), pages: Vec::new(), fonts: Fonts::default(), pictures: Pictures::default() }
    }

    /// Start a page `width` by `height` points. Everything after goes on
    /// it. Coordinates are points from the page's top-left, y down. A
    /// side that is no size at all is A4's.
    pub fn page(&mut self, width: f32, height: f32) {
        let side = |v: f32, or: f32| if v.is_finite() && v >= 1.0 { v.min(MAX_SIDE) } else { or };
        let (width, height) = (side(width, A4.0), side(height, A4.1));
        // The one place y is turned over.
        let content = format!("1 0 0 -1 0 {} cm\n", N(height));
        self.pages.push(Page { width, height, content, fonts: BTreeSet::new(), pictures: BTreeSet::new() });
    }

    /// The page being drawn on: an A4 one when none was started.
    fn current(&mut self) -> &mut Page {
        if self.pages.is_empty() {
            self.page(A4.0, A4.1);
        }
        let last = self.pages.len() - 1;
        &mut self.pages[last]
    }

    /// A filled rectangle. Colours are 0xRRGGBB.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: u32) {
        let op = format!("{} rg\n{} {} {} {} re\nf\n", ink(rgb), N(x), N(y), N(w), N(h));
        self.current().content.push_str(&op);
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, rgb: u32) {
        let op = format!("{} RG\n{} w\n{} {} m\n{} {} l\nS\n", ink(rgb), N(width.max(0.0)), N(x0), N(y0), N(x1), N(y1));
        self.current().content.push_str(&op);
    }

    /// One run of text in one look: its left edge at `x`, its baseline at
    /// `baseline`, `size` points. `style`'s own size is ignored. Returns
    /// the run's width in points. Only the first line of `s` is shown.
    #[allow(clippy::too_many_arguments)] // a run's words, look, place and ink, by design
    pub fn text(&mut self, engine: &mut TextEngine, s: &str, style: &TextStyle, x: f32, baseline: f32, size: f32, rgb: u32) -> f32 {
        if !(size.is_finite() && size > 0.0) {
            return 0.0;
        }
        let run = self.fonts.run(engine, &mut self.pictures, s, style);
        let scale = size / EM;
        if run.glyphs.is_empty() {
            return run.width * scale;
        }
        // The page is upside down to a font, so the text's own matrix
        // turns it back, at the run's start on its baseline.
        let mut op = format!("BT\n{} rg\n1 0 0 -1 {} {} Tm\n", ink(rgb), N(x), N(baseline));
        let mut font = None;
        // Where the viewer's pen is, in thousandths of a unit of glyph
        // space: it adds up the widths as the fonts give them, rounded.
        // A glyph is put where it belongs from that sum, so nothing
        // carries along the line.
        let mut pen = 0i64;
        // Whether a string of codes is open.
        let mut string = false;
        for glyph in &run.glyphs {
            if font != Some(glyph.font) {
                op.push_str(if font.is_some() { ">] TJ\n" } else { "" });
                op.push_str(&format!("/F{} {} Tf\n[", glyph.font + 1, N(size)));
                (font, string) = (Some(glyph.font), false);
            }
            // `TJ` takes its numbers away from the pen: a kern pulls left.
            let back = pen - milli(glyph.x.into());
            if back.abs() > SLACK {
                op.push_str(&format!("{}{} ", if string { ">" } else { "" }, N(back as f64 / 1000.0)));
                (pen, string) = (pen - back, false);
            }
            op.push_str(&format!("{}{:02X}", if string { "" } else { "<" }, glyph.code));
            (pen, string) = (pen + milli(glyph.width.into()), true);
        }
        op.push_str(">] TJ\nET\n");
        let page = self.current();
        page.content.push_str(&op);
        page.fonts.extend(run.glyphs.iter().map(|glyph| glyph.font));
        for (picture, [left, top, w, h]) in run.pictures {
            self.stamp(picture, x + left * scale, baseline - top * scale, w * scale, h * scale);
        }
        run.width * scale
    }

    /// A picture over a rectangle. The same picture drawn twice is stored
    /// once.
    pub fn image(&mut self, image: &Image, x: f32, y: f32, w: f32, h: f32) {
        if let Some(index) = self.pictures.add(image) {
            self.stamp(index, x, y, w, h);
        }
    }

    /// Draw a picture of the document over a rectangle. A picture's own
    /// space is a unit square with y up, so it is turned over as it is
    /// stretched. Over no room at all nothing is drawn.
    fn stamp(&mut self, index: usize, x: f32, y: f32, w: f32, h: f32) {
        if milli(w.into()) <= 0 || milli(h.into()) <= 0 {
            return;
        }
        let op = format!("q\n{} 0 0 {} {} {} cm\n/Im{} Do\nQ\n", N(w), N(-h), N(x), N(y + h), index + 1);
        let page = self.current();
        page.content.push_str(&op);
        page.pictures.insert(index);
    }

    /// The file. A document with no page has one blank A4 page.
    pub fn finish(mut self) -> Vec<u8> {
        self.current();
        let mut file = File { bytes: b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec(), offsets: vec![0] };
        let (catalog, tree, info) = (file.number(), file.number(), file.number());
        let pages: Vec<(usize, usize)> = self.pages.iter().map(|_| (file.number(), file.number())).collect();
        // Only a picture some page shows is written.
        let shown: BTreeMap<usize, usize> = self.pages.iter().flat_map(|page| &page.pictures).collect::<BTreeSet<_>>().into_iter().map(|index| (*index, file.number())).collect();
        let fonts: Vec<usize> = (0..self.fonts.len()).map(|_| file.number()).collect();

        file.object(catalog, &format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
        let kids: Vec<String> = pages.iter().map(|(page, _)| format!("{page} 0 R")).collect();
        file.object(tree, &format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()));
        file.object(info, &format!("<< /Title {} /Producer (Lantern Notepad) >>", text_string(&self.title)));
        for (page, (number, content)) in self.pages.iter().zip(&pages) {
            let mut resources = String::new();
            if !page.fonts.is_empty() {
                let list: String = page.fonts.iter().map(|font| format!("/F{} {} 0 R ", font + 1, fonts[*font])).collect();
                resources.push_str(&format!("/Font << {list}>> "));
            }
            if !page.pictures.is_empty() {
                let list: String = page.pictures.iter().map(|index| format!("/Im{} {} 0 R ", index + 1, shown[index])).collect();
                resources.push_str(&format!("/XObject << {list}>> "));
            }
            file.object(*number, &format!("<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] /Resources << {resources}>> /Contents {content} 0 R >>", N(page.width), N(page.height)));
            file.stream(*content, "", &flate(page.content.as_bytes()));
        }
        for (index, number) in &shown {
            let picture = &self.pictures.list[*index];
            let head = format!("/Type /XObject /Subtype /Image /Width {} /Height {} /BitsPerComponent 8 ", picture.width, picture.height);
            let mask = picture.alpha.as_ref().map(|alpha| {
                let mask = file.number();
                file.stream(mask, &format!("{head}/ColorSpace /DeviceGray "), alpha);
                format!("/SMask {mask} 0 R ")
            });
            file.stream(*number, &format!("{head}/ColorSpace /DeviceRGB {}", mask.unwrap_or_default()), &picture.rgb);
        }
        self.fonts.write(&mut file, &fonts);

        // The table: where every object starts, ten digits each, and the
        // trailer that says where the table itself is.
        let table = file.bytes.len();
        let count = file.offsets.len();
        let mut tail = format!("xref\n0 {count}\n0000000000 65535 f \n");
        for offset in &file.offsets[1..] {
            tail.push_str(&format!("{offset:010} 00000 n \n"));
        }
        tail.push_str(&format!("trailer\n<< /Size {count} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{table}\n%%EOF\n"));
        file.bytes.extend_from_slice(tail.as_bytes());
        file.bytes
    }
}
