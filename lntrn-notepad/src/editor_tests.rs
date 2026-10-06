//! The editor driven the way the keys and the toolbar drive it.

use lntrn_image::{Image, encode_png};
use lntrn_text::TextEngine;

use super::*;
use crate::doc::{Align, Find};
use crate::editor_ops::Toggle;

fn editor(text: &str) -> Editor {
    let mut e = Editor::of(Doc::from_text(text));
    e.move_to(e.doc.end(), false);
    e
}

fn texts(e: &Editor) -> Vec<&str> {
    e.doc.paras.iter().map(|p| p.text.as_str()).collect()
}

fn at(para: usize, byte: usize) -> Pos {
    Pos::new(para, byte)
}

#[test]
fn typing_selecting_and_taking_it_back() {
    let mut e = Editor::default();
    assert!(!e.modified());
    // A burst of typing is one step; a pause starts another.
    for (i, c) in "hello".chars().enumerate() {
        e.now = i as f64 * 0.1;
        e.type_text(&c.to_string());
    }
    e.now = 5.0;
    e.type_text(" world");
    assert_eq!((texts(&e), e.caret, e.modified()), (vec!["hello world"], at(0, 11), true));
    assert!(e.undo());
    assert_eq!((texts(&e), e.caret), (vec!["hello"], at(0, 5)));
    assert!(e.undo() && texts(&e) == [""] && !e.undo());
    assert!(e.redo() && e.redo() && !e.redo());
    assert_eq!(texts(&e), ["hello world"]);
    // Shift+Ctrl+Left selects a word; typing replaces it.
    e.step(false, true, true);
    assert_eq!(e.selection(), Some((at(0, 6), at(0, 11))));
    e.type_text("there");
    assert_eq!((texts(&e), e.selection()), (vec!["hello there"], None));
    // An arrow with something selected goes to that end of it.
    e.select(at(0, 2), at(0, 8));
    e.step(false, false, false);
    assert_eq!((e.caret, e.anchor), (at(0, 2), None));
    e.select(at(0, 2), at(0, 8));
    e.step(true, false, false);
    assert_eq!(e.caret, at(0, 8));
    // Backspace, Delete, and by word; nothing at the document's ends.
    e.erase(true, false);
    e.erase(false, true);
    assert_eq!(texts(&e), ["hello t"]);
    e.erase(true, true);
    e.erase(true, true);
    assert_eq!((texts(&e), e.caret), (vec![""], at(0, 0)));
    let rev = e.rev;
    e.erase(true, false);
    e.erase(false, false);
    assert_eq!(e.rev, rev, "nothing to erase is not a change");
    e.select_all();
    assert_eq!(e.selection(), None, "nothing in it to select");
}

#[test]
fn enter_backspace_and_tab_know_about_lists() {
    let mut e = editor("");
    // Shorthand: a mark and a blank at a paragraph's start.
    e.type_text("-");
    e.type_text(" ");
    assert_eq!((texts(&e), e.doc.para(0).attrs.list), (vec![""], List::Bullet));
    e.type_text("milk");
    e.enter();
    e.type_text("eggs");
    assert_eq!((texts(&e), e.doc.para(1).attrs.list, e.caret), (vec!["milk", "eggs"], List::Bullet, at(1, 4)));
    // Tab nests an item; Enter on an empty one un-nests it, then ends
    // the list.
    e.enter();
    e.tab(false);
    assert_eq!(e.doc.para(2).attrs.level, 1);
    e.enter();
    assert_eq!((e.doc.para(2).attrs.level, e.doc.para(2).attrs.list, e.doc.paras.len()), (0, List::Bullet, 3));
    e.enter();
    assert_eq!((e.doc.para(2).attrs.list, e.doc.paras.len()), (List::None, 3));
    // Outside a list Tab is a tab; Shift+Tab is nothing.
    e.tab(false);
    e.tab(true);
    assert_eq!(texts(&e)[2], "\t");
    // Backspace at an item's start takes its mark, then joins it to the
    // one before.
    e.move_to(at(1, 0), false);
    e.erase(true, false);
    assert_eq!((e.doc.para(1).attrs.list, e.doc.paras.len()), (List::None, 3));
    e.erase(true, false);
    assert_eq!((texts(&e), e.caret), (vec!["milkeggs", "\t"], at(0, 4)));
    // The undo of a shorthand gives the characters back.
    let mut e = editor("");
    e.type_text("1.");
    e.type_text(" ");
    assert_eq!(e.doc.para(0).attrs.list, List::Number);
    assert!(e.undo());
    assert_eq!((texts(&e), e.doc.para(0).attrs.list), (vec!["1. "], List::None));
    for (mark, style) in [("#", Style::Title), ("##", Style::Heading1), ("####", Style::Heading3), (">", Style::Quote)] {
        let mut e = editor("");
        e.type_text(mark);
        e.type_text(" ");
        assert_eq!((texts(&e), e.doc.para(0).attrs.style), (vec![""], style), "{mark}");
        // Enter after a heading's text starts body text; a quote goes on.
        e.type_text("x");
        e.enter();
        assert_eq!(e.doc.para(1).attrs.style, style.next());
    }
    // A mark that isn't at the start, or in a list already, is text.
    let mut e = editor("a -");
    e.type_text(" ");
    assert_eq!((texts(&e), e.doc.para(0).attrs.list), (vec!["a - "], List::None));
}

