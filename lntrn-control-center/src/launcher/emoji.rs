//! The launcher's emoji page: "Recently used" and the emoji groups down the
//! left, the emoji beside them as big tiles (text, in the bundled color
//! emoji font), and the name of the one lit along the bottom. A click types
//! it into the window last worked in and flashes it gold, the menu staying
//! up for more; a right-click copies it instead. A search looks through
//! every group, by name and keyword.

use lntrn_emoji::{Category, EMOJIS};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui};

use super::menu::{self, DIM, GROUP_H, NAME, Outcome};
use super::Launcher;
use crate::idlist::{self, IdList};

/// A tile, and the glyph in it, logical.
const CELL: f64 = 64.0;
const GLYPH: f64 = 40.0;
/// The glyph beside a group's name in the sidebar.
const ICON: f64 = 26.0;
/// The strip along the bottom naming the emoji lit.
const NAME_H: f64 = 44.0;
/// How long a typed or copied emoji flashes, s.
const FLASH: f64 = 0.35;
/// How many emoji "Recently used" keeps.
const RECENT: usize = 32;

/// What the page shows with no search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmojiView {
    Recent,
    Group(Category),
}

impl EmojiView {
    fn label(self) -> &'static str {
        match self {
            EmojiView::Recent => "Recently used",
            EmojiView::Group(c) => c.label(),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            EmojiView::Recent => "🕘",
            EmojiView::Group(c) => c.icon(),
        }
    }
}

pub struct EmojiPage {
    view: EmojiView,
    /// The emoji used lately, by codepoints, the latest first.
    recents: IdList,
    /// Typed since the page opened: they join the recent ones when it
    /// closes, so the tiles don't reshuffle under the pointer.
    typed: Vec<&'static str>,
    /// The emoji that flashes (by its place in [`EMOJIS`]), from when.
    flash: Option<(usize, f64)>,
    /// What the last search or view showed, kept: a search lowercases every
    /// name and keyword of some 1,400 emoji.
    shown: Option<(String, EmojiView, Vec<usize>)>,
}

impl EmojiPage {
    pub fn new() -> EmojiPage {
        EmojiPage { view: EmojiView::Group(Category::Smileys), recents: IdList::default(), typed: Vec::new(), flash: None, shown: None }
    }

    /// Read the recent emoji afresh from their file (the menu opening).
    pub fn load(&mut self) {
        self.recents = IdList::load(idlist::RECENT_EMOJI);
    }

    /// The page opens: on the recent emoji when there are some.
    pub fn open(&mut self) {
        self.view = if self.recents.ids().is_empty() { EmojiView::Group(Category::Smileys) } else { EmojiView::Recent };
        (self.flash, self.shown) = (None, None);
    }

    /// The emoji used lately, by codepoints, the latest first.
    #[cfg(test)]
    pub fn recent(&self) -> &[String] {
        self.recents.ids()
    }

    /// The page closes: what was typed joins the recent ones.
    pub fn settle(&mut self) {
        for unicode in self.typed.drain(..) {
            self.recents.bump(unicode, RECENT);
        }
    }

    /// The emoji that show, by their place in [`EMOJIS`]: what `query`
    /// finds, else the view's.
    pub fn shown(&mut self, query: &str) -> Vec<usize> {
        let query = query.trim();
        if let Some((q, view, shown)) = &self.shown
            && q == query
            && *view == self.view
        {
            return shown.clone();
        }
        let shown = match self.view {
            _ if !query.is_empty() => lntrn_emoji::visible(query, Category::Smileys),
            EmojiView::Recent => self.recents.ids().iter().filter_map(|u| EMOJIS.iter().position(|e| e.unicode == u)).collect(),
            EmojiView::Group(c) => lntrn_emoji::visible("", c),
        };
        self.shown = Some((query.to_owned(), self.view, shown.clone()));
        shown
    }

    /// Type the emoji at `index` in [`EMOJIS`]: remembered for "Recently
    /// used", flashed at `now`, and asked of the app.
    fn typed(&mut self, index: usize, now: f64) -> Outcome {
        let e = &EMOJIS[index];
        // The last typed goes last, so it settles first.
        self.typed.retain(|u| *u != e.unicode);
        self.typed.push(e.unicode);
        self.flash = Some((index, now));
        Outcome::Type(e.glyph.to_owned())
    }
}

