//! What the app does with the language servers: documents kept in step
//! after every rebuild, their answers put on screen, and every problem
//! from every source as one list.

use std::path::Path;

use lntrn_ui::{Shell, ShellRequest};

use crate::app::{App, Goto};
use crate::buffer::Pos;
use crate::editor::lsp_ui::{Completion, Hover, word_start};
use crate::lsp::Event;
use crate::lsp::pos::from_units;
use crate::problems::{LspSpan, Problem};

impl App {
    /// After every rebuild: documents to the servers, answers back.
    pub(crate) fn lsp_pump(&mut self, shell: &mut Shell<Self>) -> bool {
        self.lsp.sync(&self.docs);
        let (mut again, events) = self.lsp.poll();
        for e in events {
            again = true;
            match e {
                Event::Hover { path, pos, text } => {
                    if let Some((doc, asked, anchor)) = self.lsp_ui.asked
                        && asked == pos
                        && self.docs.iter().any(|d| d.id == doc && d.path.as_deref() == Some(&path))
                    {
                        self.lsp_ui.hover = Some(Hover { doc, pos, lines: text.lines().map(str::to_owned).collect(), anchor });
                        self.lsp_ui.asked = None;
                    }
                }
                Event::Definition { path, line, col, end_line, end_col, utf16 } => {
                    let end_col = if end_line == line { end_col } else { col };
                    self.pending_paths.push(path.clone());
                    self.pending_goto = Some((path, Goto::Units { line, col, end_col, utf16 }));
                }
                Event::Completion { path, pos, items } => {
                    if let Some(d) = self.docs.iter().find(|d| d.path.as_deref() == Some(&path))
                        && d.cursor.line == pos.line
                        && d.cursor.col >= pos.col
                        && !items.is_empty()
                    {
                        let anchor = Pos::new(pos.line, word_start(d.line(pos.line), pos.col));
                        self.lsp_ui.completion = Some(Completion { doc: d.id, anchor, items, selected: 0 });
                        match self.lsp_ui.filtered(d).first().copied() {
                            Some(i) => {
                                if let Some(c) = self.lsp_ui.completion.as_mut() {
                                    c.selected = i;
                                }
                            }
                            None => self.lsp_ui.completion = None,
                        }
                    }
                }
                Event::Message(m) => {
                    shell.request(self, ShellRequest::Toast(m));
                }
            }
        }
        again
    }

    /// The path to show for a file: relative to the project when inside it.
    fn shown_path(&self, p: &Path) -> String {
        match &self.project {
            Some(pr) if p.starts_with(&pr.root) => pr.relative(p),
            _ => p.display().to_string(),
        }
    }

    /// The 1-based character column of a server span, from the open
    /// document's text.
    fn char_col(&self, path: &Path, s: &LspSpan) -> Option<usize> {
        let d = self.docs.iter().find(|d| d.path.as_deref() == Some(path))?;
        let line = d.line(s.line.min(d.buffer.line_count().saturating_sub(1)));
        let b = from_units(line, s.col, s.utf16);
        Some(line[..b].chars().count() + 1)
    }

    /// Every problem: what the terminals read off builds and what the
    /// servers report, a build's copy dropped when a server has the same.
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = self.lsp.problems(|p| self.shown_path(p), |p, s| self.char_col(p, s));
        for t in &self.terminals {
            for d in &t.diags.items {
                let dup = d.resolved.as_ref().is_some_and(|r| out.iter().any(|p| p.path.as_deref() == Some(r.as_path()) && p.line == d.line && p.message == d.message));
                if dup {
                    continue;
                }
                let shown = match &d.resolved {
                    Some(p) => self.shown_path(p),
                    None => d.path.clone(),
                };
                out.push(Problem { severity: d.severity, message: d.message.clone(), source: "terminal".into(), path: d.resolved.clone(), shown, line: d.line, col: d.col, span: None });
            }
        }
        out
    }
}
