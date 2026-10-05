//! Opening files as what they are: text and pictures into a file tab,
//! a Studio document and the pictures we cannot read into the apps that
//! can. Also the tabs of last time, and what a tab is called and shows
//! whichever it holds.

use std::path::Path;

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_ui::{Shell, ShellRequest};

use crate::app::App;
use crate::doc::DocId;
use crate::picture::{Opens, Picture, is_svg, opens, title_of};
use crate::session::Session;

impl App {
    /// The picture in the focused tab, when it holds one.
    pub fn focus_picture(&self) -> Option<&Picture> {
        self.focus_doc.and_then(|id| self.pictures.get(id))
    }

    pub fn focus_picture_mut(&mut self) -> Option<&mut Picture> {
        let id = self.focus_doc?;
        self.pictures.get_mut(id)
    }

    /// The name on a tab, be it a document's or a picture's.
    pub fn tab_title(&self, id: DocId) -> Option<&str> {
        self.doc(id).map(|d| d.title.as_str()).or_else(|| self.pictures.get(id).map(|p| p.title.as_str()))
    }

    /// The file a tab shows, be it a document or a picture.
    pub fn tab_path(&self, id: DocId) -> Option<&Path> {
        match self.doc(id) {
            Some(d) => d.path.as_deref(),
            None => self.pictures.get(id).map(|p| p.path.as_path()),
        }
    }

    /// The picture for `path`, its read started if it is not open yet.
    pub(crate) fn load_picture(&mut self, path: &Path) -> DocId {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(p) = self.pictures.by_path(&path) {
            return p.id;
        }
        let id = self.next_doc_id();
        self.pictures.open(id, path, self.waker.clone());
        self.session_dirty = true;
        id
    }

    /// Open `path` as its kind asks: text and pictures into a file tab, a
    /// Studio document and the pictures we cannot read into the apps
    /// that can. `as_text` reads it as text whatever it is, and so does
    /// a place in its text waiting to be gone to.
    pub(crate) fn open_path(&mut self, path: &Path, as_text: bool, shell: &mut Shell<Self>) {
        let as_text = as_text || self.pending_goto.as_ref().is_some_and(|(p, _)| p == path) || self.pending_select.as_ref().is_some_and(|s| s.path == path);
        let name = title_of(path);
        let opened = match if as_text { Opens::Text } else { opens(path) } {
            Opens::Text => self.load_doc(path),
            Opens::Picture if path.is_file() => Ok(self.load_picture(path)),
            Opens::Picture => Err(format!("{}: no such file", path.display())),
            Opens::Studio => {
                crate::launch::open_studio(path);
                shell.request(self, ShellRequest::Toast(format!("Opening {name} in Lantern Studio")));
                return;
            }
            Opens::External => {
                crate::launch::open_external(&path.display().to_string());
                shell.request(self, ShellRequest::Toast(format!("Opening {name} in its default app")));
                return;
            }
        };
        match opened {
            Ok(id) => self.pending_docs.push(id),
            Err(e) => {
                shell.request(self, ShellRequest::Toast(format!("Could not open {e}")));
            }
        }
    }

    /// The tabs of last time: text as text, pictures as pictures.
    /// Nothing of last time starts another app.
    pub(crate) fn restore_tabs(&mut self, session: &Session) {
        for (p, _, _) in &session.open {
            match opens(p) {
                Opens::Text => self.pending_paths.push(p.clone()),
                // Open as text last time on purpose: an SVG's source.
                Opens::Picture if is_svg(p) => self.pending_text.push(p.clone()),
                // Open as text from before pictures showed here.
                Opens::Picture => self.pending_paths.push(p.clone()),
                Opens::Studio | Opens::External => {}
            }
        }
        self.pending_paths.extend(session.pictures.iter().cloned());
    }

    /// After every rebuild: reads in, textures up. Whether to draw again.
    pub(crate) fn pictures_pump(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let landed = self.pictures.land();
        self.pictures.upload(gpu, images) || landed
    }
}
