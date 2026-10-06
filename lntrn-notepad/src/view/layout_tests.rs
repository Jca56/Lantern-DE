//! Documents set with the real text engine and the machine's fonts.

use lntrn_image::{Image, encode_png};
use lntrn_text::TextEngine;

use super::*;
use crate::doc::{Para, Placed};

fn engine() -> TextEngine {
    TextEngine::new("Inter", "JetBrains Mono")
}

fn set_as(doc: &Doc, width: f32) -> Layout {
    let mut layout = Layout::default();
    assert!(layout.sync(&mut engine(), doc, Setting { width, scale: 1.0, body: 20.0 }));
    layout
}

fn rows(layout: &Layout, para: usize) -> Vec<&str> {
    let set = &layout.sets[para];
    set.rows.iter().map(|r| &set.src.text[r.start..r.end]).collect()
}

#[test]
fn a_paragraph_wraps_at_its_words_and_every_place_is_found_again() {
    let doc = Doc::from_text("alpha beta gamma delta epsilon zeta\n\nSupercalifragilisticexpialidocious-and-then-some more");
    let wide = set_as(&doc, 2000.0);
    assert_eq!(rows(&wide, 0), ["alpha beta gamma delta epsilon zeta"]);
    let whole = wide.sets[0].rows[0].width;
    // Room for about half of it: two or three rows, each ending with the
    // blank it wrapped at, none wider than the column.
    let layout = set_as(&doc, whole * 0.5);
    let wrapped = rows(&layout, 0);
    assert!((2..=4).contains(&wrapped.len()), "{wrapped:?}");
    assert!(wrapped[..wrapped.len() - 1].iter().all(|r| r.ends_with(' ')), "{wrapped:?}");
    assert_eq!(wrapped.concat(), doc.para(0).text);
    assert!(layout.sets[0].rows.iter().all(|r| r.width <= whole * 0.5 + 0.5));
    // A word too long for a row is cut where it has to be; one with
    // hyphens, at a hyphen.
    let narrow = set_as(&doc, whole * 0.25);
    let long = rows(&narrow, 2);
    assert!(long.len() >= 3 && long[0].ends_with('-') || long[0].len() < 34, "{long:?}");
    assert_eq!(long.concat(), doc.para(2).text);
    // An empty paragraph is one row, as tall as a line of body text.
    let empty = &layout.sets[1];
    assert_eq!((empty.rows.len(), empty.rows[0].height, empty.height), (1, 24.0, 24.0));
    assert_eq!(layout.tops, [0.0, layout.sets[0].height, layout.sets[0].height + 24.0, layout.height()]);
    // Each place has a caret, and a click on that caret finds the place:
    // all but the blank a row wraps at, which reads as the row's end.
    for para in 0..3 {
        let (set, text) = (&layout.sets[para], &doc.para(para).text);
        for (byte, _) in text.char_indices().chain([(text.len(), ' ')]) {
            let (x, y, h) = layout.caret(Pos::new(para, byte));
            let r = set.row_of(byte);
            assert_eq!((y, h), (layout.tops[para] + set.rows[r].top, set.rows[r].height));
            assert_eq!(layout.hit(x + 0.01, y + h * 0.5), Pos::new(para, byte), "{para}:{byte} at {x},{y}");
        }
    }
    // Far to the right of a wrapped row is its resting end, before the
    // blank; below everything is the document's end.
    let first = &layout.sets[0].rows[0];
    assert_eq!(layout.hit(9000.0, 5.0), Pos::new(0, first.end - 1));
    assert_eq!(layout.hit(9000.0, 9000.0), doc.end());
    assert_eq!(layout.row_ends(Pos::new(0, 2)), (Pos::new(0, 0), Pos::new(0, first.end - 1)));
    // Down keeps to the x it started from, through a short row; up from
    // the top and down from the bottom go nowhere.
    let (x, _, _) = layout.caret(Pos::new(0, 4));
    let down = layout.step(Pos::new(0, 4), 1, x).unwrap();
    assert_eq!((down.para, layout.sets[0].row_of(down.byte)), (0, 1));
    assert!((layout.caret(down).0 - x).abs() < 12.0);
    let last_row = Pos::new(0, layout.sets[0].rows.last().unwrap().start);
    assert_eq!(layout.step(last_row, 1, x), Some(Pos::new(1, 0)), "onto the empty paragraph");
    assert_eq!((layout.step(Pos::new(0, 0), -1, x), layout.step(doc.end(), 1, x)), (None, None));
    assert_eq!(layout.step(Pos::new(1, 0), -1, 0.0), Some(last_row));
}

