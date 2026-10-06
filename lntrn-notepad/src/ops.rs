//! What happens once a frame is built, where the shell is at hand: tabs
//! opened and closed as asked, their names kept in step, unsaved work
//! kept as drafts, what is open remembered, and the settings kept in
//! step with `lantern.toml` both ways.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_ui::{AreaId, Dialog, Shell, ShellRequest};

use crate::actions;
use crate::app::{App, Document, Kind, Shared};
use crate::editor::Editor;
use crate::io;
use crate::session::{self, Entry};
use crate::settings::Settings;
use crate::view::input;

/// Something to do to the tabs, or with the clipboard.
pub enum Op {
    /// A tab with an empty document.
    New,
    /// A file in a tab: the one that has it already, else a new one.
    Open(PathBuf),
    Close(Weak<RefCell<Document>>),
    /// Copy the selection; and take it out.
    Copy(bool),
    /// Paste; as text whatever it is.
    Paste(bool),
}

/// How long after the last change a draft is written, and a changed
/// setting too.
const DRAFT_AFTER: f64 = 1.0;

impl App {
    /// Every open document with its pane and place among its tabs.
    fn docs(shell: &Shell<Self>) -> Vec<(AreaId, usize, Shared)> {
        let areas = shell.screen.area_ids().filter_map(|id| shell.screen.area(id).map(|a| (id, a)));
        areas.flat_map(|(id, a)| a.tabs.iter().enumerate().filter_map(move |(i, tab)| tab.state.doc.clone().map(|doc| (id, i, doc)))).collect()
    }

    /// Put a document in a new tab of the pane in front, and show it.
    fn add_tab(&mut self, shell: &mut Shell<Self>, doc: Shared) {
        let Some(area) = shell.screen.active.or_else(|| shell.screen.area_ids().next()) else { return };
        shell.screen.add_tab(area, Kind::Document);
        if let Some(tab) = shell.screen.area_mut(area).and_then(|a| a.tabs.last_mut()) {
            tab.state.doc = Some(doc);
        }
        shell.screen.active = Some(area);
        self.refocus = true;
    }

    fn open_file(&mut self, shell: &mut Shell<Self>, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if let Some((area, tab, _)) = Self::docs(shell).into_iter().find(|(_, _, d)| d.borrow().path.as_ref() == Some(&path)) {
            shell.screen.select_tab(area, tab);
            shell.screen.active = Some(area);
            return;
        }
        match io::load(&path) {
            Ok(doc) => {
                let document = Document::of(path, doc);
                // An empty page nobody has typed on makes way for it.
                match self.front.upgrade().filter(|d| d.borrow().path.is_none() && d.borrow().ed.doc.is_blank() && !d.borrow().ed.modified()) {
                    Some(spare) => *spare.borrow_mut() = document,
                    None => self.add_tab(shell, Rc::new(RefCell::new(document))),
                }
            }
            Err(e) => drop(shell.request(self, ShellRequest::Dialog(Dialog::notice("Couldn't open that", &e)))),
        }
    }

    /// Close a document's tab. The last tab of the last pane is not
    /// taken away: an empty page takes its place.
    fn close_doc(&mut self, shell: &mut Shell<Self>, doc: &Shared) {
        doc.borrow_mut().closed = true;
        let Some((area, tab, _)) = Self::docs(shell).into_iter().find(|(_, _, d)| Rc::ptr_eq(d, doc)) else { return };
        if !shell.screen.close_tab_at(area, tab) && !shell.screen.join(area) {
            let blank = self.blank();
            if let Some(tab) = shell.screen.area_mut(area).and_then(|a| a.tabs.get_mut(tab)) {
                tab.state.doc = Some(blank);
            }
        }
    }

    fn apply_ops(&mut self, shell: &mut Shell<Self>) -> bool {
        let ops = std::mem::take(&mut self.ops);
        let any = !ops.is_empty();
        for op in ops {
            match op {
                Op::New => {
                    let blank = self.blank();
                    self.add_tab(shell, blank);
                }
                Op::Open(path) => self.open_file(shell, path),
                Op::Close(doc) => {
                    if let Some(doc) = doc.upgrade() {
                        self.close_doc(shell, &doc);
                    }
                }
                Op::Copy(cut) => {
                    if let Some(doc) = self.front.upgrade() {
                        input::copy(&mut shell.state, &mut doc.borrow_mut().ed, &mut self.clip, cut);
                    }
                }
                // The clipboard is another app's to hand over: asked for
                // now, the page puts it in once it has come.
                Op::Paste(plain) => {
                    if let Some(doc) = self.front.upgrade() {
                        input::ask_paste(&mut shell.state, &mut doc.borrow_mut().view, plain);
                        shell.state.clipboard_wanted = true;
                    }
                }
            }
        }
        any
    }

