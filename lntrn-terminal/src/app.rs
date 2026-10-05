//! The app as the shell sees it: one kind of editor, a terminal, in as
//! many panes and tabs as the shell's own machinery makes of it. A tab's
//! terminal lives in the tab's state, so it goes when the tab does,
//! whichever window that was in.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use lntrn_kit::desktop::Follow;
use lntrn_term::{Launch, Look, TermId, Terminal, Wake};
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, AreaId, Host, HostCx, Key, KeyConfig, KeyItem, KeyPress, Menu, Modifiers, ShellRequest, Trigger, Ui, actions};

use crate::look;
use crate::menu;
use crate::ops::Op;
use crate::settings::Settings;

pub const APP_ID: &str = "lntrn-terminal";
/// Lines of output kept above the screen.
const SCROLLBACK: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Terminal,
}

const EDITORS: [Editor; 1] = [Editor::Terminal];

/// A terminal as a tab holds it.
pub type Shared = Rc<RefCell<Terminal>>;
/// A terminal as everything else knows it: gone when its tab is.
pub type Held = Weak<RefCell<Terminal>>;

/// What starts in a tab the first time it shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Start {
    /// The folder; the one the terminal itself was started in when `None`.
    pub cwd: Option<PathBuf>,
    /// A program to run instead of the shell (`-e`).
    pub command: Option<Vec<OsString>>,
}

/// What a tab holds.
#[derive(Clone, Default)]
pub struct Tab {
    /// Its terminal, once it has shown.
    pub term: Option<Shared>,
    pub start: Start,
    /// The name this app last gave the tab (its program's title, or its
    /// folder). A tab whose name is something else was named by hand,
    /// and keeps that.
    pub named: Option<String>,
    /// It comes back when the terminal next starts.
    pub pinned: bool,
}

pub struct App {
    next_id: u64,
    pub wake: Option<Wake>,
    pub settings: Settings,
    pub follow: Follow,
    /// How many times `lantern.toml` had been read when the settings
    /// were last taken from it.
    pub seen_reads: u64,
    /// The settings changed in here since they were last looked at.
    pub dirty: bool,
    /// When the settings are to be written: a moment after the last
    /// change to them.
    pub save_at: Option<f64>,
    pub look: Look,
    /// What to do to the panes and tabs once the frame is built.
    pub ops: Vec<Op>,
    /// The title bar and the tab strips are hidden.
    pub bar_hidden: bool,
    /// How many panes and tabs the window has.
    pub panes: usize,
    pub tabs: usize,
    /// A pane was opened in a window of its own: tab strips stay.
    pub more_windows: bool,
    /// How each pane's tabs stand: how many, and which shows.
    pub area_tabs: HashMap<AreaId, (usize, usize)>,
    /// The terminal showing in each pane, for files dropped on it.
    pub showing: HashMap<AreaId, Held>,
    /// The terminal with the keyboard, and its pane.
    pub front: Held,
    pub front_area: Option<AreaId>,
    /// Whether the tab with the keyboard is pinned.
    pub front_pinned: bool,
    /// The panes in the order they last had the keyboard, the latest
    /// last: where it goes back to when the one that has it closes.
    pub recent: Vec<AreaId>,
    /// The pane whose terminal last took the keyboard on becoming the
    /// active one, so it takes it once.
    took_focus: Option<AreaId>,
    /// The pane and terminal the right-click menu is for.
    pub menu_area: Option<AreaId>,
    pub menu_term: Held,
    /// The terminal a paste is on its way to.
    pub paste_into: Held,
    /// When tabs were last named after their terminals.
    pub named_at: f64,
    keys: KeyConfig,
}

fn bind(keys: &mut KeyConfig, key: Key, mods: Modifiers, action: &'static str) {
    keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(key, mods), action));
}

impl App {
    pub fn new() -> App {
        let both = Modifiers::CTRL | Modifiers::SHIFT;
        let mut keys = KeyConfig::default();
        for (key, action) in [('t', "tab.new"), ('w', "tab.close"), ('n', "window.new"), ('d', "split.right"), ('e', "split.down"), ('p', actions::PALETTE)] {
            bind(&mut keys, Key::Char(key), both, action);
        }
        // With Shift down the brackets arrive as braces.
        for (key, action) in [('[', "pane.prev"), ('{', "pane.prev"), (']', "pane.next"), ('}', "pane.next")] {
            bind(&mut keys, Key::Char(key), both, action);
        }
        // With Shift down `=` and `-` arrive as `+` and `_`.
        for (key, action) in [('+', "text.bigger"), ('=', "text.bigger"), ('_', "text.smaller"), ('-', "text.smaller"), (')', "text.reset"), ('0', "text.reset")] {
            bind(&mut keys, Key::Char(key), both, action);
        }
        bind(&mut keys, Key::Tab, Modifiers::CTRL, "tab.next");
        bind(&mut keys, Key::Tab, both, "tab.prev");
        bind(&mut keys, Key::F(11), Modifiers::SUPER, "bar.toggle");
        let follow = Follow::default();
        let settings = Settings::load();
        let look = look::look(follow.desktop(), &settings);
        App {
            next_id: 1,
            wake: None,
            bar_hidden: settings.open_bar_hidden,
            settings,
            seen_reads: follow.reads(),
            follow,
            dirty: false,
            save_at: None,
            look,
            ops: Vec::new(),
            panes: 1,
            tabs: 1,
            more_windows: false,
            area_tabs: HashMap::new(),
            showing: HashMap::new(),
            front: Weak::new(),
            front_area: None,
            front_pinned: false,
            recent: Vec::new(),
            took_focus: None,
            menu_area: None,
            menu_term: Weak::new(),
            paste_into: Weak::new(),
            named_at: -1.0,
            keys,
        }
    }

