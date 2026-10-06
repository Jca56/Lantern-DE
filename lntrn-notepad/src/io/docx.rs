//! Word documents (`.docx`): a zip of XML parts, of which this writes
//! and reads the ones a document's text, looks, lists and pictures are
//! in. Reading is in `docx_read.rs`.
//!
//! ```text
//! [Content_Types].xml             what kind of thing each part is
//! _rels/.rels                     which part is the document
//! word/document.xml               the paragraphs
//! word/_rels/document.xml.rels    the parts the document uses, by id
//! word/styles.xml                 Title, Heading 1 to 3 and Quote
//! word/numbering.xml              bullets, numbers and boxes to tick
//! word/settings.xml               that it is a document of today's Word
//! word/media/image1.png ...       the pictures that show
//! ```
//!
//! A logical pixel is half a point on paper ([`POINTS`]): body text, 24
//! on the screen, is ordinary 12 point text in Word, and a Word
//! document's 12 point text is 24 here. Word has text sizes in half
//! points, room and indents in twentieths of a point, and a picture's
//! size in EMU, 12700 to the point. All three come from that one number,
//! so text, room and pictures keep their proportions: a size comes back
//! to the nearest pixel, room to the nearest tenth of one.
//!
//! - A paragraph's style is one of Word's named styles, which are given
//!   the size, weight and room the style has here. Room of a paragraph's
//!   own is on top of its style's, and Word has only the sum, so that is
//!   what is written, and reading takes the style's away again.
//! - A list is numbering: one definition each for bullets, numbers and
//!   the two boxes. Every numbered list that starts at 1 here is a list
//!   of its own there, told to start at 1, because Word goes on counting
//!   through a document where [`Doc::number`] starts again.
//! - A highlight is shading, which takes any colour; Word's own
//!   highlighter has sixteen.
//! - A picture is its file as it is when Word reads the kind (PNG, JPEG,
//!   GIF, BMP), a PNG of it otherwise, never wider than the page's text.
//!
//! Inside `w:pPr` and `w:rPr` Word wants its elements in the order its
//! schema lists them, which is the order they are written in here.

use std::borrow::Cow;

use crate::doc::{Align, Doc, List, MAX_LEVEL, Para, ParaAttrs, Placed, Style, TextAttrs};
use crate::io::{xml, zip};
use crate::paper::Paper;
use crate::settings::{BODY, POINTS};

#[path = "docx_read.rs"]
mod read;
#[cfg(test)]
#[path = "docx_tests.rs"]
mod tests;

pub use read::read;

const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const MAIN: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const DOCUMENT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006";
const DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006";
const KIND: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml";

/// Half points, twentieths of a point and EMU to a logical pixel.
const HALF_POINTS: f32 = POINTS * 2.0;
const TWIPS: f32 = POINTS * 20.0;
const EMU: f64 = POINTS as f64 * 12700.0;

/// The page's margins, in twentieths of a point: an inch all round.
const MARGIN: i32 = 1440;

const LEVELS: usize = MAX_LEVEL as usize + 1;

/// The lists every document is given: bullets, boxes to tick, and boxes
/// ticked. The numbered ones come after, in the order they start.
const BULLETS: usize = 1;
const BOXES: usize = 2;
const TICKED: usize = 3;

/// A font that has the two boxes in it, for programs that look no
/// further than the one they are told.
const BOX_FONT: &str = "Segoe UI Symbol";

/// The parts beside the document that it uses, in the order of their
/// relationships' numbers. The pictures' come after.
const BESIDE: [&str; 3] = ["styles", "numbering", "settings"];

/// A style as Word knows it: its id, Word's own name for it, and how
/// far up Word's list of styles it is shown.
fn word_style(style: Style) -> Option<(&'static str, &'static str, u8)> {
    match style {
        Style::Body => None,
        Style::Title => Some(("Title", "Title", 10)),
        Style::Heading1 => Some(("Heading1", "heading 1", 9)),
        Style::Heading2 => Some(("Heading2", "heading 2", 9)),
        Style::Heading3 => Some(("Heading3", "heading 3", 9)),
        Style::Quote => Some(("Quote", "Quote", 29)),
    }
}

/// A size in logical pixels as half points, within what Word takes.
fn half_points(px: f32) -> u32 {
    (px * HALF_POINTS).round().clamp(2.0, 3276.0) as u32
}

/// Logical pixels as twentieths of a point, within what Word takes.
fn twips(px: f32) -> i32 {
    (px * TWIPS).round().clamp(-31680.0, 31680.0) as i32
}

/// The room a style has over and under a paragraph, in twentieths of a
/// point.
fn style_room(style: Style, body: f32) -> (i32, i32) {
    let (size, (over, under)) = (body * style.scale(), style.space());
    (twips(over * size), twips(under * size))
}

