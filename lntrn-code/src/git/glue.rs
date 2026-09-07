//! What the app does with git: the gutter marks of a document against
//! its HEAD copy, and a read-only diff of a file against HEAD.

use std::path::Path;

use lntrn_ui::{Shell, ShellRequest};

use crate::app::App;
use crate::diff_view::{DiffDoc, DiffId};
use crate::doc::DocId;
use crate::git::Blob;
use crate::git::gutter::{LineMark, marks};

impl App {
    /// The gutter marks of a document against its HEAD copy, computed
    /// when the text or HEAD changed (and the typing paused).
    pub(crate) fn gutter_marks(&mut self, id: DocId, now: f64) -> Vec<LineMark> {
        let Some(git) = self.git.as_mut() else {
            return Vec::new();
        };
        let Some(doc) = self.docs.iter().find(|d| d.id == id) else {
            return Vec::new();
        };
        let Some(path) = doc.path.clone() else {
            return Vec::new();
        };
        let head = git.head.clone();
        if let Some((edit, h, marks)) = self.git_marks.get(&id)
            && *edit == doc.last_edit
            && *h == head
        {
            return marks.clone();
        }
        // Typing: wait for a pause before diffing the whole file.
        if now - doc.last_edit < 0.3
            && let Some((_, _, marks)) = self.git_marks.get(&id)
        {
            return marks.clone();
        }
        let old = match git.blob(&path) {
            Some(Blob::Text(t)) => t.clone(),
            Some(Blob::Missing) => String::new(),
            None => return self.git_marks.get(&id).map(|(_, _, m)| m.clone()).unwrap_or_default(),
        };
        let marks = marks(&old, doc.buffer.lines());
        self.git_marks.insert(id, (doc.last_edit, head, marks.clone()));
        marks
    }

    /// A read-only diff of `path` against HEAD, once the HEAD copy is
    /// here; `None` while it is on its way.
    pub(crate) fn git_diff_doc(&mut self, path: &Path, now: f64) -> Option<DiffId> {
        let git = self.git.as_mut()?;
        let old = match git.blob(path)? {
            Blob::Text(t) => t.clone(),
            Blob::Missing => String::new(),
        };
        let _ = now;
        let new = match self.doc_by_path(path) {
            Some(i) => self.docs[i].buffer.to_text(),
            None => std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let did = DiffId(self.next_diff);
        self.next_diff += 1;
        let mut d = DiffDoc::new(did, &format!("HEAD ↔ {name}"), path, &old, new, None);
        d.read_only = true;
        self.diffs.push(d);
        Some(did)
    }

    /// Take in what git answered: status, toasts for what ran, and diffs
    /// at a commit opened from the history. Returns whether to rebuild.
    pub(crate) fn git_poll(&mut self, shell: &mut Shell<Self>) -> bool {
        let mut again = false;
        if let Some(g) = self.git.as_mut() {
        again |= g.poll();
        if let Some(delay) = g.tick(shell.state.now) {
            shell.state.request_redraw_after(delay);
        }
        if let Some((ok, output)) = g.last_output.take() {
            let msg = if ok { if output.is_empty() { "Done".to_owned() } else { output } } else { format!("git: {output}") };
            shell.request(self, ShellRequest::Toast(msg));
        }
        }
        // A file at a commit, asked for in the history: a read-only diff.
        if let Some(g) = self.git.as_mut() {
        let root = g.root.clone();
        let diffs = std::mem::take(&mut g.commit_diffs);
        for d in diffs {
            let did = crate::diff_view::DiffId(self.next_diff);
            self.next_diff += 1;
            let name = std::path::Path::new(&d.rel).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mut doc = crate::diff_view::DiffDoc::new(did, &format!("{} · {name}", d.short), &root.join(&d.rel), &d.old, d.new, None);
            doc.read_only = true;
            self.diffs.push(doc);
            self.show_diff(shell, did);
            again = true;
        }
        }
        again
    }
}
