//! The page on screen: a sheet on the desk with the document set on it,
//! drawn where it is scrolled to, with the selection, the caret, what a
//! search found, and the handles a hand can take hold of (the sheet's
//! edges, a picture's corner, the scroll bar). What the keys and the
//! pointer do to it is in `input.rs`.

use std::collections::HashMap;

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::GlyphQuad;
use lntrn_ui::{CursorIcon, FILL, ImageHandle, Sense, Ui};

use super::input;
use super::layout::Setting;
use super::paint::{Inks, Mark, marks};
use crate::doc::Pos;
use crate::editor::{Clip, Editor};

/// Room the sheet keeps around its text, and around itself on the desk,
/// in logical pixels.
pub const PAGE_PAD: f64 = 56.0;
pub const PAGE_TOP: f64 = 44.0;
const DESK_MARGIN: f64 = 24.0;
/// The narrowest the sheet goes.
pub const MIN_PAGE: f64 = 460.0;
const HANDLE: f64 = 14.0;
pub const BAR: f64 = 12.0;

/// What a drag that started on the page is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Drag {
    #[default]
    None,
    Select,
    /// A side of the sheet.
    Margin,
    /// A picture's corner: its paragraph, and how wide it was (logical
    /// pixels) when the pointer was at this x.
    Picture(usize, f32, f64),
    /// The scroll bar's thumb, held this far from its top.
    Scroll(f64),
}

/// What a tab remembers of how its page is being looked at and handled.
#[derive(Default)]
pub struct View {
    pub scroll: f64,
    pub drag: Drag,
    /// When and where the last press was, and how many in a row.
    pub clicks: (f64, Vec2, u8),
    /// When the caret last moved: it shows steady from then.
    pub blink: f64,
    /// A paste is waiting for the clipboard to be read; and it is to go
    /// in as text, whatever it was.
    pub paste: bool,
    pub paste_plain: bool,
    /// What the layout was last set to, for the status line's use.
    pub setting: Option<Setting>,
}

/// What the page is shown with.
pub struct PageIn<'a> {
    pub inks: Inks,
    /// Body text's size, logical pixels.
    pub body: f32,
    /// How wide the sheet is, 0 (narrowest) to 1 (all the room).
    pub page_width: &'a mut f64,
    pub clip: &'a mut Option<Clip>,
    /// The textures of the pictures that have one.
    pub pictures: &'a HashMap<u64, ImageHandle>,
    /// What a search found, and which of them is the one it is on.
    pub hits: &'a [(Pos, Pos)],
    pub hit: Option<usize>,
    /// Take the keyboard this frame.
    pub grab: bool,
}

#[derive(Default)]
pub struct PageOut {
    pub focused: bool,
    /// A right click: where a menu goes.
    pub context: Option<Vec2>,
    /// The sheet's width was dragged.
    pub resized: bool,
}

/// How wide the sheet is, in pixels, in a desk `room` wide.
pub fn sheet_width(room: f64, wide: f64, scale: f64) -> f64 {
    let most = (room - DESK_MARGIN * scale * 2.0).max(1.0);
    let least = (MIN_PAGE * scale).min(most);
    least + (most - least) * wide.clamp(0.0, 1.0)
}

/// The other way: how wide (0 to 1) makes the sheet `pixels` wide.
pub fn wide_for(room: f64, pixels: f64, scale: f64) -> f64 {
    let most = (room - DESK_MARGIN * scale * 2.0).max(1.0);
    let least = (MIN_PAGE * scale).min(most);
    if most > least { ((pixels - least) / (most - least)).clamp(0.0, 1.0) } else { 1.0 }
}

/// Where everything is this frame.
#[derive(Clone, Copy)]
pub struct Place {
    pub desk: Rect,
    pub sheet: Rect,
    /// The document's own corner on screen: its x is the text column's
    /// left, its y where the document's top is scrolled to.
    pub origin: Vec2,
    pub column: f64,
    pub scale: f64,
}

impl Place {
    pub fn of(desk: Rect, wide: f64, scroll: f64, scale: f64) -> Place {
        let bar = BAR * scale;
        let w = sheet_width(desk.width() - bar, wide, scale);
        let x = (desk.min.x + (desk.width() - bar - w) * 0.5).round();
        let sheet = Rect::new(Vec2::new(x, desk.min.y), Vec2::new(x + w, desk.max.y));
        let pad = (PAGE_PAD * scale).min(w * 0.12);
        Place { desk, sheet, origin: Vec2::new(sheet.min.x + pad, desk.min.y + PAGE_TOP * scale - scroll), column: (w - pad * 2.0).max(40.0), scale }
    }

    /// A point of the screen, in the document.
    pub fn to_doc(self, p: Vec2) -> (f32, f32) {
        ((p.x - self.origin.x) as f32, (p.y - self.origin.y) as f32)
    }

    /// The most the page scrolls: until its last line is a little over
    /// the bottom.
    pub fn max_scroll(&self, doc_height: f32) -> f64 {
        (f64::from(doc_height) + PAGE_TOP * self.scale * 2.0 - self.desk.height()).max(0.0)
    }
}