/// What a run has of its own, as `w:rPr`.
fn run_props(a: &TextAttrs) -> String {
    let mut out = String::new();
    if let Some(font) = a.font.map(|f| xml::escape(f.name())).filter(|name| !name.is_empty()) {
        out.push_str(&format!("<w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:cs=\"{font}\"/>"));
    }
    for (on, mark) in [(a.bold, "<w:b/><w:bCs/>"), (a.italic, "<w:i/><w:iCs/>"), (a.strike, "<w:strike/>")] {
        if on {
            out.push_str(mark);
        }
    }
    if let Some(color) = a.color {
        out.push_str(&format!("<w:color w:val=\"{:06X}\"/>", color & 0xFF_FFFF));
    }
    if let Some(size) = a.size.filter(|size| size.is_finite()).map(half_points) {
        out.push_str(&format!("<w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/>"));
    }
    if a.underline {
        out.push_str("<w:u w:val=\"single\"/>");
    }
    if let Some(color) = a.highlight {
        out.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{:06X}\"/>", color & 0xFF_FFFF));
    }
    if out.is_empty() { out } else { format!("<w:rPr>{out}</w:rPr>") }
}

/// A run: its text as it is, each tab an element of its own.
fn run(text: &str, attrs: &TextAttrs, out: &mut String) {
    out.push_str("<w:r>");
    out.push_str(&run_props(attrs));
    for (i, piece) in text.split('\t').enumerate() {
        if i > 0 {
            out.push_str("<w:tab/>");
        }
        // A paragraph's text has no line ends, and its object character
        // is a picture's, written as one.
        let piece: String = piece.chars().filter(|c| !matches!(c, '\n' | '\r' | '\u{FFFC}')).collect();
        if !piece.is_empty() {
            out.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t>", xml::escape(&piece)));
        }
    }
    out.push_str("</w:r>");
}

/// What writing the paragraphs gathers for the parts beside them.
struct Writer<'a> {
    doc: &'a Doc,
    body: f32,
    /// The page's width and height, in twentieths of a point.
    page: (i32, i32),
    /// Each picture that shows, once: its number, its file's name and
    /// the file.
    media: Vec<(u64, String, Cow<'a, [u8]>)>,
    /// How many drawings there are so far: each has a number of its own.
    drawings: usize,
    /// The level of each numbered list, in the order they start.
    numbered: Vec<u8>,
    /// The numbered list going on at each level: its place in
    /// `numbered`.
    going: [Option<usize>; LEVELS],
}

impl<'a> Writer<'a> {
    /// The `w:numId` of the list a paragraph is an item of. Numbered
    /// lists are told apart as [`Doc::number`] counts them: one ends at a
    /// paragraph that is no item, at an item less deep, and at an item
    /// of another kind as deep as it is.
    fn list(&mut self, a: &ParaAttrs) -> Option<usize> {
        let level = usize::from(a.level.min(MAX_LEVEL));
        let ended = match a.list {
            List::None => 0,
            List::Number => level + 1,
            List::Bullet | List::Check(_) => level,
        };
        self.going[ended..].fill(None);
        match a.list {
            List::None => None,
            List::Bullet => Some(BULLETS),
            List::Check(false) => Some(BOXES),
            List::Check(true) => Some(TICKED),
            List::Number => {
                let numbered = &mut self.numbered;
                let nth = *self.going[level].get_or_insert_with(|| {
                    numbered.push(level as u8);
                    numbered.len() - 1
                });
                Some(TICKED + 1 + nth)
            }
        }
    }

