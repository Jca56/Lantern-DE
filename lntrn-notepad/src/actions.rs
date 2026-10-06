//! What the menus, the keys and the palette do: files opened, saved and
//! exported, tabs closed (asking first when work would be lost), and
//! everything that dresses the text.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use lntrn_props::Value;
use lntrn_ui::{Action, Dialog, HostCx, ShellRequest};

use crate::app::{App, Shared};
use crate::doc::{Align, List, Style, TextAttrs};
use crate::editor_ops::Toggle;
use crate::io::{self, Kind as FileKind};
use crate::menu;
use crate::ops::Op;
use crate::settings::BODY;
use crate::view::page::picture_at_caret;

fn path_of(action: &Action) -> Option<PathBuf> {
    match action.arg("path") {
        Some(Value::Str(path)) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => None,
    }
}

/// The folder a file dialog starts in: where the document is, or the
/// user's documents.
fn home_of(doc: Option<&Shared>) -> PathBuf {
    let beside = doc.and_then(|d| d.borrow().path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)));
    beside.or_else(|| lntrn_sys::dirs::user_dir(lntrn_sys::dirs::UserDir::Documents)).unwrap_or_default()
}

/// A file name for a document that has none: its first words.
fn name_for(doc: &Shared) -> String {
    let d = doc.borrow();
    if let Some(stem) = d.path.as_ref().and_then(|p| p.file_stem()) {
        return stem.to_string_lossy().into_owned();
    }
    let first = d.ed.doc.paras.iter().map(|p| p.text.trim()).find(|t| !t.is_empty() && !t.contains('\u{FFFC}')).unwrap_or("");
    let name: String = first.chars().filter(|c| !"/\\:*?\"<>|\t".contains(*c)).take(40).collect();
    if name.trim().is_empty() { d.name.clone() } else { name.trim().to_owned() }
}

fn ask_where(doc: &Shared, action: &str, extension: &str, cx: &mut HostCx) {
    let suggest = home_of(Some(doc)).join(format!("{}.{extension}", name_for(doc)));
    cx.request(ShellRequest::PathDialog { action: Action::new(action), save: true, suggest: suggest.display().to_string() });
}

/// Write a document to `path` and make that its file. Says so either way.
fn write(app: &mut App, doc: &Shared, path: &Path, cx: &mut HostCx) {
    let mut d = doc.borrow_mut();
    match io::save(path, &d.ed.doc, BODY) {
        Ok(()) => {
            d.ed.saved = d.ed.rev;
            d.path = Some(path.to_owned());
            d.name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
            cx.toast(&format!("Saved {}", d.name));
            if app.close_after.ptr_eq(&Rc::downgrade(doc)) {
                app.ops.push(Op::Close(Rc::downgrade(doc)));
            }
        }
        // Never quietly: a save that didn't happen is said so, in the way.
        Err(e) => cx.request(ShellRequest::Dialog(Dialog::notice("Couldn't save", &e))),
    }
}

/// Save a document to its file: asking where when it has none, and
/// asking first when its file is of a kind that can't hold all of it.
fn save(app: &mut App, doc: &Shared, cx: &mut HostCx) {
    let (path, holds) = {
        let d = doc.borrow();
        (d.path.clone(), d.lossy || d.path.as_deref().is_none_or(|p| FileKind::of(p).holds(&d.ed.doc)))
    };
    app.asked = Rc::downgrade(doc);
    let Some(path) = path else { return ask_where(doc, "file.save-path", "lnote", cx) };
    if holds {
        return write(app, doc, &path, cx);
    }
    let kind = FileKind::of(&path).label();
    let body = format!("A {kind} can't keep everything this document has (how its text is dressed, its pictures). Saved as one, that is lost.\n\nA Notepad document keeps all of it.");
    cx.request(ShellRequest::Dialog(Dialog::new("Keep the formatting?", &body).button("Cancel", None).button(&format!("Save as a {kind}"), Some(Action::new("file.save-lossy"))).button("Save as a Notepad Document\u{2026}", Some(Action::new("file.save-as"))).default_button(2)));
}

