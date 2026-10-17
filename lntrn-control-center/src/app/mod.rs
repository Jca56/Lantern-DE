//! The app as the shell sees it: the bar, filling a layer surface of its
//! own, and the overlay, a second one over its monitor while something is
//! up there: the launcher's menu, or the dock's card or menu (`overlay`).
//! It follows `lantern.toml` as it changes: the look and the shape at
//! once, a new primary monitor by starting over on it (a surface can't
//! change monitors). It docks while the compositor says a window fills
//! its monitor, and floats again when none does; with auto hide it also
//! slides out of sight and back (`hide`). Every frame works out the
//! surface the bar wants (its height, the space it keeps from windows) and
//! asks the compositor only when that changed.

mod actions;
mod overlay;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_apps::Icons;
use lntrn_core::log_info;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Action, AreaCx, Host, HostCx, LayerConfig, Shell, ShellRequest, Ui};

use crate::bar::{self, Look};
use crate::clock;
use crate::compositor::Link;
use crate::dock::{self, Dock, Press};
use crate::hide::{self, Reveal};
use crate::launcher::{self, Launcher};
use crate::morph::{Glide, Morph};
use crate::settings::{self, Settings};

pub const APP_ID: &str = "lntrn-control-center";
/// Pixels an icon is rendered at: sharp at the launcher's list and the
/// dock on a dense screen.
const ICON_PX: u32 = 96;
/// Between the dock and the launcher's button or the clock, logical.
const DOCK_CLEAR: f64 = 16.0;

/// Set when the loop ends only to start over: on another monitor, or
/// Quick Kill's fresh start.
static RESTART: AtomicBool = AtomicBool::new(false);
/// Set when the compositor took the bar's surface away (its monitor went).
static TAKEN: AtomicBool = AtomicBool::new(false);

/// Whether the app quit to start over (see [`crate::restart`]).
pub fn restart_wanted() -> bool {
    RESTART.load(Ordering::Acquire)
}

/// Whether the app quit because the compositor took the bar away, not
/// because it chose to.
pub fn taken_away() -> bool {
    TAKEN.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Bar,
    /// The overlay, in a window of its own.
    Overlay,
}

const EDITORS: [Editor; 1] = [Editor::Bar];

pub struct App {
    settings: Settings,
    look: Look,
    /// The monitor the surface was made on, `None` for the compositor's
    /// pick. Fixed for its life (U086).
    output: Option<String>,
    /// Raised by the watcher thread when `lantern.toml` changes.
    changed: Arc<AtomicBool>,
    /// The line to the compositor: whether to dock, the open windows, and
    /// the dock's commands.
    link: Link,
    /// Whether the bar is docked, or on its way there.
    docked: bool,
    morph: Morph,
    /// Auto hide's slide, and the surface it takes.
    reveal: Reveal,
    /// What the compositor was last told of the surface.
    sent: LayerConfig,
    /// Where the surface last took input, in physical pixels.
    region: Option<Vec<Rect>>,
    launcher: Launcher,
    dock: Dock,
    /// Every app's picture, the launcher's and the dock's alike.
    icons: Icons,
    overlay: overlay::Overlay,
    /// The dock's icons as the compositor was last told them, with the
    /// connection it was told on (a new one forgets them).
    icons_sent: Option<(u64, String)>,
    /// The loop's: what one window changes, the other is woken to show.
    waker: Option<Waker>,
}

impl App {
    pub fn new(settings: Settings, output: Option<String>) -> Self {
        let look = Look::of(&settings.desktop);
        // As `main` makes the surface.
        let sent = bar::layer(settings.shape, false, output.as_deref());
        let dock = Dock::new(lntrn_apps::installed());
        Self { settings, look, output, changed: Arc::new(AtomicBool::new(false)), link: Link::new(), docked: false, morph: Morph::at(false), reveal: Reveal::new(), sent, region: None, launcher: Launcher::new(), dock, icons: Icons::new(ICON_PX), overlay: overlay::Overlay::default(), icons_sent: None, waker: None }
    }

    /// Head for docked or floating when the compositor's word changed. The
    /// zone goes over at once (the frame's surface follows `docked`), so
    /// windows fill or give way while the plate glides to meet them:
    /// docking as long as the window's own maximize glide, floating again
    /// quickly, at once under Reduce Motion.
    fn follow_cover(&mut self, now: f64, reduce_motion: bool) {
        let docked = self.link.covered();
        if docked == self.docked {
            return;
        }
        self.docked = docked;
        let glide = match (reduce_motion, docked) {
            (true, _) => Glide::NONE,
            (false, true) => self.settings.dock_glide,
            (false, false) => Glide::UNDOCK,
        };
        self.morph.aim(docked, now, glide);
    }

