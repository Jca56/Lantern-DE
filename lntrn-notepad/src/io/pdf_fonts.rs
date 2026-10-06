//! The fonts of a PDF: Type 3 fonts drawn from the text engine's outlines.
//!
//! A viewer handed one of this machine's font files would not draw what
//! the engine draws (a variable font's bold is the engine's making, and a
//! letter a family lacks comes from another), so no font file goes in.
//! Each glyph is the outline the engine gives, kept as a small drawing,
//! and a font is up to 255 of them under the codes 1 to 255, with their
//! widths, the box around them and a `/ToUnicode` map from each code back
//! to its text.
//!
//! - A look is everything of a style but its size. It has a font of its
//!   own, and one more for every 255 glyphs after that.
//! - A glyph is a cluster: the text between two neighbouring pen
//!   positions of a line that are also the edges of graphemes. A ligature
//!   is one glyph and copies as its letters; a letter and the marks on it
//!   are one. Its drawing is what the engine draws for it alone, and the
//!   same cluster in the same look is the same glyph wherever it is shown.
//! - The engine shapes a line whole, so a cluster among its neighbours
//!   can be drawn differently than alone (a programming font's `!=`, the
//!   colon of `12:30`, joined Arabic, anything right to left). So every
//!   line is also drawn whole and its clusters are looked for in that.
//!   A stretch of them that is not found there is drawn as the line has
//!   it, all by the glyph of its first cluster; the others are glyphs
//!   with nothing drawn, so each still copies as its own text. Drawn the
//!   same way elsewhere, they are those glyphs again. What a file shows
//!   is always the line as the engine drew it whole.
//! - A colour glyph (an emoji) has no outline. Its glyph is only a
//!   width, so it still copies as text, and its picture is drawn over it.

use std::collections::HashMap;

use lntrn_image::Image;
use lntrn_text::unicode::{BidiClass, bidi_class, graphemes};
use lntrn_text::{PathCmd, PlacedGlyph, TextEngine, TextStyle};

use super::{File, N, Pictures, code_map, flate, milli, utf16_hex};

/// The size glyphs are asked for at: a pixel of it is a unit of glyph
/// space, a thousand to the em.
pub(super) const EM: f32 = 1000.0;

/// The size an emoji's picture is asked for at: the one Noto Color Emoji
/// keeps its pictures at, so they come through as they were drawn. Whole
/// lines are drawn at it too, so that their emoji cost no more to ask for.
const EMOJI: f32 = 109.0;

/// A line as wide as this never wraps.
const WIDE: f32 = 1.0e9;

/// The glyphs a font holds, under the codes 1 to 255. Code 0 is left
/// alone: a zero byte ends a string for too many readers.
const CODES: usize = 255;

/// How far apart two outlines may be, in units, and still be the same
/// one: more than the rounding of a point far along a line, far less than
/// any difference in shape or place.
const NEAR: f32 = 0.1;

/// The most UTF-16 units a code maps back to: what one entry of a
/// `/ToUnicode` map holds.
const MAX_TEXT: usize = 256;

/// A step of what the engine draws, in glyph space (y up, the origin on
/// the baseline at the pen). Of an outline: `m`ove, `l`ine, `c`urve or
/// `f`ill, as a PDF path names them. A curve is its two controls and its
/// end; the others hold one point three times (a fill the one before
/// it). A colour glyph is one step, `p`: its `picture`, by its top-left
/// corner, the opposite one, and the first again. So every step shifts,
/// measures and compares alike.
#[derive(Clone, Copy)]
struct Step {
    op: char,
    at: [[f32; 2]; 3],
    picture: usize,
}

/// What a glyph is made from: what the engine draws for a cluster alone,
/// or what it stands for of a stretch of a line drawn as something else.
struct Shape {
    steps: Vec<Step>,
    width: f32,
    /// The font it is in and its code there, once it has been shown.
    place: Option<(usize, u8)>,
    /// How much of the name it is kept under is its text, in bytes.
    text: usize,
}

/// A look, and the shapes met in it. A cluster's alone is kept under its
/// text; one of a stretch under its text, a zero byte and a number, as
/// one text can be drawn in more than one way.
struct Look {
    /// The style at [`EM`]: every size of it is this look.
    em: TextStyle,
    shapes: HashMap<String, Shape>,
}

