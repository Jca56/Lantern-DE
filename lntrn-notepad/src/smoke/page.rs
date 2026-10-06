//! The page under the keys and the pointer, the toolbar, and find.

use lntrn_image::{Image, encode_png};
use lntrn_math::Vec2;
use lntrn_ui::{Key, Modifiers};

use super::Rig;
use crate::doc::{Align, Font, List, Pos, Style};
use crate::view::paint::check_box;

fn at(para: usize, byte: usize) -> Pos {
    Pos::new(para, byte)
}

#[test]
fn typing_lands_on_the_page_and_the_window_fits() {
    let mut rig = Rig::new();
    // No click first: the window opens ready to type in.
    rig.type_text("Hello page");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("- milk");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("eggs");
    assert_eq!(rig.text(), "Hello page\nmilk\neggs");
    let front = rig.front();
    assert_eq!(front.borrow().ed.doc.paras.iter().map(|p| p.attrs.list).collect::<Vec<_>>(), [List::None, List::Bullet, List::Bullet]);
    assert_eq!(rig.tabs(), (vec!["Untitled \u{25CF}".to_owned()], 0), "the tab says there is unsaved work");
    // The sheet is on the desk with room either side, the toolbar is one
    // row at this size, and its buttons are big enough to hit.
    let (desk, place) = (rig.rect(rig.body().with("page")), rig.place());
    assert!(desk.height() > 480.0 && place.sheet.min.x > desk.min.x + 10.0 && place.sheet.max.x < desk.max.x - 10.0, "{desk:?} {:?}", place.sheet);
    assert!(place.column > 600.0, "room to write: {}", place.column);
    let tools = rig.body().with("tools");
    let (first, last) = (rig.rect(tools.with("style")), rig.rect(tools.with("picture")));
    assert!((first.center().y - last.center().y).abs() < 1.0 && last.max.x <= 1152.0, "one row: {first:?} .. {last:?}");
    for name in ["Bold  Ctrl+B", "Bullets", "Align left  Ctrl+L", "Text colour", "picture", "smaller", "bigger"] {
        let r = rig.rect(tools.with(name));
        assert!(r.width() >= 44.0 && r.height() >= 44.0, "{name}: {r:?}");
    }
    // Keys: the caret moves by word and by row, text is selected and
    // replaced, and it can all be taken back.
    rig.key(Key::Home, Modifiers::CTRL);
    rig.key(Key::ArrowRight, Modifiers::CTRL | Modifiers::SHIFT);
    rig.type_text("Howdy");
    assert_eq!(rig.text(), "Howdy page\nmilk\neggs");
    rig.key(Key::ArrowDown, Modifiers::NONE);
    rig.key(Key::End, Modifiers::NONE);
    rig.key(Key::Backspace, Modifiers::CTRL);
    assert_eq!(rig.text(), "Howdy page\n\neggs");
    rig.chord('z', false);
    rig.chord('z', false);
    assert_eq!(rig.text(), "Hello page\nmilk\neggs");
    rig.chord('z', true);
    assert_eq!(rig.text(), "Howdy page\nmilk\neggs");
    rig.chord('a', false);
    rig.key(Key::Delete, Modifiers::NONE);
    assert_eq!(rig.text(), "");
}