    /// Keep the settings in step with `lantern.toml`: write a change
    /// made here once the hand has stopped, and take in one made there.
    fn sync_settings(&mut self, shell: &mut Shell<Self>) -> bool {
        let now = shell.state.now;
        if std::mem::take(&mut self.settings_dirty) {
            self.save_at = Some(now + DRAFT_AFTER * 0.4);
        }
        if let Some(at) = self.save_at {
            if now < at {
                shell.state.request_redraw_after(at - now);
                return false;
            }
            self.save_at = None;
            if let Err(e) = self.settings.save() {
                shell.request(self, ShellRequest::Toast(format!("Settings not saved: {e}")));
            }
            return false;
        }
        if self.seen_reads == self.follow.reads() {
            return false;
        }
        self.seen_reads = self.follow.reads();
        let was = self.settings.clone();
        self.settings = Settings::reload().unwrap_or(was.clone());
        self.settings != was
    }

    /// What is open, as it is to be remembered.
    fn entries(&self) -> Vec<Entry> {
        let entry = |doc: &Shared| {
            let d = doc.borrow();
            Entry { path: d.path.clone(), draft: d.precious().then_some(d.draft), name: d.name.clone(), caret: d.ed.caret, scroll: d.view.scroll, front: self.front.ptr_eq(&Rc::downgrade(doc)) }
        };
        self.open.iter().map(entry).filter(|e| e.path.is_some() || e.draft.is_some()).collect()
    }

    /// Bring back what was open when Notepad last closed, and open
    /// `files`. With files asked for, only unsaved work comes back with
    /// them. A Notepad started while another runs brings nothing back.
    pub fn restore(&mut self, shell: &mut Shell<Self>, files: Vec<PathBuf>) {
        self.primary = self.remembers && session::claim();
        let mut docs: Vec<(Document, bool)> = Vec::new();
        if self.primary {
            let entries = session::load();
            for e in &entries {
                let draft = e.draft.and_then(|id| session::adopt(id).map(|doc| (id, doc)));
                let mut d = match (draft, &e.path) {
                    (Some((id, doc)), path) => {
                        // Unsaved work: as it was left, and still unsaved.
                        let mut d = path.clone().map_or_else(|| Document::untitled(e.name.clone()), |path| Document::of(path, Default::default()));
                        (d.ed, d.draft) = (Editor::of(doc), id);
                        (d.ed.rev, d.drafted, d.changed) = (1, (1, true), (1, 0.0));
                        d
                    }
                    (None, Some(path)) if files.is_empty() => match io::load(path) {
                        Ok(doc) => Document::of(path.clone(), doc),
                        Err(_) => continue,
                    },
                    _ => continue,
                };
                d.ed.caret = d.ed.doc.clamp(e.caret);
                d.view.scroll = e.scroll;
                docs.push((d, e.front));
            }
            for id in session::strays(&entries) {
                if let Some(doc) = session::adopt(id) {
                    let mut d = Document::untitled("Recovered".to_owned());
                    (d.ed, d.draft) = (Editor::of(doc), id);
                    (d.ed.rev, d.drafted, d.changed) = (1, (1, true), (1, 0.0));
                    docs.push((d, false));
                }
            }
        }
        self.named = docs.iter().map(|(d, _)| d.name.clone()).collect();
        let Some(area) = shell.screen.area_ids().next() else { return };
        let front = docs.iter().position(|(_, front)| *front);
        for (i, (doc, _)) in docs.into_iter().enumerate() {
            if i > 0 {
                shell.screen.add_tab(area, Kind::Document);
            }
            if let Some(tab) = shell.screen.area_mut(area).and_then(|a| a.tabs.last_mut()) {
                tab.state.doc = Some(Rc::new(RefCell::new(doc)));
            }
        }
        if let Some(front) = front {
            shell.screen.select_tab(area, front);
        }
        self.ops.extend(files.into_iter().map(Op::Open));
    }

    /// The window is closing: every document with unsaved work is put
    /// on the disk as a draft, and what is open is written down.
    pub fn remember_all(&mut self) {
        if !self.remembers {
            return;
        }
        for doc in &self.open {
            let d = doc.borrow();
            if d.precious() { self.saver.keep(d.draft, &d.ed.doc) } else { self.saver.forget(d.draft) }
        }
        self.saver.flush();
        if self.primary
            && let Err(e) = session::save(&self.entries())
        {
            lntrn_core::log_warn!("what was open was not remembered: {e}");
        }
    }

