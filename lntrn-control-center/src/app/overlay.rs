//! The overlay: a layer surface over the bar's whole monitor, open while
//! something is up above the bar (the launcher's menu, the dock's card or
//! menu, a tray item's, a mini monitor's or the calendar's popup, the
//! clock's card) and closed as soon as nothing is. What is up decides how
//! it takes input: the launcher and a menu take the keyboard and every
//! click (a click off them puts them away); a dock card takes the pointer
//! over itself alone, from the overlay's first frame on, so the bar under
//! it never loses the pointer resting on its icon, and the clock's card
//! takes none at all; a tray, monitor or calendar popup takes the keyboard
//! and every click but the bar's own, so the bar's other items and its
//! wheel still work while it is up.

use std::sync::atomic::Ordering;

use lntrn_core::log_info;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Anchor, AreaCx, KeyboardMode, LayerConfig, NewWindow, Shell, ShellRequest, Ui};

use super::{APP_ID, App, RESTART, actions};
use crate::calendar;
use crate::clipboard::ClipOp;
use crate::clock;
use crate::dock::{self, CardPress, Entry, Popup, Thumb};
use crate::glass;
use crate::launcher::{self, Outcome, Place};
use crate::monitors::{self, Kind};
use crate::tray::{self, Item};

/// What the compositor knows the overlay's surface as.
pub const NAMESPACE: &str = "lntrn-control-center-overlay";
/// Between the card or the dock's menu and the bar, logical.
const ABOVE_BAR: f64 = 10.0;
/// How long an app's menu is waited for.
const APP_MENU_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
/// What the System Monitor's windows call themselves.
const SYSMON: &str = "lntrn-sysmon";

/// The overlay's window, as the app keeps track of it.
#[derive(Default)]
pub struct Overlay {
    /// Its window is open, or asked for.
    pub open: bool,
    /// It asked to close: nothing more of it until it has.
    closing: bool,
    /// What its surface was last told, and where it last took input
    /// (`None` inside: everywhere).
    sent: Option<LayerConfig>,
    region: Option<Option<Vec<Rect>>>,
    /// The thumbnails the compositor was last told of, with the
    /// connection it was told on.
    thumbs: Option<(u64, String)>,
}

/// The overlay's surface: the whole of the monitor called `output` (the
/// compositor's pick when `None`), over the bar and past the zones
/// windows are kept out of, taking the keyboard as `keyboard` says.
fn layer(output: Option<&str>, keyboard: KeyboardMode) -> LayerConfig {
    let cfg = LayerConfig::new(NAMESPACE, (0, 0)).anchor(Anchor::ALL).exclusive(-1).keyboard(keyboard);
    match output {
        Some(name) => cfg.output(name),
        None => cfg,
    }
}

impl App {
    /// Open the overlay's window, unless it is.
    pub(super) fn open_overlay(&mut self, cx: &mut AreaCx<()>) {
        if self.overlay.open {
            return;
        }
        let cfg = layer(self.output.as_deref(), self.keyboard());
        let thumbs = self.overlay.thumbs.take();
        self.overlay = Overlay { open: true, sent: Some(cfg.clone()), thumbs, ..Overlay::default() };
        cx.request(ShellRequest::OpenWindow(NewWindow::single("Overlay", "Overlay").layer(cfg)));
    }

    /// The overlay's window went: nothing is up in it any more.
    pub(super) fn overlay_closed(&mut self) {
        self.overlay.open = false;
        self.overlay.closing = false;
        self.launcher.close();
        self.dock.close_popup();
        self.tray.close();
        self.monitors.close();
        self.calendar.close();
        self.calendar.tip = false;
        self.tell_thumbs(Vec::new(), 1.0);
        // The bar, which may not be drawing, puts its lit icon out.
        self.wake();
    }