/// Close a document's tab, asking first if that would lose work.
fn close(app: &mut App, doc: &Shared, cx: &mut HostCx) {
    if !doc.borrow().precious() {
        return app.ops.push(Op::Close(Rc::downgrade(doc)));
    }
    app.asked = Rc::downgrade(doc);
    let body = format!("**{}** has changes that aren't saved.", doc.borrow().name);
    cx.request(ShellRequest::Dialog(Dialog::new("Save changes?", &body).button("Cancel", None).button("Discard", Some(Action::new("tab.close-discard"))).button("Save", Some(Action::new("tab.close-save"))).default_button(2)));
}

/// Ask whether work would be lost, for a tab closed some other way.
pub fn ask_about(app: &mut App, doc: &Shared) -> ShellRequest {
    app.asked = Rc::downgrade(doc);
    let body = format!("**{}** has changes that aren't saved.", doc.borrow().name);
    ShellRequest::Dialog(Dialog::new("Save changes?", &body).button("Keep Open", None).button("Discard", Some(Action::new("tab.close-discard"))).button("Save", Some(Action::new("tab.close-save"))).default_button(2))
}

/// Carry out an action.
pub fn run(app: &mut App, action: &Action, cx: &mut HostCx) {
    let id = action.id.as_str();
    // A save that was to be followed by closing is only that save.
    if !matches!(id, "tab.close-save" | "file.save-path" | "file.save-lossy" | "file.save-as") {
        app.close_after = std::rc::Weak::new();
    }
    match id {
        "file.new" => app.ops.push(Op::New),
        "file.open" => cx.request(ShellRequest::PathDialog { action: Action::new("file.open-path"), save: false, suggest: home_of(app.front.upgrade().as_ref()).display().to_string() }),
        "file.open-path" => app.ops.extend(path_of(action).map(Op::Open)),
        "view.paper" | "view.dark" => (app.settings.paper, app.settings_dirty) = (id == "view.paper", true),
        // What a question on screen was about.
        "tab.close-discard" => app.ops.extend(app.asked.upgrade().map(|doc| Op::Close(Rc::downgrade(&doc)))),
        "tab.close-save" => {
            if let Some(doc) = app.asked.upgrade() {
                app.close_after = Rc::downgrade(&doc);
                save(app, &doc, cx);
            }
        }
        "file.save-lossy" => {
            if let Some((doc, path)) = app.asked.upgrade().and_then(|doc| doc.borrow().path.clone().map(|path| (doc.clone(), path))) {
                doc.borrow_mut().lossy = true;
                write(app, &doc, &path, cx);
            }
        }
        "file.save-path" => {
            if let (Some(doc), Some(mut path)) = (app.asked.upgrade(), path_of(action)) {
                if path.extension().is_none() {
                    path.set_extension("lnote");
                }
                // Another file: whether it may lose anything is asked anew.
                (doc.borrow_mut().path, doc.borrow_mut().lossy) = (Some(path), false);
                save(app, &doc, cx);
            }
        }
        "file.export-path" => {
            if let (Some(doc), Some(mut path)) = (app.asked.upgrade(), path_of(action)) {
                // A name typed bare is still the kind that was asked for.
                if path.extension().is_none() {
                    path.set_extension(app.export_as);
                }
                export(app, &doc, &path, cx);
            }
        }
        _ => {
            if let Some(doc) = app.front.upgrade() {
                on_document(app, &doc, id, action, cx);
            }
        }
    }
    // The page takes the keyboard back, unless the action was to give
    // it to the find bar.
    app.refocus = !matches!(id, "find.show" | "find.replace" | "find.next" | "find.prev");
    cx.rebuild();
}

/// Write a copy of a document as another kind of file. The document
/// stays the file it was.
fn export(app: &mut App, doc: &Shared, path: &Path, cx: &mut HostCx) {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        // A PDF is set with the text engine, which the next frame has.
        app.export_pdf = Some((Rc::downgrade(doc), path.to_owned()));
        return;
    }
    match io::save(path, &doc.borrow().ed.doc, BODY) {
        Ok(()) => cx.toast(&format!("Wrote {}", path.display())),
        Err(e) => cx.request(ShellRequest::Dialog(Dialog::notice("Couldn't export", &e))),
    }
}

