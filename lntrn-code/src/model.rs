//! The app's plain data: its editor kinds, what a tab of an area holds,
//! and the clipboard operations a menu can ask for.

use crate::diff_view::DiffId;
use crate::doc::DocId;
use crate::term::TermId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Code,
    Files,
    Terminal,
    Preview,
    Preferences,
    Keys,
    /// A change Claude Code proposes, to accept or reject.
    Diff,
    /// The errors and warnings read off the terminals' output.
    Problems,
}

pub const EDITORS: [Editor; 8] = [Editor::Code, Editor::Files, Editor::Terminal, Editor::Problems, Editor::Preview, Editor::Diff, Editor::Preferences, Editor::Keys];

/// What one tab of an area holds: the documents of a Code editor and
/// which shows, or the terminal of a Terminal editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabState {
    pub docs: Vec<DocId>,
    pub current: usize,
    pub term: Option<TermId>,
    /// The proposed change a Diff editor shows.
    pub diff: Option<DiffId>,
}

/// A clipboard operation asked for by a menu, done where the shell's
/// state is at hand.
pub enum ClipOp {
    Copy { cut: bool },
    Paste,
    Set(String),
}
