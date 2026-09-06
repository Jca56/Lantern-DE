//! What the code view draws around the text: the selection, search
//! hits, the other occurrences of the word at the caret, indent guides,
//! problem marks, git marks, and the colors brackets take by depth.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::Ui;

use crate::buffer::Range;
use crate::doc::Doc;
use crate::editor::fold::Layout;
use crate::editor::wrap::Wrap;
use crate::git::gutter::{LineMark, MarkKind};
use crate::problems::severity_color;
use crate::settings::{GitColors, SyntaxColors};
use crate::term::diag::Severity;
use crate::text_util::{cell_of_byte, cell_width, indent_of, is_word, word_at};

/// A problem to mark: a 0-based line and byte column (and where it
/// ends on that line, when known), and what to say about it when the
/// pointer rests there.
pub struct DiagMark {
    pub line: usize,
    pub col: usize,
    pub end: Option<usize>,
    pub severity: Severity,
    pub message: String,
}

/// Where the text sits this frame: the row layout and the geometry.
pub struct Grid<'a> {
    pub layout: &'a Layout,
    pub wrap: &'a Wrap,
    pub vp: Rect,
    pub origin: Vec2,
    pub gutter_w: f64,
    pub cell_w: f64,
    pub lh: f64,
    pub tab: usize,
    /// The rows on screen.
    pub first_row: usize,
    pub last_row: usize,
}

impl Grid<'_> {
    pub fn text_x0(&self) -> f64 {
        self.origin.x + self.gutter_w
    }

    pub fn row_top(&self, row: usize) -> f64 {
        self.origin.y + row as f64 * self.lh
    }

    /// The top of the first row `line` is on.
    pub fn row_y(&self, line: usize) -> f64 {
        self.row_top(self.layout.row_of(line))
    }

    /// The row byte `col` of `line` is on.
    pub fn pos_row(&self, line: usize, col: usize) -> usize {
        self.layout.row_of(line) + self.wrap.seg_of(line, col)
    }

    /// The top of the row byte `col` of `line` is on.
    pub fn pos_y(&self, line: usize, col: usize) -> f64 {
        self.row_top(self.pos_row(line, col))
    }

    /// Whether `line` has a row on screen.
    pub fn shown(&self, line: usize) -> bool {
        let first = self.layout.row_of(line);
        let last = first + self.layout.rows_of(line);
        !self.layout.hidden(line) && last > self.first_row && first < self.last_row
    }

    /// The left edge of byte `col` of `line`, on whichever row it wrapped to.
    pub fn cell_x(&self, doc: &Doc, line: usize, col: usize) -> f64 {
        let cell = cell_of_byte(doc.line(line), self.tab, col);
        let seg = self.wrap.seg_of(line, col);
        let (_, cell0) = self.wrap.seg_start(line, seg);
        self.text_x0() + (self.wrap.hang(line, seg) + cell - cell0) as f64 * self.cell_w
    }

    /// The bytes row `seg` of `line` shows.
    pub fn seg_range(&self, doc: &Doc, line: usize, seg: usize) -> (usize, usize) {
        let (s, _) = self.wrap.seg_start(line, seg);
        (s, self.wrap.seg_end(line, seg).unwrap_or(doc.line(line).len()))
    }

    /// The right edge of the text on row `seg` of `line`.
    pub fn row_end_x(&self, doc: &Doc, line: usize, seg: usize) -> f64 {
        let (s, e) = self.seg_range(doc, line, seg);
        let (_, cell0) = self.wrap.seg_start(line, seg);
        let cells = cell_of_byte(doc.line(line), self.tab, e).max(cell_of_byte(doc.line(line), self.tab, s));
        self.text_x0() + (self.wrap.hang(line, seg) + cells - cell0) as f64 * self.cell_w
    }

    /// The rows on screen: `(row, line, seg)`.
    pub fn rows(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        (self.first_row..self.last_row).map(|r| {
            let (l, s) = self.layout.seg_at(r);
            (r, l, s)
        })
    }

    /// The lines with a row on screen, each once, in row order.
    pub fn lines(&self) -> impl Iterator<Item = usize> + '_ {
        let mut last = None;
        self.rows().filter_map(move |(_, l, _)| if last == Some(l) { None } else { last = Some(l); Some(l) })
    }
}

pub fn selection(ui: &mut Ui, doc: &Doc, g: &Grid) {
    let sel = doc.selection();
    if sel.is_empty() {
        return;
    }
    let color = ui.theme.selection.fade(0.45);
    let m = ui.m;
    for (row, line, seg) in g.rows().filter(|&(_, l, _)| l >= sel.start.line && l <= sel.end.line) {
        let (s, e) = g.seg_range(doc, line, seg);
        let a = if line == sel.start.line { sel.start.col.max(s) } else { s };
        let b = if line == sel.end.line { sel.end.col.min(e) } else { e };
        if a > b || (line == sel.start.line && sel.start.col > e) || (line == sel.end.line && sel.end.col < s) {
            continue;
        }
        let last_seg = g.wrap.seg_end(line, seg).is_none();
        let x0 = g.cell_x(doc, line, a);
        let mut x1 = g.cell_x(doc, line, b).max(x0);
        if line < sel.end.line && last_seg {
            x1 += g.cell_w * 0.5;
        }
        let y = g.row_top(row);
        ui.draw.rect(Rect::new(Vec2::new(x0, y), Vec2::new(x1.max(x0 + m.px(2.0)), y + g.lh)), color);
    }
}

