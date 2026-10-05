//! Headless runs on Lantern UI's test harness, no window and no GPU,
//! with real shells in real ptys: the window is typed and clicked at the
//! way a hand would, and what the shells then show is checked.
//!
//! Every test has the sandbox to itself (they share its `lantern.toml`),
//! so they take turns.

mod menu_rows;
mod settings_file;

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, Once};
use std::time::Duration;

use lntrn_kit::desktop;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::testing::Harness;
use lntrn_ui::{AreaId, Host, HostCx, Key, Modifiers, Shell, WidgetId, WindowCommand};

use crate::app::{App, Editor, Shared};
use crate::menu;

static TURN: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Point every folder the app reads at a scratch tree and make the shell
/// a plain `sh`: nothing here touches the real home or the real config,
/// and nothing of the user's shell setup gets in the way.
fn sandbox() -> PathBuf {
    static ONCE: Once = Once::new();
    let root = std::env::temp_dir().join(format!("lntrn-terminal-smoke-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("work")).unwrap();
        // SAFETY: set once, under the tests' lock, before anything here
        // reads them or starts a thread that does.
        unsafe {
            std::env::set_var("HOME", &root);
            std::env::set_var("LANTERN_HOME", root.join("lantern"));
            std::env::set_var("SHELL", "/bin/sh");
            std::env::set_var("PS1", "$ ");
            std::env::remove_var("ENV");
        }
    });
    root
}

fn both() -> Modifiers {
    Modifiers::CTRL | Modifiers::SHIFT
}

fn screen_of(term: &Shared) -> String {
    let t = term.borrow();
    (0..t.grid.rows).map(|y| t.grid.row(y).iter().map(|c| c.ch).collect::<String>().trim_end().to_owned()).collect::<Vec<_>>().join("\n")
}

fn sh(script: &str) -> Option<Vec<OsString>> {
    Some(vec!["/bin/sh".into(), "-c".into(), script.into()])
}

/// The right-click menu's rows, as it lists them.
const ROWS: [(usize, &str); 8] = [(4, "edit.copy"), (5, "edit.paste"), (6, "edit.select-all"), (8, "tab.new"), (9, "tab.close"), (11, "split.right"), (12, "split.down"), (13, "pane.close")];

struct Rig {
    h: Harness,
    shell: Shell<App>,
    app: App,
    /// What the window was last asked to do.
    command: Option<WindowCommand>,
    _turn: MutexGuard<'static, ()>,
}

impl Rig {
    /// The terminal in a window of the size the compositor opens it at.
    /// `config` is what `lantern.toml` holds (`None`: what the test
    /// before left); `command` is what `-e` was given.
    fn start(config: Option<&str>, command: Option<Vec<OsString>>) -> Rig {
        let turn = turn();
        sandbox();
        let path = desktop::config_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if let Some(config) = config {
            let _ = std::fs::remove_file(path.with_file_name("terminal.toml"));
            std::fs::write(&path, config).unwrap();
        }
        let mut app = App::new();
        let mut shell = Shell::new(Editor::Terminal);
        app.open_first(&mut shell, command);
        let mut rig = Rig { h: Harness::new(1152.0, 720.0), shell, app, command: None, _turn: turn };
        rig.settle();
        rig
    }

    fn new() -> Rig {
        Rig::start(Some(""), None)
    }

