//! The Processes page: a search field, a way to fold programs or not,
//! the two ways to end what is picked, and the list itself: Lantern UI's
//! table, sorted by its headers, over the rows `rows.rs` works out.

use std::sync::Arc;

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{bits, look, small_style, text_line, title_style};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Action, AreaCx, Cell, Column, ContextMenu, CursorIcon, FILL, Item, RowStep, Sense, ShellRequest, Ui, WidgetId};

use crate::app::App;
use crate::format;
use crate::glyphs;
use crate::rows::{self, Pick, Row, Sort};
use crate::sample::Frame;

/// Room between the sidebar's edge and the page, and at the page's
/// right.
const INSET: f64 = 14.0;
const TOOLBAR_H: f64 = 64.0;
/// How far a row's name starts from its cell's edge, clear of the
/// chevron a folded program has there; and how much further when the
/// row is one of a program's processes.
const NAME_X: f64 = 30.0;
const INSIDE_X: f64 = 28.0;
/// The CPU column sorts first, busiest at the top, until a header is
/// clicked.
const CPU_COLUMN: usize = 2;

fn columns() -> [Column; 5] {
    [Column::fill("Name").sortable(), Column::new("User", 150.0).sortable(), Column::new("CPU", 130.0).right().sortable(), Column::new("Memory", 160.0).right().sortable(), Column::new("PID", 130.0).right().sortable()]
}

pub fn draw(app: &mut App, frame: &Frame, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let inset = ui.m.px(INSET);
    ui.indent(inset, |ui| {
        let width = (ui.avail_width() - inset).max(1.0);
        ui.columns(&[width], |ui, _| {
            header(frame, ui);
            toolbar(app, ui, cx);
            list(app, frame, ui, cx);
        });
    });
}

fn header(frame: &Frame, ui: &mut Ui) {
    ui.space(ui.m.px(15.0));
    let (big, small) = (title_style(ui), small_style(ui));
    text_line(ui, "Processes", &big, look::TEXT);
    let kernel = frame.procs.len() - frame.programs();
    let said = format!("{} processes and {} threads, and {} of the kernel's own.", format::count(frame.programs() as u64), format::count(frame.threads()), format::count(kernel as u64));
    text_line(ui, &said, &small, look::TEXT_DIM);
}

/// The search field, the fold switch, and at the right the two ways to
/// end what is picked.
fn toolbar(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let m = ui.m;
    let row = ui.alloc(Vec2::new(FILL, m.px(TOOLBAR_H)));
    let h = m.px(controls::BUTTON_H);
    let y = (row.center().y - h * 0.5).round();
    let gap = m.px(12.0);
    ui.push_id("toolbar");

    // Right to left: Force Kill, End. Lit only with something picked.
    let armed = app.procs.picked.is_some();
    let mut right = row.max.x;
    let mut asked = None;
    for (key, text, kind, action) in [("kill", "Force Kill", Kind::Danger, "procs.kill"), ("end", "End", Kind::Plain, "procs.end")] {
        let w = controls::button_width(ui, text);
        let rect = Rect::from_min_size(Vec2::new(right - w, y), Vec2::new(w, h));
        right -= w + gap;
        let id = ui.id(key);
        if controls::button_of(ui, id, rect, text, kind, armed) {
            asked = Some(action);
        }
    }

    let options = ["Programs", "Processes"];
    let fold_w = controls::segmented_width(ui, &options);
    // The field takes what the rest leaves, within reason.
    let field_w = (right - row.min.x - fold_w - gap).clamp(m.px(160.0), m.px(460.0));
    let field = Rect::from_min_size(Vec2::new(row.min.x, y), Vec2::new(field_w, h));
    let id = ui.id("search");
    if std::mem::take(&mut app.procs.focus_search) {
        ui.state.focus = Some(id);
    }
    let typed = controls::text_field(ui, id, field, &mut app.procs.view.search, "Search by name, user or number");
    if typed.cancelled {
        app.procs.view.search.clear();
    }

    let mut picked = usize::from(!app.procs.view.grouped);
    let slot = Rect::new(Vec2::new(field.max.x + gap, row.min.y), Vec2::new(field.max.x + gap + fold_w, row.max.y));
    let id = ui.id("fold");
    if controls::segmented(ui, id, slot, &mut picked, &options) {
        app.set_grouped(picked == 0, &mut cx.host());
    }
    ui.pop_id();

    if let Some(action) = asked {
        // The same road a menu row or a key takes: the dialog first.
        let mut host = cx.host();
        lntrn_ui::Host::run(app, &Action::new(action), &mut host);
    }
}

/// What one row says in each column.
struct Line<'a> {
    name: &'a str,
    user: &'a str,
    cpu: f32,
    memory: u64,
    /// Empty for a folded program: it has no one number.
    pid: String,
    /// A folded program's row: how many processes, and whether it is open.
    group: Option<(&'a Arc<str>, usize, bool)>,
    inside: bool,
}

