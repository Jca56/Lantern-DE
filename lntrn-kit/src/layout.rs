//! The page around the cards: a centred, capped, scrolling column, the
//! captions over cards and the notes under them, and the two text sizes
//! beside the body's.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

use crate::look;

/// The widest a page's column gets, and the least room either side of it.
const COLUMN_W: f64 = 980.0;
const MARGIN: f64 = 20.0;
const TITLE_SIZE: f64 = 40.0;
/// Hints, captions and notes. Nothing the kit draws is smaller.
const SMALL_SIZE: f64 = 20.0;

pub fn title_style(ui: &Ui) -> TextStyle {
    TextStyle::new(ui.m.px(TITLE_SIZE) as f32).bold()
}

pub fn small_style(ui: &Ui) -> TextStyle {
    TextStyle::new(ui.m.px(SMALL_SIZE) as f32)
}

/// Monospace at the small size: code, hashes, a diff.
pub fn mono_style(ui: &Ui) -> TextStyle {
    TextStyle::new(ui.m.px(SMALL_SIZE) as f32).mono()
}

/// Which end of a text too long for its room is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// The start: a name, a sentence.
    Start,
    /// The end: a path, whose last folders say the most.
    End,
}

/// `s`, cut to fit in `max_w` with an ellipsis where it was cut. Whole
/// when it fits.
pub fn elide(ui: &mut Ui, s: &str, style: &TextStyle, max_w: f64, keep: Keep) -> String {
    if ui.measure(s, style) <= max_w {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let cut = |n: usize| -> String {
        match keep {
            Keep::Start => chars[..n].iter().collect::<String>() + "…",
            Keep::End => "…".to_owned() + &chars[chars.len() - n..].iter().collect::<String>(),
        }
    };
    // The most characters that still fit, by halving.
    let (mut fits, mut over) = (0, chars.len());
    while over - fits > 1 {
        let mid = (fits + over) / 2;
        if ui.measure(&cut(mid), style) <= max_w {
            fits = mid;
        } else {
            over = mid;
        }
    }
    cut(fits)
}

/// One line of text that takes its own height in the layout.
pub fn text_line(ui: &mut Ui, s: &str, style: &TextStyle, color: Color) -> Rect {
    let r = ui.alloc(Vec2::new(FILL, style.line_height() as f64));
    ui.text_in_rect(s, style, r, color);
    r
}

/// A page: its title and a line about it, then whatever `f` declares, in
/// a centred column no wider than [`COLUMN_W`], scrolling.
pub fn page(ui: &mut Ui, title: &str, blurb: &str, f: impl FnOnce(&mut Ui)) {
    let mut f = Some(f);
    ui.scroll_area("page", None, |ui| {
        let (m, avail) = (ui.m, ui.avail_width());
        let col = (avail - m.px(MARGIN) * 2.0).min(m.px(COLUMN_W)).max(0.0);
        let left = ((avail - col) * 0.5).floor();
        // `columns` puts a gap between the two; the first is only the margin.
        ui.columns(&[(left - m.gap).max(0.0), col], |ui, i| {
            if i == 0 {
                return;
            }
            let Some(f) = f.take() else { return };
            ui.space(m.px(15.0));
            let (big, small) = (title_style(ui), small_style(ui));
            text_line(ui, title, &big, look::TEXT);
            text_line(ui, blurb, &small, look::TEXT_DIM);
            ui.space(m.px(10.0));
            f(ui);
            ui.space(m.px(40.0));
        });
    });
}

/// A small heading over the card that follows.
pub fn caption(ui: &mut Ui, s: &str) {
    let m = ui.m;
    ui.space(m.px(15.0));
    let style = small_style(ui).bold();
    let r = ui.alloc(Vec2::new(FILL, m.px(30.0)));
    let inset = Rect::new(Vec2::new(r.min.x + m.px(5.0), r.min.y), r.max);
    ui.text_in_rect(&s.to_uppercase(), &style, inset, look::TEXT_DIM);
}

/// [`caption`] with a small text button at its right end: the heading of
/// a card that can be acted on as a whole ("Stage all"). An empty
/// `action` is no button. `true` when it was pressed.
pub fn caption_action(ui: &mut Ui, s: &str, action: &str) -> bool {
    let m = ui.m;
    ui.space(m.px(15.0));
    let style = small_style(ui).bold();
    let r = ui.alloc(Vec2::new(FILL, m.px(36.0)));
    let inset = Rect::new(Vec2::new(r.min.x + m.px(5.0), r.min.y), r.max);
    ui.text_in_rect(&s.to_uppercase(), &style, inset, look::TEXT_DIM);
    if action.is_empty() {
        return false;
    }
    let w = ui.measure(action, &style) + m.px(24.0);
    let rect = Rect::new(Vec2::new(r.max.x - w, r.min.y), r.max);
    let id = ui.id(s).with("action");
    let mut resp = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    ui.key_click(id, &mut resp);
    if resp.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
        ui.draw.rounded_rect(rect, m.px(8.0), Color::WHITE.fade(0.08));
    }
    ui.text_centered(action, &style, rect, ui.theme.accent);
    ui.focus_ring(id, rect);
    resp.clicked
}

/// Dim explanatory text under a card, wrapped to the column.
pub fn note(ui: &mut Ui, s: &str) {
    let style = small_style(ui);
    let inset = ui.m.px(5.0);
    let w = (ui.avail_width() - inset * 2.0).max(1.0);
    let h = ui.text.measure_wrapped(s, &style, w as f32).height as f64;
    let r = ui.alloc(Vec2::new(FILL, h));
    ui.text_at(s, &style, Vec2::new(r.min.x + inset, r.min.y), w, look::TEXT_DIM);
}

/// `0..=1` as a percentage.
pub fn percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round())
}

/// A whole number that already is a percentage.
pub fn percent_of_100(v: f64) -> String {
    format!("{}%", v.round())
}

pub fn pixels(v: f64) -> String {
    format!("{} px", v.round())
}

/// A multiplier: `1.25×`.
pub fn times(v: f64) -> String {
    format!("{v:.2}×")
}
