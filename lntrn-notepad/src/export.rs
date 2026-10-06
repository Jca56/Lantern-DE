//! A document as pages: set at a sheet of paper's width, its rows dealt
//! out page by page, and written as a PDF.
//!
//! The setting is the screen's own (`view::layout`) and what goes on the
//! paper is the marks the screen draws (`view::paint`), so a document
//! printed is the document seen; only the inks are paper's. It is set
//! finer than a point ([`FINE`] units to one), so that nothing the
//! layout rounds to a whole pixel shows on paper.

use std::path::Path;

use lntrn_text::{TextEngine, TextStyle};

use crate::doc::{Doc, List, Style};
use crate::io::pdf::Pdf;
use crate::paper::Paper;
use crate::settings::{BODY, POINTS};
use crate::view::layout::{Layout, Setting};
use crate::view::paint::{Inks, Mark, marks};

/// Layout units to a point.
const FINE: f32 = 8.0;
/// The margins, in points: an inch all round.
const MARGIN: f32 = 72.0;
/// The page numbers' size, in points.
const FOLIO: f32 = 10.0;

/// The room for writing on a sheet: across, and down.
fn room(paper: Paper) -> (f32, f32) {
    ((paper.width - MARGIN * 2.0).max(72.0), (paper.height - MARGIN * 2.0).max(72.0))
}

/// A mark kept until its page is written, in layout units from the
/// column's left and the page's first row.
enum Put {
    Rect { x: f32, y: f32, w: f32, h: f32, color: u32 },
    Line { x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: u32 },
    Text { text: String, style: TextStyle, x: f32, baseline: f32, color: u32 },
    Picture { id: u64, x: f32, y: f32, w: f32, h: f32 },
}

/// The document set for `paper`.
fn set_for(text: &mut TextEngine, doc: &Doc, paper: Paper) -> (Layout, Setting) {
    let setting = Setting { width: room(paper).0 * FINE, scale: POINTS * FINE, body: BODY };
    let mut layout = Layout::default();
    layout.sync(text, doc, setting);
    (layout, setting)
}

/// How many of the document's paragraphs are worth paper: all but the
/// empty ones it ends with, which would only turn a page for nothing.
fn worth(doc: &Doc) -> usize {
    doc.paras.iter().rposition(|para| !para.text.is_empty() || para.attrs.list != List::None).map_or(0, |last| last + 1)
}

/// Where each page starts, down the document, in layout units. Rows go
/// on a page whole, and a heading is not left at the foot of one with
/// what it heads over the leaf.
fn page_starts(doc: &Doc, layout: &Layout, room: f32) -> Vec<f32> {
    let heads = |i: usize| matches!(doc.para(i).attrs.style, Style::Title | Style::Heading1 | Style::Heading2 | Style::Heading3);
    let (mut starts, mut start) = (vec![0.0], 0.0);
    for (i, set) in layout.sets.iter().enumerate().take(worth(doc)) {
        for (r, row) in set.rows.iter().enumerate() {
            let top = layout.tops[i] + row.top;
            if top + row.height - start <= room || top <= start {
                continue;
            }
            let heading = (r == 0 && i > 0 && heads(i - 1)).then(|| layout.tops[i - 1] + layout.sets[i - 1].rows[0].top);
            start = heading.filter(|before| *before > start).unwrap_or(top);
            starts.push(start);
        }
    }
    starts
}

/// Every mark of the document, on the page it falls on.
fn dealt(text: &mut TextEngine, doc: &Doc, layout: &Layout, setting: &Setting, starts: &[f32], room: f32) -> Vec<Vec<Put>> {
    let page_of = |y: f32| starts.partition_point(|start| *start <= y).saturating_sub(1);
    let mut pages: Vec<Vec<Put>> = starts.iter().map(|_| Vec::new()).collect();
    for (i, set) in layout.sets.iter().enumerate().take(worth(doc)) {
        marks(text, set, doc, i, layout.tops[i], setting, &Inks::PRINT, &mut |_, mark| match mark {
            // A bar down a quote runs over as many pages as the quote.
            Mark::Rect { x, y, w, h, color } => {
                let (mut y, bottom) = (y, y + h);
                loop {
                    let page = page_of(y + 0.5);
                    let end = starts.get(page + 1).map_or(bottom, |next| next.min(bottom));
                    pages[page].push(Put::Rect { x, y: y - starts[page], w, h: end - y, color });
                    if end >= bottom || end <= y {
                        break;
                    }
                    y = end;
                }
            }
            Mark::Line { x0, y0, x1, y1, width, color } => {
                let page = page_of(y0.min(y1));
                pages[page].push(Put::Line { x0, y0: y0 - starts[page], x1, y1: y1 - starts[page], width, color });
            }
            Mark::Text { text, style, x, baseline, color, .. } => {
                let page = page_of(baseline - 0.5);
                pages[page].push(Put::Text { text: text.to_owned(), style: style.clone(), x, baseline: baseline - starts[page], color });
            }
            // One taller than a page is made to fit one.
            Mark::Picture { id, x, y, w, h } => {
                let page = page_of(y + 0.5);
                let fit = (room / h.max(1.0)).min(1.0);
                pages[page].push(Put::Picture { id, x, y: y - starts[page], w: w * fit, h: h * fit });
            }
        });
    }
    pages
}

/// `doc` as a PDF on `paper`, called `title`: the file, and how many
/// pages it came to.
pub fn pages(text: &mut TextEngine, doc: &Doc, title: &str, paper: Paper) -> (Vec<u8>, usize) {
    let (layout, setting) = set_for(text, doc, paper);
    let room = room(paper).1 * FINE;
    let starts = page_starts(doc, &layout, room);
    let dealt = dealt(text, doc, &layout, &setting, &starts, room);
    let count = dealt.len();
    let at = |v: f32| v / FINE + MARGIN;
    let mut pdf = Pdf::new(title);
    for (n, puts) in dealt.into_iter().enumerate() {
        pdf.page(paper.width, paper.height);
        for put in puts {
            match put {
                Put::Rect { x, y, w, h, color } => pdf.rect(at(x), at(y), w / FINE, h / FINE, color),
                Put::Line { x0, y0, x1, y1, width, color } => pdf.line(at(x0), at(y0), at(x1), at(y1), width / FINE, color),
                Put::Text { text: words, style, x, baseline, color } => drop(pdf.text(text, &words, &style, at(x), at(baseline), style.size / FINE, color)),
                Put::Picture { id, x, y, w, h } => {
                    if let Some(picture) = doc.pictures.get(id) {
                        pdf.image(&picture.image, at(x), at(y), w / FINE, h / FINE);
                    }
                }
            }
        }
        // More than one page, and each says which it is.
        if count > 1 {
            let (label, style) = ((n + 1).to_string(), TextStyle::new(FOLIO * FINE));
            let wide = text.measure(&label, &style) / FINE;
            pdf.text(text, &label, &style, (paper.width - wide) * 0.5, paper.height - MARGIN * 0.5, FOLIO, Inks::PRINT.dim);
        }
    }
    (pdf.finish(), count)
}

/// Write `doc` as a PDF at `path`, on the paper of where this machine
/// is. Returns how many pages it came to.
pub fn pdf(text: &mut TextEngine, doc: &Doc, path: &Path) -> Result<usize, String> {
    let title = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let (bytes, count) = pages(text, doc, &title, Paper::here());
    crate::io::write_whole(path, &bytes).map(|()| count)
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
