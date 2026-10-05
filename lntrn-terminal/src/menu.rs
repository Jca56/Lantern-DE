//! The terminal's menus. The right-click one is the terminal's own, row
//! by row: a little title bar (the window's buttons, the tabs, `lntrn`),
//! the text size, then what to do with the text, the tabs and the panes,
//! each with its key beside it. The title bar's menus and the palette's
//! commands are here too, and what every one of them does.

use lntrn_kit::controls;
use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_term::grid::CursorShape;
use lntrn_ui::{Action, AreaId, Axis, ContextMenu, CursorIcon, FILL, Host, HostCx, IconFn, Item, Menu, MenuItem, Sense, ShellRequest, Ui, WidgetId, actions};

use crate::app::{App, Held, Start};
use crate::ops::Op;
use crate::settings::{FONT_SIZES, Settings};

/// What the palette offers: (action, what it says).
pub const COMMANDS: &[(&str, &str)] = &[
    ("tab.new", "New Tab"),
    ("tab.close", "Close Tab"),
    ("tab.next", "Next Tab"),
    ("tab.prev", "Previous Tab"),
    ("tab.pin", "Pin or Unpin Tab"),
    ("window.new", "New Window"),
    ("split.right", "Split Right"),
    ("split.down", "Split Down"),
    ("pane.close", "Close Pane"),
    ("pane.next", "Next Pane"),
    ("pane.prev", "Previous Pane"),
    ("edit.copy", "Copy"),
    ("edit.paste", "Paste"),
    ("edit.select-all", "Select All"),
    ("edit.find", "Find"),
    ("edit.clear", "Clear Scrollback"),
    ("text.bigger", "Bigger Text"),
    ("text.smaller", "Smaller Text"),
    ("text.reset", "Reset Text Size"),
    ("cursor.block", "Cursor: Block"),
    ("cursor.underline", "Cursor: Underline"),
    ("cursor.beam", "Cursor: Beam"),
    ("bar.toggle", "Show or Hide the Title Bar"),
    ("bar.open-hidden", "Open With the Title Bar Hidden"),
];

/// The right-click menu's rows that do one thing: (action, label, the
/// key when the terminal itself takes it rather than the app's keymap).
const ROWS: &[(&str, &str, &str)] = &[
    ("edit.copy", "Copy", "Ctrl+Shift+C"),
    ("edit.paste", "Paste", "Ctrl+Shift+V"),
    ("edit.select-all", "Select All", ""),
    ("tab.new", "New Tab", ""),
    ("tab.close", "Close Tab", ""),
    ("split.right", "Split Right", ""),
    ("split.down", "Split Down", ""),
    ("pane.close", "Close Pane", ""),
];

/// How tall a row of the right-click menu is, in logical pixels: enough
/// to hit without looking, and the whole menu still fits a window of
/// the size the desktop opens them at.
const ROW_H: f64 = 44.0;

/// The right-click menu, at `at`. It has no heading: its first row is
/// its title bar.
pub fn context(at: Vec2) -> ContextMenu {
    let mut items = vec![Item::custom("controls"), Item::Separator, Item::custom("size")];
    for group in [&["edit.copy", "edit.paste", "edit.select-all"][..], &["tab.new", "tab.close"], &["split.right", "split.down", "pane.close"]] {
        items.push(Item::Separator);
        items.extend(group.iter().map(|key| Item::custom(key)));
    }
    ContextMenu::new("", at).wide().tab("Terminal", items)
}

/// Draw one of the right-click menu's rows. `true` when it changed
/// something.
pub fn draw_row(app: &mut App, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
    match key {
        "controls" => controls_row(app, ui, cx),
        "size" => size_row(app, ui),
        _ => match ROWS.iter().find(|(id, _, _)| *id == key) {
            Some(&(id, label, hint)) => action_row(app, ui, cx, id, label, hint),
            None => false,
        },
    }
}

