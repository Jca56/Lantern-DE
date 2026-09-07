//! The integrated terminal: a shell in a pty ([`pty`]), its output parsed
//! ([`parser`]) into a grid ([`grid`], [`csi`]), drawn as a monospace
//! screen ([`render`]) and fed keys ([`input`]). Output is polled while a
//! terminal shows: the harness rebuilds on a timer that runs fast while
//! output flows and slows to a crawl when it stops.

pub mod csi;
pub mod diag;
pub mod grid;
mod grid_edit;
pub mod input;
pub mod links;
pub mod parser;
pub mod pty;
pub mod render;
pub mod search;

use std::path::PathBuf;

use lntrn_app::Waker;
pub use render::draw_terminal;

use self::diag::Diagnostics;
use self::grid::{Grid, Row};
use self::parser::Parser;
use self::pty::Pty;

/// Names a terminal for as long as it exists; never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TermId(pub u64);

pub struct Terminal {
    pub id: TermId,
    pty: Option<Pty>,
    parser: Parser,
    pub grid: Grid,
    /// Frame time of the last output, for the polling rate.
    pub last_output: f64,
    pub exited: Option<i32>,
    cwd: Option<PathBuf>,
    /// A paste was asked for; the clipboard arrives next frame.
    pub paste_pending: bool,
    buf: Vec<u8>,
    waker: Option<Waker>,
    /// Extra environment for the shell (how `claude` finds the editor).
    env: Vec<(String, String)>,
    /// Where a drag started, as a cell boundary `(absolute row, column)`.
    pub sel_anchor: Option<(u64, usize)>,
    /// The selected cells, between two boundaries in either order.
    pub selection: Option<((u64, usize), (u64, usize))>,
    /// Problems read off the output, for the editor's markers.
    pub diags: Diagnostics,
    /// The last path looked up under the pointer and what it meant.
    link_cache: Option<(String, Option<PathBuf>)>,
    /// Find in this terminal, while its bar is open.
    pub search: Option<search::TermSearch>,
}

impl Terminal {
    pub fn new(id: TermId, cwd: Option<PathBuf>, cols: usize, rows: usize, scrollback: usize, waker: Option<Waker>, env: Vec<(String, String)>) -> Self {
        let mut t = Self { id, pty: None, parser: Parser::new(), grid: Grid::new(cols, rows, scrollback), last_output: 0.0, exited: None, cwd, paste_pending: false, buf: Vec::new(), waker, env, sel_anchor: None, selection: None, diags: Diagnostics::default(), link_cache: None, search: None };
        t.spawn();
        t
    }

    /// Whether output has to be polled for (no waker to hand the shell).
    pub fn polls(&self) -> bool {
        self.waker.is_none()
    }

    fn spawn(&mut self) {
        match Pty::spawn(self.cwd.as_deref(), self.grid.cols as u16, self.grid.rows as u16, self.waker.clone(), &self.env) {
            Ok(p) => self.pty = Some(p),
            Err(e) => {
                self.exited = Some(-1);
                let msg = format!("could not start a shell: {e}\r\n");
                let Self { parser, grid, .. } = self;
                parser.feed(msg.as_bytes(), |a| csi::dispatch(grid, a));
            }
        }
    }

    /// The shell again, on a clean screen.
    pub fn respawn(&mut self) {
        self.pty = None;
        self.exited = None;
        self.grid.reset();
        self.parser = Parser::new();
        self.spawn();
    }

