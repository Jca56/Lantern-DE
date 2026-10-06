//! The app as the shell sees it: one kind of editor, a document, in the
//! shell's own panes and tabs. A tab's document lives in the tab, so it
//! goes when the tab does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::mpsc::Receiver;

use lntrn_kit::desktop::Follow;
use lntrn_kit::look;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, AreaId, FILL, Host, HostCx, ImageHandle, Key, KeyConfig, KeyItem, KeyPress, Menu, Modifiers, ShellRequest, Trigger, Ui, actions};

use crate::doc::Doc;
use crate::editor::{Clip, Editor};
use crate::menu;
use crate::ops::Op;
use crate::session::{self, Saver};
use crate::settings::{BODY, Settings};
use crate::view::find_bar::FindBar;
use crate::view::page::{PageIn, View, page};
use crate::view::paint::Inks;
use crate::view::toolbar::{Swatch, toolbar};

pub const APP_ID: &str = "lntrn-notepad";
/// The status line's height, in logical pixels.
const STATUS_H: f64 = 38.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Document,
}

const KINDS: [Kind; 1] = [Kind::Document];

/// A document that is open.
pub struct Document {
    pub ed: Editor,
    pub view: View,
    pub find: FindBar,
    /// The file it is of; none until it is saved somewhere.
    pub path: Option<PathBuf>,
    /// What it is called: its file's name, or `Untitled 2`.
    pub name: String,
    /// The number its draft is kept under; and how it stood when the
    /// draft was last seen to: the change it was at, and whether it had
    /// work worth keeping.
    pub draft: u64,
    pub drafted: (u64, bool),
    /// The last change seen, and when.
    pub changed: (u64, f64),
    /// Its tab was closed on purpose: it is not to be brought back.
    pub closed: bool,
    /// The user has said its file may lose what the file can't hold: it
    /// is not asked again for this file.
    pub lossy: bool,
}

pub type Shared = Rc<RefCell<Document>>;

impl Document {
    pub fn untitled(name: String) -> Document {
        Document { ed: Editor::default(), view: View::default(), find: FindBar::default(), path: None, name, draft: session::new_draft_id(), drafted: (0, false), changed: (0, 0.0), closed: false, lossy: false }
    }

    pub fn of(path: PathBuf, doc: Doc) -> Document {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        Document { ed: Editor::of(doc), path: Some(path), ..Document::untitled(name) }
    }

    /// Whether throwing it away would lose something: it has changes,
    /// and it is not just an empty page nobody saved.
    pub fn precious(&self) -> bool {
        self.ed.modified() && !(self.path.is_none() && self.ed.doc.is_blank())
    }
}

/// What a tab holds.
#[derive(Clone, Default)]
pub struct Tab {
    pub doc: Option<Shared>,
}

pub struct App {
    pub settings: Settings,
    pub follow: Follow,
    /// How many times `lantern.toml` had been read when the settings
    /// were last taken from it; and when they are next to be written.
    pub seen_reads: u64,
    pub settings_dirty: bool,
    pub save_at: Option<f64>,
    /// What was last cut or copied here, as it was.
    pub clip: Option<Clip>,
    /// The textures of the pictures in the open documents.
    pub pictures: HashMap<u64, ImageHandle>,
    /// The font families there are, once the text engine has been asked.
    pub fonts: Vec<String>,
    /// What to do to the tabs once the frame is built.
    pub ops: Vec<Op>,
    /// The document with the keyboard, and its pane.
    pub front: Weak<RefCell<Document>>,
    pub front_area: Option<AreaId>,
    took_focus: Option<AreaId>,
    /// The page takes the keyboard back on the next frame.
    pub refocus: bool,
    keys: KeyConfig,
    /// Names given to new documents since `open` was last gathered.
    pub named: Vec<String>,
    pub saver: Saver,
    /// Every open document, in order, as of the last frame: for when the
    /// window closes.
    pub open: Vec<Shared>,
    /// When the list of what is open is next to be written.
    pub session_at: Option<f64>,
    /// Whether drafts and the list are kept at all (not while tested);
    /// and whether this is the Notepad that brings the list back.
    pub remembers: bool,
    pub primary: bool,
    /// Files other launches of Notepad were asked to open.
    pub incoming: Option<Receiver<Vec<PathBuf>>>,
    /// Which colour the swatch menu is for.
    pub swatch: Swatch,
    /// The document a question on screen is about; and the one whose
    /// tab closes once it has been saved.
    pub asked: Weak<RefCell<Document>>,
    pub close_after: Weak<RefCell<Document>>,
    /// A PDF to write, where there is a text engine to set it with.
    pub export_pdf: Option<(Weak<RefCell<Document>>, PathBuf)>,
    /// The kind of file the export being asked about is to be.
    pub export_as: &'static str,
}

