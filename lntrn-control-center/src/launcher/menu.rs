//! The menu's panel: anchored above the bar's left end on a surface that
//! covers the monitor, the search field across its top with the emoji and
//! clipboard buttons and the list and grid buttons at its end, the page
//! below (the favorites, all apps and the groups down its left and the
//! apps beside them as a list or a grid (`grid`); or the emoji page; or
//! the clipboard's), the power row along its bottom with Quick Kill at its
//! end. The keyboard stays in the search field, so each page keeps its own
//! selection (arrows, Enter) and its own scroll, to keep that selection in
//! view. A right-click on an app asks the app for its menu, which the
//! shell draws over the panel.

use lntrn_apps::{App, Icons};
use lntrn_math::{Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{FILL, Key, Metrics, Sense, Ui};

use super::{Launcher, Layout, Page, View, clip, emoji, grid, power, toggle};
use crate::bar::Look;
use crate::clipboard::ClipOp;

/// The panel's size, logical; less on a small screen.
const PANEL_W: f64 = 780.0;
const PANEL_H: f64 = 760.0;
/// Inside the panel's edge.
const PAD: f64 = 18.0;
/// Between the panel and the bar.
const ABOVE_BAR: f64 = 10.0;
const RADIUS: f64 = 16.0;
const SIDEBAR_W: f64 = 230.0;
pub(super) const GROUP_H: f64 = 48.0;
const ROW_H: f64 = 56.0;
const APP_ICON: f64 = 40.0;
const POWER_H: f64 = 56.0;
/// Text, logical: names at 20, what is beside them at the 18 floor.
pub(super) const NAME: f64 = 20.0;
pub(super) const DIM: f64 = 18.0;
/// The search field's well and focus ring, logical: the bar's theme has
/// no lines at all, which left the ring as four arcs at its corners.
const FIELD_BORDER: f64 = 1.5;

/// What the menu asks of the app after a frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Stay,
    Close,
    /// Start the app with this command, and close.
    Launch(String),
    /// Show what can be done with the app at this index (a right-click).
    Menu(usize),
    Power(power::Action),
    /// Start the bar over, the newest build of it.
    QuickKill,
    /// The buttons by the search field switched the apps to this layout
    /// (shown already): keep it.
    Layout(Layout),
    /// Type this into the window last worked in (an emoji); the menu
    /// stays up.
    Type(String),
    /// Do this to the clipboard's history (a copy back closes the menu).
    Clip(ClipOp),
    /// Show what can be done with this clipboard entry (a right-click).
    ClipMenu(u64),
}

/// What a frame's pointer or keys did to the apps.
#[derive(Clone, Copy)]
pub(super) enum Hit {
    Launch(usize),
    Menu(usize),
}

/// Where the bar is, logical px: the panel's left inset from the
/// monitor's edge, and how much of the monitor's bottom the bar keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub left: f64,
    pub clear: f64,
}

