//! A row of the file tree: a folder's disclosure triangle, then the
//! marks that sit left of the name (a folder glyph or the file's
//! extension chip, the git dot, error and warning counts), then the
//! name. Also the house the panel's `⌂` button wears.

use std::path::Path;

use lntrn_app::lntrn_render::DrawList;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{CursorIcon, FILL, Icon, Key, Sense, Ui, icons};

use crate::settings::SyntaxColors;
use crate::syntax::Language;

/// What sits in the icon slot before the name.
pub enum Slot {
    Folder,
    /// A file: its extension chip, when it has one.
    File(Option<(String, Color)>),
}

pub struct RowSpec<'a> {
    pub label: &'a str,
    pub selected: bool,
    /// A folder with children under it, and whether it starts open.
    /// `None`: a plain row.
    pub branch: Option<bool>,
    /// No room for a disclosure triangle (a flat list).
    pub flat: bool,
    pub slot: Slot,
    pub git: Option<Color>,
    pub errors: usize,
    pub warnings: usize,
    /// Drawn dim: shown, not for taking.
    pub dim: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RowOut {
    pub clicked: bool,
    pub double_clicked: bool,
    /// Whether a folder's children show (after this frame's clicks).
    pub open: bool,
    /// Backspace on the focused row: go up a folder.
    pub back: bool,
    pub rect: Rect,
}

/// One row. A folder's children are the caller's to draw under it
/// (indented, under `push_id(label)`) when `open` comes back true. A
/// click anywhere on a folder row opens or closes it; a double click is
/// the caller's (the tree goes into the folder).
pub fn tree_row(ui: &mut Ui, spec: &RowSpec) -> RowOut {
    let id = ui.id(spec.label);
    let m = ui.m;
    let rect = ui.alloc(Vec2::new(FILL, m.widget_h));
    let disc = Rect::from_min_size(rect.min, Vec2::splat(rect.height()));
    let mut r = ui.interact(id, rect, Sense::CLICK);
    let focused = ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let mut open = match spec.branch {
        Some(default) => ui.state.open_default(id, default),
        None => false,
    };
    let back = focused && ui.state.take_key(|k| k.key == Key::Backspace).is_some();
    if spec.branch.is_some() {
        let by_key = focused && ui.state.take_key(|k| matches!(k.key, Key::ArrowLeft | Key::ArrowRight)).map(|k| k.key == Key::ArrowRight).is_some_and(|want| want != open);
        let on_disc = r.clicked && !spec.flat && disc.contains(ui.state.pointer);
        if (r.clicked && !r.double_clicked) || by_key {
            open = !open;
            *ui.state.open(id) = open;
            ui.state.request_rebuild = true;
            if on_disc {
                r.clicked = false;
            }
        }
    }

    // ---- draw ----
    let theme = ui.theme;
    let style = ui.text_style();
    if spec.selected {
        ui.fill_shaded(rect, theme.shaded(theme.selection));
    } else if r.hovered || r.held {
        ui.fill(rect, theme.hover(theme.panel.mid()));
    }
    let ink = if spec.selected {
        theme.selection_text
    } else if spec.dim {
        theme.text_dim
    } else {
        theme.text
    };
    let dim = if spec.selected { theme.selection_text } else { theme.text_dim };
    let mut x = rect.min.x + m.pad;
    if !spec.flat {
        if spec.branch.is_some() {
            triangle(ui, disc, open, dim);
        }
        x = disc.max.x;
    }
    let slot_w = (m.widget_h * 1.25).round();
    let slot = Rect::from_min_size(Vec2::new(x, rect.min.y), Vec2::new(slot_w, rect.height()));
    match &spec.slot {
        Slot::Folder => icons::draw(&mut *ui.draw, slot, Icon::Folder, dim, m.px(1.5)),
        Slot::File(Some((ext, color))) => chip(ui, slot, ext, *color),
        Slot::File(None) => {}
    }
    x += slot_w + m.gap;
    if let Some(c) = spec.git {
        let rad = (m.widget_h * 0.12).round().max(m.px(3.0));
        ui.draw.circle(Vec2::new(x + rad, rect.center().y), rad, c);
        x += rad * 2.0 + m.gap;
    }
    let small = small_style(ui);
    for (count, color) in [(spec.errors, theme.close), (spec.warnings, theme.accent)] {
        if count == 0 {
            continue;
        }
        let text = count.to_string();
        let w = ui.measure(&text, &small);
        ui.text_in_rect(&text, &small, Rect::new(Vec2::new(x, rect.min.y), Vec2::new(x + w + m.pad, rect.max.y)), color);
        x += w + m.gap;
    }
    let text_rect = Rect::new(Vec2::new(x, rect.min.y), Vec2::new(rect.max.x - m.pad, rect.max.y));
    ui.text_in_rect(spec.label, &style, text_rect, ink);
    ui.focus_ring(id, rect);
    RowOut { clicked: r.clicked, double_clicked: r.double_clicked, open, back, rect }
}