fn bind(keys: &mut KeyConfig, key: Key, mods: Modifiers, action: &'static str) {
    keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(key, mods), action));
}

impl App {
    pub fn new() -> App {
        let (ctrl, shift) = (Modifiers::CTRL, Modifiers::CTRL | Modifiers::SHIFT);
        let mut keys = KeyConfig::default();
        for (key, action) in [('n', "file.new"), ('t', "file.new"), ('o', "file.open"), ('s', "file.save"), ('w', "tab.close"), ('p', "file.export-pdf"), ('f', "find.show"), ('h', "find.replace"), ('b', "format.bold"), ('i', "format.italic"), ('u', "format.underline"), ('l', "align.left"), ('e', "align.center"), ('r', "align.right"), ('j', "align.justify"), (']', "size.bigger"), ('[', "size.smaller")] {
            bind(&mut keys, Key::Char(key), ctrl, action);
        }
        for (key, action) in [('s', "file.save-as"), ('x', "format.strike"), ('p', actions::PALETTE), ('7', "list.number"), ('8', "list.bullet"), ('9', "list.check"), ('&', "list.number"), ('*', "list.bullet"), ('(', "list.check")] {
            bind(&mut keys, Key::Char(key), shift, action);
        }
        bind(&mut keys, Key::Tab, ctrl, actions::NEXT_TAB);
        bind(&mut keys, Key::Tab, shift, actions::PREV_TAB);
        bind(&mut keys, Key::F(3), Modifiers::NONE, "find.next");
        bind(&mut keys, Key::F(3), Modifiers::SHIFT, "find.prev");
        bind(&mut keys, Key::Escape, Modifiers::NONE, "find.close");
        let follow = Follow::default();
        App { settings: Settings::load(), seen_reads: follow.reads(), follow, settings_dirty: false, save_at: None, clip: None, pictures: HashMap::new(), fonts: Vec::new(), ops: Vec::new(), front: Weak::new(), front_area: None, took_focus: None, refocus: false, keys, named: Vec::new(), saver: Saver::default(), open: Vec::new(), session_at: None, remembers: true, primary: false, incoming: None, swatch: Swatch::Text, asked: Weak::new(), close_after: Weak::new(), export_pdf: None, export_as: "txt" }
    }

    /// A new document with nothing in it, under the first name no open
    /// document has.
    pub fn blank(&mut self) -> Shared {
        let taken = |name: &str| self.named.iter().any(|n| n == name) || self.open.iter().any(|d| d.try_borrow().is_ok_and(|d| !d.closed && d.name == name));
        let name = (1..).map(|n| if n == 1 { "Untitled".to_owned() } else { format!("Untitled {n}") }).find(|name| !taken(name)).unwrap_or_default();
        self.named.push(name.clone());
        Rc::new(RefCell::new(Document::untitled(name)))
    }

    pub fn inks(&self) -> Inks {
        if self.settings.paper { Inks::PAPER } else { Inks::DARK }
    }

    /// The line under the page: how much there is (or is selected), and
    /// how the document stands with its file.
    fn status(&self, ui: &mut Ui, d: &Document) {
        let m = ui.m;
        let rect = ui.alloc(Vec2::new(FILL, m.px(STATUS_H)));
        let inner = Rect::new(Vec2::new(rect.min.x + m.px(8.0), rect.min.y), Vec2::new(rect.max.x - m.px(8.0), rect.max.y));
        let style = ui.text_style();
        let count = |(words, chars): (usize, usize)| {
            let chars = if chars >= 10_000 { format!("{:.1}k", chars as f64 / 1000.0) } else { chars.to_string() };
            format!("{words} {} \u{00B7} {chars} characters", if words == 1 { "word" } else { "words" })
        };
        let left = match d.ed.selection() {
            Some((a, b)) => format!("{} selected", count(Doc::from_text(&d.ed.doc.plain(a, b)).count())),
            None => count(d.ed.doc.count()),
        };
        ui.text_in_rect(&left, &style, inner, look::TEXT_DIM);
        let (state, color) = match (d.ed.modified(), &d.path) {
            (false, Some(_)) => ("Saved \u{2713}", look::GOOD),
            (false, None) => ("", look::TEXT_DIM),
            (true, Some(_)) => ("Edited \u{00B7} draft kept", look::WARN),
            (true, None) => ("Not saved yet \u{00B7} draft kept", look::WARN),
        };
        ui.text_right(state, &style, inner, color);
    }
}

impl Host for App {
    type Editor = Kind;
    type AreaState = Tab;

    fn editors(&self) -> &[Kind] {
        &KINDS
    }

    fn editor_label(&self, _kind: Kind) -> &str {
        "Document"
    }