/// Whether a row can be chosen as things stand.
fn enabled(app: &App, id: &str, term: &Held) -> bool {
    match id {
        "edit.copy" => term.upgrade().is_some_and(|t| t.try_borrow().is_ok_and(|t| t.selection_text().is_some())),
        "pane.close" => app.panes > 1,
        _ => true,
    }
}

/// A row that does one thing and closes the menu: its label, and the key
/// that does the same dim at the right. Dim all over when it can't be
/// chosen.
fn action_row(app: &mut App, ui: &mut Ui, cx: &mut HostCx, id: &str, label: &str, hint: &str) -> bool {
    let m = ui.m;
    let on = enabled(app, id, &app.menu_term);
    let rect = ui.alloc(Vec2::new(FILL, m.px(ROW_H)));
    let wid = ui.id(id);
    let mut r = ui.interact(wid, rect, if on { Sense::CLICK } else { Sense::NONE });
    if on {
        ui.focusable(wid, rect);
        ui.key_click(wid, &mut r);
        if r.hovered || r.held {
            ui.state.cursor_icon = CursorIcon::Pointer;
            ui.draw.rounded_rect(rect, m.px(8.0), Color::WHITE.fade(if r.held { 0.12 } else { 0.07 }));
        }
    }
    let style = ui.text_style();
    let inner = Rect::new(Vec2::new(rect.min.x + m.pad, rect.min.y), Vec2::new(rect.max.x - m.pad, rect.max.y));
    let (ink, dim) = if on { (look::TEXT, look::TEXT_DIM) } else { (look::TEXT_DIM.fade(0.55), look::TEXT_DIM.fade(0.4)) };
    ui.text_in_rect(label, &style, inner, ink);
    let key = if hint.is_empty() { app.key_hint(&Action::new(id)).unwrap_or_default() } else { hint.to_owned() };
    if !key.is_empty() {
        ui.text_right(&key, &style, inner, dim);
    }
    ui.focus_ring(wid, rect);
    if !r.clicked {
        return false;
    }
    run_on(app, id, app.menu_area, app.menu_term.clone(), cx);
    cx.request(ShellRequest::ClosePopup);
    true
}

/// The text size: a slider that changes it as it is dragged.
fn size_row(app: &mut App, ui: &mut Ui) -> bool {
    let m = ui.m;
    let rect = ui.alloc(Vec2::new(FILL, m.widget_h));
    let style = ui.text_style();
    let inner = Rect::new(Vec2::new(rect.min.x + m.pad, rect.min.y), Vec2::new(rect.max.x - m.pad, rect.max.y));
    ui.text_in_rect("Text Size", &style, inner, look::TEXT);
    let slot = Rect::new(Vec2::new(inner.min.x + ui.measure("Text Size", &style) + m.pad, inner.min.y), inner.max);
    let mut size = app.settings.font_size;
    let id = ui.id("size");
    if !controls::slider(ui, id, slot, &mut size, FONT_SIZES, 0.5, &|v| format!("{v:.1} px")) {
        return false;
    }
    app.settings.font_size = size;
    app.settings_changed();
    true
}

/// A point at `(x, y)` of the rect's half-size from its centre.
fn at(rect: Rect, x: f64, y: f64) -> Vec2 {
    let s = rect.width().min(rect.height()) * 0.5;
    rect.center() + Vec2::new(x * s, y * s)
}

const MINIMIZE: IconFn = |d, rect, color, w| d.line(at(rect, -0.9, 0.7), at(rect, 0.9, 0.7), w, color);
const MAXIMIZE: IconFn = |d, rect, color, w| d.polyline(&[at(rect, -0.85, -0.85), at(rect, 0.85, -0.85), at(rect, 0.85, 0.85), at(rect, -0.85, 0.85)], w, color, true);
const CLOSE: IconFn = |d, rect, color, w| {
    d.line(at(rect, -0.85, -0.85), at(rect, 0.85, 0.85), w, color);
    d.line(at(rect, -0.85, 0.85), at(rect, 0.85, -0.85), w, color);
};
const PREV: IconFn = |d, rect, color, w| d.polyline(&[at(rect, 0.4, -0.85), at(rect, -0.45, 0.0), at(rect, 0.4, 0.85)], w, color, false);
const NEXT: IconFn = |d, rect, color, w| d.polyline(&[at(rect, -0.4, -0.85), at(rect, 0.45, 0.0), at(rect, -0.4, 0.85)], w, color, false);

