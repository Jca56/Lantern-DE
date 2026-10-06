//! Reading a Word document: whatever of it there is a place for here,
//! and nothing refused for what there is not.
//!
//! - A paragraph's style is found through `word/styles.xml`: its id or
//!   its name, or those of a style it is based on, say whether it is a
//!   title, a heading or a quote. Where it is aligned, the list it is
//!   in and how far its first line is in are its own or its style's.
//! - A list's kind is found through `word/numbering.xml`: bullets,
//!   boxes (a bullet that is a box), or numbers of any sort.
//! - A run's looks are only the ones it has itself, not those of a
//!   character style. Black spelled out is no colour of its own: it is
//!   what Word's text is anyway, and could not be seen on a dark page.
//! - A table is its cells' paragraphs one after another; a text box's
//!   come after the paragraph it is in. Text a tracked change took out is
//!   left out, and what one put in is read.
//! - A line break parts a paragraph in two, and a picture is a paragraph
//!   of its own: there is neither inside a paragraph here.
//!
//! Headers, footers, notes, comments, equations and fields' codes are
//! not read. Elements are known by the prefixes Word writes (`w:`, `r:`,
//! `a:`, `wp:`), not by their namespaces.

use std::collections::HashMap;

use super::{EMU, HALF_POINTS, TWIPS};
use crate::doc::{Align, Doc, Font, List, MAX_LEVEL, Para, ParaAttrs, Pictures, Placed, Style, TextAttrs};
use crate::io::xml::{self, Element};
use crate::io::zip;

/// What `w:val` of the element called `name` right inside `e` says.
fn val<'a>(e: &'a Element, name: &str) -> Option<&'a str> {
    e.child(name).and_then(|child| child.attr("w:val"))
}

/// A length in twentieths of a point: a plain number is that already,
/// and a strict document may write one with its unit (`12pt`, `0.5in`).
fn length(value: &str) -> Option<f32> {
    let units = [("pt", 20.0), ("in", 1440.0), ("cm", 566.93), ("mm", 56.693), ("pc", 240.0), ("pi", 240.0)];
    let twips = value.parse::<f32>().ok().or_else(|| units.iter().find_map(|(unit, each)| value.strip_suffix(unit)?.parse::<f32>().ok().map(|n| n * each)))?;
    twips.is_finite().then_some(twips)
}

fn align_of(value: &str) -> Option<Align> {
    match value {
        "left" | "start" => Some(Align::Left),
        "center" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        "both" | "distribute" => Some(Align::Justify),
        _ => None,
    }
}

/// The colour of one of Word's sixteen highlighters.
fn highlighter(name: &str) -> Option<u32> {
    let colors = [("yellow", 0xFFFF00), ("green", 0x00FF00), ("cyan", 0x00FFFF), ("magenta", 0xFF00FF), ("blue", 0x0000FF), ("red", 0xFF0000), ("darkBlue", 0x000080), ("darkCyan", 0x008080), ("darkGreen", 0x008000), ("darkMagenta", 0x800080), ("darkRed", 0x800000), ("darkYellow", 0x808000), ("darkGray", 0x808080), ("lightGray", 0xC0C0C0), ("black", 0x000000), ("white", 0xFFFFFF)];
    colors.iter().find(|(word, _)| *word == name).map(|(_, color)| *color)
}

/// The style a name or an id of Word's stands for, in any case and
/// however it is spaced: `Title`, `heading 1`, `Heading1`, `Quote`. A
/// quote is `Block Text` to Pandoc and `Quotations` to LibreOffice.
fn look_of(word: &str) -> Option<Style> {
    let word: String = word.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    match word.as_str() {
        "title" => Some(Style::Title),
        "quote" | "intensequote" | "blocktext" | "quotations" => Some(Style::Quote),
        _ => match word.strip_prefix("heading")?.parse::<u8>().ok()? {
            1 => Some(Style::Heading1),
            2 => Some(Style::Heading2),
            3..=9 => Some(Style::Heading3),
            _ => None,
        },
    }
}

