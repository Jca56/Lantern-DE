//! An app's card, over the screen above its icon: nothing but a live
//! thumbnail of each of its windows, oldest first, each with a red X in
//! its top right corner. The compositor paints the windows, and the X's
//! over them (`thumbs:` lines say where): the card only holds the place,
//! lights the one under the pointer, and hit-tests them. A click on a
//! thumbnail focuses its window, wherever it is; on its X, closes it. The
//! thumbnails take the screen's work area's shape, so a maximized window
//! fills its own exactly. An app with no window shows its name alone.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui};

use super::Dock;
use crate::bar::Look;

/// Logical sizes.
const PAD: f64 = 10.0;
const GAP: f64 = 10.0;
const RADIUS: f64 = 14.0;
const THUMB_W: f64 = 400.0;
const MIN_THUMB_W: f64 = 200.0;
/// The X, and how far in from the thumbnail's corner.
const CLOSE: f64 = 30.0;
const CLOSE_INSET: f64 = 6.0;
/// How far the light round a thumbnail under the pointer reaches past it.
const RING: f64 = 4.0;
/// An idle app's name.
const NAME: f64 = 20.0;

/// What a press on the card asks of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardPress {
    Activate(u64),
    Close(u64),
}

/// A window's thumbnail on the card, window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    pub id: u64,
    pub rect: Rect,
    pub close: Rect,
    /// The pointer is on its X.
    pub close_hovered: bool,
}

/// A frame of the card.
#[derive(Debug, Default)]
pub struct Card {
    pub press: Option<CardPress>,
    pub thumbs: Vec<Thumb>,
    /// The card itself: where the overlay takes the pointer. `None` when
    /// there is nothing to show (the app or its icon gone).
    pub rect: Option<Rect>,
}

/// Draw the card of the app with this key, its bottom at `bottom` and
/// centred over its icon's face (the overlay and the bar share their
/// monitor's left edge, so the face's x is the card's too); `aspect` is
/// the work area's width over its height.
pub fn draw(dock: &mut Dock, ui: &mut Ui, key: &str, bottom: f64, aspect: f64, look: Look) -> Card {
    let (Some(entry), Some(face)) = (dock.entry(key).cloned(), dock.face(key)) else {
        dock.on_card = false;
        return Card::default();
    };
    let m = ui.m;
    let surface = ui.clip();
    let (pad, gap) = (m.px(PAD), m.px(GAP));
    let name_style = TextStyle::new(m.px(NAME) as f32).bold();

    // As wide as its thumbnails, narrower ones when that is too wide for
    // the screen; or its name, with none.
    let k = entry.windows.len();
    let room = surface.width() - m.px(16.0) - pad * 2.0;
    let thumb_w = if k == 0 { 0.0 } else { m.px(THUMB_W).min((room - (k - 1) as f64 * gap) / k as f64).max(m.px(MIN_THUMB_W)).floor() };
    let thumb_h = (thumb_w / aspect.max(0.5)).round();
    let (inner_w, inner_h) = match k {
        0 => (ui.measure(entry.name(), &name_style).min(room), f64::from(name_style.line_height())),
        _ => (k as f64 * thumb_w + (k - 1) as f64 * gap, thumb_h),
    };
    let (w, h) = (inner_w + pad * 2.0, inner_h + pad * 2.0);
    let left = (face.center().x - w * 0.5).clamp(surface.min.x + m.px(8.0), surface.max.x - m.px(8.0) - w).round();
    let card = Rect::new(Vec2::new(left, (bottom - h).round()), Vec2::new(left + w, bottom.round()));
    dock.on_card = ui.state.pointer_in_window && card.contains(ui.state.pointer);

    if look.glass {
        crate::glass::pane(card, m.px(RADIUS));
    } else {
        ui.draw.shadow(card, m.px(RADIUS), m.px(20.0), Color::BLACK.fade(0.45));
    }
    ui.draw.rounded_rect(card, m.px(RADIUS), look.plate);
    ui.draw.stroke_rect(card, m.px(1.0), m.px(RADIUS), look.edge);
    if k == 0 {
        let name = fit(ui, entry.name(), &name_style, inner_w);
        ui.text_in_rect(&name, &name_style, card.shrink(pad), ui.theme.text);
        return Card { rect: Some(card), ..Card::default() };
    }

    let mut press = None;
    let mut thumbs = Vec::new();
    for (i, window) in entry.windows.iter().enumerate() {
        let rect = Rect::from_min_size(Vec2::new(card.min.x + pad + i as f64 * (thumb_w + gap), card.min.y + pad), Vec2::new(thumb_w, thumb_h));
        let inset = m.px(CLOSE_INSET);
        let close = Rect::from_min_size(Vec2::new(rect.max.x - inset - m.px(CLOSE), rect.min.y + inset), Vec2::splat(m.px(CLOSE)));
        // The X first, so a press on it is its own and not the thumbnail's.
        let x = ui.interact(ui.id("close").with_u64(window.id), close, Sense::CLICK);
        let r = ui.interact(ui.id("thumb").with_u64(window.id), rect, Sense::CLICK);
        if x.clicked {
            press = Some(CardPress::Close(window.id));
        } else if r.clicked {
            press = Some(CardPress::Activate(window.id));
        }
        // A light round the one under the pointer, a fainter one round the
        // window with the focus: the thumbnail covers all but its rim.
        if r.hovered || x.hovered || window.active {
            let color = if r.hovered || x.hovered { ui.theme.accent.fade(if r.held { 0.9 } else { 0.7 }) } else { Color::WHITE.fade(0.18) };
            ui.draw.rounded_rect(rect.expand(m.px(RING)), m.px(8.0), color);
        }
        thumbs.push(Thumb { id: window.id, rect, close, close_hovered: x.hovered });
    }
    Card { press, thumbs, rect: Some(card) }
}

/// `s` cut to fit `width` in `style`, with an ellipsis where it was cut.
fn fit(ui: &mut Ui, s: &str, style: &TextStyle, width: f64) -> String {
    if ui.measure(s, style) <= width {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cut: String = chars[..mid].iter().collect::<String>() + "…";
        if ui.measure(&cut, style) <= width { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_owned() + "…"
}