/// A font: its look, and the name of each of its glyphs' shapes, code 1
/// first.
struct Font {
    look: usize,
    glyphs: Vec<String>,
}

/// A glyph of a run: where its origin is from the run's start, in units.
pub(super) struct Glyph {
    pub font: usize,
    pub code: u8,
    pub x: f32,
    pub width: f32,
}

/// A run of text as its glyphs and the pictures over them (its emoji),
/// in units from the run's start on its baseline. A picture is the
/// document's number for it, its left edge, how far its top is above the
/// baseline, and its size.
pub(super) struct Run {
    pub glyphs: Vec<Glyph>,
    pub pictures: Vec<(usize, [f32; 4])>,
    pub width: f32,
}

#[derive(Default)]
pub(super) struct Fonts {
    looks: Vec<Look>,
    fonts: Vec<Font>,
}

/// Where the glyphs of a line start: the engine's pen positions at
/// `size` that are also between two graphemes, as `(byte, x)` from
/// `(0, 0)` to `(len, width)`, x in units.
fn boundaries(engine: &mut TextEngine, s: &str, em: &TextStyle, size: f32) -> Vec<(usize, f32)> {
    let mut pens = Vec::new();
    engine.advances(s, &TextStyle { size, ..em.clone() }, &mut pens);
    let mut ends = vec![(0, 0.0)];
    let (mut edge, mut at) = (0, 0);
    for grapheme in graphemes(s) {
        edge += grapheme.len();
        while pens.get(at).is_some_and(|pen| (pen.0 as usize) < edge) {
            at += 1;
        }
        if let Some(pen) = pens.get(at).filter(|pen| pen.0 as usize == edge && edge < s.len()) {
            ends.push((edge, pen.1 * EM / size));
        }
    }
    if !s.is_empty() {
        ends.push((s.len(), pens.last().map_or(0.0, |pen| pen.1 * EM / size)));
    }
    ends
}

/// What the engine draws for `text` set on its own at `size`, in glyph
/// space. Its colour glyphs are pictures of the document at the size
/// those are kept at, and left out at any other.
fn trace(engine: &mut TextEngine, pictures: &mut Pictures, text: &str, em: &TextStyle, size: f32) -> Vec<Step> {
    let style = TextStyle { size, ..em.clone() };
    let mut placed = Vec::new();
    engine.place_outlines(text, &style, 0.0, 0.0, WIDE, &mut placed);
    // Pixels run down from the top of the line box; glyph space runs up
    // from the baseline.
    let (ascent, units) = (engine.ascent(&style), EM / size);
    let unit = |p: [f32; 2]| [p[0] * units, (ascent - p[1]) * units];
    // A quadratic's control, as either control of the cubic that draws
    // the same curve: two thirds of the way to it from that end.
    let toward = |end: [f32; 2], control: [f32; 2]| [end[0] + (control[0] - end[0]) * 2.0 / 3.0, end[1] + (control[1] - end[1]) * 2.0 / 3.0];
    let mut steps = Vec::new();
    for glyph in placed {
        let path = match glyph {
            PlacedGlyph::Outline(path) => path,
            PlacedGlyph::Color { x, y, width, height, rgba } => {
                // A picture is taken in only at the size it is kept at.
                let picture = Some(Image { width, height, rgba }).filter(|_| size == EMOJI).and_then(|image| pictures.add(&image));
                steps.extend(picture.map(|picture| Step { op: 'p', at: [unit([x, y]), unit([x + width as f32, y + height as f32]), unit([x, y])], picture }));
                continue;
            }
        };
        // Where the path is, in pixels. A path that draws before it has
        // moved anywhere starts where it first draws to.
        let mut from: Option<[f32; 2]> = None;
        for cmd in path {
            let (op, at) = match (cmd, from) {
                (PathCmd::Move(p), _) | (PathCmd::Line(p) | PathCmd::Quad(_, p) | PathCmd::Cubic(_, _, p), None) => ('m', [p; 3]),
                (PathCmd::Line(p), Some(_)) => ('l', [p; 3]),
                (PathCmd::Quad(control, p), Some(start)) => ('c', [toward(start, control), toward(p, control), p]),
                (PathCmd::Cubic(a, b, p), Some(_)) => ('c', [a, b, p]),
            };
            from = Some(at[2]);
            steps.push(Step { op, at: at.map(unit), picture: 0 });
        }
        // Each of the engine's glyphs is filled on its own, as the engine
        // fills it: two that overlap must not cut a hole in each other.
        if let Some(last) = steps.last().filter(|_| from.is_some()) {
            steps.push(Step { op: 'f', ..*last });
        }
    }
    steps
}

