//! The app as the shell sees it: one editor showing a sidebar of pages
//! and the chosen page, its menu and palette, and what its actions do.
//! The machine is looked at on the sampler's thread; each frame this
//! picks up the newest look and draws from it.

use std::sync::Arc;

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_kit::desktop::{Desktop, Follow};
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, FILL, Host, HostCx, Key, KeyConfig, KeyItem, KeyPress, Menu, MenuItem, Modifiers, Shell, Trigger, Ui, actions};

use crate::ffi::{SIGKILL, SIGTERM};
use crate::kill::{self, Doomed};
use crate::nav::Page;
use crate::pages;
use crate::rows::{self, Pick, Row, View};
use crate::sample::Frame;
use crate::settings::{SPEEDS, Settings};
use crate::sidebar;
use crate::worker::{Cmd, Link};

pub const APP_ID: &str = "lntrn-sysmon";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Monitor,
}

const EDITORS: [Editor; 1] = [Editor::Monitor];

/// What the Processes page keeps between frames.
#[derive(Default)]
pub struct Procs {
    pub view: View,
    /// The row that is selected, by what it stands for: it stays picked
    /// as the list reorders under it.
    pub picked: Option<Pick>,
    /// The rows as last worked out, and what they were worked out from.
    pub rows: Vec<Row>,
    built: Option<(u64, View)>,
    /// Put the keyboard in the search field next frame.
    pub focus_search: bool,
    /// Scroll the picked row into view next frame: it was moved to by
    /// the keyboard.
    pub reveal: bool,
    /// What an "End" or "Force Kill" still waiting on its dialog goes to.
    pub doomed: Option<Doomed>,
}

pub struct App {
    link: Link,
    follow: Follow,
    keys: KeyConfig,
    pub page: Page,
    /// The newest look at the machine. `None` until the first comes.
    pub frame: Option<Arc<Frame>>,
    pub settings: Settings,
    pub paused: bool,
    pub procs: Procs,
    /// Text on its way to the clipboard: an action can't reach it, so
    /// the next frame puts it there.
    pub copy: Option<String>,
}

impl App {
    pub fn new(link: Link, settings: Settings) -> App {
        let mut keys = KeyConfig::default();
        let mut bind = |key: Key, mods: Modifiers, action: &str| keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(key, mods), action));
        bind(Key::Char('q'), Modifiers::CTRL, actions::QUIT);
        bind(Key::F(3), Modifiers::NONE, actions::PALETTE);
        bind(Key::F(5), Modifiers::NONE, "monitor.refresh");
        bind(Key::Char('f'), Modifiers::CTRL, "procs.find");
        bind(Key::Delete, Modifiers::NONE, "procs.end");
        bind(Key::Delete, Modifiers::SHIFT, "procs.kill");
        let procs = Procs { view: View { grouped: settings.grouped, kernel: settings.kernel, ..View::default() }, ..Procs::default() };
        App { link, follow: Follow::default(), keys, page: Page::Overview, frame: None, settings, paused: false, procs, copy: None }
    }

    /// The desktop's look as it was read at the start: its font and how
    /// see-through its windows are.
    pub fn desktop(&self) -> &Desktop {
        self.follow.desktop()
    }

    /// The rows of the Processes page for `frame`, worked out again only
    /// when the look or what is asked of it has changed.
    pub fn refresh_rows(&mut self, frame: &Frame) {
        let p = &mut self.procs;
        if p.built.as_ref().is_none_or(|(seq, view)| *seq != frame.seq || *view != p.view) {
            p.rows = rows::rows(&frame.procs, &p.view);
            p.built = Some((frame.seq, p.view.clone()));
        }
        // What was picked has gone: nothing is.
        if p.picked.as_ref().is_some_and(|pick| rows::targets(&frame.procs, &p.view, pick).is_empty()) {
            p.picked = None;
        }
    }

    /// Open the program `app`'s row, or shut it.
    pub fn fold(&mut self, app: &Arc<str>) {
        let open = &mut self.procs.view.open;
        if !open.remove(app) {
            open.insert(app.clone());
        }
    }

    /// Write the settings, saying so if that couldn't be done.
    fn save(&mut self, cx: &mut HostCx) {
        self.settings.grouped = self.procs.view.grouped;
        self.settings.kernel = self.procs.view.kernel;
        if let Err(e) = self.settings.save() {
            cx.toast(&format!("Not saved: {e}"));
        }
    }

    /// Fold programs or list every process: from the page's own control,
    /// which has no [`HostCx`] of its own to hand.
    pub fn set_grouped(&mut self, grouped: bool, cx: &mut HostCx) {
        self.procs.view.grouped = grouped;
        self.save(cx);
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.link.send(Cmd::Paused(paused));
    }

    /// Ask, in a dialog, before sending `signal` to what is picked.
    fn ask_to_signal(&mut self, signal: i32, cx: &mut HostCx) {
        let (Some(frame), Some(pick)) = (&self.frame, &self.procs.picked) else { return };
        if self.page != Page::Processes {
            return;
        }
        if let Some((doomed, dialog)) = kill::ask(&frame.procs, &self.procs.view, pick, signal) {
            self.procs.doomed = Some(doomed);
            cx.request(dialog);
        }
    }

    /// What is picked, as the process it is when it is just one.
    fn picked_proc(&self) -> Option<&crate::sample::procs::Proc> {
        let Some(Pick::Pid(pid)) = &self.procs.picked else { return None };
        self.frame.as_ref()?.procs.iter().find(|p| p.pid == *pid)
    }
}