    /// What a paragraph has, as `w:pPr`.
    fn para_props(&mut self, a: &ParaAttrs) -> String {
        let mut out = String::new();
        if let Some((id, ..)) = word_style(a.style) {
            out.push_str(&format!("<w:pStyle w:val=\"{id}\"/>"));
        }
        if let Some(list) = self.list(a) {
            out.push_str(&format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{list}\"/></w:numPr>", a.level.min(MAX_LEVEL)));
        }
        let (over, under) = style_room(a.style, self.body);
        let room: String = [("w:before", a.space_before, over), ("w:after", a.space_after, under)].iter().filter(|(_, own, _)| *own != 0.0).map(|(side, own, style)| format!(" {side}=\"{}\"", (style + twips(*own)).clamp(0, 31680))).collect();
        if !room.is_empty() {
            out.push_str(&format!("<w:spacing{room}/>"));
        }
        if twips(a.first_indent) > 0 {
            out.push_str(&format!("<w:ind w:firstLine=\"{}\"/>", twips(a.first_indent)));
        }
        out.push_str(match a.align {
            Align::Left => "",
            Align::Center => "<w:jc w:val=\"center\"/>",
            Align::Right => "<w:jc w:val=\"right\"/>",
            Align::Justify => "<w:jc w:val=\"both\"/>",
        });
        if out.is_empty() { out } else { format!("<w:pPr>{out}</w:pPr>") }
    }

    /// A picture as a run holding a drawing set in the line, or nothing
    /// when the document has no such picture.
    fn drawing(&mut self, placed: Placed) -> String {
        let doc = self.doc;
        let Some(picture) = doc.pictures.get(placed.id) else { return String::new() };
        let at = self.media.iter().position(|(id, ..)| *id == placed.id).unwrap_or_else(|| {
            let (kind, file) = match picture.extension() {
                kind @ ("png" | "jpg" | "gif" | "bmp") => (kind, Cow::Borrowed(picture.bytes.as_slice())),
                _ => ("png", Cow::Owned(lntrn_image::encode_png(&picture.image))),
            };
            self.media.push((placed.id, format!("image{}.{kind}", self.media.len() + 1), file));
            self.media.len() - 1
        });
        self.drawings += 1;
        // As wide as asked, or as it is, and never wider than the page's
        // text: Word lets a picture run off the page.
        let (own, tall) = (f64::from(picture.image.width.max(1)), f64::from(picture.image.height.max(1)));
        let wide = if placed.width > 0.0 { f64::from(placed.width) } else { own }.min(f64::from(self.page.0 - 2 * MARGIN) / f64::from(TWIPS));
        let (cx, cy, n, rel, name) = ((wide * EMU).round().max(1.0) as i64, (wide * tall / own * EMU).round().max(1.0) as i64, self.drawings, BESIDE.len() + 1 + at, &self.media[at].1);
        format!(
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:docPr id=\"{n}\" name=\"Picture {n}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
             <a:graphic><a:graphicData uri=\"{DRAWING}/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{n}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"rId{rel}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
             <pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        )
    }

    fn para(&mut self, p: &Para, out: &mut String) {
        out.push_str("<w:p>");
        out.push_str(&self.para_props(&p.attrs));
        match p.picture {
            Some(placed) => out.push_str(&self.drawing(placed)),
            None => p.spans.runs(0, p.text.len()).iter().for_each(|r| run(p.text.get(r.start..r.end).unwrap_or(""), &r.attrs, out)),
        }
        out.push_str("</w:p>");
    }
}

/// `word/styles.xml`: body text's size for everything, and each named
/// style with the size, weight and room it has here.
fn styles(body: f32) -> String {
    let mut out = format!("{HEAD}<w:styles xmlns:w=\"{MAIN}\"><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>", half_points(body));
    for style in Style::ALL {
        let Some((id, name, priority)) = word_style(style) else { continue };
        // A heading is followed by body text, stays with it on a page,
        // and is listed in the document's outline.
        let heading = style.next() != style;
        let outline = name.strip_prefix("heading ").and_then(|n| n.parse::<u8>().ok()).map_or_else(String::new, |n| format!("<w:outlineLvl w:val=\"{}\"/>", n - 1));
        let (over, under) = style_room(style, body);
        out.push_str(&format!("<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/>{}<w:uiPriority w:val=\"{priority}\"/><w:qFormat/>", if heading { "<w:next w:val=\"Normal\"/>" } else { "" }));
        out.push_str(&format!("<w:pPr>{}<w:spacing w:before=\"{over}\" w:after=\"{under}\"/>{outline}</w:pPr><w:rPr>", if heading { "<w:keepNext/>" } else { "" }));
        for (on, mark) in [(style.bold(), "<w:b/><w:bCs/>"), (style.italic(), "<w:i/><w:iCs/>")] {
            if on {
                out.push_str(mark);
            }
        }
        if style.scale() != 1.0 {
            out.push_str(&format!("<w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/>", half_points(body * style.scale())));
        }
        out.push_str("</w:rPr></w:style>");
    }
    out + "</w:styles>"
}

