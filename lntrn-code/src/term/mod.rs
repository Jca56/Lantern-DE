//! The integrated terminal: Lantern UI's (`lntrn-term`: the pty, the
//! grid, the drawing, the keys), with the one thing the editor adds, the
//! problems it reads off the output as it flows ([`diag`]).

pub mod diag;

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::Arc;

use lntrn_app::Waker;
use lntrn_term::{Launch, Look};
pub use lntrn_term::{TermId, TermOut, links, parser};
use lntrn_ui::Ui;

use self::diag::Diagnostics;
use crate::settings::Settings;

/// A terminal and the problems read off what it has printed. Everything
/// the terminal itself has (its grid, its selection, its bell) is reached
/// through it as if it were one.
pub struct Terminal {
    term: lntrn_term::Terminal,
    /// Problems read off the output, for the editor's markers.
    pub diags: Diagnostics,
}

impl Deref for Terminal {
    type Target = lntrn_term::Terminal;

    fn deref(&self) -> &lntrn_term::Terminal {
        &self.term
    }
}

impl DerefMut for Terminal {
    fn deref_mut(&mut self) -> &mut lntrn_term::Terminal {
        &mut self.term
    }
}

impl Terminal {
    /// The user's shell in `cwd`, with `env` added to its environment
    /// (how `claude` finds the editor).
    pub fn new(id: TermId, cwd: Option<PathBuf>, cols: usize, rows: usize, scrollback: usize, waker: Option<Waker>, env: Vec<(String, String)>) -> Self {
        let wake = waker.map(|w| Arc::new(move || w.wake()) as lntrn_term::Wake);
        let launch = Launch { cwd, command: None, env, program: "lntrn-code".to_owned() };
        Self { term: lntrn_term::Terminal::new(id, launch, cols, rows, scrollback, wake), diags: Diagnostics::default() }
    }

    /// Take in whatever the shell wrote, reading it for problems on the
    /// way. Returns whether anything came.
    pub fn pump(&mut self, now: f64) -> bool {
        let Self { term, diags } = self;
        let any = term.pump_with(now, |a| diags.feed(a));
        if !diags.unresolved.is_empty() {
            let cwd = term.cwd_now();
            let roots: Vec<PathBuf> = term.start_dir().map(std::path::Path::to_path_buf).into_iter().collect();
            diags.resolve_pending(cwd.as_deref(), &roots);
            term.forget_links();
        }
        any
    }
}

/// Draw `term` with the editor's font and the terminal colors of its
/// settings, taking in its output first. See [`lntrn_term::draw_terminal`].
pub fn draw_terminal(ui: &mut Ui, term: &mut Terminal, settings: &Settings, area_active: bool, grab_focus: bool) -> TermOut {
    term.pump(ui.state.now);
    let look = Look { font_size: settings.font_size, text: settings.terminal.text, background: settings.terminal.background, ..Look::default() };
    lntrn_term::draw_terminal(ui, &mut term.term, &look, area_active, grab_focus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_ui::testing::Harness;

    /// A real shell in a pty, drawn through the headless harness: what
    /// the Terminal editor does on its first frames.
    #[test]
    fn spawns_and_draws() {
        let mut term = Terminal::new(TermId(1), None, 80, 24, 100, None, Vec::new());
        assert!(term.exited.is_none(), "the shell started");
        term.write(b"echo lntrn-ok\n");
        let settings = Settings::default();
        let mut h = Harness::new(1200.0, 800.0);
        let mut seen = false;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            // Drawing takes the output in.
            h.frame(|ui| {
                draw_terminal(ui, &mut term, &settings, true, false);
            });
            h.advance(0.05);
            let screen: String = (0..term.grid.rows).map(|y| term.grid.row(y).iter().map(|c| c.ch).collect::<String>()).collect::<Vec<_>>().join("\n");
            if screen.contains("lntrn-ok") {
                seen = true;
                break;
            }
        }
        assert!(seen, "the echo came back through the grid");
        assert!(term.grid.cols > 2 && term.grid.rows >= 1);
    }
}
