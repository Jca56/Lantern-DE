//! Drawing a terminal: the cells as runs of one color, backgrounds first,
//! then text, underlines, the cursor, and the wheel scrolling back into
//! the scrollback (or sending arrows to a full-screen program). A file
//! path under the pointer is underlined and opens on a click.

use std::path::PathBuf;

use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::GlyphQuad;
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

use super::grid::{BOLD, DIM, HIDDEN, INVERSE, ITALIC, STRIKE, Style, TermColor, UNDERLINE};
use super::{Terminal, input, links};
use crate::editor::{cell_metrics, code_style};
use crate::settings::Settings;

/// The sixteen ANSI colors, tuned for a dark well.
const ANSI: [u32; 16] = [
    0x3B3B42, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xDCDFE4, 0x6B6B75, 0xF07178, 0xB5E890, 0xFFD479, 0x82AAFF, 0xE0A0FF, 0x89DDFF, 0xFFFFFF,
];

fn indexed(i: u8) -> Color {
    match i {
        0..=15 => Color::hex(ANSI[i as usize]),
        16..=231 => {
            let n = i - 16;
            let step = |v: u8| if v == 0 { 0.0 } else { (55 + 40 * v as u32) as f64 / 255.0 };
            Color::rgb(step(n / 36), step((n / 6) % 6), step(n % 6))
        }
        _ => {
            let v = (8 + 10 * (i as u32 - 232)) as f64 / 255.0;
            Color::rgb(v, v, v)
        }
    }
}

fn color(c: TermColor, default: Color) -> Color {
    match c {
        TermColor::Default => default,
        TermColor::Indexed(i) => indexed(i),
        TermColor::Rgb(r, g, b) => Color::from_u8(r, g, b, 255),
    }
}