    fn title(&self) -> String {
        self.front.upgrade().and_then(|d| d.try_borrow().ok().map(|d| d.name.clone())).unwrap_or_else(|| "Notepad".to_owned())
    }

    fn paints_body(&self, _kind: Kind) -> bool {
        true
    }

    fn title_menus(&self) -> &[(&str, &str)] {
        &[("File", "file"), ("Edit", "edit"), ("Format", "format"), ("Insert", "insert"), ("View", "view")]
    }

    fn menu(&self, name: &str) -> Option<Menu> {
        menu::title_menu(self, name)
    }

    fn palette(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_lowercase();
        menu::COMMANDS.iter().filter(|(_, label)| label.to_lowercase().contains(&q)).map(|(id, label)| ((*id).to_owned(), (*label).to_owned())).collect()
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action).or_else(|| menu::fixed_hint(&action.id).map(str::to_owned))
    }

    fn draw_body(&mut self, _kind: Kind, ui: &mut Ui, cx: &mut AreaCx<Tab>) -> bool {
        let doc = match &cx.state.doc {
            Some(doc) => doc.clone(),
            None => {
                let doc = self.blank();
                cx.state.doc = Some(doc.clone());
                doc
            }
        };
        if self.fonts.is_empty() {
            self.fonts = ui.text.families();
        }
        // The pane with the keyboard: its page takes it when the pane
        // becomes the one, and whenever nothing else holds it.
        let active = cx.active;
        let grab = active && (self.took_focus != Some(cx.area) || std::mem::take(&mut self.refocus) || ui.state.focus.is_none());
        if active {
            self.took_focus = Some(cx.area);
            self.front = Rc::downgrade(&doc);
            self.front_area = Some(cx.area);
        } else if self.took_focus == Some(cx.area) {
            self.took_focus = None;
        }
        let mut d = doc.borrow_mut();
        let Document { ed, view, find, .. } = &mut *d;
        if self.export_pdf.as_ref().is_some_and(|(of, _)| of.ptr_eq(&Rc::downgrade(&doc))) {
            let (_, path) = self.export_pdf.take().unwrap_or_default();
            cx.toast(&match crate::export::pdf(ui.text, &ed.doc, &path) {
                Ok(pages) => format!("Wrote {} ({pages} {})", path.display(), if pages == 1 { "page" } else { "pages" }),
                Err(e) => format!("No PDF: {e}"),
            });
        }
        let tools = toolbar(ui, ed, &self.fonts, BODY);
        if let Some((which, at)) = tools.swatches {
            self.swatch = which;
            cx.request(ShellRequest::ContextMenu(Box::new(menu::swatches(at))));
        }
        if tools.picture {
            cx.request(menu::ask_picture());
        }
        self.refocus |= tools.acted;
        self.refocus |= find.draw(ui, ed);
        let inks = self.inks();
        let height = (ui.remaining_height() - ui.m.px(STATUS_H) - ui.m.gap).max(ui.m.widget_h * 3.0);
        let out = page(ui, ed, view, height, PageIn { inks, body: BODY, page_width: &mut self.settings.page_width, clip: &mut self.clip, pictures: &self.pictures, hits: &find.hits, hit: find.at, grab });
        self.settings_dirty |= out.resized;
        if let Some(at) = out.context {
            cx.request(ShellRequest::ContextMenu(Box::new(menu::context(ed, at))));
        }
        self.status(ui, &d);
        false
    }

    fn draw_item(&mut self, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
        menu::draw_item(self, key, ui, cx)
    }

    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        crate::actions::run(self, action, cx);
    }

    fn key(&self, press: KeyPress, _kind: Option<Kind>) -> Option<Action> {
        self.keys.resolve(&[CTX_WINDOW], &press.to_event(), |_| true).map(KeyItem::action)
    }

    /// The window is closing: nothing is asked, because nothing is lost.
    /// Every document with unsaved work is kept as a draft and comes
    /// back with the next window.
    fn close_requested(&mut self, _main: bool, _cx: &mut HostCx) -> bool {
        self.remember_all();
        true
    }

    fn dropped(&mut self, paths: &[PathBuf], _area: Option<AreaId>, _kind: Option<Kind>, cx: &mut HostCx) {
        for path in paths {
            // A picture goes into the document; anything else is opened.
            let picture = std::fs::read(path).ok().filter(|bytes| lntrn_image::Format::sniff(bytes).is_some());
            match (picture, self.front.upgrade()) {
                (Some(bytes), Some(doc)) => {
                    if let Err(e) = doc.borrow_mut().ed.insert_picture(bytes) {
                        cx.toast(&format!("{}: {e}", path.display()));
                    }
                }
                _ => self.ops.push(Op::Open(path.clone())),
            }
        }
        cx.rebuild();
    }
}