impl Host for App {
    type Editor = Editor;
    type AreaState = ();

    fn editors(&self) -> &[Editor] {
        &EDITORS
    }

    fn editor_label(&self, _editor: Editor) -> &str {
        "Monitor"
    }

    fn title(&self) -> String {
        "System Monitor".to_owned()
    }

    fn status(&self) -> String {
        if self.paused { "Paused".to_owned() } else { String::new() }
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        false
    }

    fn title_menus(&self) -> &[(&str, &str)] {
        &[("Monitor", "monitor")]
    }

    fn menu(&self, name: &str) -> Option<Menu> {
        (name == "monitor").then(|| {
            let speeds = SPEEDS.iter().enumerate().map(|(i, (seconds, label))| MenuItem::new(label, Action::new(&format!("speed.{i}"))).checked(*seconds == self.settings.interval)).collect();
            Menu::new(
                "Monitor",
                vec![
                    MenuItem::sub("Update Speed", speeds),
                    MenuItem::new("Pause", Action::new("monitor.pause")).checked(self.paused),
                    MenuItem::new("Refresh Now", Action::new("monitor.refresh")),
                    MenuItem::separator(),
                    MenuItem::new("Find a Process…", Action::new("procs.find")),
                    MenuItem::new("Fold Programs", Action::new("procs.group")).checked(self.procs.view.grouped),
                    MenuItem::new("Show Kernel Threads", Action::new("procs.kernel")).checked(self.procs.view.kernel),
                    MenuItem::separator(),
                    MenuItem::pref_toggle("Reduce Motion", "reduce_motion"),
                    MenuItem::new("Quit", Action::new(actions::QUIT)),
                ],
            )
        })
    }

    fn palette(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_lowercase();
        let pages = Page::ALL.into_iter().map(|p| (format!("page.{}", p.id()), format!("Go to {}", p.label())));
        let speeds = SPEEDS.iter().enumerate().map(|(i, (_, label))| (format!("speed.{i}"), format!("Update Speed: {label}")));
        let rest = [("monitor.pause", if self.paused { "Resume" } else { "Pause" }), ("monitor.refresh", "Refresh Now"), ("procs.find", "Find a Process…"), ("procs.group", "Fold Programs"), ("procs.kernel", "Show Kernel Threads"), (actions::QUIT, "Quit")];
        pages.chain(speeds).chain(rest.into_iter().map(|(id, label)| (id.to_owned(), label.to_owned()))).filter(|(_, label)| label.to_lowercase().contains(&q)).collect()
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action)
    }

    fn draw_body(&mut self, _editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        if let Some(frame) = self.link.latest() {
            self.frame = Some(frame);
        }
        if let Some(text) = self.copy.take() {
            ui.state.set_clipboard(text);
        }
        let width = ui.m.px(sidebar::WIDTH);
        ui.columns(&[width, FILL], |ui, col| match col {
            0 => sidebar::draw(self, ui),
            _ => {
                ui.push_id(self.page.id());
                pages::draw(self, ui, cx);
                ui.pop_id();
            }
        });
        false
    }

    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        let id = action.id.as_str();
        if let Some(page) = id.strip_prefix("page.").and_then(Page::from_id) {
            self.page = page;
            return;
        }
        if let Some((seconds, _)) = id.strip_prefix("speed.").and_then(|i| SPEEDS.get(i.parse::<usize>().ok()?)) {
            self.settings.interval = *seconds;
            self.link.send(Cmd::Interval(*seconds));
            // Picking a speed is asking to watch: a paused monitor goes on.
            self.set_paused(false);
            self.save(cx);
            return;
        }
        match id {
            "monitor.pause" => self.set_paused(!self.paused),
            "monitor.refresh" => self.link.send(Cmd::Now),
            "procs.find" => {
                self.page = Page::Processes;
                self.procs.focus_search = true;
            }
            "procs.group" => self.set_grouped(!self.procs.view.grouped, cx),
            "procs.kernel" => {
                self.procs.view.kernel = !self.procs.view.kernel;
                self.save(cx);
            }
            "procs.end" => self.ask_to_signal(SIGTERM, cx),
            "procs.kill" => self.ask_to_signal(SIGKILL, cx),
            "procs.signal" => {
                if let Some(doomed) = self.procs.doomed.take() {
                    cx.toast(&kill::carry_out(&doomed));
                    self.link.send(Cmd::Now);
                }
            }
            "procs.fold" => {
                if let Some(Pick::App(app)) = self.procs.picked.clone() {
                    self.fold(&app);
                }
            }
            "procs.copy-pid" => self.copy = self.picked_proc().map(|p| p.pid.to_string()),
            "procs.copy-command" => self.copy = self.picked_proc().map(|p| p.command.to_string()),
            other => cx.toast(&format!("unknown action {other}")),
        }
    }

    fn key(&self, press: KeyPress, _editor: Option<Editor>) -> Option<Action> {
        self.keys.resolve(&[CTX_WINDOW], &press.to_event(), |_| true).map(KeyItem::action)
    }
}

impl AppHost for App {
    fn waker(&mut self, waker: Waker) {
        self.link.set_waker(waker);
    }

    fn after_rebuild(&mut self, _gpu: &Gpu, _images: &mut Images, shell: &mut Shell<Self>) -> bool {
        self.follow.apply(shell)
    }
}