#[test]
fn the_pointer_places_selects_ticks_and_sizes() {
    let mut rig = Rig::new();
    rig.type_text("one two three");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("[] buy milk");
    let front = rig.front();
    // A click puts the caret there; Shift and a click selects to there;
    // two clicks take the word, three the paragraph.
    rig.click_at(rig.point(at(0, 5)));
    assert_eq!((front.borrow().ed.caret, front.borrow().ed.selection()), (at(0, 5), None));
    rig.h.set_mods(Modifiers::SHIFT);
    rig.click_at(rig.point(at(0, 9)));
    rig.h.set_mods(Modifiers::NONE);
    rig.wait(1.0);
    assert_eq!(front.borrow().ed.selection(), Some((at(0, 5), at(0, 9))));
    let word = rig.point(at(0, 5));
    rig.click_at(word);
    rig.click_at(word);
    assert_eq!(front.borrow().ed.selection(), Some((at(0, 4), at(0, 7))));
    rig.click_at(word);
    assert_eq!(front.borrow().ed.selection(), Some((at(0, 0), at(0, 13))));
    rig.wait(1.0);
    // A drag selects what it passes over.
    rig.drag(rig.point(at(0, 2)), rig.point(at(1, 3)));
    assert_eq!(front.borrow().ed.selection(), Some((at(0, 2), at(1, 3))));
    rig.wait(1.0);
    // A click on a box ticks it without the caret going there.
    rig.click_at(rig.point(at(0, 1)));
    let (origin, tick) = {
        let d = front.borrow();
        let (x, y, side) = check_box(&d.ed.layout.sets[1], &d.view.setting.unwrap());
        (rig.place().origin, Vec2::new(f64::from(x + side * 0.5), f64::from(d.ed.layout.tops[1] + y + side * 0.5)))
    };
    rig.click_at(origin + tick);
    assert_eq!((front.borrow().ed.doc.para(1).attrs.list, front.borrow().ed.caret), (List::Check(true), at(0, 1)));
    // A picture is picked whole by a click, and sized by its corner.
    rig.wait(1.0);
    rig.key(Key::End, Modifiers::CTRL);
    front.borrow_mut().ed.insert_picture(encode_png(&Image::solid(200, 100, [10, 120, 200, 255]))).unwrap();
    rig.settle();
    assert_eq!(front.borrow().ed.doc.para(2).picture.map(|p| p.width), Some(0.0));
    let middle = rig.place().origin + {
        let d = front.borrow();
        Vec2::new(100.0, f64::from(d.ed.layout.tops[2]) + 50.0)
    };
    rig.click_at(middle);
    assert_eq!(front.borrow().ed.selection(), Some((at(2, 0), at(2, 3))));
    let corner = rig.rect(rig.body().with("page").with("corner")).center();
    rig.drag(corner, corner + Vec2::new(100.0, 0.0));
    assert_eq!(front.borrow().ed.doc.para(2).picture.map(|p| p.width), Some(300.0));
    assert_eq!(front.borrow().ed.layout.sets[2].picture, Some((300.0, 150.0)));
    // The sheet's edge drags in, the other side with it, and the setting
    // is the new width.
    let before = rig.place().sheet;
    let edge = rig.rect(rig.body().with("page").with("right")).center();
    rig.drag(edge, edge - Vec2::new(120.0, 0.0));
    let after = rig.place().sheet;
    assert!((before.width() - after.width() - 240.0).abs() < 4.0, "{before:?} to {after:?}");
    assert!(rig.app.settings.page_width < 0.82);
}

#[test]
fn the_toolbar_dresses_what_is_selected() {
    let mut rig = Rig::new();
    rig.type_text("make this bold");
    let front = rig.front();
    let tools = rig.body().with("tools");
    // Bold from its button, then from its key; the button shows it.
    front.borrow_mut().ed.select(at(0, 5), at(0, 9));
    rig.settle();
    rig.click(tools.with("Bold  Ctrl+B"));
    assert!(front.borrow().ed.doc.para(0).spans.common(5, 9).bold);
    rig.type_text("that");
    assert_eq!((rig.text(), front.borrow().ed.doc.para(0).spans.common(5, 9).bold), ("make that bold".to_owned(), true), "the keyboard is back with the page, and typing over bold is bold");
    front.borrow_mut().ed.select(at(0, 0), at(0, 4));
    rig.settle();
    rig.chord('i', false);
    rig.chord('u', false);
    rig.chord('x', true);
    let run = front.borrow().ed.doc.para(0).spans.common(0, 4);
    assert_eq!((run.italic, run.underline, run.strike, run.bold), (true, true, true, false));
    // Size by the stepper, lists and alignment by their buttons.
    rig.click(tools.with("bigger"));
    rig.click(tools.with("bigger"));
    assert_eq!(front.borrow().ed.format_state().size, Some(28.0));
    rig.click(tools.with("Numbered list"));
    rig.click(tools.with("Centre  Ctrl+E"));
    let para = front.borrow().ed.doc.para(0).attrs;
    assert_eq!((para.list, para.align), (List::Number, Align::Center));
    rig.click(tools.with("Numbered list"));
    rig.chord('l', false);
    let para = front.borrow().ed.doc.para(0).attrs;
    assert_eq!((para.list, para.align), (List::None, Align::Left));
    // The style and the font from their lists.
    rig.click(tools.with("style"));
    rig.click(tools.with("style").with("item").with_index(2));
    assert_eq!(front.borrow().ed.doc.para(0).attrs.style, Style::Heading1);
    assert!(rig.app.fonts.len() > 3, "the machine's fonts are offered: {:?}", rig.app.fonts.len());
    let pick = rig.app.fonts[1].clone();
    rig.click(tools.with("font"));
    rig.click(tools.with("font").with("item").with_index(2));
    assert_eq!(front.borrow().ed.format_state().font, Some(Font::named(&pick)));
    // A colour from the swatches a button opens; the first chip takes
    // it off again.
    rig.click(tools.with("Text colour"));
    assert!(rig.shell.popup_open());
    let chip = |i: usize| lntrn_ui::WidgetId::ROOT.with("popup").with("context").with_index(0).with("items").with_index(0).with("chip").with_index(i);
    rig.click(chip(3));
    assert_eq!((front.borrow().ed.format_state().color, rig.shell.popup_open()), (Some(crate::menu::TEXT_COLORS[2]), false));
    rig.click(tools.with("Highlight"));
    rig.click(chip(1));
    assert_eq!(front.borrow().ed.format_state().highlight, Some(crate::menu::HIGHLIGHTS[0]));
    rig.click(tools.with("Text colour"));
    rig.click(chip(0));
    assert_eq!(front.borrow().ed.format_state().color, None);
}