    /// Look after every open document: its tab's name, its draft, and
    /// its place in what is remembered.
    fn tend(&mut self, shell: &mut Shell<Self>) -> bool {
        let now = shell.state.now;
        let mut current = Self::docs(shell);
        let mut again = false;
        // The shell closes a tab from its strip without asking anyone. A
        // document that would lose work that way is put back, and the
        // question is asked.
        let gone: Vec<Shared> = self.open.iter().filter(|was| !current.iter().any(|(_, _, d)| Rc::ptr_eq(d, was))).cloned().collect();
        for doc in gone {
            let (closed, precious, draft) = (doc.borrow().closed, doc.borrow().precious(), doc.borrow().draft);
            if !closed && precious {
                self.add_tab(shell, doc.clone());
                let ask = actions::ask_about(self, &doc);
                shell.request(self, ask);
                again = true;
            } else if self.remembers {
                self.saver.forget(draft);
            }
        }
        if again {
            current = Self::docs(shell);
        }
        for (area, i, doc) in &current {
            let mut d = doc.borrow_mut();
            let name = format!("{}{}", d.name, if d.ed.modified() { " \u{25CF}" } else { "" });
            if let Some(tab) = shell.screen.area_mut(*area).and_then(|a| a.tabs.get_mut(*i))
                && tab.name.as_deref() != Some(name.as_str())
            {
                tab.name = Some(name);
                again = true;
            }
            if d.ed.rev != d.changed.0 {
                d.changed = (d.ed.rev, now);
            }
            let stands = (d.ed.rev, d.precious());
            if !self.remembers || d.drafted == stands {
                continue;
            }
            // Unsaved work is kept once the typing has paused; a draft
            // that is no longer needed (the document was saved) goes at
            // once.
            let wait = if stands.1 { DRAFT_AFTER - (now - d.changed.1) } else { 0.0 };
            if wait > 0.0 {
                shell.state.request_redraw_after(wait);
                continue;
            }
            if stands.1 { self.saver.keep(d.draft, &d.ed.doc) } else { self.saver.forget(d.draft) }
            d.drafted = stands;
            self.session_at.get_or_insert(now + 0.3);
        }
        let same = self.open.len() == current.len() && self.open.iter().zip(&current).all(|(a, (_, _, b))| Rc::ptr_eq(a, b));
        if !same {
            self.session_at.get_or_insert(now + 0.3);
        }
        self.open = current.into_iter().map(|(_, _, doc)| doc).collect();
        self.named.clear();
        match self.session_at {
            Some(at) if now >= at => {
                self.session_at = None;
                if self.primary
                    && let Err(e) = session::save(&self.entries())
                {
                    lntrn_core::log_warn!("what is open was not remembered: {e}");
                }
            }
            Some(at) => shell.state.request_redraw_after(at - now),
            None => {}
        }
        again
    }

    /// Everything above, once a frame of `shell` is built. `true` when
    /// the frame should be built again with what changed.
    pub fn tick(&mut self, shell: &mut Shell<Self>) -> bool {
        let mut again = self.follow.apply(shell);
        again |= self.sync_settings(shell);
        if let Some(files) = self.incoming.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.ops.extend(files.into_iter().map(Op::Open));
        }
        again |= self.apply_ops(shell);
        again |= self.tend(shell);
        again
    }

    /// Give the GPU the pictures the open documents show, and take back
    /// the ones none of them shows any more.
    fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let mut live: Vec<u64> = Vec::new();
        let mut changed = false;
        for doc in &self.open {
            let d = doc.borrow();
            for placed in d.ed.doc.paras.iter().filter_map(|p| p.picture) {
                live.push(placed.id);
                if !self.pictures.contains_key(&placed.id)
                    && let Some(picture) = d.ed.doc.pictures.get(placed.id)
                {
                    self.pictures.insert(placed.id, images.add(gpu, &picture.image));
                    changed = true;
                }
            }
        }
        self.pictures.retain(|id, handle| {
            let keep = live.contains(id);
            if !keep {
                images.remove(handle.id);
            }
            keep
        });
        changed
    }
}

impl AppHost for App {
    fn waker(&mut self, _waker: Waker) {}

    fn after_rebuild(&mut self, gpu: &Gpu, images: &mut Images, shell: &mut Shell<Self>) -> bool {
        let again = self.tick(shell);
        self.upload(gpu, images) || again
    }
}
