//! The sidebar: every page as a row with its picture, under its group's
//! caption, the one showing on an accent pill.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

use crate::kit;
use crate::look;
use crate::nav::{GROUPS, Page};

/// Logical width of the sidebar.
pub const WIDTH: f64 = 270.0;
const ROW_H: f64 = 56.0;
/// Room between a row and the sidebar's right edge.
const EDGE: f64 = 12.0;

pub fn draw(ui: &mut Ui, page: &mut Page) {
    let m = ui.m;
    let (top, w) = (ui.cursor(), ui.avail_width());
    // The shell insets the body by its padding; the sidebar's shade runs
    // out to the window's edges.
    let shade = Rect::new(Vec2::new(top.x - m.pad, top.y - m.pad), Vec2::new(top.x + w, top.y + ui.remaining_height() + m.pad));
    ui.draw.rect(shade, Color::BLACK.fade(0.25));
    ui.draw.vline(shade.max.x - m.px(1.0), shade.min.y, shade.max.y, m.px(1.0), look::LINE);

    ui.push_id("sidebar");
    let small = kit::small_style(ui).bold();
    for group in GROUPS {
        let caption = ui.alloc(Vec2::new(FILL, m.px(44.0)));
        let text = Rect::new(Vec2::new(caption.min.x + m.px(16.0), caption.min.y + m.px(8.0)), caption.max);
        ui.text_in_rect(&group.label.to_uppercase(), &small, text, look::TEXT_DIM);
        for p in group.pages {
            if row(ui, *p, *page == *p) {
                *page = *p;
            }
        }
        ui.space(m.px(10.0));
    }
    ui.pop_id();
}

/// One page's row. `true` when it was clicked.
fn row(ui: &mut Ui, page: Page, showing: bool) -> bool {
    let m = ui.m;
    let full = ui.alloc(Vec2::new(FILL, m.px(ROW_H)));
    let rect = Rect::new(full.min, Vec2::new(full.max.x - m.px(EDGE), full.max.y));
    let id = ui.id(page.id());
    let mut r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let radius = m.px(12.0);
    if showing {
        ui.draw.rounded_rect(rect, radius, ui.theme.accent);
    } else if r.hovered || r.held {
        ui.draw.rounded_rect(rect, radius, Color::WHITE.fade(0.07));
    }
    let ink = if showing { ui.theme.accent_text } else { look::TEXT };
    let picture = Rect::from_center_size(Vec2::new(rect.min.x + m.px(34.0), rect.center().y), Vec2::splat(m.px(30.0)));
    (page.glyph())(ui.draw, picture, ink, m.px(2.5));
    let style = ui.text_style();
    let label = Rect::new(Vec2::new(rect.min.x + m.px(64.0), rect.min.y), Vec2::new(rect.max.x - m.px(10.0), rect.max.y));
    ui.text_in_rect(page.label(), &style, label, ink);
    ui.focus_ring(id, rect);
    r.clicked
}
