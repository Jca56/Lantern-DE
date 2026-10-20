//! The app launcher at the bar's left end: a button of dots, and the menu
//! it opens above the bar the way KDE's Kickoff does: a search field, your
//! favorites, all apps and their groups down the left, the apps beside
//! them (right-click one for what can be done with it) as a list or a
//! grid of tiles (`[control_center] launcher_view`, which the two buttons
//! beside the search field switch too), and the power buttons along the
//! bottom. Two more buttons there turn the menu into an emoji picker
//! (`emoji`) or the clipboard's history (`clip`), the search searching
//! whichever page is up. A menu can't open inside the bar, so it is
//! drawn in the overlay (`app::overlay`), a surface over the whole monitor,
//! see-through but for its panel, taking the keyboard so typing searches
//! at once; a click anywhere off the panel closes it, as Escape does and
//! starting an app.

mod button;
mod clip;
mod emoji;
mod find;
mod grid;
mod menu;
mod power;
#[cfg(test)]
mod tests;
mod toggle;

use lntrn_app::Waker;
use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_apps::{App, Group};
use lntrn_core::log_warn;
use lntrn_data::Doc;
use lntrn_math::Vec2;

pub use button::button;
use clip::ClipPage;
use emoji::EmojiPage;
pub use menu::{Outcome, Place, draw as menu};
pub use power::Action as PowerAction;

use crate::idlist::{self, IdList};


/// What the list shows when nothing is being searched for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Favorites,
    All,
    Group(Group),
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::Favorites => "Favorites",
            View::All => "All apps",
            View::Group(g) => g.label(),
        }
    }
}

/// What the menu shows under its search field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Apps,
    Emoji,
    Clipboard,
}

/// How the menu lays its apps out: `[control_center] launcher_view`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// A row each: the icon, the name and what the app is.
    #[default]
    List,
    /// Tiles: the icon above the name.
    Grid,
}

impl Layout {
    /// From its stored word; anything else is the list.
    pub fn from_word(word: &str) -> Layout {
        match word {
            "grid" => Layout::Grid,
            _ => Layout::List,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Layout::List => "list",
            Layout::Grid => "grid",
        }
    }
}

/// `[control_center]`'s `launcher_*` keys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LauncherOptions {
    pub layout: Layout,
    /// The grid's icons, logical px.
    pub grid_icon: u32,
}

impl LauncherOptions {
    /// `launcher_grid_icon`'s range, as System Settings' slider offers it.
    pub const GRID_ICONS: (i64, i64) = (48, 96);
}

impl Default for LauncherOptions {
    fn default() -> Self {
        LauncherOptions { layout: Layout::List, grid_icon: 64 }
    }
}

/// Keep `layout` in `lantern.toml`, where System Settings shows it and the
/// bar reads it back: the menu's own buttons switched it.
pub fn keep(layout: Layout) {
    let written = lntrn_kit::desktop::update("control_center", |t| t.insert("launcher_view", Doc::Str(layout.word().to_owned())));
    if let Err(e) = written {
        log_warn!("launcher: can't keep the {} view: {e}", layout.word());
    }
}

pub struct Launcher {
    /// Whether the menu is up.
    pub open: bool,
    /// List or grid, and the grid's icon size.
    options: LauncherOptions,
    page: Page,
    emoji: EmojiPage,
    clip: ClipPage,
    apps: Vec<App>,
    /// Each app's groups, beside `apps`.
    app_groups: Vec<Vec<Group>>,
    /// The groups some app is in, in [`Group::ALL`]'s order.
    groups: Vec<Group>,
    view: View,
    favorites: IdList,
    query: String,
    /// The app lit, among those showing: none as the menu opens; the
    /// first found once typing; then wherever the arrows or the pointer
    /// put it. Enter starts it.
    selected: Option<usize>,
    /// The pointer lit it (and moving off the apps puts it out), not the
    /// keyboard.
    by_pointer: bool,
    /// Where the pointer was last frame, inside the window.
    pointer: Option<Vec2>,
    /// How far the list is scrolled, window pixels.
    scroll: f64,
    /// Scroll the selected row into view on the next frame.
    reveal: bool,
    /// A power action waiting for its confirmation.
    confirm: Option<PowerAction>,
}