fn ink(rgb: u32) -> Color {
    Color::hex(rgb)
}

/// Show the document over `height` of the room and take this frame's
/// input to it.
pub fn page(ui: &mut Ui, ed: &mut Editor, view: &mut View, height: f64, mut p: PageIn) -> PageOut {
    let id = ui.id("page");
    let m = ui.m;
    let desk = ui.alloc(Vec2::new(FILL, height));
    let mut out = PageOut::default();
    let mut place = Place::of(desk, *p.page_width, view.scroll, m.scale);
    let setting = Setting { width: place.column as f32, scale: m.scale as f32, body: p.body };
    ed.now = ui.state.now;
    ed.sync(ui.text, setting);
    view.setting = Some(setting);
    view.scroll = view.scroll.clamp(0.0, place.max_scroll(ed.layout.height()));

    // ---- what a hand can take hold of, over the page itself ----
    let grip = |x: f64| Rect::new(Vec2::new(x - HANDLE * m.scale * 0.5, desk.min.y), Vec2::new(x + HANDLE * m.scale * 0.5, desk.max.y));
    let sides = [ui.interact(id.with("left"), grip(place.sheet.min.x), Sense::DRAG), ui.interact(id.with("right"), grip(place.sheet.max.x), Sense::DRAG)];
    let bar = Rect::new(Vec2::new(desk.max.x - BAR * m.scale, desk.min.y), desk.max);
    let bar_r = ui.interact(id.with("bar"), bar, Sense::DRAG);
    let corner = picture_corner(ed, &place);
    let corner_r = corner.map(|(_, rect)| ui.interact(id.with("corner"), rect.expand(m.px(6.0)), Sense::DRAG));
    let r = ui.interact(id, desk, Sense::FOCUS);
    let focused = ui.focusable(id, desk);
    if p.grab && ui.state.popup.is_none() {
        ui.state.focus = Some(id);
        ui.state.focus_visible = false;
    }
    out.focused = focused;

    let before = (ed.rev, ed.caret, ed.anchor);
    input::pointer(ui, ed, view, &mut place, &mut p, &r, &sides, &bar_r, corner.zip(corner_r), &mut out);
    if focused {
        input::keys(ui, ed, view, &mut p, desk.height() as f32);
    }
    input::pasted(ui, ed, view, &mut p);
    if before != (ed.rev, ed.caret, ed.anchor) {
        view.blink = ui.state.now;
        ui.state.request_rebuild = true;
    }
    // The text may have changed, and with it where things are.
    ed.sync(ui.text, setting);
    let most = place.max_scroll(ed.layout.height());
    if std::mem::take(&mut ed.follow) {
        let (_, y, h) = ed.layout.caret(ed.caret);
        let (top, bottom) = (f64::from(y), f64::from(y + h) + PAGE_TOP * m.scale * 1.5);
        view.scroll = view.scroll.min(top).max(bottom - desk.height());
    }
    view.scroll = view.scroll.clamp(0.0, most);
    place = Place::of(desk, *p.page_width, view.scroll, m.scale);

    // ---- the desk, the sheet on it ----
    ui.draw.push_clip(desk);
    ui.draw.rect(desk, look::BG.fade(ui.state.opacity));
    ui.draw.shadow(place.sheet, 0.0, m.px(18.0), Color::BLACK.fade(0.45));
    ui.draw.rect(place.sheet, ink(p.inks.page));
    let (from, to) = (ed.layout.para_at((view.scroll - PAGE_TOP * m.scale) as f32), ed.layout.para_at((view.scroll + desk.height()) as f32));
    let at = |x: f32, y: f32| place.origin + Vec2::new(f64::from(x), f64::from(y));
    // What is selected, and what a search found, under the text.
    let wash = |ui: &mut Ui, ed: &Editor, a: Pos, b: Pos, color: Color| {
        for para in a.para.max(from)..=b.para.min(to) {
            let (set, top) = (&ed.layout.sets[para], ed.layout.tops[para]);
            let (s, e) = (if para == a.para { a.byte } else { 0 }, if para == b.para { b.byte } else { set.src.text.len() });
            for (i, row) in set.rows.iter().enumerate() {
                let (s, e) = (s.max(row.start), e.min(row.end));
                // A paragraph's end that is selected shows as a little more.
                let nub = if para < b.para && i + 1 == set.rows.len() { row.height * 0.3 } else { 0.0 };
                let (x0, x1) = (row.x_of(s), row.x_of(e) + nub);
                if s <= e && x1 > x0 {
                    ui.draw.rect(Rect::new(at(x0, top + row.top), at(x1, top + row.top + row.height)), color);
                }
            }
        }
    };
    for (i, (a, b)) in p.hits.iter().enumerate() {
        wash(ui, ed, *a, *b, ink(0xFAC800).fade(if p.hit == Some(i) { 0.75 } else { 0.3 }));
    }
    if let Some((a, b)) = ed.selection() {
        wash(ui, ed, a, b, ui.theme.accent.fade(if focused { 0.38 } else { 0.2 }));
    }
    // The paragraphs in sight.
    let mut quads: Vec<GlyphQuad> = Vec::new();
    let Ui { draw, text, .. } = ui;
    for para in from..=to {
        let (set, top) = (&ed.layout.sets[para], ed.layout.tops[para]);
        marks(text, set, &ed.doc, para, top, &setting, &p.inks, &mut |text, mark| match mark {
            Mark::Rect { x, y, w, h, color } => draw.rect(Rect::new(at(x, y), at(x + w, y + h)), ink(color)),
            Mark::Line { x0, y0, x1, y1, width, color } => draw.line(at(x0, y0), at(x1, y1), f64::from(width), ink(color)),
            Mark::Text { text: s, style, x, baseline, ascent, color } => {
                let o = at(x, baseline - ascent);
                quads.clear();
                text.place(s, style, o.x as f32, o.y as f32, 1.0e6, ink(color).to_gpu(), &mut quads);
                draw.glyphs(&quads);
            }
            Mark::Picture { id, x, y, w, h } => {
                let rect = Rect::new(at(x, y), at(x + w, y + h));
                match p.pictures.get(&id) {
                    Some(handle) => draw.image(rect, *handle, 0.0, Color::WHITE),
                    None => draw.rect(rect, ink(p.inks.dim).fade(0.25)),
                }
            }
        });
    }
    // A picture that is selected: its edge, and the corner that sizes it.
    if let Some((_, rect)) = picture_corner(ed, &place) {
        let frame = picture_rect(ed, &place).unwrap_or(rect);
        ui.draw.stroke_rect(frame, m.px(2.0), 0.0, ui.theme.accent);
        ui.draw.rect(rect, ui.theme.accent);
    }
    // The caret, steady just after it moved and blinking at rest.
    if focused && ed.selection().is_none() {
        let since = ui.state.now - view.blink;
        if since < 0.6 || (since - 0.6) % 1.1 < 0.6 {
            let (x, y, h) = ed.layout.caret(ed.caret);
            let top = at(x, y);
            let caret = Rect::new(Vec2::new(top.x.round(), top.y), Vec2::new(top.x.round() + m.px(2.0), top.y + f64::from(h)));
            ui.draw.rect(caret, ink(p.inks.text));
            ui.state.ime_rect = Some(caret);
        }
        ui.state.request_redraw_after(0.25);
    }
    // The sheet's edges light up under the pointer.
    for (side, x) in sides.iter().zip([place.sheet.min.x, place.sheet.max.x]) {
        if side.hovered || side.held {
            ui.state.cursor_icon = CursorIcon::EwResize;
            ui.draw.rect(Rect::new(Vec2::new(x - m.px(1.5), desk.min.y), Vec2::new(x + m.px(1.5), desk.max.y)), ui.theme.accent.fade(0.8));
        }
    }
    // The scroll bar, when there is somewhere to scroll to.
    if most > 0.0 {
        let thumb = thumb_rect(bar, view.scroll, most);
        ui.draw.rounded_rect(thumb.shrink(m.px(3.0)), m.px(3.0), look::TRACK.lerp(look::TEXT, if bar_r.hovered || bar_r.held { 0.5 } else { 0.15 }));
    }
    ui.draw.pop_clip();
    if r.hovered && !sides.iter().any(|s| s.hovered) && !bar_r.hovered && place.sheet.contains(ui.state.pointer) {
        ui.state.cursor_icon = CursorIcon::Text;
    }
    out
}

