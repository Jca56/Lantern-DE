//! The app as the shell sees it: the bar, filling a layer surface of its
//! own, and the launcher's menu, in a second one while it is up. It
//! follows `lantern.toml` as it changes: the look and the shape at once, a
//! new primary monitor by starting over on it (a surface can't change
//! monitors). It docks while the compositor says a window fills its
//! monitor, and floats again when none does; with auto hide it also slides
//! out of sight and back (`hide`). Every frame works out the surface the
//! bar wants (its height, the space it keeps from windows) and asks the
//! compositor only when that changed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_core::{log_info, log_warn};
use lntrn_math::{Rect, Vec2};
use lntrn_props::Value;
use lntrn_ui::{Action, AreaCx, ContextMenu, Host, HostCx, Item, LayerConfig, NewWindow, Shell, ShellRequest, Ui};

use crate::bar::{self, Look};
use crate::clock;
use crate::dock;
use crate::hide::{self, Reveal};
use crate::launcher::{self, Launcher, Outcome, Place};
use crate::morph::{Glide, Morph};
use crate::settings::{self, Settings};

pub const APP_ID: &str = "lntrn-control-center";

/// The app menu's rows, as actions.
const OPEN: &str = "launcher.open";
const FAVORITE: &str = "launcher.favorite";

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
    /// The launcher's menu, in a window of its own.
    Launcher,
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
    /// Kept by the dock thread: whether a window fills our monitor.
    covered: Arc<AtomicBool>,
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
    /// The loop's: closing the menu wakes the bar, so its button goes dark.
    waker: Option<Waker>,
}

impl App {
    pub fn new(settings: Settings, output: Option<String>) -> Self {
        let look = Look::of(&settings.desktop);
        // As `main` makes the surface.
        let sent = bar::layer(settings.shape, false, output.as_deref());
        Self { settings, look, output, changed: Arc::new(AtomicBool::new(false)), covered: Arc::new(AtomicBool::new(false)), docked: false, morph: Morph::at(false), reveal: Reveal::new(), sent, region: None, launcher: Launcher::new(), waker: None }
    }

    /// Head for docked or floating when the compositor's word changed. The
    /// zone goes over at once (the frame's surface follows `docked`), so
    /// windows fill or give way while the plate glides to meet them:
    /// docking as long as the window's own maximize glide, floating again
    /// quickly, at once under Reduce Motion.
    fn dock(&mut self, now: f64, reduce_motion: bool) {
        let docked = self.covered.load(Ordering::Acquire);
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
    /// left, the clock at its right; out of sight, nothing but the strip
    /// along the bottom edge that brings it back.
    fn bar(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        let restyled = self.follow(cx);
        let now = ui.now();
        let (m, surface, shape, hide) = (ui.m, ui.clip(), self.settings.shape, self.settings.hide);
        self.dock(now, ui.state.reduce_motion);
        let want = !hide.auto() || ui.state.pointer_in_window || self.launcher.open;
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
        let zone = hide::zone(shown, travel / bar::per_logical(surface, m, shape), bar::zone(shape, self.docked));
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
            return restyled;
        }
        bar::draw(ui, self.look, plate);
        let (clicked, _) = launcher::button(ui, plate.rect, self.look.plate, self.launcher.open);
        if clicked && !self.launcher.open {
            self.launcher.open();
            cx.request(ShellRequest::OpenWindow(NewWindow::single("Launcher", "Launcher").layer(Launcher::layer(self.output.as_deref()))));
            // Again, with the button lit.
            cx.rebuild();
        }
        if let Some(now) = clock::Local::now() {
            clock::draw(ui, plate.rect, self.settings.clock, &now);
        }
        if let Some(soon) = clock::redraw_after(self.settings.clock) {
            ui.state.request_redraw_after(soon);
        }
        restyled
    }

    /// The launcher's menu, above the bar's left end wherever the bar is.
    fn menu(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        let shape = self.settings.shape;
        let place = if self.docked { Place { left: 8.0, clear: f64::from(shape.height) } } else { Place { left: f64::from(shape.sides.max(8)), clear: f64::from(shape.height + shape.below) } };
        let close = match launcher::menu(&mut self.launcher, ui, place, self.look) {
            Outcome::Stay => false,
            Outcome::Close => true,
            Outcome::Launch(command) => {
                launch(&command);
                true
            }
            Outcome::Menu(index) => {
                if let Some(app) = self.launcher.app(index) {
                    let menu = app_menu(app, self.launcher.is_favorite(&app.id), ui.state.pointer);
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
            self.close_launcher();
            cx.request(ShellRequest::CloseWindow);
        }
    }

    /// The menu is going: the bar, which isn't drawing, is woken to put
    /// its button out.
    fn close_launcher(&mut self) {
        self.launcher.close();
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
            Editor::Launcher => "Launcher",
        }
    }

    fn editor_id(&self, editor: Editor) -> String {
        self.editor_label(editor).to_owned()
    }

    fn editor_from_id(&self, id: &str) -> Option<Editor> {
        match id {
            "Bar" => Some(Editor::Bar),
            "Launcher" => Some(Editor::Launcher),
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
            Editor::Launcher => {
                self.menu(ui, cx);
                false
            }
        }
    }

    /// A row of an app's menu.
    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        match (action.id.as_str(), action.arg("app"), action.arg("command")) {
            (OPEN, _, Some(Value::Str(command))) => {
                launch(command);
                cx.request(ShellRequest::CloseWindow);
            }
            (FAVORITE, Some(Value::Str(id)), _) => {
                self.launcher.toggle_favorite(id);
                cx.rebuild();
            }
            _ => {}
        }
    }

    /// The menu's window going (the compositor took it away, say) leaves
    /// the launcher closed. The bar's going is the compositor taking its
    /// surface away (its monitor went): it quits, as any main window's
    /// does, and `main` says why as it exits.
    fn close_requested(&mut self, main: bool, _cx: &mut HostCx) -> bool {
        if main {
            TAKEN.store(true, Ordering::Release);
        } else {
            self.close_launcher();
        }
        true
    }
}

/// Start an app with its desktop entry's `command`.
fn launch(command: &str) {
    match lntrn_apps::launch::run(command) {
        Ok(()) => log_info!("launcher: started {command}"),
        Err(e) => log_warn!("launcher: {command}: {e}"),
    }
}

/// What can be done with `app`: the menu a right-click on it opens at
/// `at`, under its name.
fn app_menu(app: &lntrn_apps::App, favorite: bool, at: Vec2) -> ContextMenu {
    let star = if favorite { "Remove from Favorites" } else { "Add to Favorites" };
    ContextMenu::new(&app.name, at).tab(
        "App",
        vec![Item::action("Open", Action::new(OPEN).with("command", Value::Str(app.command.clone()))), Item::action(star, Action::new(FAVORITE).with("app", Value::Str(app.id.clone())))],
    )
}

impl AppHost for App {
    fn waker(&mut self, waker: Waker) {
        settings::watch(self.changed.clone(), waker.clone());
        clock::watch(waker.clone());
        self.launcher.icons.set_waker(waker.clone());
        self.waker = Some(waker.clone());
        dock::watch(self.output.clone(), self.covered.clone(), waker);
    }

    /// Icons the launcher's thread rendered go up to the GPU.
    fn after_rebuild(&mut self, gpu: &Gpu, images: &mut Images, _shell: &mut Shell<Self>) -> bool {
        self.launcher.icons.upload(gpu, images)
    }
}