/// What a run has of its own, from its `w:rPr`.
fn run_attrs(r: &Element) -> TextAttrs {
    let Some(rpr) = r.child("w:rPr") else { return TextAttrs::default() };
    // An element that is there is on, unless it says it is off.
    let on = |name: &str| rpr.child(name).is_some_and(|e| !matches!(e.attr("w:val"), Some("0" | "false" | "off")));
    let color = |hex: &str| u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6);
    // Half points, of which a twentieth of a point is a tenth.
    let size = |v: &str| v.parse::<f32>().ok().or_else(|| length(v).map(|twips| twips / 10.0)).filter(|half| half.is_finite() && *half > 0.0);
    TextAttrs {
        bold: on("w:b"),
        italic: on("w:i"),
        underline: rpr.child("w:u").is_some_and(|u| u.attr("w:val") != Some("none")),
        strike: on("w:strike") || on("w:dstrike"),
        size: val(rpr, "w:sz").and_then(size).map(|half| half / HALF_POINTS),
        font: rpr.child("w:rFonts").and_then(|f| f.attr("w:ascii")).filter(|name| !name.is_empty()).map(Font::named),
        // Black is what Word's text is anyway, and many files say so on
        // every run: kept as a colour of its own it could not be seen on
        // a dark page.
        color: val(rpr, "w:color").and_then(color).filter(|color| *color != 0),
        // White behind text is the page showing through, which text
        // copied from a web page has on every run.
        highlight: val(rpr, "w:highlight").and_then(highlighter).or_else(|| rpr.child("w:shd").and_then(|s| s.attr("w:fill")).and_then(color)).filter(|color| *color != 0xFF_FFFF),
    }
}

/// What a `w:pPr` says, a paragraph's own or a style's, of the things
/// that a paragraph without them takes from its style.
#[derive(Clone, Default)]
struct Props {
    align: Option<Align>,
    /// The `w:numId` of a list, and the level in it.
    list: Option<(String, u8)>,
    /// Room over and under, and how far in the first line is, in
    /// twentieths of a point.
    before: Option<f32>,
    after: Option<f32>,
    first: Option<f32>,
}

impl Props {
    fn of(ppr: Option<&Element>) -> Props {
        let Some(ppr) = ppr else { return Props::default() };
        let (num, spacing) = (ppr.child("w:numPr"), ppr.child("w:spacing"));
        let level = num.and_then(|n| val(n, "w:ilvl")).and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
        Props {
            align: val(ppr, "w:jc").and_then(align_of),
            list: num.and_then(|n| val(n, "w:numId")).map(|id| (id.to_owned(), level)),
            before: spacing.and_then(|s| s.attr("w:before")).and_then(length),
            after: spacing.and_then(|s| s.attr("w:after")).and_then(length),
            first: ppr.child("w:ind").and_then(|i| i.attr("w:firstLine")).and_then(length),
        }
    }
}

/// A paragraph style: the one it is based on, the style here its name or
/// id stands for, and what it gives its paragraphs.
struct Named {
    based: Option<String>,
    look: Option<Style>,
    props: Props,
}

/// `word/styles.xml`, as far as paragraphs go.
#[derive(Default)]
struct Styles {
    named: HashMap<String, Named>,
    /// The style of a paragraph that names none.
    usual: Option<String>,
    /// What the document gives every paragraph, under all styles.
    all: Props,
}

impl Styles {
    fn read(root: &Element) -> Styles {
        let mut styles = Styles { all: Props::of(root.child("w:docDefaults").and_then(|d| d.child("w:pPrDefault")).and_then(|d| d.child("w:pPr"))), ..Styles::default() };
        for style in root.named("w:style").filter(|s| s.attr("w:type").is_none_or(|kind| kind == "paragraph")) {
            let Some(id) = style.attr("w:styleId") else { continue };
            if style.attr("w:default").is_some_and(|v| v != "0" && v != "false") {
                styles.usual = Some(id.to_owned());
            }
            let look = val(style, "w:name").and_then(look_of).or_else(|| look_of(id));
            styles.named.insert(id.to_owned(), Named { based: val(style, "w:basedOn").map(str::to_owned), look, props: Props::of(style.child("w:pPr")) });
        }
        styles
    }