    /// One frame, and what the app does after one. `true` when another
    /// is wanted.
    fn frame(&mut self) -> bool {
        let out = self.h.shell_frame(&mut self.shell, &mut self.app);
        let again = self.app.tick(&mut self.shell);
        self.command = out.window_command.or(self.command);
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

    /// Frames until `done`: a shell answers in its own time.
    fn until(&mut self, what: &str, done: impl Fn(&Rig) -> bool) {
        for _ in 0..800 {
            self.frame();
            if done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never happened: {what}\n--- the screen ---\n{}", self.screen());
    }

    /// Let `seconds` go by.
    fn wait(&mut self, seconds: f64) {
        self.h.advance(seconds);
        self.settle();
    }

    fn front(&self) -> Shared {
        self.app.front.upgrade().expect("a terminal has the keyboard")
    }

    fn screen(&self) -> String {
        self.app.front.upgrade().map(|t| screen_of(&t)).unwrap_or_default()
    }

    fn sees(&mut self, text: &str) {
        self.until(&format!("{text:?} on the screen"), |r| r.screen().contains(text));
    }

    /// Type a line at the prompt and press Enter.
    fn line(&mut self, line: &str) {
        self.h.type_text(line);
        self.h.key(Key::Enter);
        self.settle();
    }

    /// Ctrl+Shift+`key`.
    fn chord(&mut self, key: char) {
        self.h.key_with(Key::Char(key), both());
        self.settle();
    }

    /// An action as the palette or a title bar menu would run it.
    fn act(&mut self, id: &str) {
        let mut requests = Vec::new();
        menu::run(&mut self.app, id, &mut HostCx { pointer: Vec2::ZERO, requests: &mut requests });
        for r in requests {
            self.shell.request(&mut self.app, r);
        }
        self.settle();
    }

    fn counts(&self) -> (usize, usize) {
        (self.app.panes, self.app.tabs)
    }

    fn areas(&self) -> Vec<AreaId> {
        self.shell.screen.area_ids().collect()
    }

    /// How a pane's tabs stand: how many, and which shows.
    fn tabs_of(&self, area: AreaId) -> (usize, usize) {
        let a = self.shell.screen.area(area).expect("the pane");
        (a.tabs.len(), a.current)
    }

    /// Where a pane's terminal is drawn.
    fn term_rect(&self, area: AreaId) -> Rect {
        self.h.rect_of(WidgetId::ROOT.with_u64(area as u64).with("body").with("term")).expect("the pane's terminal is on screen")
    }

    fn click_at(&mut self, at: Vec2) {
        self.h.move_to(at);
        self.h.press();
        self.frame();
        self.h.release();
        self.settle();
    }

    fn right_click(&mut self, area: AreaId) {
        let at = self.term_rect(area).center();
        self.h.move_to(at);
        self.h.right_press();
        self.settle();
        assert!(self.shell.popup_open(), "a right click opens the menu");
    }

    /// The id something in row `i` of the right-click menu has.
    fn in_row(i: usize, what: &str) -> WidgetId {
        WidgetId::ROOT.with("popup").with("context").with_index(0).with("items").with_index(i).with(what)
    }

    fn menu_rect(&self, id: WidgetId) -> Rect {
        self.h.rect_of(id).expect("it is in the menu")
    }

    /// Right-click a pane and choose the row for `key`.
    fn choose(&mut self, area: AreaId, key: &str) {
        self.right_click(area);
        let (i, _) = ROWS.iter().find(|(_, k)| *k == key).expect("a row of the menu");
        let rect = self.menu_rect(Rig::in_row(*i, key));
        self.click_at(rect.center());
    }
}

#[test]
fn a_shell_runs_what_is_typed_into_the_window() {
    let mut rig = Rig::new();
    assert_eq!(rig.counts(), (1, 1));
    // One pane with one tab: nothing to choose between, so no strip, and
    // the terminal has everything under the title bar.
    assert!(!rig.app.shows_header(Editor::Terminal));
    let term = rig.term_rect(0);
    assert!(term.width() > 1100.0 && term.height() > 620.0 && term.max.y > 700.0, "{term:?}");
    // No click first: the window opens ready to type in.
    rig.line("echo smoke-$((6*7))");
    rig.sees("smoke-42");
    // The tab is named after where the shell is.
    let cwd = std::env::current_dir().unwrap();
    rig.wait(0.5);
    assert_eq!(rig.shell.screen.area(0).unwrap().tabs[0].name.as_deref(), cwd.file_name().and_then(|n| n.to_str()));
    // A file dropped on the window is typed at the prompt.
    let file = sandbox().join("work/dropped here.txt");
    let mut requests = Vec::new();
    rig.app.dropped(std::slice::from_ref(&file), Some(0), Some(Editor::Terminal), &mut HostCx { pointer: Vec2::ZERO, requests: &mut requests });
    rig.sees("dropped here.txt'");
}

#[test]
fn tabs_and_panes_open_and_close_from_the_keyboard() {
    let mut rig = Rig::new();
    let work = sandbox().join("work").canonicalize().unwrap();
    rig.line(&format!("cd {}", work.display()));
    rig.until("the shell to be in the folder", |r| r.front().borrow().cwd_now().as_deref() == Some(work.as_path()));
    let first = rig.front();

    // A new tab, in front, starting where the old one is; the strip shows.
    rig.chord('t');
    assert_eq!(rig.counts(), (1, 2));
    assert!(!Rc::ptr_eq(&first, &rig.front()), "the new tab is in front");
    assert_eq!(rig.front().borrow().cwd_now().as_deref(), Some(work.as_path()));
    assert!(rig.app.shows_header(Editor::Terminal));
    let term = rig.term_rect(0);
    assert!(term.min.y > 60.0, "under the title bar and the tabs: {term:?}");
    rig.h.key_with(Key::Tab, Modifiers::CTRL);
    rig.settle();
    assert!(Rc::ptr_eq(&first, &rig.front()), "Ctrl+Tab goes round");
    assert_eq!(rig.tabs_of(0), (2, 0));

    // Split right: a pane beside this one, which takes the keyboard.
    rig.chord('d');
    assert_eq!(rig.counts(), (2, 3));
    let [left, right] = rig.areas()[..] else { panic!("two panes") };
    let (l, r) = (rig.term_rect(left), rig.term_rect(right));
    assert!(l.max.x <= r.min.x && (l.width() - r.width()).abs() < 40.0, "side by side, half each: {l:?} {r:?}");
    rig.line("echo right-$((1+1))");
    rig.sees("right-2");
    assert!(!screen_of(&first).contains("right-2"), "typed into the new pane only");
    assert_eq!(rig.front().borrow().cwd_now().as_deref(), Some(work.as_path()));

    // Split down: under the right one. Ctrl+Shift+[ goes back a pane.
    rig.chord('e');
    assert_eq!(rig.counts(), (3, 4));
    let below = *rig.areas().last().unwrap();
    let (r, b) = (rig.term_rect(right), rig.term_rect(below));
    assert!(r.max.y <= b.min.y && (r.min.x - b.min.x).abs() < 1.0, "one above the other: {r:?} {b:?}");
    assert_eq!(rig.shell.screen.active, Some(below));
    rig.chord('[');
    assert_eq!(rig.shell.screen.active, Some(right));
    rig.chord(']');
    assert_eq!(rig.shell.screen.active, Some(below));

    // Ctrl+Shift+W closes the tab; the pane with its last one; and the
    // window with the last pane.
    rig.chord('w');
    assert_eq!(rig.counts(), (2, 3));
    rig.chord('w');
    assert_eq!(rig.counts(), (1, 2));
    rig.chord('w');
    assert_eq!(rig.counts(), (1, 1));
    assert!(!rig.app.shows_header(Editor::Terminal), "back to one tab: the strip goes");
    assert_eq!(rig.command, None);
    rig.line("echo still-$((2*2))");
    rig.sees("still-4");
    rig.chord('w');
    assert_eq!(rig.command, Some(WindowCommand::Close));
}

#[test]
fn a_shell_that_exits_takes_its_tab_and_the_last_one_the_window() {
    let mut rig = Rig::new();
    rig.chord('t');
    assert_eq!(rig.counts(), (1, 2));
    rig.line("exit");
    rig.until("the tab to go with its shell", |r| r.counts() == (1, 1));
    assert_eq!(rig.command, None);
    rig.line("echo here-$((3*3))");
    rig.sees("here-9");
    rig.line("exit");
    rig.until("the window to close with its last shell", |r| r.command == Some(WindowCommand::Close));
}

#[test]
fn a_program_the_window_was_opened_for_stays_up_when_it_fails() {
    {
        let mut rig = Rig::start(Some(""), sh("echo from-e; exit 3"));
        rig.sees("from-e");
        rig.until("it to be gone", |r| r.front().borrow().exited == Some(3));
        rig.wait(0.5);
        assert_eq!((rig.counts(), rig.command), ((1, 1), None), "what it said can still be read");
    }
    let mut rig = Rig::start(Some(""), sh("echo fine"));
    rig.until("the window to close behind a program that did what it came for", |r| r.command == Some(WindowCommand::Close));
}

#[test]
fn a_bell_in_a_tab_nobody_is_looking_at_marks_it() {
    let mut rig = Rig::new();
    let first = rig.front();
    rig.line("sleep 0.4; printf '\\a'");
    rig.chord('t');
    rig.until("the bell to ring behind the new tab", |_| first.borrow().attention);
    assert!(rig.app.tab_attention(Editor::Terminal, &rig.shell.screen.area(0).unwrap().tabs[0].state));
    assert!(!rig.front().borrow().attention);
    // Looked at, it is no longer wanting a look.
    rig.h.key_with(Key::Tab, Modifiers::CTRL);
    rig.until("the tab to be seen", |_| !first.borrow().attention);
    // A bell in the tab being looked at is nothing to be told about.
    rig.line("printf '\\a'; echo rang-$((5+5))");
    rig.sees("rang-10");
    rig.settle();
    assert!(!first.borrow().attention);
}