/// Whether a step is the last of one of the engine's glyphs.
fn ends_glyph(step: &Step) -> bool {
    matches!(step.op, 'f' | 'p')
}

/// Whether `steps`, moved `x` along, are what a line has from its step
/// `at` on: whole glyphs of it, not the end of a larger one, and to the
/// line's very end when they are its `last` cluster's.
fn stands(line: &[Step], at: usize, steps: &[Step], x: f32, last: bool) -> bool {
    let near = |a: &Step, b: &Step| a.op == b.op && a.picture == b.picture && a.at.iter().zip(&b.at).all(|(p, q)| (p[0] - q[0] - x).abs() <= NEAR && (p[1] - q[1]).abs() <= NEAR);
    let (end, whole) = (at + steps.len(), at == 0 || line.get(at - 1).is_some_and(ends_glyph));
    whole && line.len() >= end && (!last || line.len() == end) && line[at..end].iter().zip(steps).all(|(a, b)| near(a, b))
}

/// A glyph as the content stream a Type 3 font keeps for it: its width,
/// then its outlines, filled (where the path winds around a point, not
/// by counting crossings) in whatever colour is current where it is shown.
fn drawing(shape: &Shape) -> String {
    let mut out = format!("{} 0 d0\n", N(shape.width));
    for step in shape.steps.iter().filter(|step| step.op != 'p') {
        for p in &step.at[match step.op { 'c' => 0, 'f' => 3, _ => 2 }..] {
            out.push_str(&format!("{} {} ", N(p[0]), N(p[1])));
        }
        out.push(step.op);
        out.push('\n');
    }
    out
}

impl Fonts {
    pub(super) fn len(&self) -> usize {
        self.fonts.len()
    }

