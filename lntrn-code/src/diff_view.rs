//! A proposed change from Claude Code: the old and new text, their line
//! diff, and the Diff editor that shows it with Accept and Reject.

use std::path::{Path, PathBuf};

use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::GlyphQuad;
use lntrn_ui::{FILL, Ui};

use crate::bridge::ClientId;
use crate::bridge::diff::{Kind, Row, counts, diff_lines};
use crate::editor::{cell_metrics, code_style};
use crate::json::Json;
use crate::settings::Settings;
use crate::syntax::{Language, LexState, Token, lex_line};
use crate::text_util::expand_tabs;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DiffId(pub u64);

pub struct DiffDoc {
    pub id: DiffId,
    pub tab_name: String,
    pub path: PathBuf,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub new_text: String,
    pub rows: Vec<Row>,
    pub added: usize,
    pub removed: usize,
    pub lang: Language,
    /// The CLI's request, answered when the user decides.
    pub pending: Option<(ClientId, Json)>,
    /// Just to look at (a file against HEAD): no accept or reject.
    pub read_only: bool,
}

impl DiffDoc {
    pub fn new(id: DiffId, tab_name: &str, path: &Path, old_text: &str, new_text: String, pending: Option<(ClientId, Json)>) -> Self {
        let old_lines: Vec<String> = if old_text.is_empty() { Vec::new() } else { old_text.lines().map(str::to_owned).collect() };
        let new_lines: Vec<String> = if new_text.is_empty() { Vec::new() } else { new_text.lines().map(str::to_owned).collect() };
        let rows = {
            let a: Vec<&str> = old_lines.iter().map(String::as_str).collect();
            let b: Vec<&str> = new_lines.iter().map(String::as_str).collect();
            diff_lines(&a, &b)
        };
        let (added, removed) = counts(&rows);
        let lang = Language::detect(path, new_lines.first().map_or("", String::as_str));
        Self { id, tab_name: tab_name.to_owned(), path: path.to_path_buf(), old_lines, new_lines, new_text, rows, added, removed, lang, pending, read_only: false }
    }

    /// The first changed row, to scroll to.
    pub fn first_change(&self) -> usize {
        self.rows.iter().position(|r| r.kind != Kind::Same).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffOut {
    pub accept: bool,
    pub reject: bool,
}

/// The header's part: what changed, and the two buttons.
pub fn draw_diff_header(ui: &mut Ui, d: &DiffDoc) -> DiffOut {
    let mut out = DiffOut::default();
    let name = d.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    ui.label_dim(&format!("{name}   +{} −{}", d.added, d.removed));
    if d.read_only {
        if ui.button("Close").clicked {
            out.reject = true;
        }
        return out;
    }
    if ui.button("Accept").clicked {
        out.accept = true;
    }
    if ui.button("Reject").clicked {
        out.reject = true;
    }
    out
}

/// The unified diff, one row per line, colored by what happened.
pub fn draw_diff(ui: &mut Ui, d: &DiffDoc, settings: &Settings) {
    let style = code_style(ui, settings);
    let (cell_w, lh) = cell_metrics(ui, &style);
    let tab = settings.tab();
    let theme = ui.theme;
    let digits = d.old_lines.len().max(d.new_lines.len()).max(1).to_string().len().max(2);
    let gutter_w = ((digits * 2 + 4) as f64 * cell_w).round();
    let added_bg = Color::hex(0x2EA043).fade(0.22);
    let removed_bg = Color::hex(0xF85149).fade(0.22);
    let id = ui.id("diff");
    // First showing: start at the first change.
    let slot = ui.state.floats(id, [-1.0; 4]);
    if slot[0] < 0.0 {
        slot[0] = 1.0;
        let first = d.first_change().saturating_sub(3) as f64;
        ui.state.scroll(id).offset.y = first * lh;
    }
    let colors = &settings.colors;
    let lang = d.lang;
    ui.virtual_list("diff", d.rows.len(), lh, None, |ui, i| {
        let row = d.rows[i];
        let x0 = ui.cursor().x;
        let y = ui.cursor().y;
        let width = ui.avail_width();
        let line = Rect::from_min_size(Vec2::new(x0, y), Vec2::new(width, lh));
        match row.kind {
            Kind::Added => ui.draw.rect(line, added_bg),
            Kind::Removed => ui.draw.rect(line, removed_bg),
            Kind::Same => {}
        }
        // Old and new line numbers, then the marker.
        let mut quads: Vec<GlyphQuad> = Vec::new();
        let dim = theme.text_dim.fade(0.7).to_gpu();
        let nums = format!("{:>w$} {:>w$} {}", row.old.map_or(String::new(), |n| (n + 1).to_string()), row.new.map_or(String::new(), |n| (n + 1).to_string()), match row.kind { Kind::Added => "+", Kind::Removed => "−", Kind::Same => " " }, w = digits);
        ui.text.place(&nums, &style, (x0 + cell_w * 0.5) as f32, y as f32, 1.0e6, dim, &mut quads);
        ui.draw.glyphs(&quads);
        let text = match row.kind {
            Kind::Removed => row.old.map(|n| d.old_lines[n].as_str()),
            _ => row.new.map(|n| d.new_lines[n].as_str()),
        }
        .unwrap_or("");
        if !text.is_empty() {
            let mut expanded = String::new();
            let mut cells = Vec::new();
            expand_tabs(text, tab, &mut expanded, &mut cells);
            let tx = x0 + gutter_w;
            quads.clear();
            ui.text.place(&expanded, &style, tx as f32, y as f32, 1.0e6, colors.text.to_gpu(), &mut quads);
            let mut tokens: Vec<Token> = Vec::new();
            lex_line(lang, text, LexState::Normal, &mut tokens);
            if !tokens.is_empty() {
                let spans: Vec<(u32, u32, crate::syntax::TokenKind)> = tokens.iter().map(|t| (cells[t.start as usize], cells[(t.end as usize).min(cells.len() - 1)], t.kind)).collect();
                let mut ti = 0;
                for q in &mut quads {
                    let c = ((q.x + q.w * 0.5 - tx as f32) / cell_w as f32).floor().max(0.0) as u32;
                    while ti < spans.len() && spans[ti].1 <= c {
                        ti += 1;
                    }
                    if ti < spans.len() && spans[ti].0 <= c {
                        q.color = colors.of(spans[ti].2).to_gpu();
                    }
                }
            }
            ui.draw.glyphs(&quads);
        }
        ui.alloc(Vec2::new(FILL, lh));
    });
}
