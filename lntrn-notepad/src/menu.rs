//! Notepad's menus: the title bar's, the right-click one, the colour
//! swatches a toolbar button opens, and the palette's commands.

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_props::Value;
use lntrn_ui::{Action, ContextMenu, CursorIcon, FILL, HostCx, Item, Menu, MenuItem, Sense, ShellRequest, Ui, actions};

use crate::app::App;
use crate::doc::{Align, List, Style};
use crate::editor::Editor;
use crate::view::page::picture_at_caret;
use crate::view::toolbar::Swatch;

/// What the palette offers: (action, what it says).
pub const COMMANDS: &[(&str, &str)] = &[
    ("file.new", "New Document"),
    ("file.open", "Open\u{2026}"),
    ("file.save", "Save"),
    ("file.save-as", "Save As\u{2026}"),
    ("file.export-pdf", "Export as PDF\u{2026}"),
    ("file.export-docx", "Export as Word Document\u{2026}"),
    ("file.export-md", "Export as Markdown\u{2026}"),
    ("file.export-txt", "Export as Plain Text\u{2026}"),
    ("tab.close", "Close Tab"),
    ("edit.undo", "Undo"),
    ("edit.redo", "Redo"),
    ("edit.cut", "Cut"),
    ("edit.copy", "Copy"),
    ("edit.paste", "Paste"),
    ("edit.paste-plain", "Paste as Plain Text"),
    ("edit.select-all", "Select All"),
    ("find.show", "Find\u{2026}"),
    ("find.replace", "Replace\u{2026}"),
    ("find.next", "Find Next"),
    ("find.prev", "Find Previous"),
    ("format.bold", "Bold"),
    ("format.italic", "Italic"),
    ("format.underline", "Underline"),
    ("format.strike", "Strike Through"),
    ("format.clear", "Clear Formatting"),
    ("size.bigger", "Bigger Text"),
    ("size.smaller", "Smaller Text"),
    ("style.body", "Style: Body"),
    ("style.title", "Style: Title"),
    ("style.h1", "Style: Heading 1"),
    ("style.h2", "Style: Heading 2"),
    ("style.h3", "Style: Heading 3"),
    ("style.quote", "Style: Quote"),
    ("list.bullet", "Bulleted List"),
    ("list.number", "Numbered List"),
    ("list.check", "Checklist"),
    ("list.in", "Indent List Item"),
    ("list.out", "Outdent List Item"),
    ("align.left", "Align Left"),
    ("align.center", "Centre"),
    ("align.right", "Align Right"),
    ("align.justify", "Justify"),
    ("picture.insert", "Insert Picture\u{2026}"),
    ("picture.natural", "Picture: Its Own Size"),
    ("picture.fit", "Picture: Fit the Page"),
    ("view.paper", "Paper Page"),
    ("view.dark", "Dark Page"),
];

/// The keys the page itself takes, which the keymap doesn't know.
pub fn fixed_hint(action: &str) -> Option<&'static str> {
    Some(match action {
        "edit.undo" => "Ctrl+Z",
        "edit.redo" => "Ctrl+Shift+Z",
        "edit.cut" => "Ctrl+X",
        "edit.copy" => "Ctrl+C",
        "edit.paste" => "Ctrl+V",
        "edit.paste-plain" => "Ctrl+Shift+V",
        "edit.select-all" => "Ctrl+A",
        "list.in" => "Tab",
        "list.out" => "Shift+Tab",
        _ => return None,
    })
}

fn item(label: &str, id: &str) -> MenuItem {
    MenuItem::new(label, Action::new(id))
}