    /// A style and the ones it is based on, nearest first.
    fn line<'a>(&'a self, id: Option<&'a str>) -> impl Iterator<Item = &'a Named> {
        // A style based on itself, through however many others, ends
        // the line where a real one never reaches.
        let (mut next, mut left) = (id.or(self.usual.as_deref()), 32);
        std::iter::from_fn(move || {
            let style = self.named.get(next.filter(|_| left > 0)?)?;
            (next, left) = (style.based.as_deref(), left - 1);
            Some(style)
        })
    }

    /// The style here that a paragraph of the style `id` is of.
    fn look(&self, id: Option<&str>) -> Style {
        let known = id.is_some_and(|id| self.named.contains_key(id));
        let look = if known { self.line(id).find_map(|s| s.look) } else { id.and_then(look_of) };
        look.unwrap_or_default()
    }

    /// The first thing `pick` finds, from a style up through the ones it
    /// is based on to what every paragraph is given.
    fn find<T>(&self, id: Option<&str>, pick: impl Fn(&Props) -> Option<T>) -> Option<T> {
        self.line(id).find_map(|s| pick(&s.props)).or_else(|| pick(&self.all))
    }
}

/// The kinds of list at each level that the `w:lvl` elements say.
fn levels<'a>(lvls: impl Iterator<Item = &'a Element>) -> Vec<(u8, List)> {
    let kind = |lvl: &Element| {
        let mark = val(lvl, "w:lvlText").unwrap_or("");
        match val(lvl, "w:numFmt").unwrap_or("decimal") {
            "none" => List::None,
            "bullet" if mark.contains(['\u{2611}', '\u{2612}']) => List::Check(true),
            "bullet" if mark.contains('\u{2610}') => List::Check(false),
            "bullet" => List::Bullet,
            _ => List::Number,
        }
    };
    lvls.filter_map(|lvl| Some((lvl.attr("w:ilvl")?.parse().ok()?, kind(lvl)))).collect()
}

/// `word/numbering.xml`: each definition's levels, and for each list
/// the definition it is of and the levels it has its own way.
#[derive(Default)]
struct Lists {
    definitions: HashMap<String, Vec<(u8, List)>>,
    lists: HashMap<String, (String, Vec<(u8, List)>)>,
}

impl Lists {
    fn read(root: &Element) -> Lists {
        let definitions = root.named("w:abstractNum").filter_map(|a| Some((a.attr("w:abstractNumId")?.to_owned(), levels(a.named("w:lvl"))))).collect();
        let lists = root.named("w:num").filter_map(|n| Some((n.attr("w:numId")?.to_owned(), (val(n, "w:abstractNumId")?.to_owned(), levels(n.named("w:lvlOverride").filter_map(|o| o.child("w:lvl"))))))).collect();
        Lists { definitions, lists }
    }

    /// What an item of the list `id` is marked with at `level`.
    fn kind(&self, id: &str, level: u8) -> List {
        let Some((definition, own)) = self.lists.get(id) else { return List::None };
        let defined = self.definitions.get(definition).map_or(&[][..], Vec::as_slice);
        own.iter().chain(defined).find(|(l, _)| *l == level).map_or(List::None, |(_, kind)| *kind)
    }
}

/// The part a relationship of the part `from` means by `target`: a path
/// from the package's top when it starts with `/`, else from the folder
/// `from` is in.
fn resolve(from: &str, target: &str) -> String {
    let mut path: Vec<&str> = if target.starts_with('/') { Vec::new() } else { from.split('/').collect() };
    path.pop();
    for step in target.split('/') {
        match step {
            "" | "." => {}
            ".." => {
                path.pop();
            }
            name => path.push(name),
        }
    }
    path.join("/")
}