    /// How the overlay takes the keyboard for what is up: the launcher's
    /// search, a menu's, a tray or monitor popup's Escape and the
    /// calendar's typing want it; a card leaves it where it is.
    fn keyboard(&self) -> KeyboardMode {
        let popup = self.tray.popup.is_some() || self.monitors.popup.is_some() || self.calendar.popup;
        match (&self.dock.popup, self.launcher.open, popup) {
            (_, true, _) | (Some(Popup::Menu { .. }), _, _) | (_, _, true) => KeyboardMode::Exclusive,
            _ if self.tray.app_menu.is_some() => KeyboardMode::Exclusive,
            _ => KeyboardMode::None,
        }
    }

    /// Where the bar is, for what opens above it: the floating plate's
    /// left inset and how much of the monitor's bottom it keeps, logical.
    fn place(&self) -> Place {
        let shape = self.settings.shape;
        match self.docked {
            true => Place { left: 8.0, clear: f64::from(shape.height) },
            false => Place { left: f64::from(shape.sides.max(8)), clear: f64::from(shape.height + shape.below) },
        }
    }

    /// The overlay's frame: whichever of its parts is up, and its surface
    /// set for that; with nothing up, it closes.
    pub(super) fn overlay_frame(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        if self.overlay.closing {
            return;
        }
        // Only this frame's panels count.
        glass::take();
        self.dock.refresh(&self.link);
        let popup = self.dock.popup.clone();
        let region = if self.launcher.open {
            self.tell_thumbs(Vec::new(), 1.0);
            self.launcher_frame(ui, cx);
            None
        } else if let Some(item) = self.tray.popup {
            self.tell_thumbs(Vec::new(), 1.0);
            Some(self.tray_frame(ui, item))
        } else if self.tray.app_menu.is_some() {
            self.tell_thumbs(Vec::new(), 1.0);
            self.app_menu_frame(ui, cx);
            None
        } else if let Some(kind) = self.monitors.popup {
            self.tell_thumbs(Vec::new(), 1.0);
            Some(self.monitor_frame(ui, kind))
        } else if self.calendar.up() {
            self.tell_thumbs(Vec::new(), 1.0);
            Some(self.calendar_frame(ui))
        } else {
            match self.dock.popup.clone() {
                Some(Popup::Card(key)) => self.card_frame(ui, &key),
                Some(Popup::Menu { key, asked }) => {
                    self.tell_thumbs(Vec::new(), 1.0);
                    if !asked {
                        self.ask_menu(ui, &key, cx);
                    }
                    None
                }
                None => None,
            }
        };
        if self.dock.popup != popup {
            self.wake();
        }
        // The panels painted see-through: the compositor blurs behind them.
        let panes = glass::take();
        self.told.tell(&self.link, "overlay blur", glass::line(NAMESPACE, &panes, Vec2::splat(ui.m.scale)));
        if !self.launcher.open && self.dock.popup.is_none() && self.tray.popup.is_none() && self.tray.app_menu.is_none() && self.monitors.popup.is_none() && !self.calendar.up() {
            self.overlay.closing = true;
            self.told.tell(&self.link, "overlay blur", glass::line(NAMESPACE, &[], Vec2::splat(1.0)));
            self.tell_thumbs(Vec::new(), 1.0);
            cx.request(ShellRequest::CloseWindow);
            return;
        }
        let cfg = layer(self.output.as_deref(), self.keyboard());
        if self.overlay.sent.as_ref() != Some(&cfg) {
            cx.request(ShellRequest::SetLayer(cfg.clone()));
            self.overlay.sent = Some(cfg);
        }
        if self.overlay.region.as_ref() != Some(&region) {
            cx.request(ShellRequest::SetInputRegion(region.clone()));
            self.overlay.region = Some(region);
        }
    }

    /// After the overlay's rebuild: a dock menu the shell no longer has
    /// was chosen from, Escaped or clicked off, so the overlay goes with
    /// it. Returns whether to rebuild again (to close).
    pub(super) fn after_overlay(&mut self, shell: &Shell<Self>) -> bool {
        let asked = matches!(self.dock.popup, Some(Popup::Menu { asked: true, .. }));
        if asked && !shell.popup_open() {
            self.dock.close_popup();
            self.wake();
            return true;
        }
        // An app's menu chosen from, Escaped or clicked off.
        if self.tray.app_menu.as_ref().is_some_and(|m| m.shown) && !shell.popup_open() {
            self.tray.app_menu = None;
            self.wake();
            return true;
        }
        false
    }

