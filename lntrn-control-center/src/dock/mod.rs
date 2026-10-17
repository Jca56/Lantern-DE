//! The dock in the middle of the bar: one icon an app, the pinned ones
//! kept in the order they were dragged into, the running ones lit, with a
//! line under them and a dot for each window past the first (`model`,
//! `bar`). Every window shows, from every monitor and workspace: the
//! compositor's window feed (`compositor`) says which there are.
//!
//! A click on an app with no window starts it; on one window, focuses it,
//! or minimizes it when it already has the focus; on several, shows them
//! all. Resting the pointer on an icon brings up its card over the screen
//! (`card`): a live thumbnail of each window the compositor paints in,
//! with a red X in its corner, to pick one or close it.
//! Right-click asks the app's menu; middle-click opens a new window.
//! The card and the menu are drawn in the overlay, the surface above the
//! bar that the launcher's menu uses too.

mod bar;
mod card;
mod model;

pub use bar::{Press, draw};
pub use card::{CardPress, Thumb, draw as card};
pub use model::Entry;

use lntrn_apps::App;
use lntrn_math::Rect;

use crate::compositor::{Link, Window};
use crate::idlist::{self, IdList};

/// How long the pointer rests on an icon before its card shows.
const HOVER_DELAY: f64 = 0.4;
/// How long the card stays once the pointer has left both it and its
/// icon: time enough to cross the gap between them.
const LEAVE_GRACE: f64 = 0.3;

/// What the dock has up in the overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    /// The card of the app with this key.
    Card(String),
    /// The menu of the app with this key: `asked` once requested of the
    /// shell, which then has it until it goes.
    Menu { key: String, asked: bool },
}

pub struct Dock {
    apps: Vec<App>,
    pins: IdList,
    windows: Vec<Window>,
    /// The link's revision `windows` is from; `None` to build anew.
    seen: Option<u64>,
    entries: Vec<Entry>,
    /// Each entry's face as last drawn, by key, in the bar's pixels.
    faces: Vec<(String, Rect)>,
    /// The icon under the pointer, and since when.
    hover: Option<(String, f64)>,
    /// The pointer is on the card.
    pub on_card: bool,
    leave_at: Option<f64>,
    pub popup: Option<Popup>,
    /// The pin being dragged to a new place, by key.
    drag: Option<String>,
}

impl Dock {
    /// The dock for these apps, with its pins as their file has them.
    pub fn new(apps: Vec<App>) -> Dock {
        Dock::with(apps, IdList::load(idlist::PINS))
    }

    fn with(apps: Vec<App>, pins: IdList) -> Dock {
        Dock { apps, pins, windows: Vec::new(), seen: None, entries: Vec::new(), faces: Vec::new(), hover: None, on_card: false, leave_at: None, popup: None, drag: None }
    }

    /// The installed apps, read afresh.
    pub fn set_apps(&mut self, apps: Vec<App>) {
        self.apps = apps;
        self.seen = None;
    }

