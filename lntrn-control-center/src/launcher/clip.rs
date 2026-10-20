//! The launcher's clipboard page: what was copied, as the compositor's
//! clipboard history keeps it (the pinned first, then the newest first),
//! "All" or "Pinned" down the left with Clear all under them. A click puts
//! an entry back on the clipboard and closes the menu, ready to paste; a
//! right-click pins or deletes it. A search looks through the text.

use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui};

use super::menu::{self, DIM, GROUP_H, NAME, Outcome};
use super::{Launcher, grid};
use crate::clipboard::{self, ClipEntry, ClipHistory, ClipOp};
use crate::thumbs::Thumbs;

/// A row, the picture in one, and the Clear all button, logical.
const ROW_H: f64 = 68.0;
const THUMB: f64 = 52.0;
const BUTTON_H: f64 = 52.0;
/// The pixels a thumbnail is read at: sharp at 1.5x and more.
const THUMB_PX: u32 = 96;

/// Which entries show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipView {
    All,
    Pinned,
}

impl ClipView {
    fn label(self) -> &'static str {
        match self {
            ClipView::All => "All",
            ClipView::Pinned => "Pinned",
        }
    }
}

pub struct ClipPage {
    pub history: ClipHistory,
    pub thumbs: Thumbs,
    view: ClipView,
    /// Clear all asked; waiting for its confirmation.
    confirm: bool,
}

impl ClipPage {
    pub fn new() -> ClipPage {
        ClipPage { history: ClipHistory::default(), thumbs: Thumbs::new(THUMB_PX), view: ClipView::All, confirm: false }
    }

    /// The page opens: the entries read afresh, every one showing.
    pub fn open(&mut self) {
        (self.view, self.confirm) = (ClipView::All, false);
        self.history.refresh();
    }

    /// Put away a Clear all question. Whether there was one.
    pub fn unask(&mut self) -> bool {
        std::mem::take(&mut self.confirm)
    }

    /// The entries that show: the view's, holding `query` when there is
    /// one (a picture has no text to hold it). `None` until first read.
    pub fn shown(&self, query: &str) -> Option<Vec<ClipEntry>> {
        let needle = query.trim().to_lowercase();
        let entries = self.history.entries()?;
        Some(entries.into_iter().filter(|e| (self.view == ClipView::All || e.pinned) && (needle.is_empty() || e.text.to_lowercase().contains(&needle))).collect())
    }
}