/// Of the ways a thing is written for programs of different ages, the
/// one that is read: the first choice, or the fallback when it has none.
fn chosen(e: &Element) -> Option<&Element> {
    e.child("mc:Choice").or_else(|| e.child("mc:Fallback"))
}

/// The paragraphs one `w:p` comes to: one, unless a line break or a
/// picture parts it.
struct Parts {
    attrs: ParaAttrs,
    done: Vec<Para>,
    /// The one text is going into: none right after a picture, which
    /// is a paragraph of its own.
    open: Option<Para>,
    /// The paragraphs of the text boxes in it, which come after it.
    boxed: Vec<Para>,
}

impl Parts {
    fn text(&mut self, text: &str, attrs: TextAttrs) {
        let para = self.open.get_or_insert_with(|| Para { attrs: self.attrs, ..Para::default() });
        let at = para.text.len();
        // A paragraph's text has no line ends in it, and the object
        // character only where a picture is.
        para.text.extend(text.chars().filter(|c| *c != '\u{FFFC}').map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c }));
        para.spans.insert(at, para.text.len() - at, attrs);
    }

    /// A line break: what comes after is a paragraph like this one.
    fn part(&mut self) {
        self.done.extend(self.open.take());
        self.open = Some(Para { attrs: self.attrs, ..Para::default() });
    }

    fn picture(&mut self, placed: Placed) {
        self.done.extend(self.open.take().filter(|para| !para.text.is_empty()));
        self.done.push(Para { attrs: self.attrs, ..Para::of_picture(placed) });
    }
}

struct Reader<'a> {
    files: &'a [(String, Vec<u8>)],
    /// The parts the document uses, pictures among them: relationship
    /// id and part.
    media: Vec<(String, String)>,
    styles: Styles,
    lists: Lists,
    pictures: Pictures,
    /// The parts taken in as pictures so far, and the number each got:
    /// none for one that is no picture this reads.
    taken: Vec<(String, Option<u64>)>,
}

/// The part called `name`, however its name is cased.
fn part<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    let same = |a: u8, b: u8| a.eq_ignore_ascii_case(&b) || (a == b'\\' && b == b'/');
    files.iter().find(|(file, _)| file.len() == name.len() && file.bytes().zip(name.bytes()).all(|(a, b)| same(a, b))).map(|(_, bytes)| bytes.as_slice())
}

/// The relationships of the part `from` (the package itself when it is
/// empty): id, the last word of the kind, and the part meant.
fn relationships(files: &[(String, Vec<u8>)], from: &str) -> Vec<(String, String, String)> {
    let (folder, file) = from.rsplit_once('/').map_or(("", from), |(folder, file)| (folder, file));
    let name = format!("{folder}{}_rels/{file}.rels", if folder.is_empty() { "" } else { "/" });
    let Some(root) = part(files, &name).and_then(|bytes| xml::parse_bytes(bytes).ok()) else { return Vec::new() };
    let inside = root.named("Relationship").filter(|r| r.attr("TargetMode") != Some("External"));
    inside.filter_map(|r| Some((r.attr("Id")?.to_owned(), r.attr("Type")?.rsplit('/').next()?.to_owned(), resolve(from, r.attr("Target")?)))).collect()
}

