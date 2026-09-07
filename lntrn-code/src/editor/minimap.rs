//! The minimap: the whole document as a strip of colored dashes at the
//! right of the editor, one thin row per line, in the token colors; the
//! part on screen boxed; a click or drag there scrolls to it.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, Sense, Ui};

use crate::doc::Doc;
use crate::editor::fold::Layout;
use crate::settings::SyntaxColors;
use crate::syntax::TokenKind;
use crate::text_util::indent_of;

/// The strip's width in logical pixels.
pub const WIDTH: f64 = 88.0;

pub struct MapIn<'a> {
    pub strip: Rect,
    pub layout: &'a Layout,
    pub first_row: usize,
    pub last_row: usize,
    /// The editor's line height and scroll position.
    pub lh: f64,
    pub scroll_y: f64,
    pub content_h: f64,
    pub view_h: f64,
}

/// Draw the map. Returns a new scroll position when it was clicked.
pub fn draw_minimap(ui: &mut Ui, doc: &Doc, colors: &SyntaxColors, mi: MapIn) -> Option<f64> {
    let m = ui.m;
    let theme = ui.theme;
    let strip = mi.strip;
    let rh = m.px(2.0).round().max(2.0);
    let cw = m.px(1.0).round().max(1.0);
    let pad = m.px(4.0);
    let max_chars = ((strip.width() - pad * 2.0) / cw).floor().max(1.0) as usize;
    let rows = mi.layout.rows();
    let map_h = rows as f64 * rh;
    let strip_h = strip.height();
    // A long file's map scrolls in step with the text.
    let map_off = if map_h > strip_h {
        let frac = (mi.scroll_y / (mi.content_h - mi.view_h).max(1.0)).clamp(0.0, 1.0);
        frac * (map_h - strip_h)
    } else {
        0.0
    };
    ui.draw.vline(strip.min.x, strip.min.y, strip.max.y, m.border, theme.border_light.fade(0.35));
    ui.draw.push_clip(strip);
    let base = colors.text.fade(0.28);
    let r0 = (map_off / rh).floor() as usize;
    let r1 = (((map_off + strip_h) / rh).ceil() as usize + 1).min(rows);
    for r in r0..r1 {
        let (line, seg) = mi.layout.seg_at(r);
        if seg > 0 {
            continue;
        }
        let text = doc.line(line);
        if text.trim().is_empty() {
            continue;
        }
        let y = strip.min.y + r as f64 * rh - map_off;
        let x0 = strip.min.x + pad;
        let indent = indent_of(text).len().min(max_chars);
        let len = text.len().min(max_chars);
        if len > indent {
            ui.draw.rect(Rect::new(Vec2::new(x0 + indent as f64 * cw, y), Vec2::new(x0 + len as f64 * cw, y + rh - 1.0)), base);
        }
        for t in doc.highlight.tokens(line) {
            if t.kind == TokenKind::Text {
                continue;
            }
            let (a, b) = ((t.start as usize).min(max_chars), (t.end as usize).min(max_chars));
            if b > a {
                ui.draw.rect(Rect::new(Vec2::new(x0 + a as f64 * cw, y), Vec2::new(x0 + b as f64 * cw, y + rh - 1.0)), colors.of(t.kind).fade(0.85));
            }
        }
    }
    // The rows on screen.
    let vy0 = strip.min.y + mi.first_row as f64 * rh - map_off;
    let vy1 = strip.min.y + mi.last_row.min(rows) as f64 * rh - map_off;
    let seen = Rect::new(Vec2::new(strip.min.x, vy0), Vec2::new(strip.max.x, vy1.max(vy0 + rh)));
    ui.draw.rect(seen, theme.text.fade(0.07));
    ui.draw.hline(seen.min.x, seen.max.x, seen.min.y, m.border, theme.text.fade(0.2));
    ui.draw.hline(seen.min.x, seen.max.x, seen.max.y - m.border, m.border, theme.text.fade(0.2));
    ui.draw.pop_clip();
    // A press or a drag scrolls the text to the row under the pointer.
    let id = ui.id("minimap");
    let r = ui.interact(id, strip, Sense::DRAG);
    if r.hovered || r.dragging {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    if (r.pressed || r.dragging) && strip.contains(ui.state.pointer) {
        let row = ((ui.state.pointer.y - strip.min.y + map_off) / rh).max(0.0);
        let y = (row * mi.lh - mi.view_h * 0.5).clamp(0.0, (mi.content_h - mi.view_h).max(0.0));
        return Some(y);
    }
    None
}
