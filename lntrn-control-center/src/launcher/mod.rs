//! The app launcher at the bar's left end: a button of dots, and the menu
//! it opens above the bar the way KDE's Kickoff does: a search field, your
//! favorites, all apps and their groups down the left, the apps beside
//! them (right-click one for what can be done with it), and the power
//! buttons along the bottom. A menu can't open inside the bar, so it is
//! drawn in the overlay (`app::overlay`), a surface over the whole monitor,
//! see-through but for its panel, taking the keyboard so typing searches
//! at once; a click anywhere off the panel closes it, as Escape does and
//! starting an app.

mod button;
mod find;
mod menu;
mod power;
#[cfg(test)]
mod tests;

use lntrn_apps::{App, Group};

pub use button::button;
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

pub struct Launcher {
    /// Whether the menu is up.
    pub open: bool,
    apps: Vec<App>,
    /// Each app's groups, beside `apps`.
    app_groups: Vec<Vec<Group>>,
    /// The groups some app is in, in [`Group::ALL`]'s order.
    groups: Vec<Group>,
    view: View,
    favorites: IdList,
    query: String,
    /// The row the keyboard is on, among those showing.
    selected: usize,
    /// How far the list is scrolled, window pixels.
    scroll: f64,
    /// Scroll the selected row into view on the next frame.
    reveal: bool,
    /// A power action waiting for its confirmation.
    confirm: Option<PowerAction>,
}

impl Launcher {
    pub fn new() -> Self {
        Self { open: false, apps: Vec::new(), app_groups: Vec::new(), groups: Vec::new(), view: View::Favorites, favorites: IdList::default(), query: String::new(), selected: 0, scroll: 0.0, reveal: false, confirm: None }
    }

    /// Open the menu on `apps` (read afresh, so one installed since
    /// shows) and the favorites as their file has them now, on the
    /// favorites, from the top.
    pub fn open(&mut self, apps: Vec<App>) {
        self.favorites = IdList::load(idlist::FAVORITES);
        self.load(apps);
    }

    /// Open the menu on `apps`.
    fn load(&mut self, apps: Vec<App>) {
        self.apps = apps;
        self.app_groups = self.apps.iter().map(|a| Group::of(&a.categories)).collect();
        self.groups = Group::ALL.into_iter().filter(|g| self.app_groups.iter().any(|gs| gs.contains(g))).collect();
        self.view = View::Favorites;
        self.query.clear();
        (self.selected, self.scroll, self.reveal, self.confirm) = (0, 0.0, false, None);
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.confirm = None;
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
