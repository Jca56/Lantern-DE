//! The app launcher at the bar's left end: a button of dots, and the menu
//! it opens above the bar the way KDE's Kickoff does: a search field, your
//! favorites, all apps and their groups down the left, the apps beside
//! them (right-click one for what can be done with it), and the power
//! buttons along the bottom. A menu can't open inside the bar, so it is a layer
//! surface of its own over the whole monitor, see-through but for its
//! panel, taking the keyboard so typing searches at once; a click
//! anywhere off the panel closes it, as Escape does and starting an app.

mod button;
mod favorites;
mod find;
mod menu;
mod power;
#[cfg(test)]
mod tests;

use lntrn_apps::{App, Group, Icons};
use lntrn_ui::{Anchor, KeyboardMode, LayerConfig};

pub use button::button;
pub use favorites::Favorites;
pub use menu::{Outcome, Place, draw as menu};
pub use power::Action as PowerAction;

/// What the compositor knows the menu's surface as.
const NAMESPACE: &str = "lntrn-control-center-launcher";
/// Pixels an icon is rendered at: sharp at the list's size on a dense
/// screen.
const ICON_PX: u32 = 96;

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
    favorites: Favorites,
    query: String,
    /// The row the keyboard is on, among those showing.
    selected: usize,
    /// How far the list is scrolled, window pixels.
    scroll: f64,
    /// Scroll the selected row into view on the next frame.
    reveal: bool,
    /// A power action waiting for its confirmation.
    confirm: Option<PowerAction>,
    pub icons: Icons,
}

impl Launcher {
    pub fn new() -> Self {
        Self { open: false, apps: Vec::new(), app_groups: Vec::new(), groups: Vec::new(), view: View::Favorites, favorites: Favorites::default(), query: String::new(), selected: 0, scroll: 0.0, reveal: false, confirm: None, icons: Icons::new(ICON_PX) }
    }

    /// Open the menu: the apps and favorites read afresh (one installed
    /// or edited in since shows), on the favorites, from the top.
    pub fn open(&mut self) {
        self.favorites = Favorites::load();
        self.load(lntrn_apps::installed());
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

    /// The menu's surface: the whole of the monitor called `output` (the
    /// compositor's pick when `None`), over the bar and past the zones
    /// windows are kept out of, taking the keyboard while it is up.
    pub fn layer(output: Option<&str>) -> LayerConfig {
        let cfg = LayerConfig::new(NAMESPACE, (0, 0)).anchor(Anchor::ALL).exclusive(-1).keyboard(KeyboardMode::Exclusive);
        match output {
            Some(name) => cfg.output(name),
            None => cfg,
        }
    }

    /// The app at `index` in the list the menu was opened on.
    pub fn app(&self, index: usize) -> Option<&App> {
        self.apps.get(index)
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