/// A menu of the title bar's.
pub fn title_menu(app: &App, name: &str) -> Option<Menu> {
    let front = app.front.upgrade();
    let d = front.as_ref().and_then(|d| d.try_borrow().ok());
    let (run, para) = d.as_ref().map(|d| (d.ed.format_state(), d.ed.para_state())).unwrap_or_default();
    let (undo, redo) = d.as_ref().map_or((false, false), |d| d.ed.can_undo());
    let selected = d.as_ref().is_some_and(|d| d.ed.selection().is_some());
    let (title, items) = match name {
        "file" => (
            "File",
            vec![
                item("New", "file.new"),
                item("Open\u{2026}", "file.open"),
                MenuItem::separator(),
                item("Save", "file.save"),
                item("Save As\u{2026}", "file.save-as"),
                MenuItem::sub("Export", vec![item("PDF\u{2026}", "file.export-pdf"), item("Word Document\u{2026}", "file.export-docx"), item("Markdown\u{2026}", "file.export-md"), item("Plain Text\u{2026}", "file.export-txt")]),
                MenuItem::separator(),
                item("Close Tab", "tab.close"),
                item("Quit", actions::QUIT),
            ],
        ),
        "edit" => (
            "Edit",
            vec![
                item("Undo", "edit.undo").enabled(undo),
                item("Redo", "edit.redo").enabled(redo),
                MenuItem::separator(),
                item("Cut", "edit.cut").enabled(selected),
                item("Copy", "edit.copy").enabled(selected),
                item("Paste", "edit.paste"),
                item("Paste as Plain Text", "edit.paste-plain"),
                item("Select All", "edit.select-all"),
                MenuItem::separator(),
                item("Find\u{2026}", "find.show"),
                item("Replace\u{2026}", "find.replace"),
            ],
        ),
        "format" => (
            "Format",
            vec![
                item("Bold", "format.bold").checked(run.bold),
                item("Italic", "format.italic").checked(run.italic),
                item("Underline", "format.underline").checked(run.underline),
                item("Strike Through", "format.strike").checked(run.strike),
                MenuItem::separator(),
                MenuItem::sub("Style", Style::ALL.iter().map(|s| item(s.label(), &format!("style.{}", s.word())).checked(para.style == *s)).collect()),
                MenuItem::sub("List", vec![item("Bullets", "list.bullet").checked(para.list == List::Bullet), item("Numbers", "list.number").checked(para.list == List::Number), item("Checklist", "list.check").checked(para.list.same_kind(List::Check(false))), MenuItem::separator(), item("Indent", "list.in"), item("Outdent", "list.out")]),
                MenuItem::sub("Align", vec![item("Left", "align.left").checked(para.align == Align::Left), item("Centre", "align.center").checked(para.align == Align::Center), item("Right", "align.right").checked(para.align == Align::Right), item("Justify", "align.justify").checked(para.align == Align::Justify)]),
                MenuItem::separator(),
                item("Bigger Text", "size.bigger"),
                item("Smaller Text", "size.smaller"),
                item("Clear Formatting", "format.clear"),
            ],
        ),
        "insert" => ("Insert", vec![item("Picture\u{2026}", "picture.insert")]),
        "view" => ("View", vec![item("Paper Page", "view.paper").checked(app.settings.paper), item("Dark Page", "view.dark").checked(!app.settings.paper), MenuItem::separator(), item("Command Palette", actions::PALETTE)]),
        _ => return None,
    };
    Some(Menu::new(title, items))
}

/// The right-click menu for the page, at `at`: what can be done with
/// what is selected, or with the picture under the pointer.
pub fn context(ed: &Editor, at: Vec2) -> ContextMenu {
    let row = |label: &str, id: &str| Item::action(label, Action::new(id));
    let mut items = vec![row("Cut", "edit.cut"), row("Copy", "edit.copy"), row("Paste", "edit.paste"), row("Paste as Plain Text", "edit.paste-plain"), row("Select All", "edit.select-all"), Item::Separator];
    if picture_at_caret(ed).is_some() {
        items.extend([row("Its Own Size", "picture.natural"), row("Fit the Page", "picture.fit")]);
    } else {
        items.extend([row("Bold", "format.bold"), row("Italic", "format.italic"), row("Underline", "format.underline"), row("Clear Formatting", "format.clear"), Item::Separator, row("Bullets", "list.bullet"), row("Numbers", "list.number"), row("Checklist", "list.check")]);
    }
    ContextMenu::new("", at).tab("Edit", items)
}