/// A picture to press, lit in `hot` under the pointer. `true` when
/// pressed.
fn glyph(ui: &mut Ui, id: WidgetId, rect: Rect, picture: IconFn, tip: &str, hot: Color) -> bool {
    let m = ui.m;
    let mut r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered || r.held {
        ui.state.cursor_icon = CursorIcon::Pointer;
        ui.draw.rounded_rect(rect.shrink(m.px(3.0)), m.px(10.0), hot.fade(if r.held { 0.4 } else { 0.25 }));
    }
    let ink = if r.hovered || r.held { hot.lerp(Color::WHITE, 0.45) } else { look::TEXT };
    picture(ui.draw, Rect::from_center_size(rect.center(), Vec2::splat(rect.height() * 0.34)), ink, m.px(2.5));
    ui.focus_ring(id, rect);
    ui.tooltip(&r, tip);
    r.clicked
}

/// The little title bar: `lntrn` at the left, the pane's tabs in the
/// middle when it has more than one, the window's buttons at the right.
/// It is all of a title bar there is while the real one is hidden.
fn controls_row(app: &mut App, ui: &mut Ui, cx: &mut HostCx) -> bool {
    let m = ui.m;
    let side = m.widget_h;
    let rect = ui.alloc(Vec2::new(FILL, side));
    let square = |x: f64| Rect::from_min_size(Vec2::new(x, rect.min.y), Vec2::splat(side));
    let accent = ui.theme.accent;
    let mut changed = false;

    // ---- the window's buttons ----
    let buttons: [(&str, IconFn, &str, Color, ShellRequest); 3] = [("min", MINIMIZE, "Minimize", accent, ShellRequest::MinimizeWindow), ("max", MAXIMIZE, "Maximize", accent, ShellRequest::MaximizeWindow), ("close", CLOSE, "Close the window", look::BAD, ShellRequest::CloseWindow)];
    let buttons_x = rect.max.x - side * 3.0;
    for (i, (key, picture, tip, hot, ask)) in buttons.into_iter().enumerate() {
        if glyph(ui, ui.id(key), square(buttons_x + side * i as f64), picture, tip, hot) {
            cx.request(ShellRequest::ClosePopup);
            cx.request(ask);
            changed = true;
        }
    }

    // ---- lntrn: runs it in this terminal ----
    let style = ui.text_style().bold();
    let name_w = ui.measure("lntrn", &style) + m.pad * 2.0;
    let name = Rect::from_min_size(rect.min, Vec2::new(name_w, side));
    let id = ui.id("lntrn");
    let mut r = ui.interact(id, name, Sense::CLICK);
    ui.focusable(id, name);
    ui.key_click(id, &mut r);
    if r.hovered || r.held {
        ui.state.cursor_icon = CursorIcon::Pointer;
        ui.draw.rounded_rect(name.shrink(m.px(3.0)), m.px(10.0), accent.fade(if r.held { 0.3 } else { 0.18 }));
    }
    ui.text_centered("lntrn", &style, name, accent);
    ui.focus_ring(id, name);
    ui.tooltip(&r, "Run lntrn here");
    if r.clicked {
        if let Some(t) = app.menu_term.upgrade() {
            t.borrow_mut().write(b"lntrn\r");
        }
        cx.request(ShellRequest::ClosePopup);
        changed = true;
    }

    // ---- the pane's tabs: one before, a dot each, one after ----
    let Some(area) = app.menu_area else { return changed };
    let (count, current) = app.area_tabs.get(&area).copied().unwrap_or((1, 0));
    if count < 2 {
        return changed;
    }
    let room = Rect::new(Vec2::new(name.max.x + m.gap, rect.min.y), Vec2::new(buttons_x - m.gap, rect.max.y));
    let arrow = side * 0.7;
    // A dot each while they are still big enough to hit; more tabs than
    // that, which of how many in words.
    let dot_w = ((room.width() - arrow * 2.0) / count as f64).min(m.px(28.0));
    let fits = dot_w >= m.px(20.0);
    let middle_w = if fits { dot_w * count as f64 } else { (room.width() - arrow * 2.0).max(0.0) };
    let x0 = room.center().x - (middle_w * 0.5 + arrow);
    let step = |x: f64| Rect::from_min_size(Vec2::new(x, rect.min.y), Vec2::new(arrow, side));
    if glyph(ui, ui.id("prev"), step(x0), PREV, "Previous tab", accent) {
        app.ops.push(Op::CycleTab(Some(area), -1));
        changed = true;
    }
    if glyph(ui, ui.id("next"), step(x0 + arrow + middle_w), NEXT, "Next tab", accent) {
        app.ops.push(Op::CycleTab(Some(area), 1));
        changed = true;
    }
    let middle = Rect::from_min_size(Vec2::new(x0 + arrow, rect.min.y), Vec2::new(middle_w, side));
    if !fits {
        let style = ui.text_style();
        ui.text_centered(&format!("{} / {count}", current + 1), &style, middle, look::TEXT);
        return changed;
    }
    for i in 0..count {
        let hit = Rect::from_min_size(Vec2::new(middle.min.x + dot_w * i as f64, rect.min.y), Vec2::new(dot_w, side));
        let r = ui.interact(ui.id("dot").with_index(i), hit, Sense::CLICK);
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        let lit = i == current;
        let radius = m.px(if lit || r.hovered { 7.0 } else { 5.0 });
        ui.draw.circle(hit.center(), radius, if lit { accent } else { look::TRACK.lerp(look::TEXT, if r.hovered { 0.5 } else { 0.15 }) });
        if r.clicked && !lit {
            app.ops.push(Op::SelectTab(area, i));
            changed = true;
        }
    }
    changed
}

