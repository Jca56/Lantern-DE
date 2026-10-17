//! The overlay: a layer surface over the bar's whole monitor, open while
//! something is up above the bar (the launcher's menu, or the dock's card
//! or menu) and closed as soon as nothing is. What is up decides how it
//! takes input: the launcher and a menu take the keyboard and every click
//! (a click off them puts them away); a card takes the pointer over itself
//! alone, from the overlay's first frame on, so the bar under it never
//! loses the pointer resting on its icon.

use std::sync::atomic::Ordering;

use lntrn_core::log_info;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Anchor, AreaCx, KeyboardMode, LayerConfig, NewWindow, Shell, ShellRequest, Ui};

use super::{APP_ID, App, RESTART, actions};
use crate::dock::{self, CardPress, Entry, Popup, Thumb};
use crate::launcher::{self, Outcome, Place};

/// What the compositor knows the overlay's surface as.
pub const NAMESPACE: &str = "lntrn-control-center-overlay";
/// Between the card or the dock's menu and the bar, logical.
const ABOVE_BAR: f64 = 10.0;

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
        self.tell_thumbs(Vec::new(), 1.0);
        // The bar, which may not be drawing, puts its lit icon out.
        self.wake();
    }

    /// How the overlay takes the keyboard for what is up: the launcher's
    /// search and a menu's Escape want it; a card leaves it where it is.
    fn keyboard(&self) -> KeyboardMode {
        match (&self.dock.popup, self.launcher.open) {
            (_, true) | (Some(Popup::Menu { .. }), _) => KeyboardMode::Exclusive,
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
        self.dock.refresh(&self.link);
        let popup = self.dock.popup.clone();
        let region = if self.launcher.open {
            self.tell_thumbs(Vec::new(), 1.0);
            self.launcher_frame(ui, cx);
            None
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
        if !self.launcher.open && self.dock.popup.is_none() {
            self.overlay.closing = true;
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
        false
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
