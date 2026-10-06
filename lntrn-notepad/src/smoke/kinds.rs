//! A document written as each kind of file there is, and read back.

use lntrn_ui::{Key, Modifiers};

use super::{Rig, sandbox};
use crate::doc::{List, Style};

#[test]
fn a_document_goes_out_as_every_kind_and_comes_back() {
    let docs = sandbox().join("docs/kinds");
    let mut rig = Rig::start_after(false, &[], || std::fs::create_dir_all(sandbox().join("docs/kinds")).unwrap());
    // A title, a line with a bold word in it, and two things to tick.
    rig.type_text("# ");
    rig.type_text("Trip");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("Leave ");
    rig.chord('b', false);
    rig.type_text("early");
    rig.chord('b', false);
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("[] ");
    rig.type_text("boots");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("map");
    let front = rig.front();
    let paras = front.borrow().ed.doc.paras.clone();
    assert_eq!(paras.iter().map(|p| (p.attrs.style, p.attrs.list)).collect::<Vec<_>>(), [(Style::Title, List::None), (Style::Body, List::None), (Style::Body, List::Check(false)), (Style::Body, List::Check(false))]);
    assert!(paras[1].spans.at(8).bold && !paras[1].spans.at(2).bold);
    // Each kind is asked for by name, and where to put it is asked. A
    // name typed bare still gets the ending of the kind asked for.
    for (kind, name) in [("docx", "trip.docx"), ("md", "trip"), ("txt", "trip.txt"), ("pdf", "trip")] {
        rig.act(&format!("file.export-{kind}"), None);
        assert!(rig.shell.popup_open(), "{kind}: where to is asked");
        rig.act("file.export-path", Some(&docs.join(name)));
        assert!(docs.join("trip").with_extension(kind).exists(), "{kind} is written");
    }
    // The document is what it was: it has no file, and is not saved.
    assert_eq!((front.borrow().path.clone(), front.borrow().ed.modified(), rig.tabs().0.len()), (None, true, 1));
    // Word and Markdown hold all of this one, and open as it was.
    for kind in ["docx", "md"] {
        rig.act("file.open-path", Some(&docs.join("trip").with_extension(kind)));
        assert_eq!(rig.tabs().0.last().map(String::as_str), Some(format!("trip.{kind}").as_str()));
        assert_eq!(rig.front().borrow().ed.doc.paras, paras, "{kind}");
        assert!(!rig.front().borrow().ed.modified());
    }
    // Text keeps the words and the boxes; the PDF is a page of paper.
    assert_eq!(std::fs::read_to_string(docs.join("trip.txt")).unwrap(), "Trip\nLeave early\n- [ ] boots\n- [ ] map\n");
    let pdf = std::fs::read(docs.join("trip.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.") && pdf.ends_with(b"%%EOF\n") && pdf.len() > 2000, "{} bytes", pdf.len());
    let _ = std::fs::remove_dir_all(&docs);
}