    /// An app's menu: shown once it comes from the app, over its icon. An
    /// app with nothing in it, or none answering in a while, and the
    /// overlay goes.
    fn app_menu_frame(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        let Some(asked) = self.tray.app_menu.clone().filter(|m| !m.shown) else { return };
        let state = self.tray.apps.as_ref().map(lntrn_status::Apps::state).unwrap_or_default();
        let menu = state.menu.as_ref().filter(|m| m.ask == asked.ask && m.key == asked.key);
        let item = state.items.iter().find(|i| i.key == asked.key);
        match (menu, item) {
            (Some(menu), Some(item)) if !menu.entries.is_empty() => {
                let m = ui.m;
                let h = (menu.entries.len() as f64 + 1.0) * m.widget_h + m.pad * 4.0;
                let x = self.tray.app_face(&asked.key).map_or(ui.state.pointer.x, |f| f.min.x);
                let at = Vec2::new(x, ui.clip().max.y - m.px(self.place().clear + ABOVE_BAR) - h);
                cx.request(ShellRequest::ContextMenu(Box::new(tray::apps::menu(item, &menu.entries, at))));
                if let Some(m) = &mut self.tray.app_menu {
                    m.shown = true;
                }
            }
            _ if menu.is_some() || asked.at.elapsed() > APP_MENU_WAIT => {
                self.tray.app_menu = None;
                self.wake();
            }
            // Its answer wakes the loop; this is in case it never comes.
            _ => ui.state.request_redraw_after(0.25),
        }
    }

    /// The launcher's menu, above the bar's left end wherever the bar is.
    fn launcher_frame(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        let place = self.place();
        let close = match launcher::menu(&mut self.launcher, &mut self.icons, ui, place, self.look) {
            Outcome::Stay => false,
            Outcome::Close => true,
            Outcome::Launch(command) => {
                actions::launch(&command);
                true
            }
            Outcome::Menu(index) => {
                if let Some(app) = self.launcher.app(index) {
                    let menu = actions::app_menu(app, self.launcher.is_favorite(&app.id), self.dock.is_pinned(&app.id), ui.state.pointer);
                    cx.request(ShellRequest::ContextMenu(Box::new(menu)));
                }
                false
            }
            Outcome::Power(action) => {
                action.run();
                true
            }
            Outcome::Layout(layout) => {
                launcher::keep(layout);
                false
            }
            // An emoji: into the window last worked in, the compositor
            // typing it while the menu keeps the keyboard.
            Outcome::Type(text) => {
                self.link.send(&format!("type:{text}"));
                false
            }
            Outcome::Clip(op) => {
                self.launcher.run_clip(op);
                // Copied back: off to paste it.
                matches!(op, ClipOp::Copy(_))
            }
            Outcome::ClipMenu(id) => {
                if let Some(entry) = self.launcher.clip_entry(id) {
                    cx.request(ShellRequest::ContextMenu(Box::new(actions::clip_menu(&entry, ui.state.pointer))));
                }
                false
            }
            Outcome::QuickKill => {
                log_info!("{APP_ID}: Quick Kill: starting over");
                RESTART.store(true, Ordering::Release);
                cx.request(ShellRequest::Quit);
                false
            }
        };
        if close {
            self.launcher.close();
            self.wake();
        }
    }

    /// A tray item's popup above its face. Returns where the overlay takes
    /// the pointer: everywhere but the bar's plate, which stays the bar's.
    fn tray_frame(&mut self, ui: &mut Ui, item: Item) -> Vec<Rect> {
        let (m, surface) = (ui.m, ui.clip());
        let x = self.tray.face(item).map_or(surface.max.x - m.px(240.0), |face| face.center().x);
        let frame = tray::popup::Frame { x, bottom: surface.max.y - m.px(self.place().clear + ABOVE_BAR), look: self.look };
        if tray::popup::draw(&mut self.tray, ui, item, &frame).is_none() {
            self.tray.close();
            self.wake();
        }
        self.off_plate(surface, m.scale)
    }