    /// Start what a tab is for.
    fn spawn(&mut self, start: &Start) -> Shared {
        let id = TermId(self.next_id);
        self.next_id += 1;
        let launch = Launch { cwd: start.cwd.clone(), command: start.command.clone(), env: Vec::new(), program: APP_ID.to_owned() };
        let mut term = Terminal::new(id, launch, 80, 24, SCROLLBACK, self.wake.clone());
        term.reserved = lntrn_term::input::terminal_keys;
        Rc::new(RefCell::new(term))
    }

    /// The folder the terminal with the keyboard is in: where a new tab,
    /// pane or window starts.
    pub fn front_dir(&self) -> Option<PathBuf> {
        self.front.upgrade().and_then(|t| t.borrow().cwd_now())
    }

    /// A change to the settings: shown now, written in a moment.
    pub fn settings_changed(&mut self) {
        self.look = look::look(self.follow.desktop(), &self.settings);
        self.dirty = true;
    }

    /// Whether the tab strips show: not in a bare window, and not while
    /// there is one pane with one tab in it and so nothing to choose.
    fn strips_show(&self) -> bool {
        !self.bar_hidden && (self.more_windows || self.panes > 1 || self.tabs > 1)
    }
}

/// Hand an address or a file to the desktop to open.
pub fn open_outside(what: &str) {
    if let Err(e) = std::process::Command::new("xdg-open").arg(what).spawn() {
        lntrn_core::log_warn!("opening {what}: {e}");
    }
}

impl Host for App {
    type Editor = Editor;
    type AreaState = Tab;

    fn editors(&self) -> &[Editor] {
        &EDITORS
    }

    fn editor_label(&self, _editor: Editor) -> &str {
        "Terminal"
    }

    fn title(&self) -> String {
        self.front.upgrade().map_or_else(|| "Terminal".to_owned(), |t| t.borrow().title())
    }

    fn tab_attention(&self, _editor: Editor, state: &Tab) -> bool {
        state.term.as_ref().is_some_and(|t| t.try_borrow().is_ok_and(|t| t.attention))
    }

    fn paints_body(&self, _editor: Editor) -> bool {
        true
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        self.strips_show()
    }

    fn title_menus(&self) -> &[(&str, &str)] {
        &[("Terminal", "terminal"), ("View", "view"), ("Split", "split")]
    }

    fn menu(&self, name: &str) -> Option<Menu> {
        menu::title_menu(self, name)
    }

    fn palette(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_lowercase();
        menu::COMMANDS.iter().filter(|(_, label)| label.to_lowercase().contains(&q)).map(|(id, label)| ((*id).to_owned(), (*label).to_owned())).collect()
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action)
    }

    fn draw_body(&mut self, _editor: Editor, ui: &mut Ui, cx: &mut AreaCx<Tab>) -> bool {
        let term = match &cx.state.term {
            Some(term) => term.clone(),
            None => {
                // A tab the shell's own header made says nothing of where
                // to start: where the pane's terminal is, as a new tab
                // made in here does.
                let mut start = cx.state.start.clone();
                if start.cwd.is_none() && start.command.is_none() {
                    start.cwd = self.showing.get(&cx.area).and_then(Weak::upgrade).and_then(|t| t.borrow().cwd_now()).or_else(|| self.front_dir());
                }
                let term = self.spawn(&start);
                cx.state.term = Some(term.clone());
                term
            }
        };
        self.showing.insert(cx.area, Rc::downgrade(&term));
        let active = cx.active;
        // The frame this pane became the active one, its terminal takes
        // the keyboard: changing pane means typing, not clicking first.
        // So it does whenever nothing else holds it (a menu just closed):
        // there is nothing else in the window to type into.
        let grab = active && (self.took_focus != Some(cx.area) || ui.state.focus.is_none());
        if active {
            self.took_focus = Some(cx.area);
            self.front = Rc::downgrade(&term);
            self.front_area = Some(cx.area);
        } else if self.took_focus == Some(cx.area) {
            self.took_focus = None;
        }
        let mut t = term.borrow_mut();
        t.pump(ui.state.now);
        let out = lntrn_term::draw_terminal(ui, &mut t, &self.look, active, grab);
        if let Some(url) = out.open_url {
            open_outside(&url);
            cx.toast(&format!("Opening {url}"));
        }
        if let Some((path, _, _)) = out.open {
            open_outside(&path.display().to_string());
        }
        if let Some(at) = out.context {
            self.menu_area = Some(cx.area);
            self.menu_term = Rc::downgrade(&term);
            cx.request(ShellRequest::ContextMenu(Box::new(menu::context(at))));
        }
        false
    }

    fn draw_item(&mut self, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
        menu::draw_row(self, key, ui, cx)
    }

    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        menu::run(self, &action.id, cx);
    }

    fn key(&self, press: KeyPress, _editor: Option<Editor>) -> Option<Action> {
        self.keys.resolve(&[CTX_WINDOW], &press.to_event(), |_| true).map(KeyItem::action)
    }

    fn dropped(&mut self, paths: &[PathBuf], area: Option<AreaId>, _editor: Option<Editor>, _cx: &mut HostCx) {
        let onto = area.and_then(|a| self.showing.get(&a)).and_then(Weak::upgrade).or_else(|| self.front.upgrade());
        if let Some(term) = onto {
            for path in paths {
                term.borrow_mut().type_path(path);
            }
        }
    }
}
