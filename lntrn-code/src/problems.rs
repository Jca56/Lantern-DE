//! The Problems editor: every error and warning the terminals read off
//! their output, one row each, a click to jump to it in the code.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

use crate::term::diag::Severity;

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