impl Reader<'_> {
    /// What a paragraph has, from its `w:pPr` and its style.
    fn para_attrs(&self, ppr: Option<&Element>) -> ParaAttrs {
        let own = Props::of(ppr);
        let id = ppr.and_then(|ppr| val(ppr, "w:pStyle"));
        let style = self.styles.look(id);
        // A heading that Word numbers through its style has no number
        // here: only body text takes a list from its style.
        let from_style = || self.styles.find(id, |p| p.list.clone()).filter(|_| matches!(style, Style::Body | Style::Quote));
        let (list, level) = own.list.clone().or_else(from_style).map_or((List::None, 0), |(list, level)| (self.lists.kind(&list, level), level.min(MAX_LEVEL)));
        // Room of its own is what it has over its style's.
        let room = |own: Option<f32>, pick: fn(&Props) -> Option<f32>| own.map_or(0.0, |twips| ((twips - self.styles.find(id, pick).unwrap_or(0.0)) / TWIPS).max(0.0));
        ParaAttrs {
            style,
            align: own.align.or_else(|| self.styles.find(id, |p| p.align)).unwrap_or_default(),
            list,
            level: if list == List::None { 0 } else { level },
            space_before: room(own.before, |p| p.before),
            space_after: room(own.after, |p| p.after),
            first_indent: own.first.or_else(|| self.styles.find(id, |p| p.first)).map_or(0.0, |twips| (twips / TWIPS).max(0.0)),
        }
    }

    /// The number of the picture a relationship means, taken in once.
    fn picture(&mut self, rel: &str) -> Option<u64> {
        let name = &self.media.iter().find(|(id, _)| id == rel)?.1;
        if let Some((_, id)) = self.taken.iter().find(|(taken, _)| taken == name) {
            return *id;
        }
        let id = part(self.files, name).and_then(|bytes| self.pictures.add(bytes.to_vec()).ok());
        self.taken.push((name.clone(), id));
        id
    }

    /// A drawing, new or old: each picture in it a paragraph of its own,
    /// and the paragraphs of any text box in it.
    fn drawing(&mut self, drawing: &Element, parts: &mut Parts) {
        let wide = drawing.find("wp:extent").and_then(|e| e.attr("cx")).and_then(|cx| cx.parse::<f64>().ok()).map_or(0.0, |emu| (emu / EMU) as f32);
        let (mut shown, mut boxes) = (Vec::new(), Vec::new());
        drawing.walk(&mut |e| {
            match e.name.as_str() {
                "a:blip" => shown.extend(e.attr("r:embed")),
                "v:imagedata" => shown.extend(e.attr("r:id")),
                "w:txbxContent" => boxes.push(e),
                _ => {}
            }
            // A text box is read as paragraphs, below; a fallback is the
            // same shape again, drawn the old way for old programs.
            !matches!(e.name.as_str(), "w:txbxContent" | "mc:Fallback")
        });
        for rel in shown {
            let Some(id) = self.picture(rel) else { continue };
            // As wide as it is anyway is no width of its own.
            let own = self.pictures.get(id).map_or(0.0, |p| p.image.width as f32);
            let width = if wide.is_finite() && wide > 0.0 && (wide - own).abs() >= 0.5 { (wide * 100.0).round() / 100.0 } else { 0.0 };
            parts.picture(Placed { id, width });
        }
        for content in boxes {
            self.blocks(content, &mut parts.boxed);
        }
    }

    /// What is inside a run.
    fn run(&mut self, e: &Element, attrs: TextAttrs, parts: &mut Parts) {
        for child in e.elements() {
            match child.name.as_str() {
                "w:t" => parts.text(&child.text(), attrs),
                "w:tab" | "w:ptab" => parts.text("\t", attrs),
                "w:noBreakHyphen" => parts.text("\u{2011}", attrs),
                "w:cr" => parts.part(),
                // A page ends between paragraphs here, so its break with
                // nothing before it in the paragraph parts nothing.
                "w:br" if matches!(child.attr("w:type"), Some("page" | "column")) && parts.open.as_ref().is_none_or(|para| para.text.is_empty()) => {}
                "w:br" => parts.part(),
                "w:drawing" | "w:pict" | "w:object" => self.drawing(child, parts),
                "mc:AlternateContent" => {
                    if let Some(way) = chosen(child) {
                        self.run(way, attrs, parts);
                    }
                }
                _ => {}
            }
        }
    }

    /// The runs in a paragraph, or in anything in one that holds runs: a
    /// link, a smart tag, a field, what a tracked change put in.
    fn inline(&mut self, e: &Element, parts: &mut Parts) {
        for child in e.elements() {
            match child.name.as_str() {
                "w:r" => self.run(child, run_attrs(child), parts),
                // Its settings, and what a tracked change took out.
                "w:pPr" | "w:del" | "w:moveFrom" => {}
                "mc:AlternateContent" => {
                    if let Some(way) = chosen(child) {
                        self.inline(way, parts);
                    }
                }
                _ => self.inline(child, parts),
            }
        }
    }

    /// Every paragraph in `e`, in reading order: a table, its rows and
    /// its cells are gone into like anything else that holds paragraphs.
    fn blocks(&mut self, e: &Element, out: &mut Vec<Para>) {
        for child in e.elements() {
            match child.name.as_str() {
                "w:p" => {
                    let attrs = self.para_attrs(child.child("w:pPr"));
                    let mut parts = Parts { attrs, done: Vec::new(), open: Some(Para { attrs, ..Para::default() }), boxed: Vec::new() };
                    self.inline(child, &mut parts);
                    out.append(&mut parts.done);
                    out.extend(parts.open);
                    out.append(&mut parts.boxed);
                }
                // Settings, and what a tracked change took out.
                "w:sectPr" | "w:tblPr" | "w:tblGrid" | "w:trPr" | "w:tcPr" | "w:sdtPr" | "w:sdtEndPr" | "w:del" | "w:moveFrom" => {}
                "mc:AlternateContent" => {
                    if let Some(way) = chosen(child) {
                        self.blocks(way, out);
                    }
                }
                _ => self.blocks(child, out),
            }
        }
    }
}