/// Ask for a picture file to put in.
pub fn ask_picture() -> ShellRequest {
    let from = lntrn_sys::dirs::user_dir(lntrn_sys::dirs::UserDir::Pictures).map(|d| d.display().to_string()).unwrap_or_default();
    ShellRequest::PathDialog { action: Action::new("picture.insert-path"), save: false, suggest: from }
}

/// The colours a swatch menu offers: for text, and for the highlighter.
pub const TEXT_COLORS: [u32; 11] = [0x1C1B18, 0x706C64, 0xCC3A34, 0xD9822B, 0xC89600, 0x4A9C54, 0x2A9D8F, 0x2563EB, 0x7C3AED, 0xD6408A, 0xF5F0E6];
pub const HIGHLIGHTS: [u32; 8] = [0xFFF0A0, 0xFAD860, 0xCDEFC4, 0xB8ECE4, 0xCFE4FF, 0xE6D5FF, 0xFFD3E2, 0xFFDDB8];

/// The menu a colour button opens: one row, drawn by [`draw_item`].
pub fn swatches(at: Vec2) -> ContextMenu {
    ContextMenu::new("", at).tab("Colour", vec![Item::custom("swatches")])
}

/// Draw one of the menus' own rows.
pub fn draw_item(app: &mut App, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
    if key != "swatches" {
        return false;
    }
    let m = ui.m;
    let (which, colors): (Swatch, &[u32]) = match app.swatch {
        Swatch::Text => (Swatch::Text, &TEXT_COLORS),
        Swatch::Highlight => (Swatch::Highlight, &HIGHLIGHTS),
    };
    let current = app.front.upgrade().map(|d| d.borrow().ed.format_state()).map(|a| if which == Swatch::Text { a.color } else { a.highlight });
    let style = ui.text_style();
    let heading = ui.alloc(Vec2::new(FILL, style.line_height() as f64 + m.gap));
    ui.text_in_rect(if which == Swatch::Text { "Text colour" } else { "Highlight" }, &style, heading, look::TEXT_DIM);
    // A grid of chips, the first of them for none.
    let (side, gap, per_row) = (m.px(48.0), m.px(8.0), 6);
    let rows = (colors.len() + 1).div_ceil(per_row);
    let area = ui.alloc(Vec2::new(FILL, rows as f64 * (side + gap)));
    let mut picked = None;
    for i in 0..=colors.len() {
        let color = i.checked_sub(1).map(|i| colors[i]);
        let rect = Rect::from_min_size(area.min + Vec2::new((i % per_row) as f64 * (side + gap), (i / per_row) as f64 * (side + gap)), Vec2::splat(side));
        let r = ui.interact(ui.id("chip").with_index(i), rect, Sense::CLICK);
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        let ring = if current == Some(color) { ui.theme.accent } else if r.hovered { look::TEXT } else { look::TRACK };
        ui.draw.rounded_rect(rect, m.px(12.0), ring);
        let inner = rect.shrink(m.px(3.0));
        match color {
            Some(rgb) => ui.draw.rounded_rect(inner, m.px(9.0), Color::hex(rgb)),
            None => {
                // None: an empty chip, struck through.
                ui.draw.rounded_rect(inner, m.px(9.0), look::CARD);
                ui.draw.line(Vec2::new(inner.min.x + m.px(8.0), inner.max.y - m.px(8.0)), Vec2::new(inner.max.x - m.px(8.0), inner.min.y + m.px(8.0)), m.px(2.5), look::BAD);
            }
        }
        if r.clicked {
            picked = Some(color);
        }
    }
    let Some(color) = picked else { return false };
    let id = match which {
        Swatch::Text => "color.text",
        Swatch::Highlight => "color.highlight",
    };
    let action = Action::new(id).with("rgb", Value::Str(color.map_or_else(String::new, |rgb| format!("{rgb:06x}"))));
    crate::actions::run(app, &action, cx);
    cx.request(ShellRequest::ClosePopup);
    true
}