pub fn draw(l: &mut Launcher, icons: &mut Icons, ui: &mut Ui, place: Place, look: Look) -> Outcome {
    let m = ui.m;
    let surface = ui.clip();
    let w = m.px(PANEL_W).min(surface.width() - m.px(16.0));
    let bottom = surface.max.y - m.px(place.clear + ABOVE_BAR);
    let h = m.px(PANEL_H).min(bottom - surface.min.y - m.px(8.0));
    let left = surface.min.x + m.px(place.left);
    let panel = Rect::new(Vec2::new(left, bottom - h), Vec2::new(left + w, bottom));

    // A click anywhere off the panel closes it, over the bar too; one on
    // an app's menu (which may hang past the panel's edge) is the menu's.
    let on_popup = ui.state.popup.is_some_and(|(r, _)| r.contains(ui.state.pointer));
    if ui.state.pressed && !panel.contains(ui.state.pointer) && !on_popup {
        return Outcome::Close;
    }
    let keys = keys(l, ui, l.page == Page::Emoji || (l.page == Page::Apps && l.options.layout == Layout::Grid));
    if keys.close {
        return Outcome::Close;
    }
    // The pointer pointing anew: moved since the frame before (where it
    // first turns up is no move, so a menu opening under a resting pointer
    // lights nothing), or wheeling what is under it. Not while an app's
    // menu is up over the panel.
    let at = ui.state.pointer_in_window.then_some(ui.state.pointer);
    let pointed = ui.state.popup.is_none() && at.is_some() && (l.pointer.is_some_and(|p| Some(p) != at) || ui.state.wheel != Vec2::ZERO);
    l.pointer = at;

    if look.glass {
        crate::glass::pane(panel, m.px(RADIUS));
    } else {
        ui.draw.shadow(panel, m.px(RADIUS), m.px(24.0), lntrn_math::Color::BLACK.fade(0.45));
    }
    ui.draw.rounded_rect(panel, m.px(RADIUS), look.plate);
    ui.draw.stroke_rect(panel, m.px(1.0), m.px(RADIUS), look.edge);

    let inner = panel.shrink(m.px(PAD));
    let (id, layer) = (ui.id("panel"), ui.layer());
    let lined = Metrics { border: (FIELD_BORDER * m.scale).round().max(1.0), ..m };
    let mut child = Ui::new(&mut *ui.draw, &mut *ui.text, ui.theme, lined, &mut *ui.state, inner, inner, id, layer);
    child.set_window_rect(surface);
    // The field keeps the keyboard whatever was clicked (a group, say), so
    // typing always searches the page up. The emoji and clipboard buttons
    // at its end, then the list and grid ones, each as wide as the field is
    // tall.
    let mut switched = None;
    let button_w = child.m.widget_h;
    child.columns(&[FILL, button_w * 2.0, button_w * 2.0], |ui, col| {
        if col == 0 {
            let field = ui.id("search");
            ui.state.focus = Some(field);
            let hint = match l.page {
                Page::Apps => "Search apps…",
                Page::Emoji => "Search emoji…",
                Page::Clipboard => "Search the clipboard…",
            };
            if ui.text_field_hint("search", &mut l.query, hint).changed {
                l.restart();
            }
            return;
        }
        let rect = ui.alloc(Vec2::new(FILL, ui.m.widget_h));
        if col == 1 {
            if let Some(page) = toggle::pages(ui, rect, l.page) {
                // Over at once below; the field was drawn with the page before.
                l.go(page);
                ui.state.request_rebuild = true;
            }
            return;
        }
        if let Some(layout) = toggle::draw(ui, rect, l.options.layout, l.page == Page::Apps) {
            // Over at once: the apps below draw in it this very frame,
            // from another page too.
            if l.page != Page::Apps {
                l.go(Page::Apps);
                ui.state.request_rebuild = true;
            }
            if layout != l.options.layout {
                l.options.layout = layout;
                (l.scroll, l.reveal) = (0.0, true);
                switched = Some(layout);
            }
        }
    });
    child.space(m.px(12.0));
    let body_h = (child.remaining_height() - m.px(POWER_H) - m.px(12.0)).max(0.0);
    let mut out = None;
    child.columns(&[m.px(SIDEBAR_W), FILL], |ui, col| {
        let rect = ui.alloc(Vec2::new(FILL, body_h));
        let got = match (l.page, col) {
            (Page::Apps, 0) => {
                sidebar(l, ui, rect);
                None
            }
            (Page::Emoji, 0) => {
                emoji::sidebar(l, ui, rect);
                None
            }
            (Page::Clipboard, 0) => clip::sidebar(l, ui, rect),
            (Page::Apps, _) => apps(l, icons, ui, rect, &keys, pointed),
            (Page::Emoji, _) => emoji_side(l, ui, rect, &keys, pointed),
            (Page::Clipboard, _) => clip_side(l, ui, rect, &keys, pointed),
        };
        out = out.take().or(got);
    });
    child.space(m.px(12.0));
    let row = child.alloc(Vec2::new(FILL, m.px(POWER_H)));
    // A power question takes the whole row.
    let (kill, row) = if l.confirm.is_none() { power::quick_kill(&mut child, row) } else { (false, row) };
    let power = power::draw(&mut child, icons, row, &mut l.confirm);
    child.finish();

    if let Some(layout) = switched {
        return Outcome::Layout(layout);
    }
    if let Some(out) = out {
        return out;
    }
    match power {
        Some(action) => Outcome::Power(action),
        None if kill => Outcome::QuickKill,
        None => Outcome::Stay,
    }
}

