//! The file tabs above an area's editor: the open documents, the current
//! one lit, a close button (a dot while the file is unsaved), middle
//! click to close, a right click for the tab's menu, a drag to put a tab
//! somewhere else in the row.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

pub struct TabItem<'a> {
    pub label: &'a str,
    pub dirty: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TabsOut {
    pub select: Option<usize>,
    pub close: Option<usize>,
    /// A right click on a tab: which, and where its menu goes.
    pub context: Option<(usize, Vec2)>,
    /// A tab dropped elsewhere in the row: `(from, to)` as indices.
    pub reorder: Option<(usize, usize)>,
}

/// The pointer has to move this far from the press before a tab drags.
const DRAG_START: f64 = 8.0;

pub fn draw_tabs(ui: &mut Ui, items: &[TabItem], current: usize) -> TabsOut {
    let mut out = TabsOut::default();
    if items.is_empty() {
        return out;
    }
    let m = ui.m;
    let theme = ui.theme;
    let style = ui.text_style();
    let close_w = (m.widget_h * 0.55).round();
    // The strip runs from the area's top edge to the text below it, edge
    // to edge, and the tabs fill its whole height.
    let clip = ui.clip();
    let row = ui.alloc(Vec2::new(FILL, m.widget_h));
    let strip = Rect::new(Vec2::new(clip.min.x, clip.min.y), Vec2::new(clip.max.x, row.max.y + m.gap));
    let h = strip.height();
    ui.fill(strip, theme.panel.bottom);
    ui.draw.hline(strip.min.x, strip.max.x, strip.max.y - m.border, m.border, theme.border_dark);
    let widths: Vec<f64> = items.iter().map(|t| ui.measure(t.label, &style) + m.pad * 2.0 + close_w + m.gap).collect();
    let total: f64 = widths.iter().sum::<f64>() + m.gap * (items.len() as f64 - 1.0);
    let avail = strip.width() - m.border * 2.0;
    let scale = if total > avail { avail / total } else { 1.0 };
    let mut x = strip.min.x + m.border;
    let mut rects: Vec<Rect> = Vec::with_capacity(items.len());
    let pointer = ui.state.pointer;
    for (i, item) in items.iter().enumerate() {
        let w = (widths[i] * scale).floor().max(close_w * 2.5);
        let rect = Rect::from_min_size(Vec2::new(x.round(), strip.min.y), Vec2::new(w, h));
        rects.push(rect);
        x += w + m.gap;
        let id = ui.id("tab").with_index(i);
        // The close button is hit-tested first so it wins the press.
        let cr = Rect::from_center_size(Vec2::new(rect.max.x - close_w * 0.5 - m.gap, rect.center().y), Vec2::splat(close_w));
        let cres = ui.interact(id.with("close"), cr, Sense::CLICK);
        let mut r = ui.interact(id, rect, Sense::CLICK);
        ui.focusable(id, rect);
        ui.key_click(id, &mut r);
        if r.hovered || cres.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        if ui.state.middle_pressed && rect.contains(ui.state.middle_press_pos) {
            out.close = Some(i);
        }
        let is_current = i == current;
        if is_current {
            ui.raised(rect, theme.widget, false);
            let bar = Rect::new(Vec2::new(rect.min.x + m.border, rect.max.y - m.px(3.0)), Vec2::new(rect.max.x - m.border, rect.max.y - m.border));
            ui.draw.rect(bar, theme.accent);
        } else {
            let base = if r.hovered { theme.hover(theme.panel.mid()) } else { theme.panel.bottom };
            ui.fill(rect, base);
        }
        let ink = if is_current { theme.text } else { theme.text_dim };
        let label_rect = Rect::new(Vec2::new(rect.min.x + m.pad, rect.min.y), Vec2::new(cr.min.x - m.gap, rect.max.y));
        ui.text_in_rect(item.label, &style, label_rect, ink);
        // The close mark: a dot while unsaved, a cross otherwise (and on hover).
        if cres.hovered {
            ui.fill(cr, theme.close);
        }
        if item.dirty && !cres.hovered {
            let dot = Rect::from_center_size(cr.center(), Vec2::splat(close_w * 0.45));
            ui.draw.rounded_rect(dot, dot.width() * 0.5, theme.accent);
        } else {
            let s = close_w * 0.22;
            let c = cr.center();
            let w = m.px(2.0);
            let col = if cres.hovered { theme.selection_text } else { ink };
            ui.draw.line(Vec2::new(c.x - s, c.y - s), Vec2::new(c.x + s, c.y + s), w, col);
            ui.draw.line(Vec2::new(c.x - s, c.y + s), Vec2::new(c.x + s, c.y - s), w, col);
        }
        ui.focus_ring(id, rect);
        if ui.state.right_pressed && rect.contains(pointer) {
            out.context = Some((i, pointer));
        }
        if cres.clicked {
            out.close = Some(i);
        } else if r.clicked && !is_current {
            out.select = Some(i);
        }
    }
    // ---- a tab dragged along the row: a ghost follows the pointer and a
    // bar shows where it would land ----
    let press = ui.state.press_pos;
    let from = rects.iter().position(|r| r.contains(press));
    if let Some(from) = from
        && (ui.state.down || ui.state.released)
        && (pointer - press).length() > m.px(DRAG_START)
    {
        let to = rects.iter().filter(|r| r.center().x < pointer.x).count();
        let to = if to > from { to - 1 } else { to };
        if ui.state.released {
            if to != from {
                out.reorder = Some((from, to));
            }
            ui.state.request_rebuild = true;
        } else {
            let saved = ui.draw.layer();
            ui.draw.set_layer(saved + 2);
            let slot_x = if to >= from { rects[to].max.x + m.gap * 0.5 } else { rects[to].min.x - m.gap * 0.5 };
            if to != from {
                ui.draw.vline(slot_x.round(), strip.min.y, strip.max.y, m.px(3.0), theme.accent);
            }
            let label = items[from].label;
            let w = ui.measure(label, &style) + m.pad * 2.0;
            let ghost = Rect::from_min_size(Vec2::new(pointer.x - w * 0.5, strip.min.y), Vec2::new(w, h));
            ui.floating_panel(ghost, theme.header);
            ui.text_in_rect(label, &style, Rect::new(Vec2::new(ghost.min.x + m.pad, ghost.min.y), ghost.max), theme.text);
            ui.draw.set_layer(saved);
            ui.state.cursor_icon = CursorIcon::Grabbing;
            ui.state.request_rebuild = true;
        }
    }
    out
}