/// A menu of the title bar's.
pub fn title_menu(app: &App, name: &str) -> Option<Menu> {
    let item = |label: &str, id: &str| MenuItem::new(label, Action::new(id));
    let cursor = |label: &str, id: &str, shape: CursorShape| item(label, id).checked(app.settings.cursor == shape);
    let (title, items) = match name {
        "terminal" => (
            "Terminal",
            vec![
                item("New Tab", "tab.new"),
                item("New Window", "window.new"),
                MenuItem::separator(),
                item("Copy", "edit.copy").hint("Ctrl+Shift+C").enabled(enabled(app, "edit.copy", &app.front)),
                item("Paste", "edit.paste").hint("Ctrl+Shift+V"),
                item("Select All", "edit.select-all"),
                item("Find", "edit.find").hint("Ctrl+Shift+F"),
                item("Clear Scrollback", "edit.clear"),
                MenuItem::separator(),
                item("Pin Tab", "tab.pin").checked(app.front_pinned),
                item("Close Tab", "tab.close"),
                MenuItem::separator(),
                item("Quit", actions::QUIT),
            ],
        ),
        "view" => (
            "View",
            vec![
                item("Bigger Text", "text.bigger"),
                item("Smaller Text", "text.smaller"),
                item("Reset Text Size", "text.reset"),
                MenuItem::separator(),
                MenuItem::sub("Cursor", vec![cursor("Block", "cursor.block", CursorShape::Block), cursor("Underline", "cursor.underline", CursorShape::Underline), cursor("Beam", "cursor.beam", CursorShape::Beam)]),
                MenuItem::separator(),
                item("Command Palette", actions::PALETTE),
                item("Hide the Title Bar", "bar.toggle"),
                item("Open With the Title Bar Hidden", "bar.open-hidden").checked(app.settings.open_bar_hidden),
            ],
        ),
        "split" => (
            "Split",
            vec![
                item("Split Right", "split.right"),
                item("Split Down", "split.down"),
                MenuItem::separator(),
                item("Close Pane", "pane.close").enabled(app.panes > 1),
                MenuItem::separator(),
                item("Previous Pane", "pane.prev"),
                item("Next Pane", "pane.next"),
            ],
        ),
        _ => return None,
    };
    Some(Menu::new(title, items))
}