#[test]
fn looks_styles_and_alignment_shape_the_rows() {
    let mut doc = Doc::from_text("small BIG small\nA Heading\ncentred\nright\none two three four five six seven eight nine ten\n- item\nquote");
    doc.format(Pos::new(0, 6), Pos::new(0, 9), |a| (a.size, a.bold) = (Some(60.0), true));
    doc.set_paras(1, 1, |a| a.style = Style::Heading1);
    doc.set_paras(2, 2, |a| a.align = Align::Center);
    doc.set_paras(3, 3, |a| a.align = Align::Right);
    doc.set_paras(4, 4, |a| (a.align, a.first_indent) = (Align::Justify, 40.0));
    doc.set_paras(5, 5, |a| (a.list, a.level) = (List::Check(false), 1));
    doc.set_paras(6, 6, |a| a.style = Style::Quote);
    let layout = set_as(&doc, 400.0);
    // A big run makes its row as tall as it needs and sets the baseline
    // the small text beside it sits on.
    let mixed = &layout.sets[0];
    assert_eq!((mixed.rows.len(), mixed.rows[0].pieces.len(), mixed.looks.len()), (1, 3, 2));
    assert_eq!(mixed.rows[0].height, 72.0);
    assert!(mixed.rows[0].baseline > 40.0 && mixed.looks[mixed.rows[0].pieces[0].look].ascent < 22.0);
    let ends: Vec<f32> = mixed.rows[0].pieces.iter().map(|p| p.x + p.width).collect();
    assert!(mixed.rows[0].pieces.windows(2).all(|w| (w[0].x + w[0].width - w[1].x).abs() < 0.01), "pieces follow one another: {ends:?}");
    // A heading is bigger and bold, with room over and under it.
    let heading = &layout.sets[1];
    assert_eq!((heading.looks[0].style.size, heading.rows[0].height), (32.0, 38.4));
    assert!((heading.rows[0].top - 0.55 * 32.0).abs() < 0.01 && (heading.height - (0.8 * 32.0 + 38.4)).abs() < 0.01);
    // Centred and to the right.
    let (centred, right) = (&layout.sets[2].rows[0], &layout.sets[3].rows[0]);
    assert!((centred.x - ((400.0 - centred.width) * 0.5).floor()).abs() < 0.01 && (right.x + right.width - 400.0).abs() < 0.01);
    // Justified: every row but the last reaches both edges, its first
    // starting further in; the last is left as it falls.
    let just = &layout.sets[4];
    assert!(just.rows.len() >= 2, "{:?}", rows(&layout, 4));
    assert_eq!(just.rows[0].x, 40.0);
    for row in &just.rows[..just.rows.len() - 1] {
        assert!((row.x + row.width - 400.0).abs() < 0.5, "{row:?}");
        let last = row.pieces.iter().rev().find(|p| !doc.para(4).text[p.start..p.end].trim().is_empty()).unwrap();
        assert!((last.x + last.width - 400.0).abs() < 0.5);
    }
    let last = just.rows.last().unwrap();
    assert!(last.x == 0.0 && last.width < 399.0);
    // A list item starts past its marker, deeper for each level; a
    // quote past its bar, in italics.
    assert_eq!((layout.sets[5].indent, layout.sets[5].rows[0].x), (60.0, 60.0));
    assert_eq!((layout.sets[6].indent, layout.sets[6].looks[0].style.clone()), (22.0, lntrn_text::TextStyle::new(20.0).italic()));
}