pub fn matches(ui: &mut Ui, doc: &Doc, g: &Grid, matches: &[Range], current: Option<usize>) {
    let m = ui.m;
    let theme = ui.theme;
    for (i, mr) in matches.iter().enumerate() {
        if !g.shown(mr.start.line) {
            continue;
        }
        let y = g.pos_y(mr.start.line, mr.start.col);
        let x1 = if g.pos_row(mr.end.line, mr.end.col) == g.pos_row(mr.start.line, mr.start.col) {
            g.cell_x(doc, mr.end.line, mr.end.col)
        } else {
            g.row_end_x(doc, mr.start.line, g.wrap.seg_of(mr.start.line, mr.start.col))
        };
        let rect = Rect::new(Vec2::new(g.cell_x(doc, mr.start.line, mr.start.col), y), Vec2::new(x1, y + g.lh));
        let is_current = current == Some(i);
        ui.draw.rounded_rect(rect, m.radius * 0.5, theme.accent.fade(if is_current { 0.5 } else { 0.22 }));
        if is_current {
            ui.draw.stroke_rect(rect, m.border, m.radius * 0.5, theme.accent);
        }
    }
}

/// Every other place the word at the caret appears on screen.
pub fn occurrences(ui: &mut Ui, doc: &Doc, g: &Grid) {
    let sel = doc.selection();
    let cur = doc.cursor;
    let line = doc.line(cur.line);
    let (a, b) = if sel.is_empty() {
        word_at(line, cur.col)
    } else if sel.start.line == sel.end.line {
        (sel.start.col, sel.end.col)
    } else {
        return;
    };
    let word = &line[a.min(line.len())..b.min(line.len())];
    if word.chars().count() < 2 || !word.chars().all(is_word) {
        return;
    }
    let color = ui.theme.selection.fade(0.2);
    let m = ui.m;
    for l in g.lines() {
        let text = doc.line(l);
        let mut from = 0;
        while let Some(at) = text[from..].find(word) {
            let s = from + at;
            let e = s + word.len();
            from = e;
            let before = text[..s].chars().next_back().is_none_or(|c| !is_word(c));
            let after = text[e..].chars().next().is_none_or(|c| !is_word(c));
            if !before || !after || (l == cur.line && s == a) {
                continue;
            }
            let y = g.pos_y(l, s);
            ui.draw.rounded_rect(Rect::new(Vec2::new(g.cell_x(doc, l, s), y), Vec2::new(g.cell_x(doc, l, e), y + g.lh)), m.radius * 0.4, color);
        }
    }
}

/// One guide per indent level to the left of each line, colored like the
/// bracket that opens that level; the block the caret is in draws its
/// guide bright. A blank line takes the indent of the next line with text.
pub fn indent_guides(ui: &mut Ui, doc: &Doc, g: &Grid, colors: &SyntaxColors) {
    let tab = g.tab.max(1);
    let n = doc.buffer.line_count();
    let w = ui.m.border.max(1.0);
    let indent = |l: usize| -> Option<usize> {
        let t = doc.line(l);
        if t.trim().is_empty() { None } else { Some(cell_width(indent_of(t), doc.tab())) }
    };
    let levels = |l: usize| -> usize {
        match indent(l) {
            Some(c) => c / tab,
            None => (l + 1..n.min(l + 60)).find_map(indent).unwrap_or(0) / tab,
        }
    };
    // The caret's block: the run of lines indented past the guide's level.
    let caret = doc.cursor.line.min(n.saturating_sub(1));
    let active = levels(caret).checked_sub(1);
    let (mut lo, mut hi) = (caret, caret);
    if let Some(k) = active {
        while lo > 0 && levels(lo - 1) > k {
            lo -= 1;
        }
        while hi + 1 < n && levels(hi + 1) > k {
            hi += 1;
        }
    }
    for l in g.lines() {
        let y = g.row_y(l);
        for k in 0..levels(l) {
            let x = (g.text_x0() + (k * tab) as f64 * g.cell_w).round();
            let bright = active == Some(k) && (lo..=hi).contains(&l);
            let color = bracket_color(k as u16, colors).fade(if bright { 0.9 } else { 0.3 });
            ui.draw.vline(x, y, y + g.lh, if bright { w * 2.0 } else { w }, color);
        }
    }
}

/// The color of a bracket `depth` levels in: the syntax palette, cycled.
pub fn bracket_color(depth: u16, colors: &SyntaxColors) -> Color {
    match depth % 3 {
        0 => colors.types,
        1 => colors.keyword,
        _ => colors.function,
    }
}