#[test]
fn runs_and_paragraphs_are_dressed_from_the_selection_or_for_what_comes_next() {
    let mut e = editor("plain words here\nsecond\nthird");
    // With nothing selected a format waits for the next thing typed, and
    // is forgotten when the caret moves away.
    e.move_to(at(0, 5), false);
    e.toggle(Toggle::Bold);
    assert!(e.format_state().bold && e.doc.para(0).spans.list().is_empty());
    e.type_text("!");
    e.type_text("?");
    assert!(e.doc.para(0).spans.at(5).bold && e.doc.para(0).spans.at(6).bold && !e.doc.para(0).spans.at(7).bold);
    e.toggle(Toggle::Italic);
    e.step(true, false, false);
    assert!(!e.format_state().italic && e.pending.is_none());
    // On a selection: on if any of it is without, off if all of it has.
    e.select(at(0, 3), at(0, 9));
    assert!(!e.format_state().bold, "part of it is not bold");
    e.toggle(Toggle::Bold);
    assert!(e.format_state().bold && e.doc.para(0).spans.common(3, 9).bold);
    e.toggle(Toggle::Bold);
    assert!(!e.doc.para(0).spans.at(5).bold);
    e.format(|a| (a.size, a.color, a.highlight) = (Some(30.0), Some(0x2563eb), Some(0xfff0c0)));
    assert_eq!((e.format_state().size, e.format_state().color), (Some(30.0), Some(0x2563eb)));
    assert!(e.undo() && e.format_state().size.is_none());
    // Paragraphs: the ones the selection touches, not one it only
    // reaches the start of.
    e.select(at(0, 2), at(2, 0));
    assert_eq!(e.paras(), (0, 1));
    e.set_style(Style::Heading2);
    e.set_align(Align::Center);
    assert_eq!(e.doc.paras.iter().map(|p| (p.attrs.style, p.attrs.align)).collect::<Vec<_>>(), [(Style::Heading2, Align::Center), (Style::Heading2, Align::Center), (Style::Body, Align::Left)]);
    assert_eq!(e.para_state().style, Style::Heading2);
    // A list on, another kind over it, the same kind again off.
    e.set_list(List::Number);
    e.nest(1);
    assert_eq!((e.doc.para(1).attrs.list, e.doc.para(1).attrs.level, e.doc.number(1)), (List::Number, 1, 2));
    e.set_list(List::Check(false));
    assert_eq!(e.doc.para(0).attrs.list, List::Check(false));
    e.tick(1);
    assert_eq!(e.doc.para(1).attrs.list, List::Check(true));
    e.set_list(List::Check(false));
    assert_eq!((e.doc.para(0).attrs.list, e.doc.para(1).attrs.list, e.doc.para(1).attrs.level), (List::None, List::None, 0), "a ticked box is still a box");
}