/// "All" and "Pinned", and Clear all at the bottom (with its question
/// while it waits).
pub fn sidebar(l: &mut Launcher, ui: &mut Ui, rect: Rect) -> Option<Outcome> {
    let m = ui.m;
    let style = TextStyle::new(m.px(NAME) as f32);
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    for (i, view) in [ClipView::All, ClipView::Pinned].into_iter().enumerate() {
        let row = Rect::from_min_size(Vec2::new(rect.min.x, rect.min.y + i as f64 * m.px(GROUP_H)), Vec2::new(rect.width(), m.px(GROUP_H)));
        let r = ui.interact(ui.id(view.label()), row, Sense::CLICK);
        let current = l.clip.view == view;
        if r.clicked {
            l.clip.view = view;
            l.restart();
            ui.state.request_rebuild = true;
        }
        if current {
            ui.draw.rounded_rect(row, m.px(10.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(row, m.px(10.0), Color::WHITE.fade(0.06));
        }
        ui.text_in_rect(view.label(), &style, Rect::new(Vec2::new(row.min.x + m.px(14.0), row.min.y), row.max), if current { accent } else { ink });
    }
    let any = l.clip.history.entries().is_some_and(|e| !e.is_empty());
    if !any {
        l.clip.confirm = false;
        return None;
    }
    let danger = ui.theme.close;
    let h = m.px(BUTTON_H);
    if !l.clip.confirm {
        let button = Rect::new(Vec2::new(rect.min.x, rect.max.y - h), rect.max);
        if self::button(ui, "clear", "Clear all", button, danger, danger.fade(0.14)) {
            l.clip.confirm = true;
            ui.state.request_rebuild = true;
        }
        return None;
    }
    // The question: what goes, then yes or no.
    let small = TextStyle::new(m.px(DIM) as f32);
    let ask = Rect::new(Vec2::new(rect.min.x, rect.max.y - h * 2.0 - m.px(8.0) - f64::from(small.line_height()) * 2.0), Vec2::new(rect.max.x, rect.max.y - h * 2.0 - m.px(8.0)));
    let dim = ui.theme.text_dim;
    for (k, line) in ["Clear everything,", "pinned ones too?"].into_iter().enumerate() {
        let band = Rect::from_min_size(Vec2::new(ask.min.x + m.px(4.0), ask.min.y + k as f64 * f64::from(small.line_height())), Vec2::new(ask.width(), f64::from(small.line_height())));
        ui.text_in_rect(line, &small, band, dim);
    }
    let yes = Rect::new(Vec2::new(rect.min.x, rect.max.y - h * 2.0 - m.px(4.0)), Vec2::new(rect.max.x, rect.max.y - h - m.px(4.0)));
    let no = Rect::new(Vec2::new(rect.min.x, rect.max.y - h), rect.max);
    let (plain, cancel) = (ui.theme.text, Color::WHITE.fade(0.08));
    if button(ui, "clear-yes", "Clear", yes, Color::WHITE, danger) {
        l.clip.confirm = false;
        return Some(Outcome::Clip(ClipOp::Clear));
    }
    if button(ui, "clear-no", "Cancel", no, plain, cancel) {
        l.clip.confirm = false;
        ui.state.request_rebuild = true;
    }
    None
}

/// A button: `label` in `ink` on `fill`, washed lighter while hovered.
/// Whether it was clicked.
fn button(ui: &mut Ui, id: &str, label: &str, rect: Rect, ink: Color, fill: Color) -> bool {
    let m = ui.m;
    let r = ui.interact(ui.id(id), rect, Sense::CLICK);
    ui.draw.rounded_rect(rect, m.px(10.0), fill);
    if r.hovered {
        ui.draw.rounded_rect(rect, m.px(10.0), Color::WHITE.fade(0.10));
    }
    ui.text_centered(label, &TextStyle::new(m.px(NAME) as f32).bold(), rect, ink);
    r.clicked
}

/// What the clipboard side says when nothing shows: still reading, the
/// history not answering, nothing copied yet, nothing pinned, or nothing
/// holding the search.
pub fn empty(l: &Launcher, ui: &mut Ui, rect: Rect) {
    let name = TextStyle::new(ui.m.px(NAME) as f32);
    let what = match (l.clip.history.entries(), l.clip.history.failed()) {
        (None, Some(_)) => "The clipboard history isn't answering.".to_owned(),
        (None, None) => "Reading the clipboard…".to_owned(),
        _ if !l.query.trim().is_empty() => format!("Nothing copied matches “{}”.", l.query.trim()),
        _ if l.clip.view == ClipView::Pinned => "Nothing pinned yet. Right-click an entry to pin it.".to_owned(),
        _ => "Nothing copied yet.".to_owned(),
    };
    let dim = ui.theme.text_dim;
    ui.text_centered(&what, &name, rect, dim);
}

/// The entries that show (`shown`), a row each, scrolled by the wheel and
/// to keep the lit one in view, the light following a pointer that
/// `pointed`. A click copies one back, a right-click asks for its menu.
pub fn body(l: &mut Launcher, ui: &mut Ui, rect: Rect, shown: &[ClipEntry], pointed: bool) -> Option<Outcome> {
    let m = ui.m;
    let row_h = m.px(ROW_H);
    let content = shown.len() as f64 * row_h;
    l.selected = l.selected.map(|s| s.min(shown.len() - 1));
    let max = menu::scroll(l, ui, rect, content, l.selected.map(|s| (s as f64 * row_h, row_h)));
    let wanted: Vec<&std::path::Path> = shown.iter().filter_map(|e| e.image.as_deref()).collect();
    l.clip.thumbs.keep(&wanted);

    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    let (name, small) = (TextStyle::new(m.px(NAME) as f32), TextStyle::new(m.px(DIM) as f32));
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let first = (l.scroll / row_h).floor() as usize;
    let last = (((l.scroll + rect.height()) / row_h).ceil() as usize).min(shown.len());
    let (mut out, mut hovered) = (None, None);
    ui.draw.push_clip(rect);
    for (pos, e) in shown.iter().enumerate().take(last).skip(first) {
        let row = Rect::from_min_size(Vec2::new(rect.min.x, rect.min.y + pos as f64 * row_h - l.scroll), Vec2::new(rect.width() - m.px(10.0), row_h));
        let r = ui.interact(ui.id("clip").with_index(e.id as usize), row.intersection(&rect), Sense::CLICK);
        if r.hovered {
            hovered = Some(pos);
        }
        if r.clicked {
            out = Some(Outcome::Clip(ClipOp::Copy(e.id)));
        }
        if r.hovered && ui.state.right_pressed {
            (l.selected, l.by_pointer) = (Some(pos), true);
            out = Some(Outcome::ClipMenu(e.id));
        }
        if Some(pos) == l.selected {
            ui.draw.rounded_rect(row, m.px(10.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(row, m.px(10.0), Color::WHITE.fade(0.06));
        }
        let mut text_left = row.min.x + m.px(14.0);
        if let Some(path) = &e.image {
            let well = Rect::from_min_size(Vec2::new(row.min.x + m.px(10.0), (row.center().y - m.px(THUMB) * 0.5).round()), Vec2::splat(m.px(THUMB)));
            match l.clip.thumbs.get(path) {
                Some(handle) => {
                    // The picture's shape kept, centred in its well.
                    let (w, h) = (f64::from(handle.width), f64::from(handle.height));
                    let k = (well.width() / w).min(well.height() / h);
                    ui.draw.image(Rect::from_center_size(well.center(), Vec2::new(w * k, h * k)), handle, m.px(6.0), Color::WHITE);
                }
                None => ui.draw.rounded_rect(well, m.px(8.0), ink.fade(0.10)),
            }
            text_left = well.max.x + m.px(14.0);
        }
        let lines_h = f64::from(name.line_height()) + f64::from(small.line_height());
        let top = (row.center().y - lines_h * 0.5).round();
        let width = row.max.x - m.px(10.0) - text_left;
        let what = if e.image.is_some() { "Picture".to_owned() } else { one_line(&e.text) };
        let line = grid::wrap(&mut |s| ui.measure(s, &name), &what, width - m.px(4.0), 1).pop().unwrap_or_default();
        ui.text_in_rect(&line, &name, Rect::from_min_size(Vec2::new(text_left, top), Vec2::new(width, f64::from(name.line_height()))), ink);
        let mut under = clipboard::age(e.at_ms, now_ms);
        if e.pinned {
            under.push_str(" · pinned");
        }
        let band = Rect::from_min_size(Vec2::new(text_left, top + f64::from(name.line_height())), Vec2::new(width, f64::from(small.line_height())));
        ui.text_in_rect(&under, &small, band, if e.pinned { accent } else { dim });
    }
    ui.draw.pop_clip();
    menu::scrollbar(ui, rect, l.scroll, max, content);
    menu::follow(l, ui, pointed, hovered);
    out
}

/// Text as one line: every run of whitespace (new lines, tabs) one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Enter on the entry lit: copied back.
pub fn enter(l: &Launcher, shown: &[ClipEntry]) -> Option<Outcome> {
    let e = l.selected.and_then(|s| shown.get(s))?;
    Some(Outcome::Clip(ClipOp::Copy(e.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, text: &str, pinned: bool) -> ClipEntry {
        ClipEntry { id, text: text.into(), image: None, at_ms: 0, pinned }
    }

    #[test]
    fn the_view_and_the_search_pick_what_shows() {
        let mut page = ClipPage::new();
        assert!(page.shown("").is_none(), "nothing read yet");
        page.history = ClipHistory::holding(vec![entry(2, "Kept Text", true), entry(9, "hello world", false)]);
        let ids = |shown: Option<Vec<ClipEntry>>| shown.unwrap().iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(page.shown("")), [2, 9]);
        assert_eq!(ids(page.shown("  KEPT ")), [2], "any case, trimmed");
        page.view = ClipView::Pinned;
        assert_eq!(ids(page.shown("")), [2]);
    }

    #[test]
    fn text_reads_as_one_line() {
        assert_eq!(one_line("fn main() {\n\tprintln!();\n}"), "fn main() { println!(); }");
    }
}