    /// A run of text as glyphs at their places: what `text` of the writer
    /// shows. Only the first line of `s` is one, as the engine measures it.
    pub(super) fn run(&mut self, engine: &mut TextEngine, pictures: &mut Pictures, s: &str, style: &TextStyle) -> Run {
        let s = s.split('\n').next().unwrap_or("");
        let em = TextStyle { size: EM, ..style.clone() };
        let look = self.looks.iter().position(|look| look.em == em).unwrap_or_else(|| {
            self.looks.push(Look { em: em.clone(), shapes: HashMap::new() });
            self.looks.len() - 1
        });
        let ends = boundaries(engine, s, &em, EM);
        let last = ends.len() - 1;
        let cluster = |k: usize| &s[ends[k].0..ends[k + 1].0];
        let mut run = Run { glyphs: Vec::new(), pictures: Vec::new(), width: ends[last].1 };
        // The line as the engine draws it whole, to find the clusters in,
        // with the pens that drawing was made from: far along a line they
        // are not `ends` to the last bit, and outlines are compared closer
        // than that.
        let line = trace(engine, pictures, s, &em, EMOJI);
        let pens = Some(boundaries(engine, s, &em, EMOJI)).filter(|pens| pens.len() == ends.len()).unwrap_or_else(|| ends.clone());
        // With anything right to left in it, the engine sets a line in
        // another order than it reads in.
        let turned = s.chars().any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL | BidiClass::AN | BidiClass::RLE | BidiClass::RLO | BidiClass::RLI));
        // The cluster to show next, and the steps of the line shown so far.
        let (mut k, mut done) = (0, 0);
        while k < last {
            let alone = &self.shape(engine, pictures, look, cluster(k)).steps;
            if stands(&line, done, alone, pens[k].1, k + 1 == last) {
                done += alone.len();
                self.show(&mut run, look, cluster(k), ends[k].1);
                k += 1;
                continue;
            }
            // Here the line has something else than this cluster as it is
            // alone. The stretch that is drawn differently runs to the
            // next cluster the line does have as it is alone, where it
            // belongs; or, in a line that is set in the order it reads
            // in, to a blank at which the ink parts: some of it wholly
            // before the blank's middle, the next glyph wholly after.
            let (mut next, mut resume) = (last, line.len());
            for j in k + 1..last {
                let alone = &self.shape(engine, pictures, look, cluster(j)).steps;
                let found = if !alone.is_empty() {
                    (done..=line.len().saturating_sub(alone.len())).find(|at| stands(&line, *at, alone, pens[j].1, j + 1 == last))
                } else if turned {
                    None
                } else {
                    let middle = (pens[j].1 + pens[j + 1].1) / 2.0;
                    let before = line[done..].iter().take_while(|step| step.at.iter().all(|p| p[0] <= middle)).count();
                    let parts = done + line[done..done + before].iter().rposition(ends_glyph).map_or(0, |end| end + 1);
                    Some(parts).filter(|at| *at > done && line[*at..].iter().take_while(|step| !ends_glyph(step)).all(|step| step.at.iter().all(|p| p[0] >= middle)))
                };
                if let Some(at) = found {
                    (next, resume) = (j, at);
                    break;
                }
            }
            // The stretch is drawn by the glyph of its first cluster: that
            // cluster's text, with all of the stretch's drawing. Its other
            // clusters are glyphs with nothing drawn, there to be copied.
            // Each is the glyph already made for its text drawn this way,
            // or a new one under the next number.
            for j in k..next {
                let steps: Vec<Step> = line[if j == k { done } else { resume }..resume].iter().map(|step| Step { at: step.at.map(|p| [p[0] - pens[k].1, p[1]]), ..*step }).collect();
                let shapes = &mut self.looks[look].shapes;
                let mut number = 0;
                let name = loop {
                    let name = format!("{}\0{number}", cluster(j));
                    match shapes.get(&name) {
                        Some(known) if !stands(&known.steps, 0, &steps, 0.0, true) => number += 1,
                        _ => break name,
                    }
                };
                if !shapes.contains_key(&name) {
                    shapes.insert(name.clone(), Shape { steps, width: ends[j + 1].1 - ends[j].1, place: None, text: cluster(j).len() });
                }
                self.show(&mut run, look, &name, ends[j].1);
            }
            (k, done) = (next, resume);
        }
        run
    }

    /// The shape of a cluster in a look: what the engine draws for it
    /// alone, asked the first time.
    fn shape(&mut self, engine: &mut TextEngine, pictures: &mut Pictures, look: usize, text: &str) -> &Shape {
        let look = &mut self.looks[look];
        if !look.shapes.contains_key(text) {
            // A cluster with a colour glyph is as it is at the size its
            // picture is kept at; any other, as it is at the full size.
            let small = trace(engine, pictures, text, &look.em, EMOJI);
            let steps = if small.iter().any(|step| step.op == 'p') { small } else { trace(engine, pictures, text, &look.em, EM) };
            let mut pens = Vec::new();
            engine.advances(text, &look.em, &mut pens);
            look.shapes.insert(text.to_owned(), Shape { steps, width: pens.last().map_or(0.0, |pen| pen.1), place: None, text: text.len() });
        }
        &look.shapes[text]
    }

    /// Put the shape of a look kept under `name` into a run at `x`. The
    /// first time it is given the next free code of its look's last font,
    /// or of a new font when that one is full.
    fn show(&mut self, run: &mut Run, look: usize, name: &str, x: f32) {
        let Some(shape) = self.looks[look].shapes.get_mut(name) else { return };
        let (font, code) = match shape.place {
            Some(place) => place,
            None => {
                let font = self.fonts.iter().rposition(|font| font.look == look && font.glyphs.len() < CODES).unwrap_or_else(|| {
                    self.fonts.push(Font { look, glyphs: Vec::new() });
                    self.fonts.len() - 1
                });
                self.fonts[font].glyphs.push(name.to_owned());
                *shape.place.insert((font, self.fonts[font].glyphs.len() as u8))
            }
        };
        run.glyphs.push(Glyph { font, code, x, width: shape.width });
        run.pictures.extend(shape.steps.iter().filter(|step| step.op == 'p').map(|step| (step.picture, [step.at[0][0] + x, step.at[0][1], step.at[1][0] - step.at[0][0], step.at[0][1] - step.at[1][1]])));
    }

    /// Write every font: each glyph's drawing and the map from codes back
    /// to text, then the font itself under the number the pages know it by.
    pub(super) fn write(&self, file: &mut File, numbers: &[usize]) {
        for (font, number) in self.fonts.iter().zip(numbers) {
            let (mut names, mut drawings, mut widths) = (String::new(), String::new(), String::new());
            let mut texts = Vec::new();
            // The box around every point of every outline as it is
            // written, the controls of its curves too, in thousandths: no
            // ink of the font is outside it.
            let mut bounds: Option<[i64; 4]> = None;
            for (i, name) in font.glyphs.iter().enumerate() {
                let (shape, code) = (&self.looks[font.look].shapes[name], i + 1);
                let stream = file.number();
                file.stream(stream, "", &flate(drawing(shape).as_bytes()));
                names.push_str(&format!("/g{code} "));
                drawings.push_str(&format!("/g{code} {stream} 0 R "));
                widths.push_str(&format!("{} ", N(shape.width)));
                // A code maps back to so much text and no more, cut
                // between characters.
                let mut units: Vec<u16> = name[..shape.text].encode_utf16().take(MAX_TEXT).collect();
                if units.last().is_some_and(|unit| (0xD800..0xDC00).contains(unit)) {
                    units.pop();
                }
                texts.push(format!("<{code:02X}> <{}>\n", utf16_hex(units.into_iter())));
                for (x, y) in shape.steps.iter().filter(|step| step.op != 'p').flat_map(|step| step.at).map(|p| (milli(p[0].into()), milli(p[1].into()))) {
                    let b = bounds.unwrap_or([x, y, x, y]);
                    bounds = Some([b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]);
                }
            }
            let unicode = file.number();
            file.stream(unicode, "", &flate(code_map(&texts).as_bytes()));
            // Rounded out to whole units.
            let b = bounds.map_or([0; 4], |b| [b[0].div_euclid(1000), b[1].div_euclid(1000), -(-b[2]).div_euclid(1000), -(-b[3]).div_euclid(1000)]);
            file.object(*number, &format!("<< /Type /Font /Subtype /Type3 /FontBBox [{} {} {} {}] /FontMatrix [0.001 0 0 0.001 0 0] /Resources << >> /Encoding << /Type /Encoding /Differences [1 {names}] >> /CharProcs << {drawings}>> /FirstChar 1 /LastChar {} /Widths [{widths}] /ToUnicode {unicode} 0 R >>", b[0], b[1], b[2], b[3], font.glyphs.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    //! The fonts as a file holds them, read back with the reader of
    //! `pdf_tests.rs`.

    use lntrn_text::{PathCmd, PlacedGlyph, TextStyle};

    use super::super::Pdf;
    use super::super::tests::{engine, pens, read};
    use super::EM;

    #[test]
    fn a_look_with_more_glyphs_than_a_font_holds_spills_into_another() {
        let mut e = engine();
        let style = TextStyle::new(12.0);
        // 300 letters, a space after each: 301 glyphs.
        let line: String = (0x100..0x22C).filter_map(char::from_u32).flat_map(|c| [c, ' ']).collect();
        let mut pdf = Pdf::new("Many");
        pdf.text(&mut e, &line, &style, 10.0, 50.0, 6.0, 0);
        pdf.text(&mut e, "\u{22B} \u{100}", &style, 10.0, 80.0, 6.0, 0);
        let file = read(&pdf.finish());
        let page = file.pages()[0];
        assert_eq!(file.listed(page, "Font").iter().map(|(_, font)| file.key(file.of(font), "LastChar").num()).collect::<Vec<_>>(), [255.0, 46.0]);
        assert_eq!(file.text(page), format!("{line}|\u{22B} \u{100}"));
        // The second run finds its glyphs where the first put them: the last
        // letter in the second font, the space and the first letter in the first.
        let content = file.content(page);
        assert!(content.ends_with("/F2 6 Tf\n[<2E>] TJ\n/F1 6 Tf\n[<0201>] TJ\nET\n"), "{}", &content[content.len() - 80..]);
    }

    #[test]
    fn glyphs_are_as_wide_as_the_engine_says_and_sit_where_it_puts_them() {
        let mut e = engine();
        for style in [TextStyle::new(12.0), TextStyle::new(12.0).bold().italic(), TextStyle::new(12.0).mono()] {
            let line = "AV Ta ffi Wo. Tjeld";
            let mut pdf = Pdf::new("Widths");
            let width = pdf.text(&mut e, line, &style, 50.0, 50.0, 20.0, 0);
            let file = read(&pdf.finish());
            let page = file.pages()[0];
            // The run's width is the engine's, at the size asked for.
            let all = pens(&mut e, line, &style);
            let measured = e.measure(line, &TextStyle { size: 20.0, ..style.clone() });
            assert!(width > 20.0 && (width - all[all.len() - 1].1 * 20.0 / EM).abs() < 1e-3 && (width - measured).abs() < 0.01, "{width} against {measured}");
            // Each glyph's width is the advance of its text alone.
            let font = file.of(&file.listed(page, "Font")[0].1);
            let widths = file.key(font, "Widths").list();
            for (code, text) in file.unicode(font) {
                let alone = pens(&mut e, &text, &style);
                assert!((widths[code as usize - 1].num() - f64::from(alone[alone.len() - 1].1)).abs() < 0.00051, "{text:?} is {:?} wide", widths[code as usize - 1]);
            }
            // And each sits at the pen position the engine gives it in the
            // line, kerning and all, within the slack the writer allows itself.
            let runs = file.runs(page);
            assert_eq!((runs.len(), runs[0].0.as_str(), runs[0].1.len()), (1, line, all.len() - 1));
            for (at, pen) in runs[0].1.iter().zip(&all) {
                assert!((at - f64::from(pen.1)).abs() < 0.0215, "{at} against {pen:?}");
            }
        }
    }

    #[test]
    fn a_line_is_shown_as_the_engine_draws_it_whole() {
        let mut e = engine();
        let (plain, mono) = (TextStyle::new(12.0), TextStyle::new(12.0).mono());
        // A programming font draws `!=` as one sign, a text font sets a
        // colon between figures higher than between words, and words that
        // read right to left are set in another order than they read in.
        let hebrew = "\u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{5E2}\u{5D5}\u{5DC}\u{5DD}";
        let lines = [("if a != b { a = b; }".to_owned(), &mono), ("c!=d != ! -> :: // ...".to_owned(), &mono), ("At 12:30 (A-B): AV Ta ffi, x:y 3x4".to_owned(), &plain), (format!("in {hebrew} out"), &plain), ("a fox \u{1F98A} here".to_owned(), &plain)];
        let mut pdf = Pdf::new("Shaped");
        for (i, (line, style)) in lines.iter().enumerate() {
            pdf.text(&mut e, line, style, 50.0, 50.0 + 20.0 * i as f32, 12.0, 0);
        }
        let file = read(&pdf.finish());
        let page = file.pages()[0];
        let runs = file.runs(page);
        assert_eq!(file.text(page), lines.iter().map(|line| line.0.as_str()).collect::<Vec<_>>().join("|"));
        let mut colour = 0;
        for ((line, style), run) in lines.iter().zip(&runs) {
            // Where every stroke of the engine's own drawing of the line
            // ends, in glyph space: the file's glyphs, each at its pen,
            // end theirs in the same places.
            let em = TextStyle { size: EM, ..(*style).clone() };
            let (mut placed, ascent) = (Vec::new(), e.ascent(&em));
            e.place_outlines(line, &em, 0.0, 0.0, 1.0e9, &mut placed);
            colour += placed.iter().filter(|glyph| matches!(glyph, PlacedGlyph::Color { .. })).count();
            let paths = placed.into_iter().flat_map(|glyph| if let PlacedGlyph::Outline(path) = glyph { path } else { Vec::new() });
            let ends: Vec<[f32; 2]> = paths.map(|(PathCmd::Move(p) | PathCmd::Line(p) | PathCmd::Quad(_, p) | PathCmd::Cubic(_, _, p))| [p[0], ascent - p[1]]).collect();
            assert!(run.2.len() == ends.len() && run.2.iter().zip(&ends).all(|(a, b)| (a[0] - f64::from(b[0])).abs() < 0.2 && (a[1] - f64::from(b[1])).abs() < 0.2), "{line}");
            // And every cluster is still a glyph of its own.
            assert_eq!(run.1.len(), pens(&mut e, line, style).len() - 1, "{line}");
        }
        // The sign is the same glyphs wherever it stands: at most a `!`
        // alone, one that draws the sign, an `=` alone and one that draws
        // nothing.
        let texts = file.unicode(file.of(&file.listed(page, "Font")[0].1));
        assert!(["!", "="].iter().all(|sign| (1..=2).contains(&texts.values().filter(|text| text == sign).count())), "{texts:?}");
        // The emoji is a glyph with no outline, under a picture of it.
        assert_eq!((runs[4].1.len(), file.listed(page, "XObject").len(), file.content(page).matches(" Do\n").count()), (12, colour, colour));
    }
}