/// Where the scroll bar's thumb is, in its track.
pub fn thumb_rect(bar: Rect, scroll: f64, most: f64) -> Rect {
    let len = (bar.height() * bar.height() / (bar.height() + most)).max(bar.width() * 2.0).min(bar.height());
    let top = bar.min.y + (bar.height() - len) * (scroll / most.max(1.0)).clamp(0.0, 1.0);
    Rect::new(Vec2::new(bar.min.x, top), Vec2::new(bar.max.x, top + len))
}

/// The picture the caret is at or that is selected, as its paragraph.
pub fn picture_at_caret(ed: &Editor) -> Option<usize> {
    let para = ed.caret.para;
    let whole = ed.selection().is_none_or(|(a, b)| a.para == para && b.para == para);
    (whole && ed.doc.para(para).is_picture()).then_some(para)
}

fn picture_rect(ed: &Editor, place: &Place) -> Option<Rect> {
    let para = picture_at_caret(ed)?;
    let (set, top) = (ed.layout.sets.get(para)?, *ed.layout.tops.get(para)?);
    let ((w, h), row) = (set.picture?, &set.rows[0]);
    let min = place.origin + Vec2::new(f64::from(row.x), f64::from(top + row.top));
    Some(Rect::new(min, min + Vec2::new(f64::from(w), f64::from(h))))
}

/// Its paragraph and the corner it is sized by.
pub fn picture_corner(ed: &Editor, place: &Place) -> Option<(usize, Rect)> {
    let rect = picture_rect(ed, place)?;
    let side = HANDLE * place.scale;
    Some((picture_at_caret(ed)?, Rect::new(rect.max - Vec2::splat(side), rect.max)))
}