impl Launcher {
    pub fn new() -> Self {
        Self { open: false, options: LauncherOptions::default(), page: Page::Apps, emoji: EmojiPage::new(), clip: ClipPage::new(), apps: Vec::new(), app_groups: Vec::new(), groups: Vec::new(), view: View::Favorites, favorites: IdList::default(), query: String::new(), selected: None, by_pointer: false, pointer: None, scroll: 0.0, reveal: false, confirm: None }
    }

    /// Open the menu on `apps` (read afresh, so one installed since
    /// shows) and the favorites as their file has them now, on the
    /// favorites, from the top.
    pub fn open(&mut self, apps: Vec<App>) {
        self.favorites = IdList::load(idlist::FAVORITES);
        self.emoji.load();
        self.load(apps);
    }

    /// Open the menu on `apps`.
    fn load(&mut self, apps: Vec<App>) {
        self.apps = apps;
        self.app_groups = self.apps.iter().map(|a| Group::of(&a.categories)).collect();
        self.groups = Group::ALL.into_iter().filter(|g| self.app_groups.iter().any(|gs| gs.contains(g))).collect();
        self.view = View::Favorites;
        self.page = Page::Apps;
        self.clip.unask();
        self.query.clear();
        (self.selected, self.by_pointer, self.pointer) = (None, false, None);
        (self.scroll, self.reveal, self.confirm) = (0.0, false, None);
        self.open = true;
    }

    /// Take up `options` from the settings. A new layout starts from the
    /// top, with the selection in view.
    pub fn follow(&mut self, options: LauncherOptions) {
        if options.layout != self.options.layout {
            (self.scroll, self.reveal) = (0.0, true);
        }
        self.options = options;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.confirm = None;
        self.emoji.settle();
        self.clip.unask();
    }

    /// Turn to `page`, from the top with nothing searched or lit.
    fn go(&mut self, page: Page) {
        if self.page == Page::Emoji {
            self.emoji.settle();
        }
        self.page = page;
        self.query.clear();
        self.restart();
        match page {
            Page::Apps => {}
            Page::Emoji => self.emoji.open(),
            Page::Clipboard => self.clip.open(),
        }
    }

    /// The loop's waker, for the clipboard's answers and its pictures.
    pub fn set_waker(&self, waker: Waker) {
        self.clip.history.set_waker(waker.clone());
        self.clip.thumbs.set_waker(waker);
    }

    /// Hand the clipboard's pictures to the GPU. Whether any went up.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        self.clip.thumbs.upload(gpu, images)
    }

    /// The clipboard entry called `id`, as last read.
    pub fn clip_entry(&self, id: u64) -> Option<crate::clipboard::ClipEntry> {
        self.clip.history.entries()?.into_iter().find(|e| e.id == id)
    }

    /// Do `op` to the clipboard's history.
    pub fn run_clip(&self, op: crate::clipboard::ClipOp) {
        self.clip.history.run(op);
    }

    /// Light nothing, or the first found while there is a search, and
    /// start from the top: the apps showing changed.
    fn restart(&mut self) {
        self.selected = (!self.query.trim().is_empty()).then_some(0);
        (self.by_pointer, self.scroll) = (false, 0.0);
    }

    /// The app at `index` in the list the menu was opened on.
    pub fn app(&self, index: usize) -> Option<&App> {
        self.apps.get(index)
    }

    /// Read the favorites afresh from their file, for a change made from
    /// outside the menu (the dock's) while it isn't open.
    pub fn reload_favorites(&mut self) {
        self.favorites = IdList::load(idlist::FAVORITES);
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.contains(id)
    }

    /// Make the app called `id` a favorite, or no longer one.
    pub fn toggle_favorite(&mut self, id: &str) {
        let now = self.favorites.toggle(id);
        lntrn_core::log_info!("launcher: {id} {}", if now { "is a favorite" } else { "is no longer a favorite" });
        // The list may have lost the row the keyboard was on.
        self.reveal = true;
    }

    /// The apps that show, by index: what the search finds, else the
    /// view's: the favorites in their order (those still installed), all
    /// apps, or a group's.
    fn shown(&self) -> Vec<usize> {
        if !self.query.trim().is_empty() {
            return find::matches(&self.apps, &self.query);
        }
        match self.view {
            View::Favorites => self.favorites.ids().iter().filter_map(|id| self.apps.iter().position(|a| a.id == *id)).collect(),
            View::All => (0..self.apps.len()).collect(),
            View::Group(g) => (0..self.apps.len()).filter(|i| self.app_groups[*i].contains(&g)).collect(),
        }
    }
}