    /// Take in whatever the shell wrote. Returns whether anything came.
    pub fn pump(&mut self, now: f64) -> bool {
        let Self { pty, parser, grid, buf, diags, .. } = self;
        let Some(pty) = pty.as_mut() else {
            return false;
        };
        buf.clear();
        let any = pty.drain(buf);
        if any {
            // A bug in the grid must not take the editor down: the screen
            // resets and the bug goes to the log.
            let fed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                parser.feed(buf, |a| {
                    diags.feed(&a);
                    csi::dispatch(grid, a);
                });
            }));
            if fed.is_err() {
                lntrn_core::log_error!("terminal: reset after a bug in the grid");
                grid.reset();
                *parser = Parser::new();
            }
            if !grid.replies.is_empty() {
                pty.write(&grid.replies);
                grid.replies.clear();
            }
            self.last_output = now;
        }
        if self.exited.is_none()
            && let Some(code) = pty.poll_exit()
        {
            self.exited = Some(code);
            let msg = format!("\r\n[process exited with code {code}]  Enter to start again\r\n");
            parser.feed(msg.as_bytes(), |a| csi::dispatch(grid, a));
        }
        if !self.diags.unresolved.is_empty() {
            let cwd = self.cwd_now();
            let roots: Vec<PathBuf> = self.cwd.iter().cloned().collect();
            self.diags.resolve_pending(cwd.as_deref(), &roots);
            self.link_cache = None;
        }
        any
    }

    /// Wipe the screen and the scrollback, as `clear` would.
    pub fn clear(&mut self) {
        let Self { parser, grid, .. } = self;
        parser.feed(b"\x1b[H\x1b[2J\x1b[3J", |a| csi::dispatch(grid, a));
        grid.view_offset = 0;
        self.selection = None;
    }

    /// The file a path printed in this terminal means, looked for from
    /// the shell's folder; the last answer is kept for the next frame.
    pub fn resolve_link(&mut self, path: &str) -> Option<PathBuf> {
        if let Some((p, hit)) = &self.link_cache
            && p == path
        {
            return hit.clone();
        }
        let cwd = self.cwd_now();
        let roots: Vec<PathBuf> = self.cwd.iter().cloned().collect();
        let hit = links::resolve(path, cwd.as_deref(), &roots);
        self.link_cache = Some((path.to_owned(), hit.clone()));
        hit
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if let Some(p) = &self.pty {
            p.write(bytes);
        }
    }

    /// A row as one char per cell (a wide char's second cell is blank).
    pub fn row_chars(row: &Row) -> Vec<char> {
        row.iter().map(|c| if c.spacer { ' ' } else { c.ch }).collect()
    }

    /// The selected text, if anything is selected.
    pub fn selection_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        let text = self.grid.text_between(a, b);
        (!text.is_empty()).then_some(text)
    }

    /// A wheel notch at cell `(col, row)` as the mouse report the program
    /// asked for: SGR, or the X10 bytes.
    pub fn wheel_report(&mut self, up: bool, col: usize, row: usize) {
        let button = if up { 64 } else { 65 };
        let bytes = if self.grid.mouse_sgr {
            format!("\x1b[<{button};{};{}M", col + 1, row + 1).into_bytes()
        } else {
            vec![0x1b, b'[', b'M', 32 + button, (32 + col + 1).min(255) as u8, (32 + row + 1).min(255) as u8]
        };
        self.write(&bytes);
    }

    /// Pasted text, wrapped when the program asked for bracketed pastes.
    pub fn paste(&mut self, text: &str) {
        let mut out = Vec::new();
        if self.grid.bracketed_paste {
            out.extend_from_slice(b"\x1b[200~");
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(b"\x1b[201~");
        } else {
            out.extend_from_slice(text.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
        }
        self.write(&out);
        self.grid.view_offset = 0;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.grid.cols && rows == self.grid.rows {
            return;
        }
        self.grid.resize(cols, rows);
        if let Some(p) = &self.pty {
            p.resize(cols as u16, rows as u16);
        }
    }

    /// Where the shell is now, read from `/proc`.
    pub fn cwd_now(&self) -> Option<PathBuf> {
        let pid = self.pty.as_ref()?.pid();
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    /// The program's title, else the shell's folder.
    pub fn title(&self) -> String {
        if !self.grid.title.is_empty() {
            return self.grid.title.clone();
        }
        match self.cwd_now() {
            Some(cwd) => {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                match home.as_ref().and_then(|h| cwd.strip_prefix(h).ok()) {
                    Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
                    Some(rest) => format!("~/{}", rest.display()),
                    None => cwd.display().to_string(),
                }
            }
            None => "Terminal".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use lntrn_ui::testing::Harness;

    /// A real shell in a pty, drawn through the headless harness: what
    /// the Terminal editor does on its first frames.
    #[test]
    fn spawns_and_draws() {
        let mut term = Terminal::new(TermId(1), None, 80, 24, 100, None, Vec::new());
        assert!(term.exited.is_none(), "the shell started");
        term.write(b"echo lntrn-ok\n");
        let mut seen = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            term.pump(0.0);
            let screen: String = (0..term.grid.rows).map(|y| term.grid.row(y).iter().map(|c| c.ch).collect::<String>()).collect::<Vec<_>>().join("\n");
            if screen.contains("lntrn-ok") {
                seen = true;
                break;
            }
        }
        assert!(seen, "the echo came back through the grid");
        let settings = Settings::default();
        let mut h = Harness::new(1200.0, 800.0);
        for _ in 0..3 {
            h.frame(|ui| {
                draw_terminal(ui, &mut term, &settings, true);
            });
            h.advance(0.05);
        }
        assert!(term.grid.cols > 2 && term.grid.rows >= 1);
    }
}