/// `word/numbering.xml`: what each kind of list is marked with at every
/// level, the lists of them every document has, and one more for each
/// numbered list, by the level it is at.
fn numbering(numbered: &[u8]) -> String {
    let mut out = format!("{HEAD}<w:numbering xmlns:w=\"{MAIN}\">");
    let boxes = format!("<w:rPr><w:rFonts w:ascii=\"{BOX_FONT}\" w:eastAsia=\"{BOX_FONT}\" w:hAnsi=\"{BOX_FONT}\" w:cs=\"{BOX_FONT}\"/></w:rPr>");
    for (id, (format, mark, font)) in [("bullet", "\u{2022}", ""), ("decimal", "", ""), ("bullet", "\u{2610}", boxes.as_str()), ("bullet", "\u{2611}", boxes.as_str())].into_iter().enumerate() {
        out.push_str(&format!("<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"multilevel\"/>"));
        for level in 0..LEVELS {
            let text = if mark.is_empty() { format!("%{}.", level + 1) } else { mark.to_owned() };
            out.push_str(&format!("<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr>{font}</w:lvl>", 720 * (level + 1)));
        }
        out.push_str("</w:abstractNum>");
    }
    for (list, definition) in [(BULLETS, 0), (BOXES, 2), (TICKED, 3)] {
        out.push_str(&format!("<w:num w:numId=\"{list}\"><w:abstractNumId w:val=\"{definition}\"/></w:num>"));
    }
    // Lists of one definition are one list to Word, counted through,
    // unless each is told where to start.
    for (nth, level) in numbered.iter().enumerate() {
        out.push_str(&format!("<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"{level}\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>", TICKED + 1 + nth));
    }
    out + "</w:numbering>"
}

/// `body` is the size of body text in logical pixels (the app's
/// setting), which sizes and headings are relative to; `paper` is the
/// sheet its pages are.
pub fn write(doc: &Doc, body: f32, paper: Paper) -> Vec<u8> {
    let body = if body.is_finite() && body > 0.0 { body } else { BODY };
    let page = ((paper.width * 20.0).round() as i32, (paper.height * 20.0).round() as i32);
    let mut w = Writer { doc, body, page, media: Vec::new(), drawings: 0, numbered: Vec::new(), going: [None; LEVELS] };
    let mut document = format!("{HEAD}<w:document xmlns:w=\"{MAIN}\" xmlns:r=\"{DOCUMENT}\" xmlns:wp=\"{DRAWING}/wordprocessingDrawing\" xmlns:a=\"{DRAWING}/main\" xmlns:pic=\"{DRAWING}/picture\"><w:body>");
    for p in &doc.paras {
        w.para(p, &mut document);
    }
    document.push_str(&format!("<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{MARGIN}\" w:right=\"{MARGIN}\" w:bottom=\"{MARGIN}\" w:left=\"{MARGIN}\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>", page.0, page.1));
    // Without this Word takes the document for one of 2007 and says so.
    let settings = format!("{HEAD}<w:settings xmlns:w=\"{MAIN}\"><w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>");
    let mut types = format!("{HEAD}<Types xmlns=\"{PACKAGE}/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>");
    for (kind, mime) in [("png", "image/png"), ("jpg", "image/jpeg"), ("gif", "image/gif"), ("bmp", "image/bmp")] {
        if w.media.iter().any(|(_, name, _)| name.ends_with(kind)) {
            types.push_str(&format!("<Default Extension=\"{kind}\" ContentType=\"{mime}\"/>"));
        }
    }
    types.push_str(&format!("<Override PartName=\"/word/document.xml\" ContentType=\"{KIND}.document.main+xml\"/>"));
    let mut uses = format!("{HEAD}<Relationships xmlns=\"{PACKAGE}/relationships\">");
    for (n, part) in BESIDE.iter().enumerate() {
        types.push_str(&format!("<Override PartName=\"/word/{part}.xml\" ContentType=\"{KIND}.{part}+xml\"/>"));
        uses.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"{DOCUMENT}/{part}\" Target=\"{part}.xml\"/>", n + 1));
    }
    for (n, (_, name, _)) in w.media.iter().enumerate() {
        uses.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"{DOCUMENT}/image\" Target=\"media/{name}\"/>", BESIDE.len() + 1 + n));
    }
    let (types, uses) = (types + "</Types>", uses + "</Relationships>");
    let top = format!("{HEAD}<Relationships xmlns=\"{PACKAGE}/relationships\"><Relationship Id=\"rId1\" Type=\"{DOCUMENT}/officeDocument\" Target=\"word/document.xml\"/></Relationships>");
    let (looks, lists) = (styles(body), numbering(&w.numbered));
    let names: Vec<String> = w.media.iter().map(|(_, name, _)| format!("word/media/{name}")).collect();
    let mut parts: Vec<(&str, &[u8])> = vec![("[Content_Types].xml", types.as_bytes()), ("_rels/.rels", top.as_bytes()), ("word/document.xml", document.as_bytes()), ("word/_rels/document.xml.rels", uses.as_bytes()), ("word/styles.xml", looks.as_bytes()), ("word/numbering.xml", lists.as_bytes()), ("word/settings.xml", settings.as_bytes())];
    parts.extend(names.iter().zip(&w.media).map(|(name, (_, _, file))| (name.as_str(), file.as_ref())));
    zip::write(&parts)
}