/// Another window of the terminal, starting in `dir`.
fn new_window(dir: Option<std::path::PathBuf>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(exe);
    if let Some(dir) = dir.filter(|d| d.is_dir()) {
        command.current_dir(dir);
    }
    command.spawn().map(drop).map_err(|e| e.to_string())
}

/// Carry out an action from a key, the palette or the title bar: on the
/// terminal with the keyboard.
pub fn run(app: &mut App, id: &str, cx: &mut HostCx) {
    run_on(app, id, None, app.front.clone(), cx);
}

/// Carry out an action on a pane (the one with the keyboard when `None`)
/// and the terminal showing in it.
pub fn run_on(app: &mut App, id: &str, area: Option<AreaId>, term: Held, cx: &mut HostCx) {
    let dir = || term.upgrade().and_then(|t| t.try_borrow().ok().and_then(|t| t.cwd_now()));
    let op = match id {
        "tab.new" => Op::NewTab { area, start: Start { cwd: dir(), command: None }, name: None, pinned: false },
        "tab.close" => Op::CloseTab(area),
        "tab.next" => Op::CycleTab(area, 1),
        "tab.prev" => Op::CycleTab(area, -1),
        "tab.pin" => Op::TogglePin(area),
        "split.right" => Op::Split(area, Axis::Horizontal),
        "split.down" => Op::Split(area, Axis::Vertical),
        "pane.close" => Op::ClosePane(area),
        "pane.next" => Op::CyclePane(1),
        "pane.prev" => Op::CyclePane(-1),
        "edit.copy" => Op::Copy(term),
        "edit.paste" => Op::Paste(term),
        "edit.select-all" => Op::SelectAll(term),
        "edit.find" => Op::Find(term),
        _ => {
            set(app, id, &term, cx);
            cx.rebuild();
            return;
        }
    };
    app.ops.push(op);
    cx.rebuild();
}

/// The actions that change a setting or reach outside the window.
fn set(app: &mut App, id: &str, term: &Held, cx: &mut HostCx) {
    let size = app.settings.font_size;
    match id {
        "window.new" => {
            let dir = term.upgrade().and_then(|t| t.try_borrow().ok().and_then(|t| t.cwd_now()));
            if let Err(e) = new_window(dir) {
                cx.toast(&format!("No new window: {e}"));
            }
            return;
        }
        "edit.clear" => {
            if let Some(t) = term.upgrade() {
                t.borrow_mut().clear();
            }
            return;
        }
        "bar.toggle" => {
            app.bar_hidden = !app.bar_hidden;
            if app.bar_hidden {
                cx.toast("Title bar hidden: Super+F11 brings it back");
            }
            return;
        }
        "text.bigger" => app.settings.font_size = (size + 1.0).min(FONT_SIZES.1),
        "text.smaller" => app.settings.font_size = (size - 1.0).max(FONT_SIZES.0),
        "text.reset" => app.settings.font_size = Settings::default().font_size,
        "cursor.block" => app.settings.cursor = CursorShape::Block,
        "cursor.underline" => app.settings.cursor = CursorShape::Underline,
        "cursor.beam" => app.settings.cursor = CursorShape::Beam,
        "bar.open-hidden" => app.settings.open_bar_hidden = !app.settings.open_bar_hidden,
        _ => return,
    }
    app.settings_changed();
}
