//! The menus a right-click opens, the launcher's on an app in its list
//! and the dock's on an icon, and what their rows do (they come back to
//! the app as actions, through `Host::run`).

use lntrn_core::{log_info, log_warn};
use lntrn_math::Vec2;
use lntrn_props::Value;
use lntrn_ui::{Action, ContextMenu, HostCx, Item, ShellRequest};

use super::App;
use crate::dock::Entry;

/// The rows, as actions.
const OPEN: &str = "launcher.open";
const FAVORITE: &str = "launcher.favorite";
const NEW_WINDOW: &str = "dock.new";
const PIN: &str = "dock.pin";
const CLOSE: &str = "dock.close";

/// Start an app with its desktop entry's `command`.
pub fn launch(command: &str) {
    match lntrn_apps::launch::run(command) {
        Ok(()) => log_info!("started {command}"),
        Err(e) => log_warn!("{command}: {e}"),
    }
}

fn favorite_row(id: &str, favorite: bool) -> Item {
    let label = if favorite { "Remove from Favorites" } else { "Add to Favorites" };
    Item::action(label, Action::new(FAVORITE).with("app", Value::Str(id.to_owned())))
}

fn pin_row(id: &str, pinned: bool) -> Item {
    let label = if pinned { "Unpin from Dock" } else { "Pin to Dock" };
    Item::action(label, Action::new(PIN).with("app", Value::Str(id.to_owned())))
}

/// What can be done with an app in the launcher's list: the menu a
/// right-click on it opens at `at`, under its name.
pub fn app_menu(app: &lntrn_apps::App, favorite: bool, pinned: bool, at: Vec2) -> ContextMenu {
    let open = Item::action("Open", Action::new(OPEN).with("command", Value::Str(app.command.clone())));
    ContextMenu::new(&app.name, at).tab("App", vec![open, favorite_row(&app.id, favorite), pin_row(&app.id, pinned)])
}

/// What can be done with an app on the dock: a new window, pinning it,
/// making it a favorite (an installed app's alone), and closing its
/// windows.
pub fn dock_menu(entry: &Entry, favorite: bool, pinned: bool, at: Vec2) -> ContextMenu {
    let mut rows = Vec::new();
    if let Some(app) = &entry.app {
        rows.push(Item::action("New Window", Action::new(NEW_WINDOW).with("command", Value::Str(app.command.clone()))));
        rows.push(pin_row(&app.id, pinned));
        rows.push(favorite_row(&app.id, favorite));
    }
    if entry.running() {
        if !rows.is_empty() {
            rows.push(Item::Separator);
        }
        let ids: Vec<String> = entry.windows.iter().map(|w| w.id.to_string()).collect();
        let label = match ids.len() {
            1 => "Close Window".to_owned(),
            n => format!("Close All {n} Windows"),
        };
        rows.push(Item::danger(&label, Action::new(CLOSE).with("windows", Value::Str(ids.join(",")))));
    }
    ContextMenu::new(entry.name(), at).tab("App", rows)
}

/// How many rows [`dock_menu`] has for `entry`, a separator counted as one.
pub fn dock_menu_rows(entry: &Entry) -> usize {
    let app = if entry.app.is_some() { 3 } else { 0 };
    let running = if entry.running() { 1 + usize::from(app > 0) } else { 0 };
    app + running
}

impl App {
    /// A row of a menu was chosen.
    pub(super) fn act(&mut self, action: &Action, cx: &mut HostCx) {
        let text = |name: &str| match action.arg(name) {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        };
        match action.id.as_str() {
            OPEN => {
                if let Some(command) = text("command") {
                    launch(&command);
                }
                // The launcher's menu goes with it.
                cx.request(ShellRequest::CloseWindow);
            }
            NEW_WINDOW => {
                if let Some(command) = text("command") {
                    launch(&command);
                }
            }
            FAVORITE => {
                if let Some(id) = text("app") {
                    self.launcher.reload_favorites();
                    self.launcher.toggle_favorite(&id);
                }
                cx.rebuild();
            }
            PIN => {
                if let Some(id) = text("app") {
                    self.dock.toggle_pin(&id);
                    self.dock.refresh(&self.link);
                }
                cx.rebuild();
                self.wake();
            }
            CLOSE => {
                for id in text("windows").unwrap_or_default().split(',').filter_map(|id| id.parse::<u64>().ok()) {
                    self.link.send(&format!("close:{id}"));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::Window;

    fn entry(app: bool, windows: usize) -> Entry {
        let app = app.then(|| lntrn_apps::App { id: "foot".into(), name: "Foot".into(), command: "foot".into(), ..Default::default() });
        let windows = (0..windows as u64).map(|id| Window { id, ..Window::default() }).collect();
        Entry { app, key: "foot".into(), pinned: false, windows }
    }

    fn labels(menu: &ContextMenu) -> Vec<String> {
        menu.tabs[0]
            .items
            .iter()
            .map(|i| match i {
                Item::Action { label, .. } | Item::Danger { label, .. } => label.clone(),
                Item::Separator => "--".to_owned(),
                _ => "?".to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_dock_menu_offers_what_fits_the_app_and_its_rows_are_counted() {
        for (e, want) in [
            (entry(true, 0), vec!["New Window", "Pin to Dock", "Add to Favorites"]),
            (entry(true, 1), vec!["New Window", "Pin to Dock", "Add to Favorites", "--", "Close Window"]),
            (entry(true, 3), vec!["New Window", "Pin to Dock", "Add to Favorites", "--", "Close All 3 Windows"]),
            (entry(false, 2), vec!["Close All 2 Windows"]),
        ] {
            let menu = dock_menu(&e, false, false, Vec2::ZERO);
            assert_eq!(labels(&menu), want);
            assert_eq!(dock_menu_rows(&e), want.len());
        }
        let pinned = dock_menu(&entry(true, 0), true, true, Vec2::ZERO);
        assert_eq!(labels(&pinned)[1..], ["Unpin from Dock", "Remove from Favorites"]);
    }

    #[test]
    fn the_launchers_menu_can_pin_too() {
        let app = entry(true, 0).app.unwrap();
        assert_eq!(labels(&app_menu(&app, false, true, Vec2::ZERO)), vec!["Open", "Add to Favorites", "Unpin from Dock"]);
    }
}