#[test]
fn a_picture_takes_the_room_it_is_given_and_tabs_are_room() {
    let mut doc = Doc::from_paras(vec![Para::plain("a\tb"), Para::default(), Para::default(), Para::default()]);
    let small = doc.pictures.add(encode_png(&Image::solid(100, 50, [9, 9, 9, 255]))).unwrap();
    let huge = doc.pictures.add(encode_png(&Image::solid(3000, 1000, [1, 1, 1, 255]))).unwrap();
    *doc.para_mut(1) = Para::of_picture(Placed { id: small, width: 0.0 });
    *doc.para_mut(2) = Para { attrs: ParaAttrs { align: Align::Center, ..ParaAttrs::default() }, ..Para::of_picture(Placed { id: small, width: 200.0 }) };
    *doc.para_mut(3) = Para::of_picture(Placed { id: huge, width: 0.0 });
    let layout = set_as(&doc, 600.0);
    // Its own size; the size it was given, in the middle; and no wider
    // than the column, keeping its shape.
    assert_eq!(layout.sets[1].picture, Some((100.0, 50.0)));
    assert_eq!((layout.sets[2].picture, layout.sets[2].rows[0].x), (Some((200.0, 100.0)), 200.0));
    assert_eq!(layout.sets[3].picture, Some((600.0, 200.0)));
    // The caret is before a picture or after it.
    assert_eq!((layout.caret(Pos::new(1, 0)).0, layout.caret(Pos::new(1, 3)).0, layout.caret(Pos::new(1, 0)).2), (0.0, 100.0, 50.0));
    assert_eq!((layout.hit(10.0, layout.tops[1] + 5.0), layout.hit(90.0, layout.tops[1] + 5.0)), (Pos::new(1, 0), Pos::new(1, 3)));
    // A tab is four spaces of room with nothing drawn in it.
    let tabbed = &layout.sets[0].rows[0];
    assert_eq!(tabbed.pieces.iter().map(|p| p.blank).collect::<Vec<_>>(), [false, true, false]);
    assert!(tabbed.pieces[1].width > 15.0 && (tabbed.pieces[2].x - tabbed.pieces[1].x - tabbed.pieces[1].width).abs() < 0.01);
}

#[test]
fn only_what_changed_is_set_again() {
    let mut doc = Doc::from_text("one\ntwo\nthree");
    let mut text = engine();
    let s = Setting { width: 500.0, scale: 1.0, body: 20.0 };
    let mut layout = Layout::default();
    assert!(layout.sync(&mut text, &doc, s));
    assert!(!layout.sync(&mut text, &doc, s), "nothing changed, nothing done");
    let kept = |layout: &Layout| layout.sets.iter().map(|set| set.rows.as_ptr()).collect::<Vec<_>>();
    let before = kept(&layout);
    // Typing in the middle paragraph sets it alone.
    doc.insert_text(Pos::new(1, 3), "!", TextAttrs::default());
    assert!(layout.sync(&mut text, &doc, s));
    let after = kept(&layout);
    assert_eq!((after[0], after[2]), (before[0], before[2]));
    assert_ne!(after[1], before[1]);
    // A paragraph put in above moves the others down as they are.
    doc.split(Pos::new(0, 1));
    assert!(layout.sync(&mut text, &doc, s));
    let moved = kept(&layout);
    assert_eq!((moved.len(), moved[2], moved[3]), (4, after[1], after[2]));
    // A page set another way sets everything again.
    assert!(layout.sync(&mut text, &doc, Setting { width: 300.0, ..s }));
    assert!(kept(&layout).iter().zip(&moved).all(|(a, b)| a != b));
    // An undo's copy of the document shares its paragraphs: going back
    // to it finds their layouts again.
    let snapshot = doc.clone();
    assert!(!layout.sync(&mut text, &snapshot, Setting { width: 300.0, ..s }));
}