/// A house, for the `⌂` button: a roof over a box.
pub fn house(d: &mut DrawList, r: Rect, color: Color, stroke: f64) {
    let c = r.center();
    let s = r.width().min(r.height()) * 0.3;
    d.line(Vec2::new(c.x - s * 1.1, c.y), Vec2::new(c.x, c.y - s), stroke, color);
    d.line(Vec2::new(c.x, c.y - s), Vec2::new(c.x + s * 1.1, c.y), stroke, color);
    let body = Rect::new(Vec2::new(c.x - s * 0.7, c.y - s * 0.15), Vec2::new(c.x + s * 0.7, c.y + s * 0.9));
    d.stroke_rect(body, stroke, 0.0, color);
}

fn triangle(ui: &mut Ui, disc: Rect, open: bool, color: Color) {
    let s = ui.m.px(6.0);
    let c = disc.center();
    let w = ui.m.px(2.0);
    if open {
        ui.draw.line(Vec2::new(c.x - s, c.y - s * 0.5), Vec2::new(c.x, c.y + s * 0.5), w, color);
        ui.draw.line(Vec2::new(c.x, c.y + s * 0.5), Vec2::new(c.x + s, c.y - s * 0.5), w, color);
    } else {
        ui.draw.line(Vec2::new(c.x - s * 0.5, c.y - s), Vec2::new(c.x + s * 0.5, c.y), w, color);
        ui.draw.line(Vec2::new(c.x + s * 0.5, c.y), Vec2::new(c.x - s * 0.5, c.y + s), w, color);
    }
}

/// The extension in a small chip filling the slot.
fn chip(ui: &mut Ui, slot: Rect, ext: &str, color: Color) {
    let m = ui.m;
    let style = small_style(ui);
    let h = (m.widget_h * 0.62).round();
    let rect = Rect::from_min_size(Vec2::new(slot.min.x, slot.center().y - h * 0.5), Vec2::new(slot.width() - m.px(2.0), h));
    ui.draw.rounded_rect(rect, m.radius * 0.6, color.fade(0.22));
    ui.text_centered(ext, &style, rect, color);
}

/// A file's extension (at most four letters) and the color of its
/// language, for its chip.
pub fn ext_of(path: &Path, colors: &SyntaxColors, dim: Color) -> Option<(String, Color)> {
    let ext = path.extension()?.to_str()?;
    let ext: String = ext.chars().take(4).collect::<String>().to_lowercase();
    let color = match Language::detect(path, "") {
        Language::Rust => colors.number,
        Language::Toml => colors.attribute,
        Language::Markdown => colors.heading,
        Language::Json => colors.string,
        Language::Python => colors.function,
        Language::JavaScript => colors.types,
        Language::C => colors.keyword,
        Language::Shell => colors.emphasis,
        Language::Yaml => colors.link,
        Language::Plain => dim,
    };
    Some((ext, color))
}

pub fn small_style(ui: &Ui) -> TextStyle {
    TextStyle::new((ui.m.text_size * 0.72).round().max(8.0))
}
