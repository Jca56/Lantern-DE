//! The app as the shell sees it: one editor, the bar, filling a layer
//! surface of its own. It follows `lantern.toml` as it changes: the look
//! and the gap at once, a new primary monitor by starting over on it (a
//! surface can't change monitors). It docks while the compositor says a
//! window fills its monitor, and floats again when none does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lntrn_app::{AppHost, Waker};
use lntrn_core::log_info;
use lntrn_math::Rect;
use lntrn_ui::{Action, AreaCx, Host, HostCx, ShellRequest, Ui};

use crate::bar::{self, Look};
use crate::dock;
use crate::morph::{Glide, Morph};
use crate::settings::{self, Settings};

pub const APP_ID: &str = "lntrn-control-center";

/// Set when the loop ends only to start over on another monitor.
static RESTART: AtomicBool = AtomicBool::new(false);

/// Whether the app quit to start over (see [`crate::restart`]).
pub fn restart_wanted() -> bool {
    RESTART.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Bar,
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
    /// Where the surface last took input: the plate, in physical pixels.
    region: Option<Rect>,
}

impl App {
    pub fn new(settings: Settings, output: Option<String>) -> Self {
        let look = Look::of(&settings.desktop);
        Self { settings, look, output, changed: Arc::new(AtomicBool::new(false)), covered: Arc::new(AtomicBool::new(false)), docked: false, morph: Morph::at(false), region: None }
    }

    /// Head for docked or floating when the compositor's word changed. The
    /// zone goes over at once, so windows fill or give way while the
    /// plate glides to meet them: docking as long as the window's own
    /// maximize glide, floating again quickly, at once under Reduce Motion.
    fn dock(&mut self, now: f64, reduce_motion: bool, cx: &mut AreaCx<()>) {
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
        cx.request(ShellRequest::SetLayer(bar::layer(self.settings.gap, docked, self.output.as_deref())));
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
        if new.gap != self.settings.gap {
            cx.request(ShellRequest::SetLayer(bar::layer(new.gap, self.docked, self.output.as_deref())));
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

    fn editor_label(&self, _editor: Editor) -> &str {
        "Bar"
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

    fn draw_body(&mut self, _editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        let restyled = self.follow(cx);
        let now = ui.now();
        self.dock(now, ui.state.reduce_motion, cx);
        if self.morph.moving(now) {
            // Every frame until it lands: the present waits for vsync.
            ui.state.request_redraw_after(0.0);
        }
        let plate = bar::plate(ui.clip(), ui.m, self.settings.gap, self.morph.value(now));
        bar::draw(ui, self.look, plate);
        // Clicks off the plate go to whatever is under it.
        if self.region != Some(plate.rect) {
            self.region = Some(plate.rect);
            cx.request(ShellRequest::SetInputRegion(Some(vec![plate.rect])));
        }
        restyled
    }

    fn run(&mut self, _action: &Action, _cx: &mut HostCx) {}
}

impl AppHost for App {
    fn waker(&mut self, waker: Waker) {
        settings::watch(self.changed.clone(), waker.clone());
        dock::watch(self.output.clone(), self.covered.clone(), waker);
    }
}