/// The actions that are about the document in front.
fn on_document(app: &mut App, doc: &Shared, id: &str, action: &Action, cx: &mut HostCx) {
    match id {
        "tab.close" => return close(app, doc, cx),
        "file.save" => return save(app, doc, cx),
        "file.save-as" => {
            app.asked = Rc::downgrade(doc);
            return ask_where(doc, "file.save-path", "lnote", cx);
        }
        "file.export-pdf" | "file.export-docx" | "file.export-md" | "file.export-txt" => {
            app.asked = Rc::downgrade(doc);
            app.export_as = ["pdf", "docx", "md"].into_iter().find(|kind| id.ends_with(kind)).unwrap_or("txt");
            return ask_where(doc, "file.export-path", app.export_as, cx);
        }
        "edit.cut" | "edit.copy" => return app.ops.push(Op::Copy(id == "edit.cut")),
        "edit.paste" | "edit.paste-plain" => return app.ops.push(Op::Paste(id == "edit.paste-plain")),
        "picture.insert" => return cx.request(menu::ask_picture()),
        _ => {}
    }
    let mut d = doc.borrow_mut();
    let d = &mut *d;
    let ed = &mut d.ed;
    let size = ed.format_state().size.unwrap_or(BODY * ed.para_state().style.scale());
    match id {
        "edit.undo" => drop(ed.undo()),
        "edit.redo" => drop(ed.redo()),
        "edit.select-all" => ed.select_all(),
        "find.show" | "find.replace" => d.find.show(id == "find.replace", ed.copy().map(|clip| clip.text)),
        "find.next" | "find.prev" => {
            if !d.find.open {
                d.find.show(false, ed.copy().map(|clip| clip.text));
            }
            d.find.go(ed, if id == "find.next" { 1 } else { -1 });
        }
        "find.close" => d.find.close(),
        "format.bold" => ed.toggle(Toggle::Bold),
        "format.italic" => ed.toggle(Toggle::Italic),
        "format.underline" => ed.toggle(Toggle::Underline),
        "format.strike" => ed.toggle(Toggle::Strike),
        "format.clear" => ed.format(|a| *a = TextAttrs::default()),
        "size.bigger" => ed.format(|a| a.size = Some((size + 2.0).min(200.0))),
        "size.smaller" => ed.format(|a| a.size = Some((size - 2.0).max(6.0))),
        "list.bullet" => ed.set_list(List::Bullet),
        "list.number" => ed.set_list(List::Number),
        "list.check" => ed.set_list(List::Check(false)),
        "list.in" => ed.nest(1),
        "list.out" => ed.nest(-1),
        "align.left" => ed.set_align(Align::Left),
        "align.center" => ed.set_align(Align::Center),
        "align.right" => ed.set_align(Align::Right),
        "align.justify" => ed.set_align(Align::Justify),
        "color.text" | "color.highlight" => {
            let rgb = match action.arg("rgb") {
                Some(Value::Str(hex)) => u32::from_str_radix(hex, 16).ok(),
                _ => None,
            };
            ed.format(|a| if id == "color.text" { a.color = rgb } else { a.highlight = rgb });
        }
        "picture.insert-path" => {
            let Some(path) = path_of(action) else { return };
            let put = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| ed.insert_picture(bytes));
            if let Err(e) = put {
                cx.request(ShellRequest::Dialog(Dialog::notice("That picture can't be put in", &format!("{}: {e}", path.display()))));
            }
        }
        "picture.natural" | "picture.fit" => {
            // As wide as the column, or back to its own width.
            let column = d.view.setting.map_or(600.0, |s| s.width / s.scale);
            if let Some(para) = picture_at_caret(ed) {
                ed.picture_width(para, (id == "picture.fit").then_some(column));
            }
        }
        _ => {
            if let Some(style) = id.strip_prefix("style.").and_then(Style::from_word) {
                ed.set_style(style);
            }
        }
    }
}
