//! Headless runs on Lantern UI's test harness, no window and no GPU: the
//! whole app typed and clicked at the way a hand would, and the document
//! (and what is on the disk) checked afterwards.
//!
//! Every test has the sandbox to itself (they share its `lantern.toml`
//! and its drafts), so they take turns.

mod files;
mod kinds;
mod page;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, Once};

use lntrn_math::{Rect, Vec2};
use lntrn_props::Value;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Action, HostCx, Key, Modifiers, Shell, WidgetId};

use crate::app::{App, Kind, Shared};
use crate::doc::Pos;
use crate::view::page::Place;

static TURN: Mutex<()> = Mutex::new(());

/// Point every folder the app reads at a scratch tree: nothing here
/// touches the real home, the real config or anyone's real notes.
fn sandbox() -> PathBuf {
    static ONCE: Once = Once::new();
    let root = std::env::temp_dir().join(format!("lntrn-notepad-smoke-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).unwrap();
        // SAFETY: set once, under the tests' lock, before anything here
        // reads them or starts a thread that does.
        unsafe {
            std::env::set_var("HOME", &root);
            std::env::set_var("LANTERN_HOME", root.join("lantern"));
            std::env::set_var("XDG_DOCUMENTS_DIR", root.join("docs"));
        }
    });
    root
}

/// The desktop's settings file, in the sandbox and nowhere else.
fn config_file() -> PathBuf {
    let path = lntrn_kit::desktop::config_path();
    assert!(path.starts_with(sandbox()), "{} is not in the sandbox", path.display());
    path
}

fn ctrl() -> Modifiers {
    Modifiers::CTRL
}

struct Rig {
    h: Harness,
    shell: Shell<App>,
    app: App,
    _turn: MutexGuard<'static, ()>,
}

impl Rig {
    /// Notepad in a window of the size the desktop opens it at, started
    /// with `files` to open. With `remembers` it keeps drafts and brings
    /// back what was open, as a real one does.
    fn start(remembers: bool, files: &[PathBuf]) -> Rig {
        Rig::start_after(remembers, files, || {})
    }

    /// The same, with `before` run first: inside the sandbox and with
    /// the tests' turn held, which is the only place a test may put
    /// files where the app will look for them.
    fn start_after(remembers: bool, files: &[PathBuf], before: impl FnOnce()) -> Rig {
        let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        sandbox();
        crate::sandboxed();
        before();
        let mut app = App::new();
        app.remembers = remembers;
        let mut shell = Shell::new(Kind::Document);
        app.restore(&mut shell, files.to_vec());
        let mut rig = Rig { h: Harness::new(1152.0, 720.0), shell, app, _turn: turn };
        rig.settle();
        rig
    }

    fn new() -> Rig {
        Rig::start(false, &[])
    }

    /// One frame, and what the app does after one. The clipboard a
    /// paste waits for is answered as the app's loop would: no picture.
    fn frame(&mut self) -> bool {
        let out = self.h.shell_frame(&mut self.shell, &mut self.app);
        let again = self.app.tick(&mut self.shell);
        (self.shell.state.clipboard_wanted, self.shell.state.clipboard_image_wanted) = (false, false);
        self.h.advance(1.0 / 60.0);
        out.rebuild_again || again
    }

    fn settle(&mut self) {
        for _ in 0..12 {
            if !self.frame() {
                break;
            }
        }
    }

    /// Let `seconds` go by.
    fn wait(&mut self, seconds: f64) {
        self.h.advance(seconds);
        self.settle();
    }

    fn front(&self) -> Shared {
        self.app.front.upgrade().expect("a document has the keyboard")
    }

    /// The document in front, as text.
    fn text(&self) -> String {
        self.front().borrow().ed.doc.text()
    }

    fn type_text(&mut self, text: &str) {
        self.h.type_text(text);
        self.settle();
    }

    fn key(&mut self, key: Key, mods: Modifiers) {
        self.h.key_with(key, mods);
        self.settle();
    }

    /// Ctrl+`c`, or with Shift too.
    fn chord(&mut self, c: char, shift: bool) {
        self.key(Key::Char(c), if shift { ctrl() | Modifiers::SHIFT } else { ctrl() });
    }

    /// An action as a menu, the palette or a dialog's button runs it,
    /// with a path when it is one a file dialog answers. What was asking
    /// closes, as it does when its button is pressed.
    fn act(&mut self, id: &str, path: Option<&std::path::Path>) {
        let mut action = Action::new(id);
        if let Some(path) = path {
            action = action.with("path", Value::Str(path.display().to_string()));
        }
        let mut requests = Vec::new();
        crate::actions::run(&mut self.app, &action, &mut HostCx { pointer: Vec2::ZERO, requests: &mut requests });
        self.shell.request(&mut self.app, lntrn_ui::ShellRequest::ClosePopup);
        for r in requests {
            self.shell.request(&mut self.app, r);
        }
        self.settle();
    }

    /// Something in the front pane's body, by its name there.
    fn body(&self) -> WidgetId {
        let area = self.app.front_area.unwrap_or(0);
        WidgetId::ROOT.with_u64(area as u64).with("body")
    }

    fn rect(&self, id: WidgetId) -> Rect {
        self.h.rect_of(id).unwrap_or_else(|| panic!("{id:?} is not on screen"))
    }

    /// Where the page's parts are on screen.
    fn place(&self) -> Place {
        let front = self.front();
        let d = front.borrow();
        Place::of(self.rect(self.body().with("page")), self.app.settings.page_width, d.view.scroll, self.h.scale)
    }

    /// The point on screen in the middle of the caret's place at `pos`.
    fn point(&self, pos: Pos) -> Vec2 {
        let front = self.front();
        let d = front.borrow();
        let (x, y, h) = d.ed.layout.caret(pos);
        self.place().origin + Vec2::new(f64::from(x) + 0.5, f64::from(y + h * 0.5))
    }

    fn click_at(&mut self, at: Vec2) {
        self.h.move_to(at);
        self.h.press();
        self.frame();
        self.h.release();
        self.settle();
    }

    fn click(&mut self, id: WidgetId) {
        let rect = self.rect(id);
        self.click_at(rect.center());
    }

    fn drag(&mut self, from: Vec2, to: Vec2) {
        self.h.move_to(from);
        self.h.press();
        self.frame();
        for step in 1..=4 {
            self.h.move_to(from + (to - from) * (f64::from(step) / 4.0));
            self.frame();
        }
        self.h.release();
        self.settle();
    }

    /// Tabs in the front pane: their names, and which shows.
    fn tabs(&self) -> (Vec<String>, usize) {
        let area = self.shell.screen.area(self.app.front_area.unwrap_or(0)).expect("the pane");
        (area.tabs.iter().map(|t| t.name.clone().unwrap_or_default()).collect(), area.current)
    }
}