/// The colors a cell draws with after inverse and dim.
fn effective(s: Style, fg_default: Color, bg_default: Color) -> (Color, Option<Color>) {
    let mut fg = color(s.fg, fg_default);
    let mut bg = (s.bg != TermColor::Default).then(|| color(s.bg, bg_default));
    if s.flags & INVERSE != 0 {
        let new_bg = fg;
        fg = bg.unwrap_or(bg_default);
        bg = Some(new_bg);
    }
    if s.flags & DIM != 0 {
        fg = fg.fade(0.6);
    }
    (fg, bg)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TermOut {
    pub focused: bool,
    /// A path was clicked: the file, and the 1-based line and column it
    /// named.
    pub open: Option<(PathBuf, Option<usize>, Option<usize>)>,
    /// A right click: where the terminal's menu goes.
    pub context: Option<Vec2>,
}

/// The file path under a point of the terminal, if it names a file
/// that exists: its row, the cells it spans, and the target.
fn link_under(term: &mut Terminal, row: usize, col: usize) -> Option<(usize, links::Link, PathBuf)> {
    let cells = Terminal::row_chars(term.grid.viewed_row(row));
    let link = links::link_at(&cells, col)?;
    let target = term.resolve_link(&link.path)?;
    Some((row, link, target))
}

pub fn draw_terminal(ui: &mut Ui, term: &mut Terminal, settings: &Settings, area_active: bool) -> TermOut {
    let id = ui.id("term");
    let m = ui.m;
    let theme = ui.theme;
    let style = code_style(ui, settings);
    let (cell_w, lh) = cell_metrics(ui, &style);
    let rect = ui.alloc(Vec2::new(FILL, ui.remaining_height().max(lh * 2.0)));
    let inner = rect.shrink(m.pad);
    let cols = ((inner.width() / cell_w).floor() as usize).max(2);
    let rows = ((inner.height() / lh).floor() as usize).max(1);
    term.resize(cols, rows);
    let now = ui.state.now;
    term.pump(now);

    let r = ui.interact(id, rect, Sense::FOCUS);
    let focused = ui.focusable(id, rect);
    let popup_blocks = ui.state.popup.is_some_and(|(p, layer)| layer > ui.layer() && p.contains(ui.state.pointer));
    let mut out = TermOut { focused, open: None, context: None };
    // The pointer as a cell (row, column) and as a boundary between cells.
    let cell_at = |p: Vec2| -> (usize, usize) {
        let y = (((p.y - inner.min.y) / lh).floor().max(0.0) as usize).min(rows - 1);
        let x = (((p.x - inner.min.x) / cell_w).floor().max(0.0) as usize).min(cols - 1);
        (y, x)
    };
    let boundary_at = |p: Vec2| -> (usize, usize) {
        let y = (((p.y - inner.min.y) / lh).floor().max(0.0) as usize).min(rows - 1);
        let x = (((p.x - inner.min.x) / cell_w).round().max(0.0) as usize).min(cols);
        (y, x)
    };
    // ---- a path under the pointer: underlined, opened by a click ----
    let mut hover_link: Option<(usize, usize, usize)> = None;
    if r.hovered && !popup_blocks && ui.state.pointer_in_window {
        let (y, x) = cell_at(ui.state.pointer);
        if let Some((row, link, target)) = link_under(term, y, x) {
            hover_link = Some((row, link.start, link.end));
            // A click, not the end of a drag that happened to stop here.
            if r.clicked && term.selection.is_none() {
                let (py_, px_) = cell_at(ui.state.press_pos);
                if link_under(term, py_, px_).is_some_and(|(r2, l2, _)| r2 == row && l2.start == link.start) {
                    out.open = Some((target, link.line, link.col));
                }
            }
        }
    }
    if r.hovered {
        ui.state.cursor_icon = if hover_link.is_some() { CursorIcon::Pointer } else { CursorIcon::Text };
    }
    if ui.state.pointer_in_window && !popup_blocks && rect.contains(ui.state.pointer) && ui.state.wheel.y != 0.0 {
        let lines = (ui.state.wheel.y / lh).round() as isize;
        if lines != 0 {
            let up = lines > 0;
            let count = lines.unsigned_abs().min(20);
            if term.grid.mouse_reporting {
                // The program handles the wheel itself (Claude Code scrolls its transcript).
                let (row, col) = cell_at(ui.state.pointer);
                for _ in 0..count {
                    term.wheel_report(up, col, row);
                }
            } else if term.grid.alt_screen() {
                let seq: &[u8] = if up { if term.grid.app_cursor { b"\x1bOA" } else { b"\x1b[A" } } else if term.grid.app_cursor { b"\x1bOB" } else { b"\x1b[B" };
                let bytes: Vec<u8> = seq.iter().copied().cycle().take(seq.len() * count).collect();
                term.write(&bytes);
            } else {
                term.grid.scroll_view(lines);
            }
        }
        ui.state.wheel = Vec2::ZERO;
    }
    // ---- selection: drag over cells, double click a word, release copies ----
    if r.pressed {
        let (y, x) = boundary_at(ui.state.pointer);
        let abs = term.grid.abs_row(y);
        term.selection = None;
        term.sel_anchor = Some((abs, x));
        if r.double_clicked {
            let (cy, cx) = cell_at(ui.state.pointer);
            let row = term.grid.viewed_row(cy);
            let is_word = |c: char| c.is_alphanumeric() || "_./-~:".contains(c);
            let mut a = cx;
            while a > 0 && is_word(row[a - 1].ch) {
                a -= 1;
            }
            let mut b = cx;
            while b < cols && is_word(row[b].ch) {
                b += 1;
            }
            if b > a {
                term.selection = Some(((abs, a), (abs, b)));
            }
        }
    } else if r.dragging
        && let Some(anchor) = term.sel_anchor
    {
        let (y, x) = boundary_at(inner.clamp_point(ui.state.pointer));
        let end = (term.grid.abs_row(y), x);
        term.selection = (end != anchor).then_some((anchor, end));
    }
    if r.released
        && let Some(text) = term.selection_text()
    {
        ui.state.set_clipboard(text);
    }
    if focused {
        input::handle(ui, term);
    }
    if ui.state.right_pressed && rect.contains(ui.state.pointer) {
        out.context = Some(ui.state.pointer);
    }

    // ---- draw ----
    let fg_default = settings.terminal.text;
    let bg_default = settings.terminal.background;
    ui.draw.rect(rect, bg_default);
    ui.draw.push_clip(rect);
    let g = &term.grid;
    let mut quads: Vec<GlyphQuad> = Vec::new();
    let mut run = String::new();
    let bold = style.clone().bold();
    let italic = style.clone().italic();
    let bold_italic = style.clone().bold().italic();
    for y in 0..rows {
        let row = g.viewed_row(y);
        let py = inner.min.y + y as f64 * lh;
        // Backgrounds, merged into runs.
        let mut x = 0;
        while x < cols {
            let (_, bg) = effective(row[x].style, fg_default, bg_default);
            let mut end = x + 1;
            while end < cols && effective(row[end].style, fg_default, bg_default).1 == bg {
                end += 1;
            }
            if let Some(c) = bg {
                ui.draw.rect(Rect::new(Vec2::new(inner.min.x + x as f64 * cell_w, py), Vec2::new(inner.min.x + end as f64 * cell_w, py + lh)), c);
            }
            x = end;
        }
        // The selection, over the backgrounds and under the text.
        if let Some((a, b)) = term.selection {
            let (s, e) = if a <= b { (a, b) } else { (b, a) };
            let abs = g.abs_row(y);
            if abs >= s.0 && abs <= e.0 {
                let from = if abs == s.0 { s.1 } else { 0 };
                let to = if abs == e.0 { e.1 } else { cols };
                if to > from {
                    ui.draw.rect(Rect::new(Vec2::new(inner.min.x + from as f64 * cell_w, py), Vec2::new(inner.min.x + to as f64 * cell_w, py + lh)), theme.selection.fade(0.45));
                }
            }
        }
        // Text, in runs of one color and weight.
        x = 0;
        while x < cols {
            let cell = &row[x];
            if cell.spacer || (cell.ch == ' ' && cell.style.flags & (UNDERLINE | STRIKE) == 0) || cell.style.flags & HIDDEN != 0 {
                x += 1;
                continue;
            }
            let (fg, _) = effective(cell.style, fg_default, bg_default);
            let flags = cell.style.flags & (BOLD | ITALIC | UNDERLINE | STRIKE);
            run.clear();
            let start = x;
            let mut cells = 0;
            while x < cols {
                let c = &row[x];
                if c.spacer {
                    x += 1;
                    continue;
                }
                let (f, _) = effective(c.style, fg_default, bg_default);
                if f != fg || c.style.flags & (BOLD | ITALIC | UNDERLINE | STRIKE) != flags || c.style.flags & HIDDEN != 0 {
                    break;
                }
                run.push(c.ch);
                cells += if c.wide { 2 } else { 1 };
                x += 1;
            }
            let st = match (flags & BOLD != 0, flags & ITALIC != 0) {
                (true, true) => &bold_italic,
                (true, false) => &bold,
                (false, true) => &italic,
                _ => &style,
            };
            let px = inner.min.x + start as f64 * cell_w;
            if run.trim().is_empty() && flags & (UNDERLINE | STRIKE) == 0 {
                continue;
            }
            quads.clear();
            ui.text.place(&run, st, px as f32, py as f32, 1.0e6, fg.to_gpu(), &mut quads);
            ui.draw.glyphs(&quads);
            let x1 = px + cells as f64 * cell_w;
            if flags & UNDERLINE != 0 {
                ui.draw.hline(px, x1, py + lh - m.px(2.0), m.px(1.0), fg);
            }
            if flags & STRIKE != 0 {
                ui.draw.hline(px, x1, (py + lh * 0.55).round(), m.px(1.0), fg);
            }
        }
    }
    if let Some((y, a, b)) = hover_link {
        let py = inner.min.y + y as f64 * lh;
        ui.draw.hline(inner.min.x + a as f64 * cell_w, inner.min.x + b as f64 * cell_w, py + lh - m.px(2.0), m.px(2.0), theme.focus);
    }
    // The cursor: a block while focused, an outline otherwise.
    if g.cursor_visible && g.view_offset == 0 && term.exited.is_none() && g.cursor.y < rows {
        let (cx, cy) = (g.cursor.x.min(cols - 1), g.cursor.y);
        let cell = g.row(cy)[cx];
        let w = if cell.wide { 2.0 } else { 1.0 };
        let crect = Rect::new(Vec2::new(inner.min.x + cx as f64 * cell_w, inner.min.y + cy as f64 * lh), Vec2::new(inner.min.x + (cx as f64 + w) * cell_w, inner.min.y + (cy + 1) as f64 * lh));
        if focused && area_active {
            ui.draw.rect(crect, theme.accent);
            if cell.ch != ' ' && !cell.spacer {
                quads.clear();
                ui.text.place(&cell.ch.to_string(), &style, crect.min.x as f32, crect.min.y as f32, 1.0e6, theme.accent_text.to_gpu(), &mut quads);
                ui.draw.glyphs(&quads);
            }
        } else {
            ui.draw.stroke_rect(crect, m.border, 0.0, theme.accent);
        }
    }
    if g.view_offset > 0 {
        let label = format!("↑ {}", g.view_offset);
        let ts = ui.text_style();
        let w = ui.measure(&label, &ts) + m.pad * 2.0;
        let badge = Rect::from_min_size(Vec2::new(rect.max.x - w - m.gap, rect.min.y + m.gap), Vec2::new(w, m.widget_h));
        ui.floating_panel(badge, theme.header);
        ui.text_centered(&label, &ts, badge, theme.text);
    }
    ui.draw.pop_clip();
    if focused {
        ui.state.ime_rect = None;
    }
    // Without a waker, poll for output: fast while it flows, slow when it stops.
    if term.exited.is_none() && term.polls() {
        let since = now - term.last_output;
        let interval = if since < 0.5 {
            1.0 / 60.0
        } else if since < 3.0 {
            0.1
        } else {
            0.5
        };
        ui.state.request_redraw_after(interval);
    }
    term.grid.bell = false;
    out
}