#[test]
fn find_walks_the_page_and_cut_and_paste_keep_their_dress() {
    let mut rig = Rig::new();
    rig.type_text("a cat and a cat");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("one more cat");
    let front = rig.front();
    // Ctrl+F opens the bar with the keyboard in it; typing finds.
    rig.chord('f', false);
    rig.type_text("cat");
    let find = rig.body().with("find");
    assert_eq!((front.borrow().find.hits.len(), front.borrow().ed.selection()), (3, Some((at(0, 2), at(0, 5)))));
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(front.borrow().ed.selection(), Some((at(0, 12), at(0, 15))));
    rig.click(find.with("next"));
    assert_eq!(front.borrow().ed.selection(), Some((at(1, 9), at(1, 12))));
    rig.click(find.with("prev"));
    assert_eq!(front.borrow().ed.selection(), Some((at(0, 12), at(0, 15))));
    // Replace: one, then all the rest.
    rig.chord('h', false);
    front.borrow_mut().find.with = "dog".into();
    rig.settle();
    rig.click(find.with("one"));
    assert_eq!(rig.text(), "a cat and a dog\none more cat");
    rig.click(find.with("all"));
    assert_eq!((rig.text(), front.borrow().find.hits.len()), ("a dog and a dog\none more dog".to_owned(), 0));
    rig.click(find.with("close"));
    assert!(!front.borrow().find.open);
    // The page has the keyboard again: bold a word, cut it, and paste it
    // twice. It comes back bold, here and in another tab.
    front.borrow_mut().ed.select(at(0, 2), at(0, 5));
    rig.settle();
    rig.chord('b', false);
    rig.chord('x', false);
    assert_eq!((rig.text(), rig.shell.state.clipboard.as_str()), ("a  and a dog\none more dog".to_owned(), "dog"));
    rig.key(Key::End, Modifiers::CTRL);
    rig.chord('v', false);
    assert_eq!(rig.text(), "a  and a dog\none more dogdog");
    assert!(front.borrow().ed.doc.para(1).spans.common(12, 15).bold && !front.borrow().ed.doc.para(1).spans.at(11).bold);
    rig.chord('t', false);
    rig.chord('v', false);
    let other = rig.front();
    assert_eq!((rig.text(), other.borrow().ed.doc.para(0).spans.common(0, 3).bold), ("dog".to_owned(), true));
    // As plain text, and text from another app, take after where they
    // land.
    rig.chord('v', true);
    assert!(!other.borrow().ed.doc.para(0).spans.at(4).bold || other.borrow().ed.doc.para(0).spans.at(2).bold);
    rig.shell.state.set_clipboard("from elsewhere");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.chord('v', false);
    assert_eq!(other.borrow().ed.doc.para(1).text, "from elsewhere");
}
