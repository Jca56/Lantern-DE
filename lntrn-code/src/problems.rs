//! Problems from anywhere: what the terminals read off build output and
//! what the language servers report, as one list ([`Problem`]), and the
//! Problems editor that shows them, a click jumping to the code.

use std::path::PathBuf;

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }
}

/// A span in a file as a language server gave it: 0-based lines, columns
/// in the server's units (see [`crate::lsp::pos`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LspSpan {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub utf16: bool,
}

/// One problem, wherever it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    pub message: String,
    /// `rustc`, `rust-analyzer`, `terminal`...
    pub source: String,
    /// The file, when it was found.
    pub path: Option<PathBuf>,
    /// The path to show.
    pub shown: String,
    /// 1-based line and character column, for the list.
    pub line: usize,
    pub col: usize,
    /// The exact span, when a server gave one.
    pub span: Option<LspSpan>,
}

pub struct ProblemRow {
    pub severity: Severity,
    /// `src/app.rs:12:5`, relative to the project when it can be.
    pub place: String,
    pub message: String,
    /// The file was found on disk: the row can be opened.
    pub openable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProblemsOut {
    pub open: Option<usize>,
    pub clear: bool,
}

/// The color a severity draws in.
pub fn severity_color(ui: &Ui, s: Severity) -> lntrn_math::Color {
    match s {
        Severity::Error => ui.theme.close,
        Severity::Warning => ui.theme.accent,
        Severity::Info => ui.theme.focus,
        Severity::Hint => ui.theme.text_dim,
    }
}

pub fn draw_problems(ui: &mut Ui, rows: &[ProblemRow]) -> ProblemsOut {
    let mut out = ProblemsOut::default();
    let errors = rows.iter().filter(|r| r.severity == Severity::Error).count();
    let warnings = rows.iter().filter(|r| r.severity == Severity::Warning).count();
    ui.row(|ui| {
        let summary = match (errors, warnings) {
            (0, 0) => "No problems".to_owned(),
            (e, w) => format!("{e} error{} · {w} warning{}", if e == 1 { "" } else { "s" }, if w == 1 { "" } else { "s" }),
        };
        ui.heading(&summary);
        let one = ui.m.widget_h * 2.2 + ui.m.gap;
        let spacer = (ui.avail_width() - one).max(0.0);
        ui.alloc(Vec2::new(spacer, ui.m.widget_h));
        if !rows.is_empty() && ui.button("Clear").clicked {
            out.clear = true;
        }
    });
    if rows.is_empty() {
        ui.label_dim("Errors and warnings from builds run in the terminal show up here.");
        return out;
    }
    let m = ui.m;
    let style = ui.text_style();
    let h = m.widget_h;
    ui.scroll_area("problems", None, |ui| {
        for (i, row) in rows.iter().enumerate() {
            let id = ui.id("problem").with_index(i);
            let rect = ui.alloc(Vec2::new(FILL, h));
            let r = ui.interact(id, rect, Sense::CLICK);
            ui.focusable(id, rect);
            let theme = ui.theme;
            if r.hovered && row.openable {
                ui.state.cursor_icon = CursorIcon::Pointer;
                ui.fill(rect, theme.hover(theme.panel));
            }
            if r.clicked && row.openable {
                out.open = Some(i);
            }
            let color = severity_color(ui, row.severity);
            let dot_r = (h * 0.14).round().max(m.px(3.0));
            ui.draw.circle(Vec2::new(rect.min.x + m.pad + dot_r, rect.center().y), dot_r, color);
            let x0 = rect.min.x + m.pad * 2.0 + dot_r * 2.0;
            let place_w = ui.measure(&row.place, &style);
            let place_x = (rect.max.x - m.pad - place_w).max(x0);
            let msg_rect = Rect::new(Vec2::new(x0, rect.min.y), Vec2::new((place_x - m.pad).max(x0), rect.max.y));
            let text_color = if row.openable { theme.text } else { theme.text_dim };
            ui.text_in_rect(&row.message, &style, msg_rect, text_color);
            ui.text_in_rect(&row.place, &style, Rect::new(Vec2::new(place_x, rect.min.y), rect.max), theme.text_dim);
            ui.focus_ring(id, rect);
        }
    });
    out
}