    /// Tell the compositor `cfg` when the surface isn't that already.
    fn set_layer(&mut self, cfg: LayerConfig, cx: &mut AreaCx<()>) {
        if self.sent != cfg {
            cx.request(ShellRequest::SetLayer(cfg.clone()));
            self.sent = cfg;
        }
    }

    /// The bar's own surface: the plate, the launcher's button at its
    /// left, the clock at its right, the dock between; out of sight,
    /// nothing but the strip along the bottom edge that brings it back.
    fn bar(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        let restyled = self.follow(cx);
        let now = ui.now();
        let (m, surface, shape, hide) = (ui.m, ui.clip(), self.settings.shape, self.settings.hide);
        self.follow_cover(now, ui.state.reduce_motion);
        self.dock.refresh(&self.link);
        let want = !hide.auto() || ui.state.pointer_in_window || self.overlay.open;
        let grown = hide::grown(surface, m);
        if let Some(wait) = self.reveal.step(now, want, grown, hide.glide(ui.state.reduce_motion), hide.delay) {
            ui.state.request_redraw_after(wait);
        }
        if self.morph.moving(now) || self.reveal.moving(now) {
            // Every frame until it lands: the present waits for vsync.
            ui.state.request_redraw_after(0.0);
        }
        let shown = if grown { self.reveal.value(now) } else { 0.0 };
        let plate = bar::plate(surface, m, shape, self.morph.value(now));
        // All of it up, how far the plate's top is from the bottom edge;
        // the windows give way to as much of that as shows.
        let travel = surface.max.y - plate.rect.min.y;
        let per = bar::per_logical(surface, m, shape);
        let zone = hide::zone(shown, travel / per, bar::zone(shape, self.docked));
        let plate = plate.lowered(((1.0 - shown) * travel).round());
        self.set_layer(bar::layer_with(shape, self.docked, self.output.as_deref(), self.reveal.full(), zone), cx);
        // Clicks off the plate go to whatever is under it; with auto hide,
        // the edge strip keeps the bar up under a pointer resting there.
        let mut region = Vec::new();
        let on_screen = plate.rect.intersection(&surface);
        if grown && on_screen.width() > 0.0 && on_screen.height() > 0.0 {
            region.push(on_screen);
        }
        if grown && hide.auto() {
            region.push(hide::strip(surface, m));
        }
        if region.is_empty() {
            region.push(surface);
        }
        if self.region.as_ref() != Some(&region) {
            cx.request(ShellRequest::SetInputRegion(Some(region.clone())));
            self.region = Some(region);
        }
        if shown <= 0.0 {
            self.dock.away();
            return restyled;
        }
        bar::draw(ui, self.look, plate);
        let (clicked, button) = launcher::button(ui, plate.rect, self.look.plate, self.launcher.open);
        if clicked && !self.launcher.open {
            self.open_launcher(cx);
        }
        let clock = clock::Local::now().map(|now| clock::draw(ui, plate.rect, self.settings.clock, &now));
        if let Some(soon) = clock::redraw_after(self.settings.clock) {
            ui.state.request_redraw_after(soon);
        }
        // The dock, between the button and the clock.
        let popup = self.dock.popup.clone();
        let clear = m.px(DOCK_CLEAR);
        let right = clock.map_or(plate.rect.max.x, |c| c.min.x) - clear;
        let span = Rect::new(Vec2::new(button.max.x + clear, plate.rect.min.y), Vec2::new(right, plate.rect.max.y));
        if let Some(press) = dock::draw(&mut self.dock, ui, &mut self.icons, span, plate.rect, self.look.plate, now) {
            self.press(press);
        }
        // No card under the launcher's menu.
        if self.launcher.open {
            self.dock.close_popup();
        } else if let Some(wait) = self.dock.step(now) {
            ui.state.request_redraw_after(wait);
        }
        self.tell_icons(per);
        if self.dock.popup != popup {
            self.popup_changed(cx);
        }
        restyled
    }

    /// What a press on the dock asks for.
    fn press(&mut self, press: Press) {
        match press {
            Press::Launch(command) => actions::launch(&command),
            Press::Activate(id) => {
                self.link.send(&format!("activate:{id}"));
                self.dock.close_popup();
            }
            Press::Minimize(id) => self.link.send(&format!("minimize:{id}")),
            Press::Card(key) => self.dock.open_card(&key),
            Press::Menu(key) => self.dock.open_menu(&key),
        }
    }