/// The arrows move the light among `len` things `cols` across (one: a
/// list). With nothing lit, the first press lights the first.
fn step(l: &mut Launcher, keys: &Keys, len: usize, cols: usize) {
    if keys.rows == 0 && keys.across == 0 {
        return;
    }
    let (from, rows, across) = match l.selected {
        Some(s) => (s, keys.rows, keys.across),
        None if keys.across != 0 => (0, keys.rows, keys.across - keys.across.signum()),
        None => (0, keys.rows - keys.rows.signum(), 0),
    };
    (l.selected, l.by_pointer, l.reveal) = (Some(grid::moved(from, len, cols, rows, across)), false, true);
}

/// The apps side: what shows after this frame's typing and the sidebar,
/// as a list or a grid. Enter starts the app lit.
fn apps(l: &mut Launcher, icons: &mut Icons, ui: &mut Ui, rect: Rect, keys: &Keys, pointed: bool) -> Option<Outcome> {
    let m = ui.m;
    let shown = l.shown();
    if shown.is_empty() {
        empty(l, ui, rect);
        return None;
    }
    let hit = match l.options.layout {
        Layout::List => {
            step(l, keys, shown.len(), 1);
            list(l, icons, ui, rect, &shown, pointed)
        }
        Layout::Grid => {
            let line_h = f64::from(TextStyle::new(m.px(NAME) as f32).line_height());
            let fit = grid::fit(rect.width() - m.px(10.0), m.px(f64::from(l.options.grid_icon)), line_h, m);
            step(l, keys, shown.len(), fit.cols);
            grid::draw(l, icons, ui, rect, &shown, fit, pointed)
        }
    };
    let entered = || l.selected.and_then(|s| shown.get(s)).copied().map(Hit::Launch);
    match hit.or_else(|| if keys.enter { entered() } else { None })? {
        Hit::Launch(i) => Some(Outcome::Launch(l.apps[i].command.clone())),
        Hit::Menu(i) => Some(Outcome::Menu(i)),
    }
}

/// The emoji side: what the search or the view shows, as tiles. Enter
/// types the emoji lit.
fn emoji_side(l: &mut Launcher, ui: &mut Ui, rect: Rect, keys: &Keys, pointed: bool) -> Option<Outcome> {
    let shown = l.emoji.shown(&l.query);
    if shown.is_empty() {
        emoji::empty(l, ui, rect);
        return None;
    }
    step(l, keys, shown.len(), emoji::columns(rect.width() - ui.m.px(10.0), ui.m));
    let out = emoji::body(l, ui, rect, &shown, pointed);
    if keys.enter { out.or_else(|| emoji::enter(l, &shown, ui.now())) } else { out }
}

/// The clipboard side: the history's entries the view and the search
/// leave, a row each. Enter copies the one lit back.
fn clip_side(l: &mut Launcher, ui: &mut Ui, rect: Rect, keys: &Keys, pointed: bool) -> Option<Outcome> {
    let shown = l.clip.shown(&l.query).unwrap_or_default();
    if shown.is_empty() {
        clip::empty(l, ui, rect);
        return None;
    }
    step(l, keys, shown.len(), 1);
    let out = clip::body(l, ui, rect, &shown, pointed);
    if keys.enter { out.or_else(|| clip::enter(l, &shown)) } else { out }
}

/// What the keys the menu takes before the search field sees them ask.
#[derive(Default)]
struct Keys {
    close: bool,
    /// Start the selected app.
    enter: bool,
    /// Rows to move the selection by.
    rows: isize,
    /// Tiles to move it by, along the grid.
    across: isize,
}

/// Escape (puts a question away, then clears the search, then closes),
/// the arrows (move the selection) and Enter (does what a click on the one
/// lit does), every press of them this frame. Left and Right only in a
/// `grid` (the apps' or the emoji): in a list they stay the search
/// field's.
fn keys(l: &mut Launcher, ui: &mut Ui, grid: bool) -> Keys {
    let mut keys = Keys::default();
    let mut take = |key: Key| ui.state.take_key(|k| k.key == key && k.mods.is_empty()).is_some();
    while take(Key::Escape) {
        if l.confirm.take().is_some() || l.clip.unask() {
            continue;
        }
        if l.query.is_empty() {
            keys.close = true;
        }
        l.query.clear();
        l.restart();
    }
    while take(Key::ArrowDown) {
        keys.rows += 1;
    }
    while take(Key::ArrowUp) {
        keys.rows -= 1;
    }
    while grid && take(Key::ArrowRight) {
        keys.across += 1;
    }
    while grid && take(Key::ArrowLeft) {
        keys.across -= 1;
    }
    while take(Key::Enter) {
        keys.enter = true;
    }
    keys
}