#[test]
fn cut_copy_and_paste_keep_what_they_carry() {
    let mut e = editor("one two\nthree");
    e.doc.format(at(0, 4), at(0, 7), |a| a.italic = true);
    e.doc.set_paras(1, 1, |a| a.style = Style::Quote);
    let picture = encode_png(&Image::solid(8, 4, [200, 100, 50, 255]));
    e.move_to(e.doc.end(), false);
    e.insert_picture(picture).unwrap();
    assert_eq!(texts(&e), ["one two", "three", "\u{FFFC}", ""]);
    assert!(e.insert_picture(b"junk".to_vec()).is_err());
    e.resize_picture(2, 300.0, true);
    e.resize_picture(2, 320.0, false);
    assert_eq!(e.doc.para(2).picture.map(|p| p.width), Some(320.0));
    assert!(e.undo());
    assert_eq!(e.doc.para(2).picture.map(|p| p.width), Some(0.0), "a drag is one step");
    assert!(e.redo());
    // Copied from the middle of the first line to after the picture.
    e.select(at(0, 4), at(2, 3));
    let clip = e.copy().unwrap();
    assert_eq!((clip.text.as_str(), clip.paras.len(), clip.pictures.len()), ("two\nthree\n", 3, 1));
    assert!(e.cut().is_some());
    assert_eq!((texts(&e), e.caret), (vec!["one ", ""], at(0, 4)));
    // Into another document: the words still italic, the quote a quote,
    // the picture there.
    let mut other = editor("AB");
    other.move_to(at(0, 1), false);
    other.paste(&clip);
    assert_eq!(texts(&other), ["Atwo", "three", "\u{FFFC}", "B"]);
    assert!(other.doc.para(0).spans.at(2).italic && !other.doc.para(0).spans.at(0).italic);
    assert_eq!((other.doc.para(1).attrs.style, other.doc.para(2).picture.map(|p| p.width)), (Style::Quote, Some(320.0)));
    assert!(other.doc.pictures.get(other.doc.para(2).picture.unwrap().id).is_some());
    assert!(other.undo() && texts(&other) == ["AB"]);
    // Text from anywhere takes after where it lands.
    other.doc.format(at(0, 0), at(0, 2), |a| a.bold = true);
    other.move_to(at(0, 1), false);
    other.paste_text("x\ny");
    assert_eq!(texts(&other), ["Ax", "yB"]);
    assert!(other.doc.para(0).spans.at(1).bold && other.doc.para(1).spans.at(0).bold);
    assert_eq!(e.copy().map(|c| c.text), None, "nothing selected, nothing copied");
}

#[test]
fn found_text_is_replaced_one_place_or_everywhere() {
    let mut e = editor("the cat and THE cat\ncats");
    e.doc.format(at(0, 4), at(0, 7), |a| a.bold = true);
    let find = Find { text: "cat".into(), ..Find::default() };
    let hits = find.all(&e.doc);
    e.replace(hits[0].0, hits[0].1, "dog");
    assert_eq!((texts(&e), e.caret), (vec!["the dog and THE cat", "cats"], at(0, 7)));
    assert!(e.doc.para(0).spans.common(4, 7).bold, "dressed as what it replaced");
    assert_eq!(e.replace_all(&find, "ferret"), 2);
    assert_eq!(texts(&e), ["the dog and THE ferret", "ferrets"]);
    assert_eq!(e.replace_all(&find, "x"), 0);
    assert!(e.undo());
    assert_eq!(texts(&e), ["the dog and THE cat", "cats"], "all of them in one step");
}

#[test]
fn the_caret_moves_over_the_page() {
    let mut e = editor("first line of several words that will wrap\n\nlast");
    let mut text = TextEngine::new("Inter", "JetBrains Mono");
    assert!(e.sync(&mut text, Setting { width: 220.0, scale: 1.0, body: 20.0 }));
    let rows = e.layout.sets[0].rows.len();
    assert!(rows >= 2, "it wraps");
    // Home and End are the row's; with Ctrl, the document's.
    e.move_to(at(0, 3), false);
    e.edge(true, false, false);
    assert_eq!(e.caret, at(0, e.layout.sets[0].rows[0].end - 1));
    e.edge(false, false, true);
    assert_eq!(e.selection(), Some((at(0, 0), at(0, e.layout.sets[0].rows[0].end - 1))));
    e.edge(true, true, false);
    assert_eq!(e.caret, e.doc.end());
    // Up through the empty paragraph and back down lands where it set
    // out from, not at the start the empty line would leave it at.
    e.move_to(at(2, 3), false);
    e.climb(-1.0, false);
    assert_eq!(e.caret, at(1, 0));
    e.climb(-1.0, false);
    assert_eq!(e.caret.para, 0);
    e.climb(1.0, false);
    e.climb(1.0, false);
    assert_eq!(e.caret, at(2, 3));
    // Up from the top row is the start; down from the bottom, the end;
    // a page at a time goes as far as there is.
    e.move_to(at(0, 2), false);
    e.climb(-1.0, true);
    assert_eq!((e.caret, e.anchor), (at(0, 0), Some(at(0, 2))));
    e.climb(9000.0, false);
    assert_eq!(e.caret.para, 2);
    e.climb(1.0, false);
    assert_eq!(e.caret, e.doc.end());
    e.climb(-9000.0, false);
    assert_eq!(e.caret.para, 0);
}