    /// A mini monitor's popup above its face. Returns where the overlay
    /// takes the pointer: everywhere but the bar's plate, as a tray
    /// popup's does.
    fn monitor_frame(&mut self, ui: &mut Ui, kind: Kind) -> Vec<Rect> {
        let (m, surface) = (ui.m, ui.clip());
        let x = self.monitors.face(kind).map_or(surface.min.x + m.px(240.0), |face| face.center().x);
        let frame = tray::popup::Frame { x, bottom: surface.max.y - m.px(self.place().clear + ABOVE_BAR), look: self.look };
        let popup = monitors::popup::draw(&self.monitors, ui, kind, &frame);
        if popup.sysmon {
            self.open_sysmon(kind);
        }
        if popup.panel.is_none() || popup.sysmon {
            self.monitors.close();
            self.wake();
        }
        self.off_plate(surface, m.scale)
    }

    /// The calendar's popup above the clock, or the clock's card. Returns
    /// where the overlay takes the pointer: the popup's, everywhere but
    /// the bar's plate, as a tray popup's does; the card's, nowhere.
    fn calendar_frame(&mut self, ui: &mut Ui) -> Vec<Rect> {
        let (m, surface) = (ui.m, ui.clip());
        let x = self.calendar.face().map_or(surface.max.x - m.px(240.0), |face| face.center().x);
        let frame = tray::popup::Frame { x, bottom: surface.max.y - m.px(self.place().clear + ABOVE_BAR), look: self.look };
        if !self.calendar.popup {
            calendar::face::tip(&self.calendar, ui, &frame, self.settings.clock);
            return Vec::new();
        }
        let was = self.calendar.revision();
        if calendar::popup::draw(&mut self.calendar, ui, &frame, self.settings.clock).is_none() {
            self.calendar.close();
            self.wake();
        }
        // The bar's dot shows the change.
        if self.calendar.revision() != was {
            self.wake();
        }
        // Its clock turns over, and its day at midnight.
        if let Some(soon) = clock::redraw_after(self.settings.clock) {
            ui.state.request_redraw_after(soon);
        }
        self.off_plate(surface, m.scale)
    }

    /// The System Monitor: brought forward where it is open (its newest
    /// window), else started on `kind`'s page.
    fn open_sysmon(&self, kind: Kind) {
        match self.link.windows().into_iter().filter(|w| w.app_id == SYSMON).max_by_key(|w| w.id) {
            Some(window) => self.link.send(&format!("activate:{}", window.id)),
            None => {
                if let Err(e) = lntrn_apps::launch::run_args(SYSMON, &["--page", kind.page()]) {
                    lntrn_core::log_warn!("{SYSMON}: {e}");
                }
            }
        }
    }

    /// The overlay's `surface` less the bar's plate as it sits now (`per`
    /// pixels a logical one): up to four rects round it.
    fn off_plate(&self, surface: Rect, per: f64) -> Vec<Rect> {
        let shape = self.settings.shape;
        let px = |v: u32| (f64::from(v) * per).round();
        let (left, right, top, bottom) = if self.docked {
            (surface.min.x, surface.max.x, surface.max.y - px(shape.height), surface.max.y)
        } else {
            (surface.min.x + px(shape.sides), surface.max.x - px(shape.sides), surface.max.y - px(shape.height + shape.below), surface.max.y - px(shape.below))
        };
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| Rect::new(Vec2::new(x0, y0), Vec2::new(x1, y1));
        [rect(surface.min.x, surface.min.y, surface.max.x, top), rect(surface.min.x, top, left, surface.max.y), rect(right, top, surface.max.x, surface.max.y), rect(left, bottom, right, surface.max.y)]
            .into_iter()
            .filter(|r| r.width() > 0.0 && r.height() > 0.0)
            .collect()
    }