fn line<'a>(row: &'a Row, frame: &'a Frame) -> Option<Line<'a>> {
    Some(match row {
        Row::Group { app, count, cpu, memory, user, open } => Line { name: app, user, cpu: *cpu, memory: *memory, pid: String::new(), group: Some((app, *count, *open)), inside: false },
        Row::Proc { index, inside } => {
            let p = frame.procs.get(*index)?;
            Line { name: &p.name, user: &p.user, cpu: p.cpu, memory: p.memory, pid: p.pid.to_string(), group: None, inside: *inside }
        }
    })
}

/// The ink for a share of the processors: red from two of them flat
/// out, amber from most of one, dim for next to nothing.
fn cpu_ink(cpu: f32, threads: usize, plain: Color) -> Color {
    let processors = f64::from(cpu) / 100.0 * threads.max(1) as f64;
    if processors >= 2.0 {
        look::BAD
    } else if processors >= 0.9 {
        look::WARN
    } else if cpu < 0.05 {
        look::TEXT_DIM
    } else {
        plain
    }
}

/// One cell of row `l` of the list `table`. `fold` is set to a program
/// whose chevron was pressed.
fn cell(c: &mut Cell, table: WidgetId, l: &Line, selected: bool, threads: usize, fold: &mut Option<Arc<str>>) {
    let m = c.m;
    let style = c.text_style();
    // A selected row is filled with the accent: its ink is the one that
    // reads on it, and the heat colours give way to it.
    let ink = if selected { c.theme.selection_text } else { look::TEXT };
    let dim = if selected { ink.fade(0.8) } else { look::TEXT_DIM };
    let rect = c.rect;
    match c.col {
        0 => {
            let mut x = rect.min.x + m.px(NAME_X) + if l.inside { m.px(INSIDE_X) } else { 0.0 };
            if let Some((app, count, open)) = l.group {
                let mark = Rect::from_center_size(Vec2::new(rect.min.x + m.px(12.0), rect.center().y), Vec2::splat(m.px(16.0)));
                let id = c.id("fold");
                let r = c.interact(id, mark.expand(m.px(8.0)), Sense::CLICK);
                if r.hovered {
                    c.state.cursor_icon = CursorIcon::Pointer;
                }
                // The press is the chevron's, not the row's: the list
                // keeps the keyboard all the same, so the arrows go on.
                if r.pressed {
                    c.state.focus = Some(table);
                }
                if r.clicked {
                    *fold = Some(app.clone());
                }
                let mark_ink = if r.hovered && !selected { c.theme.accent } else { dim };
                glyphs::chevron(c.draw, mark, mark_ink, m.px(2.5), open);
                let w = c.measure(l.name, &style);
                c.text_in_rect(l.name, &style, Rect::new(Vec2::new(x, rect.min.y), rect.max), ink);
                x += w + m.px(12.0);
                if x + m.px(40.0) < rect.max.x {
                    bits::badge(c, x, rect.center().y, &count.to_string(), if selected { look::TEXT } else { look::TEXT_DIM });
                }
            } else {
                c.text_in_rect(l.name, &style, Rect::new(Vec2::new(x, rect.min.y), rect.max), if l.inside { dim } else { ink });
            }
        }
        1 => c.text_in_rect(l.user, &style, rect, dim),
        2 => c.text_right(&format::percent(f64::from(l.cpu), true), &style, rect, if selected { ink } else { cpu_ink(l.cpu, threads, look::TEXT) }),
        3 => c.text_right(&format::bytes(l.memory), &style, rect, ink),
        _ => c.text_right(&l.pid, &style, rect, dim),
    }
}

/// The right-click menu for what is picked.
fn menu(app: &App, frame: &Frame, pick: &Pick, at: Vec2) -> ContextMenu {
    let mut items = vec![Item::action("End", Action::new("procs.end")), Item::danger("Force Kill", Action::new("procs.kill")), Item::Separator];
    match pick {
        Pick::App(name) => {
            let open = app.procs.view.open.contains(name);
            items.push(Item::action(if open { "Hide Its Processes" } else { "Show Its Processes" }, Action::new("procs.fold")));
        }
        Pick::Pid(_) => {
            items.push(Item::action("Copy Process Number", Action::new("procs.copy-pid")));
            items.push(Item::action("Copy Command Line", Action::new("procs.copy-command")));
        }
    }
    ContextMenu::new(&rows::pick_name(&frame.procs, pick), at).tab("Process", items)
}

