//! What the dock shows: one icon an app. The pinned apps come first in
//! the order they were pinned (and dragged into), lit while they run; the
//! apps running unpinned follow in the order their first window opened.
//! A window belongs to the installed app whose desktop id, StartupWMClass,
//! icon name or program it calls itself by, whatever their case; one that
//! belongs to none gets an icon of its own, by its own name.

use lntrn_apps::App;

use crate::compositor::Window;

/// One icon on the dock.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Its desktop entry, when there is one: what launches it again.
    pub app: Option<App>,
    /// What it goes by: its desktop id, or its windows' own name.
    pub key: String,
    pub pinned: bool,
    /// Its windows, oldest first.
    pub windows: Vec<Window>,
}

impl Entry {
    /// What it is called: the app's name, else its first window's title.
    pub fn name(&self) -> &str {
        match (&self.app, self.windows.first()) {
            (Some(app), _) => &app.name,
            (None, Some(w)) if !w.title.is_empty() => &w.title,
            _ => &self.key,
        }
    }

    /// Its picture's name: the app's icon, else the windows' own name
    /// (Lantern's apps and many others are named after theirs).
    pub fn icon(&self) -> &str {
        match &self.app {
            Some(app) if !app.icon.is_empty() => &app.icon,
            _ => self.windows.first().map_or(self.key.as_str(), |w| w.app_id.as_str()),
        }
    }

    /// One of its windows has the focus.
    pub fn active(&self) -> bool {
        self.windows.iter().any(|w| w.active)
    }

    pub fn running(&self) -> bool {
        !self.windows.is_empty()
    }
}

/// The dock's icons for these `apps`, `pins` (desktop ids, in order) and
/// open `windows` (oldest first).
pub fn entries(apps: &[App], pins: &[String], windows: &[Window]) -> Vec<Entry> {
    let mut out: Vec<Entry> = pins
        .iter()
        .filter_map(|id| apps.iter().find(|a| a.id == *id))
        .map(|app| Entry { app: Some(app.clone()), key: app.id.clone(), pinned: true, windows: Vec::new() })
        .collect();
    for window in windows {
        let app = owner(apps, &window.app_id);
        // A window with no name is an app of its own.
        let key = match app {
            Some(app) => app.id.clone(),
            None if window.app_id.is_empty() => format!("#{}", window.id),
            None => window.app_id.clone(),
        };
        match out.iter_mut().find(|e| e.key == key) {
            Some(entry) => entry.windows.push(window.clone()),
            None => out.push(Entry { app: app.cloned(), key, pinned: false, windows: vec![window.clone()] }),
        }
    }
    out
}

/// The installed app a window calling itself `app_id` belongs to: by
/// desktop id first, then StartupWMClass, then the last part of a
/// reverse-domain id (`org.gnome.Nautilus`), its icon's name, and the
/// program it runs. Case never matters.
pub fn owner<'a>(apps: &'a [App], app_id: &str) -> Option<&'a App> {
    if app_id.is_empty() {
        return None;
    }
    let is = |s: &str| !s.is_empty() && s.eq_ignore_ascii_case(app_id);
    let ways: [&dyn Fn(&App) -> bool; 5] = [
        &|a| is(&a.id),
        &|a| is(&a.wm_class),
        &|a| a.id.contains('.') && is(a.id.rsplit('.').next().unwrap_or_default()),
        &|a| is(&a.icon),
        &|a| is(program(&a.command)),
    ];
    ways.iter().find_map(|way| apps.iter().find(|a| way(a)))
}

/// The name of the program a command runs: its first word's last path
/// part, past an `env` and its settings.
fn program(command: &str) -> &str {
    let mut words = command.split_whitespace().skip_while(|w| *w == "env" || w.contains('='));
    let first = words.next().unwrap_or_default().trim_matches('"');
    first.rsplit('/').next().unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, wm_class: &str, icon: &str, command: &str) -> App {
        App { id: id.into(), name: id.into(), wm_class: wm_class.into(), icon: icon.into(), command: command.into(), ..App::default() }
    }

    fn window(id: u64, app_id: &str) -> Window {
        Window { id, app_id: app_id.into(), title: format!("{app_id} {id}"), ..Window::default() }
    }

    fn apps() -> Vec<App> {
        vec![
            app("firefox-bin", "firefox", "firefox-bin", "firefox-bin --name=firefox-bin"),
            app("code", "Code", "vscode", "/usr/bin/vscode --ozone-platform-hint=auto"),
            app("org.gnome.Nautilus", "", "org.gnome.Nautilus", "nautilus --new-window"),
            app("lntrn-terminal", "", "lntrn-terminal", "lntrn-terminal"),
            app("wine-game", "", "game", "env WINEPREFIX=/w \"/opt/games/run.sh\""),
        ]
    }

    #[test]
    fn a_window_finds_its_app_however_it_names_itself() {
        let apps = apps();
        let of = |app_id: &str| owner(&apps, app_id).map(|a| a.id.as_str());
        assert_eq!(of("firefox"), Some("firefox-bin"), "StartupWMClass");
        assert_eq!(of("Code"), Some("code"));
        assert_eq!(of("code"), Some("code"), "the id, before a class of another case");
        assert_eq!(of("nautilus"), Some("org.gnome.Nautilus"), "a reverse-domain id's last part");
        assert_eq!(of("vscode"), Some("code"), "the icon's name, or the program");
        assert_eq!(of("run.sh"), Some("wine-game"), "past env and its settings");
        assert_eq!(of("LNTRN-TERMINAL"), Some("lntrn-terminal"));
        assert_eq!(of("steam_app_123"), None);
        assert_eq!(of(""), None);
    }

    #[test]
    fn pins_stay_in_order_lit_while_they_run_and_the_rest_follow_as_they_opened() {
        let apps = apps();
        let pins = vec!["lntrn-terminal".to_owned(), "gone-app".to_owned(), "firefox-bin".to_owned()];
        let windows = vec![window(3, "steam_app_9"), window(4, "firefox"), window(5, "Code"), window(7, "firefox"), window(8, "")];
        let e = entries(&apps, &pins, &windows);
        let keys: Vec<(&str, bool, usize)> = e.iter().map(|e| (e.key.as_str(), e.pinned, e.windows.len())).collect();
        assert_eq!(keys, vec![("lntrn-terminal", true, 0), ("firefox-bin", true, 2), ("steam_app_9", false, 1), ("code", false, 1), ("#8", false, 1)]);
        // Names and pictures, with and without a desktop entry.
        assert_eq!((e[1].name(), e[1].icon()), ("firefox-bin", "firefox-bin"));
        assert_eq!((e[2].name(), e[2].icon()), ("steam_app_9 3", "steam_app_9"));
        assert!(e[1].running() && !e[0].running());
    }

    #[test]
    fn the_app_with_the_focus_is_the_active_one() {
        let mut w = window(1, "firefox");
        w.active = true;
        let e = entries(&apps(), &[], &[window(0, "code"), w]);
        assert_eq!(e.iter().map(Entry::active).collect::<Vec<_>>(), vec![false, true]);
    }
}