/// Underlines for the problems on screen. Returns the one the pointer
/// rests on, for its message.
pub fn diag_marks(ui: &mut Ui, doc: &Doc, g: &Grid, diags: &[DiagMark], hovered: bool) -> Option<usize> {
    let m = ui.m;
    let pointer = ui.state.pointer;
    let mut hover = None;
    for (i, d) in diags.iter().enumerate() {
        if !g.shown(d.line) || d.line >= doc.buffer.line_count() {
            continue;
        }
        let text = doc.line(d.line);
        let col = d.col.min(text.len());
        let seg = g.wrap.seg_of(d.line, col);
        let x0 = g.cell_x(doc, d.line, col);
        let x1 = match d.end {
            Some(e) if e > d.col => g.cell_x(doc, d.line, e.min(text.len())).max(x0 + g.cell_w * 0.5),
            _ => g.row_end_x(doc, d.line, seg).max(x0 + g.cell_w),
        };
        let y0 = g.pos_y(d.line, col);
        let y1 = y0 + g.lh;
        ui.draw.hline(x0, x1, y1 - m.px(2.0), m.px(2.0), severity_color(ui, d.severity));
        let band = Rect::new(Vec2::new(g.vp.min.x, y0), Vec2::new(g.vp.max.x, y1));
        let on_text = pointer.x >= x0 && pointer.x <= x1;
        let on_gutter = pointer.x < g.vp.min.x + g.gutter_w;
        if hover.is_none() && hovered && band.contains(pointer) && (on_text || on_gutter) {
            hover = Some(i);
        }
    }
    hover
}

/// A dot in the gutter beside every line with a problem.
pub fn diag_dots(ui: &mut Ui, g: &Grid, gutter: Rect, diags: &[DiagMark]) {
    let m = ui.m;
    for d in diags {
        if g.shown(d.line) {
            let color = severity_color(ui, d.severity);
            let rad = (g.lh * 0.16).round().max(m.px(3.0));
            ui.draw.circle(Vec2::new(gutter.min.x + g.cell_w * 0.6, g.row_y(d.line) + g.lh * 0.5), rad, color);
        }
    }
}

/// What the problem under the pointer says, on a panel under its line.
pub fn diag_panel(ui: &mut Ui, g: &Grid, diags: &[DiagMark], i: usize) {
    let m = ui.m;
    let theme = ui.theme;
    let d = &diags[i];
    let more = diags.iter().filter(|o| o.line == d.line).count() - 1;
    let text = if more > 0 { format!("{}: {}  (+{more} more)", d.severity.label(), d.message) } else { format!("{}: {}", d.severity.label(), d.message) };
    let ts = ui.text_style();
    let w = (ui.measure(&text, &ts) + m.pad * 2.0).min(g.vp.width() - m.gap * 2.0);
    let below = g.row_y(d.line) + g.lh + m.gap;
    let y = if below + m.widget_h > g.vp.max.y { g.row_y(d.line) - m.gap - m.widget_h } else { below };
    let x = (g.vp.min.x + g.gutter_w).min(g.vp.max.x - w - m.gap).max(g.vp.min.x + m.gap);
    let panel = Rect::from_min_size(Vec2::new(x, y), Vec2::new(w, m.widget_h));
    ui.floating_panel(panel, theme.header);
    ui.draw.rect(Rect::new(panel.min, Vec2::new(panel.min.x + m.px(4.0), panel.max.y)), severity_color(ui, d.severity));
    ui.text_in_rect(&text, &ts, Rect::new(Vec2::new(panel.min.x + m.pad, panel.min.y), panel.max), theme.text);
}

/// A bar beside the lines git says changed, a tick where lines went.
pub fn git_marks(ui: &mut Ui, g: &Grid, gutter: Rect, marks: &[LineMark], colors: &GitColors) {
    let m = ui.m;
    let bar_x = gutter.max.x - m.px(6.0);
    let n = g.layout.rows();
    for mk in marks {
        let color = match mk.kind {
            MarkKind::Added => colors.added,
            MarkKind::Modified => colors.modified,
            MarkKind::Deleted => colors.deleted,
        };
        match mk.kind {
            MarkKind::Deleted => {
                let row = if mk.line >= g.layout.rows() { n } else { g.layout.row_of(mk.line) };
                if row >= g.first_row && row <= g.last_row {
                    let y = g.row_top(row);
                    ui.draw.rect(Rect::new(Vec2::new(bar_x - m.px(4.0), y - m.px(2.0)), Vec2::new(bar_x + m.px(3.0), y + m.px(2.0))), color);
                }
            }
            _ => {
                let last = (mk.line + mk.len).saturating_sub(1);
                let (a, b) = (g.layout.row_of(mk.line).max(g.first_row), (g.layout.row_of(last) + g.layout.rows_of(last).max(1)).min(g.last_row));
                if a < b {
                    ui.draw.rect(Rect::new(Vec2::new(bar_x, g.row_top(a)), Vec2::new(bar_x + m.px(3.0), g.row_top(b))), color);
                }
            }
        }
    }
}