/// Read a Word document. An error is for a person: it says the file is
/// no Word document, or is one that is broken.
pub fn read(bytes: &[u8]) -> Result<Doc, String> {
    let files = zip::read(bytes).map_err(|why| match bytes {
        // Word's files from before 2007, and the ones it locks with a
        // password, are not zips but compound files.
        [0xD0, 0xCF, 0x11, 0xE0, ..] => "This is a Word document of the old kind (.doc) or one locked with a password, and neither can be read.".to_owned(),
        [b'P', b'K', ..] => format!("This Word document is damaged ({why})."),
        _ => format!("This doesn't look like a Word document ({why})."),
    })?;
    // The package says which part the document is; most call it this.
    let usual = "word/document.xml".to_owned();
    let said = relationships(&files, "").into_iter().find(|(_, kind, _)| kind == "officeDocument").map(|(.., name)| name);
    let main = said.filter(|name| part(&files, name).is_some()).unwrap_or(usual);
    let text = part(&files, &main).ok_or("This doesn't look like a Word document (there is no word/document.xml in it).")?;
    let root = xml::parse_bytes(text).map_err(|why| format!("This Word document is damaged ({why})."))?;
    // The parts beside it that it says it uses, or the usual ones. One
    // that is missing or broken is done without.
    let uses = relationships(&files, &main);
    let beside = |kind: &str| {
        let name = uses.iter().find(|(_, k, _)| k == kind).map_or_else(|| resolve(&main, &format!("{kind}.xml")), |(.., name)| name.clone());
        part(&files, &name).and_then(|bytes| xml::parse_bytes(bytes).ok())
    };
    let (styles, lists) = (beside("styles").map_or_else(Styles::default, |root| Styles::read(&root)), beside("numbering").map_or_else(Lists::default, |root| Lists::read(&root)));
    let media = uses.iter().map(|(id, _, name)| (id.clone(), name.clone())).collect();
    let mut reader = Reader { files: &files, media, styles, lists, pictures: Pictures::default(), taken: Vec::new() };
    let mut paras = Vec::new();
    reader.blocks(root.child("w:body").unwrap_or(&root), &mut paras);
    let mut doc = Doc::from_paras(paras);
    doc.pictures = reader.pictures;
    Ok(doc)
}