    /// Follow the windows the compositor has. Returns whether anything
    /// the dock shows changed.
    pub fn refresh(&mut self, link: &Link) -> bool {
        let revision = link.revision();
        if self.seen == Some(revision) {
            return false;
        }
        self.seen = Some(revision);
        self.windows = link.windows();
        let entries = model::entries(&self.apps, self.pins.ids(), &self.windows);
        if entries == self.entries {
            return false;
        }
        self.entries = entries;
        // A card or menu for an app gone from the dock goes with it.
        if self.popup.as_ref().is_some_and(|p| self.entry(p.key()).is_none()) {
            self.popup = None;
        }
        true
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// The face of the entry with this key, as last drawn.
    pub fn face(&self, key: &str) -> Option<Rect> {
        self.faces.iter().find(|(k, _)| k == key).map(|(_, r)| *r)
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pins.contains(id)
    }

    /// Pin the app with this desktop id, or unpin it.
    pub fn toggle_pin(&mut self, id: &str) {
        let now = self.pins.toggle(id);
        lntrn_core::log_info!("dock: {id} {}", if now { "pinned" } else { "unpinned" });
        self.seen = None;
    }

    /// Nothing of the dock is under the pointer: the bar is out of sight.
    pub fn away(&mut self) {
        self.hover = None;
    }

    /// Note the icon under the pointer at `now`.
    fn hover(&mut self, key: Option<&str>, now: f64) {
        match (key, &self.hover) {
            (Some(k), Some((was, _))) if k == was => {}
            (Some(k), _) => self.hover = Some((k.to_owned(), now)),
            (None, _) => self.hover = None,
        }
    }

    /// The card's comings and goings at `now`: up once the pointer has
    /// rested on an icon a moment, over to another icon's at once, gone a
    /// moment after the pointer leaves both it and its icon. Returns how
    /// long until it needs another frame for itself.
    pub fn step(&mut self, now: f64) -> Option<f64> {
        if matches!(self.popup, Some(Popup::Menu { .. })) {
            return None;
        }
        let hovered = self.hover.clone();
        match (&self.popup, hovered) {
            (Some(Popup::Card(shown)), Some((key, _))) => {
                self.leave_at = None;
                if *shown != key {
                    self.popup = Some(Popup::Card(key));
                }
                None
            }
            (Some(Popup::Card(_)), None) if self.on_card => {
                self.leave_at = None;
                None
            }
            (Some(Popup::Card(_)), None) => {
                let at = *self.leave_at.get_or_insert(now + LEAVE_GRACE);
                if now < at {
                    return Some(at - now);
                }
                (self.popup, self.leave_at) = (None, None);
                None
            }
            (None, Some((key, since))) => {
                let at = since + HOVER_DELAY;
                if now < at {
                    return Some(at - now);
                }
                self.popup = Some(Popup::Card(key));
                None
            }
            _ => None,
        }
    }

    /// Show the menu of the app with this key, over any card.
    pub fn open_menu(&mut self, key: &str) {
        self.popup = Some(Popup::Menu { key: key.to_owned(), asked: false });
        self.leave_at = None;
    }

    /// Show the card of the app with this key now, without the wait.
    pub fn open_card(&mut self, key: &str) {
        self.popup = Some(Popup::Card(key.to_owned()));
        self.leave_at = None;
    }

    pub fn close_popup(&mut self) {
        (self.popup, self.leave_at, self.on_card) = (None, None, false);
    }

    /// Where each running app's icon is, for the compositor to fly its
    /// minimized windows into: `icons:<namespace>\t<app_id> x y w h\t…`,
    /// in the bar's logical pixels (`per`: its pixels per logical one).
    pub fn icons_line(&self, namespace: &str, per: f64) -> String {
        let mut line = format!("icons:{namespace}");
        for (key, face) in &self.faces {
            let Some(entry) = self.entry(key) else { continue };
            let mut named: Vec<&str> = Vec::new();
            for w in &entry.windows {
                if w.app_id.is_empty() || w.app_id.contains(char::is_whitespace) || named.contains(&w.app_id.as_str()) {
                    continue;
                }
                named.push(&w.app_id);
                let l = |v: f64| (v / per).round() as i64;
                line.push_str(&format!("\t{} {} {} {} {}", w.app_id, l(face.min.x), l(face.min.y), l(face.width()), l(face.height())));
            }
        }
        line
    }
}

impl Popup {
    /// The key of the app it is for.
    pub fn key(&self) -> &str {
        match self {
            Popup::Card(key) | Popup::Menu { key, .. } => key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dock() -> Dock {
        Dock::with(Vec::new(), IdList::in_memory(&[]))
    }

    #[test]
    fn a_card_waits_for_the_pointer_to_rest_then_follows_it_and_lingers() {
        let mut d = dock();
        d.hover(Some("firefox"), 1.0);
        assert!(d.step(1.1).is_some_and(|w| (w - 0.3).abs() < 1e-9));
        assert_eq!(d.popup, None, "not yet");
        d.hover(Some("firefox"), 1.4);
        assert_eq!(d.step(1.4), None);
        assert_eq!(d.popup, Some(Popup::Card("firefox".into())));
        // Over to the next icon: its card at once.
        d.hover(Some("code"), 1.5);
        d.step(1.5);
        assert_eq!(d.popup, Some(Popup::Card("code".into())));
        // Off the icon, onto the card: it stays.
        d.hover(None, 1.6);
        d.on_card = true;
        assert_eq!(d.step(5.0), None);
        assert!(d.popup.is_some());
        // Off both: gone after the grace.
        d.on_card = false;
        assert!(d.step(5.0).is_some());
        assert!(d.step(5.2).is_some() && d.popup.is_some());
        assert_eq!(d.step(5.3), None);
        assert_eq!(d.popup, None);
    }

    #[test]
    fn a_menu_holds_until_closed() {
        let mut d = dock();
        d.open_menu("code");
        d.hover(None, 0.0);
        assert_eq!(d.step(10.0), None);
        assert_eq!(d.popup.as_ref().map(Popup::key), Some("code"));
        d.close_popup();
        assert_eq!(d.popup, None);
    }

    #[test]
    fn icons_name_each_window_app_once_in_logical_pixels() {
        let mut d = dock();
        let w = |id, app_id: &str| Window { id, app_id: app_id.into(), ..Window::default() };
        d.entries = model::entries(&[], &[], &[w(1, "foot"), w(2, "foot"), w(3, "odd name")]);
        d.faces = vec![("foot".into(), Rect::from_min_size(lntrn_math::Vec2::new(130.0, 13.0), lntrn_math::Vec2::new(65.0, 65.0)))];
        assert_eq!(d.icons_line("lntrn-control-center", 1.3), "icons:lntrn-control-center\tfoot 100 10 50 50");
    }
}