/// How many tiles go across `width` (window px).
pub fn columns(width: f64, m: lntrn_ui::Metrics) -> usize {
    ((width / m.px(CELL)).floor() as usize).max(1)
}

/// "Recently used" (once there are some) and each group, its glyph beside
/// its name. Picking one ends a search.
pub fn sidebar(l: &mut Launcher, ui: &mut Ui, rect: Rect) {
    let m = ui.m;
    let recent = !l.emoji.recents.ids().is_empty() || !l.emoji.typed.is_empty();
    let entries: Vec<EmojiView> = recent.then_some(EmojiView::Recent).into_iter().chain(Category::ALL.map(EmojiView::Group)).collect();
    let row_h = m.px(GROUP_H).min(rect.height() / entries.len().max(1) as f64);
    let (style, icon) = (TextStyle::new(m.px(NAME) as f32), TextStyle::new(m.px(ICON) as f32));
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    for (i, entry) in entries.into_iter().enumerate() {
        let row = Rect::from_min_size(Vec2::new(rect.min.x, rect.min.y + i as f64 * row_h), Vec2::new(rect.width(), row_h));
        let r = ui.interact(ui.id(entry.label()), row, Sense::CLICK);
        let current = l.query.trim().is_empty() && l.emoji.view == entry;
        if r.clicked {
            l.emoji.view = entry;
            l.query.clear();
            l.restart();
            // The rows above were drawn with the view before.
            ui.state.request_rebuild = true;
        }
        if current {
            ui.draw.rounded_rect(row, m.px(10.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(row, m.px(10.0), Color::WHITE.fade(0.06));
        }
        let glyph = Rect::from_min_size(Vec2::new(row.min.x + m.px(8.0), row.min.y), Vec2::new(m.px(ICON + 12.0), row_h));
        ui.text_centered(entry.icon(), &icon, glyph, ink);
        let text = Rect::new(Vec2::new(glyph.max.x + m.px(8.0), row.min.y), row.max);
        ui.text_in_rect(entry.label(), &style, text, if current { accent } else { ink });
    }
}

/// What the emoji side says when nothing shows.
pub fn empty(l: &Launcher, ui: &mut Ui, rect: Rect) {
    let name = TextStyle::new(ui.m.px(NAME) as f32);
    let what = if l.query.trim().is_empty() { "No emoji used yet.".to_owned() } else { format!("No emoji matches “{}”.", l.query.trim()) };
    let dim = ui.theme.text_dim;
    ui.text_centered(&what, &name, rect, dim);
}

/// The emoji that show (`shown`, by their place in [`EMOJIS`]) as tiles
/// over the strip naming the one lit, scrolled by the wheel and to keep it
/// in view, the light following a pointer that `pointed`. A click types
/// one, a right-click copies it.
pub fn body(l: &mut Launcher, ui: &mut Ui, rect: Rect, shown: &[usize], pointed: bool) -> Option<Outcome> {
    let m = ui.m;
    let now = ui.now();
    let tiles = Rect::new(rect.min, Vec2::new(rect.max.x, rect.max.y - m.px(NAME_H)));
    let width = tiles.width() - m.px(10.0);
    let cols = columns(width, m);
    let cell = Vec2::new(width / cols as f64, m.px(CELL));
    let rows = shown.len().div_ceil(cols);
    let content = rows as f64 * cell.y;
    l.selected = l.selected.map(|s| s.min(shown.len() - 1));
    let selected_row = l.selected.map(|s| ((s / cols) as f64 * cell.y, cell.y));
    let max = menu::scroll(l, ui, tiles, content, selected_row);

    let glyph = TextStyle::new(m.px(GLYPH) as f32);
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    let flash = l.emoji.flash.filter(|(_, at)| now - at < FLASH);
    if let Some((_, at)) = flash {
        ui.state.request_redraw_after((FLASH - (now - at)).max(0.0));
    }
    let first = (l.scroll / cell.y).floor() as usize;
    let last = (((l.scroll + tiles.height()) / cell.y).ceil() as usize).min(rows);
    let (mut out, mut hovered) = (None, None);
    ui.draw.push_clip(tiles);
    for row in first..last {
        for col in 0..cols {
            let pos = row * cols + col;
            let Some(&index) = shown.get(pos) else { break };
            let at = Vec2::new(tiles.min.x + col as f64 * cell.x, tiles.min.y + row as f64 * cell.y - l.scroll);
            let tile = Rect::from_min_size(at, cell);
            let r = ui.interact(ui.id("glyph").with_index(index), tile.intersection(&tiles), Sense::CLICK);
            if r.hovered {
                hovered = Some(pos);
            }
            if r.clicked {
                out = Some(l.emoji.typed(index, now));
            }
            if r.hovered && ui.state.right_pressed {
                // Copied, not typed: on the clipboard to paste anywhere.
                ui.state.set_clipboard(EMOJIS[index].glyph);
                l.emoji.flash = Some((index, now));
                (l.selected, l.by_pointer) = (Some(pos), true);
            }
            let lit = tile.shrink(m.px(3.0));
            if Some(pos) == l.selected {
                ui.draw.rounded_rect(lit, m.px(12.0), accent.fade(0.20));
            } else if r.hovered {
                ui.draw.rounded_rect(lit, m.px(12.0), Color::WHITE.fade(0.06));
            }
            if let Some((_, at)) = flash.filter(|(i, _)| *i == index) {
                ui.draw.rounded_rect(lit, m.px(12.0), accent.fade(0.55 * (1.0 - (now - at) / FLASH)));
            }
            ui.text_centered(EMOJIS[index].glyph, &glyph, tile, ink);
        }
    }
    ui.draw.pop_clip();
    menu::scrollbar(ui, tiles, l.scroll, max, content);
    menu::follow(l, ui, pointed, hovered);
    strip(l, ui, Rect::new(Vec2::new(rect.min.x, tiles.max.y), rect.max), shown);
    out
}

/// Along the bottom: the lit emoji and its name, or what a click does.
fn strip(l: &Launcher, ui: &mut Ui, rect: Rect, shown: &[usize]) {
    let m = ui.m;
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let band = rect.shrink(m.px(4.0));
    match l.selected.and_then(|s| shown.get(s)) {
        Some(&index) => {
            let e = &EMOJIS[index];
            let glyph = TextStyle::new(m.px(ICON) as f32);
            let pic = Rect::from_min_size(band.min, Vec2::new(m.px(ICON + 12.0), band.height()));
            ui.text_centered(e.glyph, &glyph, pic, ink);
            let name = TextStyle::new(m.px(NAME) as f32);
            ui.text_in_rect(e.name, &name, Rect::new(Vec2::new(pic.max.x + m.px(8.0), band.min.y), band.max), ink);
        }
        None => {
            let small = TextStyle::new(m.px(DIM) as f32);
            ui.text_in_rect("Click to type it · right-click to copy it", &small, band, dim);
        }
    }
}

/// Enter on the emoji lit: typed.
pub fn enter(l: &mut Launcher, shown: &[usize], now: f64) -> Option<Outcome> {
    let index = *l.selected.and_then(|s| shown.get(s))?;
    Some(l.emoji.typed(index, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_reaches_every_group_and_a_view_shows_its_own() {
        let mut page = EmojiPage::new();
        let grinning = lntrn_emoji::position("😀").unwrap();
        assert_eq!(page.shown("").first(), Some(&grinning), "Smileys from the start");
        page.view = EmojiView::Group(Category::Flags);
        assert!(page.shown("").iter().all(|&i| EMOJIS[i].category == Category::Flags));
        assert!(page.shown("grinning").contains(&grinning), "a search isn't held to the view");
    }

    #[test]
    fn typed_emoji_join_the_recent_ones_once_and_latest_first_on_settling() {
        let mut page = EmojiPage::new();
        let (grin, cat) = (lntrn_emoji::position("😀").unwrap(), lntrn_emoji::position("🐱").unwrap());
        assert_eq!(page.typed(grin, 0.0), Outcome::Type("😀".into()));
        page.typed(cat, 0.1);
        page.typed(grin, 0.2);
        assert!(page.recents.ids().is_empty(), "nothing reshuffles while the page is up");
        page.settle();
        assert_eq!(page.recents.ids(), [EMOJIS[grin].unicode, EMOJIS[cat].unicode]);
        page.view = EmojiView::Recent;
        assert_eq!(page.shown(""), [grin, cat]);
    }
}