    /// An app's card above its icon. Returns where the overlay takes the
    /// pointer: the card alone.
    fn card_frame(&mut self, ui: &mut Ui, key: &str) -> Option<Vec<Rect>> {
        let (m, surface) = (ui.m, ui.clip());
        let clear = m.px(self.place().clear);
        // The work area's shape: a maximized window fills its thumbnail.
        let aspect = surface.width() / (surface.height() - clear).max(1.0);
        let card = dock::card(&mut self.dock, ui, key, surface.max.y - clear - m.px(ABOVE_BAR), aspect, self.look);
        match card.press {
            Some(CardPress::Activate(id)) => {
                self.link.send(&format!("activate:{id}"));
                self.dock.close_popup();
            }
            Some(CardPress::Close(id)) => self.link.send(&format!("close:{id}")),
            None => {}
        }
        self.tell_thumbs(card.thumbs, m.scale);
        if let Some(wait) = self.dock.step(ui.now()) {
            ui.state.request_redraw_after(wait);
        }
        match card.rect {
            Some(rect) => Some(vec![rect]),
            // The app or its icon gone: nothing to show.
            None => {
                self.dock.close_popup();
                Some(Vec::new())
            }
        }
    }

    /// Ask the shell for the dock's menu of the app with this key, above
    /// its icon.
    fn ask_menu(&mut self, ui: &mut Ui, key: &str, cx: &mut AreaCx<()>) {
        let Some(entry) = self.dock.entry(key).cloned() else {
            self.dock.close_popup();
            return;
        };
        // Freshly from its file: the launcher may not have read it yet.
        self.launcher.reload_favorites();
        let id = entry.app.as_ref().map_or("", |a| a.id.as_str());
        let (favorite, pinned) = (self.launcher.is_favorite(id), self.dock.is_pinned(id));
        let menu = actions::dock_menu(&entry, favorite, pinned, self.menu_at(ui, &entry, key));
        cx.request(ShellRequest::ContextMenu(Box::new(menu)));
        self.dock.popup = Some(Popup::Menu { key: key.to_owned(), asked: true });
    }

    /// Where the dock's menu goes (its top left, window pixels): over its
    /// icon, its bottom just above the bar. Its height is a guess until it
    /// is drawn, so the guess is generous.
    fn menu_at(&self, ui: &Ui, entry: &Entry, key: &str) -> Vec2 {
        let m = ui.m;
        let rows = actions::dock_menu_rows(entry) as f64 + 1.0;
        let h = rows * m.widget_h + m.pad * 4.0;
        let x = self.dock.face(key).map_or(ui.state.pointer.x, |f| f.min.x);
        Vec2::new(x, ui.clip().max.y - m.px(self.place().clear + ABOVE_BAR) - h)
    }

    /// Tell the compositor which windows to paint live where over the
    /// overlay, and their red X's (`thumbs:` in its logical pixels, `scale`
    /// its pixels per logical one), when that changed: none clears them.
    pub(super) fn tell_thumbs(&mut self, thumbs: Vec<Thumb>, scale: f64) {
        let l = |v: f64| (v / scale).round() as i64;
        let rect = |r: Rect| format!("{} {} {} {}", l(r.min.x), l(r.min.y), l(r.width()), l(r.height()));
        let mut line = format!("thumbs:{NAMESPACE}");
        for t in &thumbs {
            line.push_str(&format!("\t{} {} {} {}", t.id, rect(t.rect), rect(t.close), u8::from(t.close_hovered)));
        }
        let told = (self.link.connection(), line);
        // Nothing to clear on a connection never told any.
        let fresh = self.overlay.thumbs.as_ref().is_none_or(|(c, _)| *c != told.0);
        if self.overlay.thumbs.as_ref() == Some(&told) || (fresh && thumbs.is_empty()) {
            return;
        }
        self.link.send(&told.1);
        self.overlay.thumbs = Some(told);
    }
}
