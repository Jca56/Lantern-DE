use super::*;
use crate::doc::{Para, Placed};

fn engine() -> TextEngine {
    TextEngine::new("Inter", "JetBrains Mono")
}

/// How many pages a PDF says it has.
fn count_in(file: &[u8]) -> usize {
    let text = String::from_utf8_lossy(file);
    text.matches("/Type /Page").count() - text.matches("/Type /Pages").count()
}

#[test]
fn rows_go_on_pages_whole_and_a_heading_stays_with_its_text() {
    let mut text = engine();
    // Enough short paragraphs for three pages and a bit.
    let mut doc = Doc::from_text(&(1..=120).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n"));
    let (layout, _) = set_for(&mut text, &doc, Paper::A4);
    let room = room(Paper::A4).1 * FINE;
    let starts = page_starts(&doc, &layout, room);
    assert!(starts.len() >= 3 && starts[0] == 0.0, "{starts:?}");
    // Every page starts at a paragraph's top, and none is over-full.
    let row_h = layout.sets[0].rows[0].height;
    for pair in starts.windows(2) {
        assert!(layout.tops.contains(&pair[1]), "{pair:?}");
        assert!(pair[1] - pair[0] <= room && pair[1] - pair[0] > room - row_h * 1.01, "a page is as full as it gets: {pair:?} of {room}");
    }
    // The paragraph before the second page's first is made a heading:
    // now it goes over the leaf with what it heads.
    let first = layout.tops.iter().position(|top| *top == starts[1]).unwrap();
    doc.set_paras(first - 1, first - 1, |a| a.style = Style::Heading2);
    let (layout, setting) = set_for(&mut text, &doc, Paper::A4);
    let again = page_starts(&doc, &layout, room);
    let heading_top = layout.tops[first - 1] + layout.sets[first - 1].rows[0].top;
    assert!(again[1] <= heading_top && again[1] > layout.tops[first - 2], "{} then {heading_top}", again[1]);
    // Dealt out, every mark is on its page and inside it.
    let dealt = dealt(&mut text, &doc, &layout, &setting, &again, room);
    assert_eq!(dealt.len(), again.len());
    for (n, puts) in dealt.iter().enumerate() {
        assert!(!puts.is_empty(), "page {n} is blank");
        for put in puts {
            let y = match put {
                Put::Text { baseline, .. } => *baseline,
                Put::Rect { y, h, .. } | Put::Picture { y, h, .. } => y + h,
                Put::Line { y0, y1, .. } => y0.max(*y1),
            };
            assert!((0.0..=room + 1.0).contains(&y), "page {n}: {y} of {room}");
        }
    }
    let words = |page: &[Put]| page.iter().filter_map(|put| if let Put::Text { text, .. } = put { Some(text.clone()) } else { None }).collect::<Vec<_>>();
    assert_eq!(words(&dealt[1])[..2], [format!("line {first}"), format!("line {}", first + 1)], "the heading leads its page");
}

#[test]
fn a_quote_runs_over_the_leaf_and_a_tall_picture_fits_a_page() {
    let mut text = engine();
    // One long quote: its bar is on every page it is on.
    let mut doc = Doc::from_text(&"said and said again ".repeat(900));
    doc.set_paras(0, 0, |a| a.style = Style::Quote);
    let tall = lntrn_image::encode_png(&lntrn_image::Image::solid(400, 3000, [9, 9, 9, 255]));
    let id = doc.pictures.add(tall).unwrap();
    doc.insert_paras(doc.end(), &[Para::of_picture(Placed { id, width: 0.0 })]);
    let (layout, setting) = set_for(&mut text, &doc, Paper::LETTER);
    let room = room(Paper::LETTER).1 * FINE;
    let starts = page_starts(&doc, &layout, room);
    let dealt = dealt(&mut text, &doc, &layout, &setting, &starts, room);
    assert!(dealt.len() >= 3, "{}", dealt.len());
    let bars: Vec<f32> = dealt.iter().filter_map(|page| page.iter().find_map(|put| if let Put::Rect { y, h, .. } = put { Some(y + h) } else { None })).collect();
    assert_eq!(bars.len(), dealt.len() - 1, "a bar on each page of the quote");
    assert!(bars.iter().all(|bottom| *bottom <= room + 1.0));
    // The empty paragraph a picture leaves after it turns no page.
    let Some(Put::Picture { y, w, h, .. }) = dealt.last().and_then(|page| page.last()) else { panic!("the picture is last, on {} pages", dealt.len()) };
    assert!(*y == 0.0 && (h - room).abs() < 1.0 && (w / h - 400.0 / 3000.0).abs() < 0.001, "a page of its own, whole: {y} {w}x{h}");
}

#[test]
fn a_document_is_written_as_a_pdf() {
    let mut text = engine();
    let mut doc = Doc::from_text("Shopping\nmilk\neggs\nBack before dark");
    doc.set_paras(0, 0, |a| a.style = Style::Title);
    doc.set_paras(1, 2, |a| a.list = crate::doc::List::Check(false));
    doc.format(crate::doc::Pos::new(3, 0), crate::doc::Pos::new(3, 4), |a| (a.bold, a.highlight) = (true, Some(0xfff0a0)));
    let (file, count) = pages(&mut text, &doc, "Shopping", Paper::A4);
    assert!(file.starts_with(b"%PDF-1.") && file.ends_with(b"%%EOF\n"), "a whole file");
    assert_eq!((count, count_in(&file)), (1, 1));
    // A long one is paged, and every page is its paper's size.
    let long = Doc::from_text(&(1..=200).map(|n| format!("paragraph {n}")).collect::<Vec<_>>().join("\n"));
    let (file, count) = pages(&mut text, &long, "Long", Paper::LETTER);
    assert!(count >= 4 && count_in(&file) == count, "{count} and {}", count_in(&file));
    assert_eq!(String::from_utf8_lossy(&file).matches("/MediaBox [0 0 612 792]").count(), count);
    // An empty document is one blank page, not no file; and the empty
    // lines a document ends with don't make it a page longer.
    let (file, count) = pages(&mut text, &Doc::from_text(""), "", Paper::A4);
    assert_eq!((count, count_in(&file)), (1, 1));
    let trailing = Doc::from_text(&format!("{}{}", "word\n".repeat(30), "\n".repeat(60)));
    assert_eq!(pages(&mut text, &trailing, "", Paper::A4).1, 1);
}