    /// Tell the compositor where the dock's icons are, when that changed
    /// or it hasn't been told on this connection.
    fn tell_icons(&mut self, per: f64) {
        let line = self.dock.icons_line(bar::NAMESPACE, per);
        let told = (self.link.connection(), line);
        if self.icons_sent.as_ref() != Some(&told) {
            self.link.send(&told.1);
            self.icons_sent = Some(told);
        }
    }

    /// The dock's card or menu came or went: the overlay opens for it (or
    /// closes itself, woken), and the bar shows the icon lit or not.
    fn popup_changed(&mut self, cx: &mut AreaCx<()>) {
        if self.dock.popup.is_some() {
            self.open_overlay(cx);
        }
        self.wake();
    }

    fn open_launcher(&mut self, cx: &mut AreaCx<()>) {
        let apps = lntrn_apps::installed();
        self.dock.set_apps(apps.clone());
        self.dock.close_popup();
        self.launcher.open(apps);
        self.open_overlay(cx);
        // Again, with the button lit.
        cx.rebuild();
    }

    /// The line to the compositor, for the tests to talk on.
    #[cfg(test)]
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Wake the loop, so every window draws what changed.
    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker.wake();
        }
    }

    /// Take in a change to `lantern.toml`. Returns `true` when the theme
    /// changed, so the frame is rebuilt with it.
    fn follow(&mut self, cx: &mut AreaCx<()>) -> bool {
        if !self.changed.swap(false, Ordering::AcqRel) {
            return false;
        }
        let Some(new) = Settings::read() else { return false };
        // Only toward a monitor that is there: a primary unplugged or
        // switched off leaves the bar where it is.
        let home = bar::home(new.primary.as_deref());
        if home.is_some() && home != self.output {
            log_info!("{APP_ID}: the primary monitor is now {home:?}, was {:?}: starting over there", self.output);
            RESTART.store(true, Ordering::Release);
            cx.request(ShellRequest::Quit);
            return false;
        }
        self.look = Look::of(&new.desktop);
        let theme = bar::theme(&new.desktop);
        let restyled = cx.prefs.theme != theme;
        if restyled {
            cx.prefs.theme = theme;
        }
        self.settings = new;
        restyled
    }
}

impl Host for App {
    type Editor = Editor;
    type AreaState = ();

    fn editors(&self) -> &[Editor] {
        &EDITORS
    }

    fn editor_label(&self, editor: Editor) -> &str {
        match editor {
            Editor::Bar => "Bar",
            Editor::Overlay => "Overlay",
        }
    }

    fn editor_id(&self, editor: Editor) -> String {
        self.editor_label(editor).to_owned()
    }

    fn editor_from_id(&self, id: &str) -> Option<Editor> {
        match id {
            "Bar" => Some(Editor::Bar),
            "Overlay" => Some(Editor::Overlay),
            _ => None,
        }
    }

    fn title(&self) -> String {
        "Control Center".to_owned()
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        false
    }

    fn paints_body(&self, _editor: Editor) -> bool {
        true
    }

    fn draw_body(&mut self, editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        match editor {
            Editor::Bar => self.bar(ui, cx),
            Editor::Overlay => {
                self.overlay_frame(ui, cx);
                false
            }
        }
    }

    /// A row of a menu: the launcher's or the dock's.
    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        self.act(action, cx);
    }

    /// The overlay's window going (we closed it, or the compositor took it
    /// away) leaves nothing up in it. The bar's going is the compositor
    /// taking its surface away (its monitor went): it quits, as any main
    /// window's does, and `main` says why as it exits.
    fn close_requested(&mut self, main: bool, _cx: &mut HostCx) -> bool {
        if main {
            TAKEN.store(true, Ordering::Release);
        } else {
            self.overlay_closed();
        }
        true
    }
}

impl AppHost for App {
    fn waker(&mut self, waker: Waker) {
        settings::watch(self.changed.clone(), waker.clone());
        clock::watch(waker.clone());
        self.icons.set_waker(waker.clone());
        self.link.start(self.output.clone(), waker.clone());
        self.waker = Some(waker);
    }

    /// Icons the icon thread rendered go up to the GPU; after the
    /// overlay's rebuild, a dock menu that went takes the overlay with it.
    fn after_rebuild(&mut self, gpu: &Gpu, images: &mut Images, shell: &mut Shell<Self>) -> bool {
        let again = shell.title.is_some() && self.after_overlay(shell);
        self.icons.upload(gpu, images) || again
    }
}