/// "Favorites", "All apps" and each group with an app in it. Picking one
/// ends a search.
fn sidebar(l: &mut Launcher, ui: &mut Ui, rect: Rect) {
    let m = ui.m;
    let entries: Vec<View> = [View::Favorites, View::All].into_iter().chain(l.groups.iter().copied().map(View::Group)).collect();
    // Shorter rows when many groups would run past the bottom.
    let row_h = m.px(GROUP_H).min(rect.height() / entries.len().max(1) as f64);
    let style = TextStyle::new(m.px(NAME) as f32);
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    for (i, entry) in entries.into_iter().enumerate() {
        let row = Rect::from_min_size(Vec2::new(rect.min.x, rect.min.y + i as f64 * row_h), Vec2::new(rect.width(), row_h));
        let label = entry.label();
        let r = ui.interact(ui.id(label), row, Sense::CLICK);
        let current = l.query.trim().is_empty() && l.view == entry;
        if r.clicked {
            l.view = entry;
            l.query.clear();
            l.restart();
            // The rows above were drawn with the group before.
            ui.state.request_rebuild = true;
        }
        if current {
            ui.draw.rounded_rect(row, m.px(10.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(row, m.px(10.0), lntrn_math::Color::WHITE.fade(0.06));
        }
        ui.text_in_rect(label, &style, Rect::new(Vec2::new(row.min.x + m.px(14.0), row.min.y), row.max), if current { accent } else { ink });
    }
}

/// What the apps' side says when nothing shows: how to get favorites, or
/// that the search or the group has none.
fn empty(l: &Launcher, ui: &mut Ui, rect: Rect) {
    let m = ui.m;
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let name = TextStyle::new(m.px(NAME) as f32);
    if l.query.trim().is_empty() && l.view == View::Favorites {
        // How to get some, under what is missing.
        let small = TextStyle::new(m.px(DIM) as f32);
        let (mid, h) = (rect.center().y, f64::from(name.line_height()));
        ui.text_centered("No favorites yet", &name, Rect::new(Vec2::new(rect.min.x, mid - h), Vec2::new(rect.max.x, mid)), ink);
        ui.text_centered("Right-click any app to add it here", &small, Rect::new(Vec2::new(rect.min.x, mid), Vec2::new(rect.max.x, mid + h)), dim);
        return;
    }
    let what = if l.query.trim().is_empty() { "No apps here.".to_owned() } else { format!("Nothing matches “{}”.", l.query.trim()) };
    ui.text_centered(&what, &name, rect, dim);
}

/// The wheel over `rect`, then the lit app's row (`selected`: its top and
/// height in the `content` px the apps take) brought into view when asked,
/// and the scroll held to what there is. How far it can scroll.
pub(super) fn scroll(l: &mut Launcher, ui: &mut Ui, rect: Rect, content: f64, selected: Option<(f64, f64)>) -> f64 {
    let max = (content - rect.height()).max(0.0);
    if rect.contains(ui.state.pointer) && ui.state.wheel.y != 0.0 {
        l.scroll -= ui.state.wheel.y;
        ui.state.wheel = Vec2::ZERO;
    }
    if std::mem::take(&mut l.reveal)
        && let Some((top, h)) = selected
    {
        l.scroll = l.scroll.min(top).max(top + h - rect.height());
    }
    l.scroll = l.scroll.clamp(0.0, max);
    max
}

/// The pointer's word on the light once the apps are laid out: having
/// `pointed` anew, it lights the app it is over (`hovered`, by its place
/// among those showing) or, off the apps, puts out a light it lit. The
/// keyboard's light stays under a pointer at rest. A change draws the
/// frame again: the apps were drawn lit as they were.
pub(super) fn follow(l: &mut Launcher, ui: &mut Ui, pointed: bool, hovered: Option<usize>) {
    if !pointed {
        return;
    }
    let (to, by_pointer) = match hovered {
        Some(pos) => (Some(pos), true),
        None if l.by_pointer => (None, false),
        None => return,
    };
    if (to, by_pointer) != (l.selected, l.by_pointer) {
        (l.selected, l.by_pointer) = (to, by_pointer);
        ui.state.request_rebuild = true;
    }
}

/// Where the view is, along `rect`'s right edge, when there is more.
pub(super) fn scrollbar(ui: &mut Ui, rect: Rect, scroll: f64, max: f64, content: f64) {
    if max <= 0.0 {
        return;
    }
    let m = ui.m;
    let track = rect.height();
    let thumb = (track * rect.height() / content).max(m.px(32.0));
    let y = rect.min.y + (track - thumb) * (scroll / max);
    ui.draw.rounded_rect(Rect::from_min_size(Vec2::new(rect.max.x - m.px(4.0), y), Vec2::new(m.px(4.0), thumb)), m.px(2.0), ui.theme.text_dim.fade(0.5));
}

/// `app`'s picture in `pic`; while it is on its way, or for an app
/// without one, its initial on a tile.
pub(super) fn picture(ui: &mut Ui, icons: &mut Icons, app: &App, pic: Rect) {
    match icons.get(&app.icon) {
        Some(handle) => ui.draw.image(pic, handle, 0.0, lntrn_math::Color::WHITE),
        None => {
            let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
            ui.draw.rounded_rect(pic, (pic.width() * 0.22).round(), ink.fade(0.12));
            let initial: String = app.name.chars().take(1).collect();
            let style = TextStyle::new((pic.height() * 0.45).max(ui.m.px(NAME)) as f32);
            ui.text_centered(&initial, &style, pic, dim);
        }
    }
}

/// The apps that show, a row each, scrolled by the wheel and to keep the
/// lit one in view, the light following a pointer that `pointed`. Returns
/// the app clicked or right-clicked, by index; a right-click lights its
/// row too, so the row the menu is about stays lit under it.
fn list(l: &mut Launcher, icons: &mut Icons, ui: &mut Ui, rect: Rect, shown: &[usize], pointed: bool) -> Option<Hit> {
    let m = ui.m;
    let row_h = m.px(ROW_H);
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    let content = shown.len() as f64 * row_h;
    l.selected = l.selected.map(|s| s.min(shown.len() - 1));
    let max = scroll(l, ui, rect, content, l.selected.map(|s| (s as f64 * row_h, row_h)));

    let name = TextStyle::new(m.px(NAME) as f32);
    let small = TextStyle::new(m.px(DIM) as f32);
    let icon = m.px(APP_ICON);
    let first = (l.scroll / row_h).floor() as usize;
    let last = (((l.scroll + rect.height()) / row_h).ceil() as usize).min(shown.len());
    let (mut hit, mut hovered) = (None, None);
    ui.draw.push_clip(rect);
    for (pos, &i) in shown.iter().enumerate().take(last).skip(first) {
        let row = Rect::from_min_size(Vec2::new(rect.min.x, rect.min.y + pos as f64 * row_h - l.scroll), Vec2::new(rect.width() - m.px(10.0), row_h));
        let r = ui.interact(ui.id("app").with_index(i), row.intersection(&rect), Sense::CLICK);
        if r.clicked {
            hit = Some(Hit::Launch(i));
        }
        if r.hovered {
            hovered = Some(pos);
        }
        if r.hovered && ui.state.right_pressed {
            (l.selected, l.by_pointer) = (Some(pos), true);
            hit = Some(Hit::Menu(i));
        }
        if Some(pos) == l.selected {
            ui.draw.rounded_rect(row, m.px(10.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(row, m.px(10.0), lntrn_math::Color::WHITE.fade(0.06));
        }
        let app = &l.apps[i];
        let pic = Rect::from_min_size(Vec2::new(row.min.x + m.px(10.0), (row.center().y - icon * 0.5).round()), Vec2::new(icon, icon));
        picture(ui, icons, app, pic);
        let text = Rect::new(Vec2::new(pic.max.x + m.px(14.0), row.min.y), Vec2::new(row.max.x - m.px(10.0), row.max.y));
        ui.text_in_rect(&app.name, &name, text, ink);
        if !app.generic.is_empty() && app.generic != app.name {
            let after = text.min.x + ui.measure(&app.name, &name) + m.px(12.0);
            if after + m.px(60.0) < text.max.x {
                ui.text_in_rect(&app.generic, &small, Rect::new(Vec2::new(after, text.min.y), text.max), dim);
            }
        }
    }
    ui.draw.pop_clip();
    scrollbar(ui, rect, l.scroll, max, content);
    follow(l, ui, pointed, hovered);
    hit
}