/// Scroll the list `table` so its row `at` shows: the table lays out
/// only the rows in view and can't be asked to, but where it is scrolled
/// to is kept where it can be moved.
fn reveal(ui: &mut Ui, table: WidgetId, at: usize, height: f64) {
    let m = ui.m;
    let row_h = m.widget_h + m.gap;
    // Under its header, and above the room it keeps for a bar.
    let view_h = (height - m.widget_h - m.scrollbar_w).max(row_h);
    let (top, scroll) = (at as f64 * row_h, ui.state.scroll(table.with("body")));
    if top < scroll.offset.y {
        scroll.offset.y = top;
    } else if top + row_h > scroll.offset.y + view_h {
        scroll.offset.y = top + row_h - view_h;
    }
}

fn list(app: &mut App, frame: &Frame, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let m = ui.m;
    app.refresh_rows(frame);
    let height = (ui.remaining_height() - m.px(10.0)).max(m.widget_h * 3.0);
    let area = Rect::from_min_size(ui.cursor(), Vec2::new(ui.avail_width(), height));
    let id = ui.id("list");
    let p = &mut app.procs;
    let picked_at = p.picked.as_ref().and_then(|pick| p.rows.iter().position(|r| r.pick(&frame.procs).as_ref() == Some(pick)));
    if std::mem::take(&mut p.reveal)
        && let Some(at) = picked_at
    {
        reveal(ui, id, at, height);
    }

    let threads = frame.cpu.threads.len();
    let rows = &p.rows;
    let mut fold = None;
    let resp = ui.table("list", &columns(), rows.len(), Some(height), |t| {
        for i in t.visible() {
            let Some(l) = rows.get(i).and_then(|row| line(row, frame)) else { continue };
            let selected = picked_at == Some(i);
            t.row(i, selected, |c| cell(c, id, &l, selected, threads, &mut fold));
        }
    });
    if rows.is_empty() {
        let (_, body) = area.take_top(m.widget_h);
        let style = small_style(ui);
        let said = if p.view.search.trim().is_empty() { "Nothing is running that can be read.".to_owned() } else { format!("Nothing matches “{}”.", p.view.search.trim()) };
        ui.text_centered(&said, &style, body, look::TEXT_DIM);
    }

    // A click picks; a second one on a folded program opens it.
    let row_pick = |i: usize| rows.get(i).and_then(|r| r.pick(&frame.procs));
    let mut picked = None;
    if let Some(i) = resp.clicked {
        picked = row_pick(i);
    }
    if let Some(Pick::App(name)) = resp.double_clicked.and_then(row_pick) {
        fold = Some(name);
    }
    let menu_for = if ui.state.right_pressed { resp.hovered.and_then(row_pick) } else { None };
    // The arrows move the pick through the rows, and the list follows.
    let last = rows.len().saturating_sub(1);
    let stepped = match resp.step {
        RowStep::None => None,
        RowStep::By(n) => Some(picked_at.map_or(0, |at| (at as i64 + i64::from(n)).clamp(0, last as i64) as usize)),
        RowStep::First => Some(0),
        RowStep::Last => Some(last),
    };
    if let Some(i) = stepped
        && let Some(pick) = row_pick(i)
    {
        picked = Some(pick);
        p.reveal = true;
        ui.state.request_rebuild = true;
    }

    if let Some(pick) = menu_for.clone().or(picked) {
        p.picked = Some(pick);
    }
    match resp.sort {
        Some((column, ascending)) => {
            let by = rows::Column::ALL[column.min(rows::Column::ALL.len() - 1)];
            // A header's first click sorts upwards, which suits names. A
            // column of amounts is clicked for its biggest: turn it over.
            let turned = resp.sort_changed && by != p.view.sort.by && matches!(by, rows::Column::Cpu | rows::Column::Memory);
            if turned {
                ui.state.table_mem(id).sort = Some((column, false));
            }
            let sort = Sort { by, ascending: ascending && !turned };
            if sort != p.view.sort {
                p.view.sort = sort;
                ui.state.request_rebuild = true;
            }
        }
        // Not sorted by a header yet: show it sorted the way it starts.
        None => {
            ui.state.table_mem(id).sort = Some((CPU_COLUMN, false));
            ui.state.request_rebuild = true;
        }
    }
    if let Some(name) = fold {
        app.fold(&name);
        ui.state.request_rebuild = true;
    }
    if let Some(pick) = menu_for {
        cx.request(ShellRequest::ContextMenu(Box::new(menu(app, frame, &pick, cx.pointer))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_of_the_processors_is_coloured_by_how_many_it_is() {
        let plain = look::TEXT;
        // Of 28 threads: 3.6% is one of them flat out.
        assert_eq!(cpu_ink(0.01, 28, plain), look::TEXT_DIM);
        assert_eq!(cpu_ink(1.0, 28, plain), plain);
        assert_eq!(cpu_ink(3.5, 28, plain), look::WARN);
        assert_eq!(cpu_ink(8.0, 28, plain), look::BAD);
        // On one thread, all of it is all there is.
        assert_eq!(cpu_ink(95.0, 1, plain), look::WARN);
        assert_eq!(cpu_ink(50.0, 0, plain), plain);
    }
}
