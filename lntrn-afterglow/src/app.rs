//! The app as the shell sees it: one editor showing the sidebar, the
//! chosen page and the bar under it; the menu, the palette and the keys;
//! and what every action does. The hardware is looked at, and the helper
//! talked to, on the sampler's thread: each frame this picks up the
//! newest look, and hands the thread what the user asked for.

use std::sync::Arc;

use lntrn_afterglowd::tune::Tune;
use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_kit::desktop::{Desktop, Follow};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, FILL, Host, HostCx, Key, KeyConfig, KeyItem, KeyPress, Menu, MenuItem, Modifiers, Shell, ShellRequest, Trigger, Ui, actions};

use crate::bar;
use crate::dialogs::{self, Asking};
use crate::edit;
use crate::nav::Page;
use crate::pages::{self, fans};
use crate::profiles::{self, Profile, Profiles};
use crate::sample::Frame;
use crate::sidebar;
use crate::worker::{Cmd, Link};

pub const APP_ID: &str = "lntrn-afterglow";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Main,
}

const EDITORS: [Editor; 1] = [Editor::Main];

pub struct App {
    link: Link,
    follow: Follow,
    keys: KeyConfig,
    pub page: Page,
    /// The newest look at the machine. `None` until the first comes.
    pub frame: Option<Arc<Frame>>,
    /// What the pages edit; `None` until the helper first says what is
    /// applied (see `edit.rs`).
    pub draft: Option<Tune>,
    pub profiles: Profiles,
    pub fans: fans::Remembered,
    pub asking: Asking,
    /// What the page and the bar asked for while drawing, carried out
    /// once drawing is done.
    pub todo: Vec<Action>,
    /// A menu to open under a button, once drawing is done.
    pub menu_at: Option<(String, Vec2)>,
    /// What the helper last said: how many trials had run out, and what
    /// was wrong. A change is worth a word.
    reverted: Option<u64>,
    trouble: Option<String>,
}

impl App {
    pub fn new(link: Link, profiles: Profiles) -> App {
        let mut keys = KeyConfig::default();
        let mut bind = |key: Key, mods: Modifiers, action: &str| keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(key, mods), action));
        bind(Key::Char('q'), Modifiers::CTRL, actions::QUIT);
        bind(Key::F(3), Modifiers::NONE, actions::PALETTE);
        bind(Key::F(5), Modifiers::NONE, "helper.retry");
        bind(Key::Enter, Modifiers::CTRL, "tune.apply");
        bind(Key::Char('s'), Modifiers::CTRL, "profile.new");
        App { link, follow: Follow::default(), keys, page: Page::Overview, frame: None, draft: None, profiles, fans: fans::Remembered::default(), asking: Asking::default(), todo: Vec::new(), menu_at: None, reverted: None, trouble: None }
    }

    /// The desktop's look as it was read at the start.
    pub fn desktop(&self) -> &Desktop {
        self.follow.desktop()
    }

    /// Whether a command is on its way to the helper: its buttons wait.
    pub fn link_busy(&self) -> bool {
        self.link.busy()
    }

    /// What a test's fixed link was asked to do.
    #[cfg(test)]
    pub fn link_sent(&self) -> Vec<String> {
        self.link.sent.borrow().clone()
    }

    /// A test's helper changing its mind.
    #[cfg(test)]
    pub fn link_show(&self, frame: Frame) {
        self.link.show(frame);
    }

    /// The newest look, and what it has to say.
    fn pick_up(&mut self, cx: &mut HostCx) {
        if let Some(frame) = self.link.latest() {
            if let Some(status) = frame.status() {
                if self.draft.is_none() {
                    self.draft = Some(status.applied.clone());
                }
                if self.reverted.is_some_and(|seen| status.reverted > seen) {
                    cx.toast("Your changes weren't kept in time, so they went back.");
                }
                self.reverted = Some(status.reverted);
                if status.trouble != self.trouble {
                    if let Some(t) = &status.trouble {
                        cx.toast(t);
                    }
                    self.trouble = status.trouble.clone();
                }
            }
            self.frame = Some(frame);
        }
        for said in self.link.take_said() {
            cx.toast(&said);
        }
    }

    fn applied(&self) -> Option<&Tune> {
        self.frame.as_ref()?.status().map(|s| &s.applied)
    }

    fn stock(&self) -> Option<Tune> {
        self.frame.as_ref()?.limits().map(|l| l.stock.clone())
    }

    fn dirty(&self) -> bool {
        matches!((self.applied(), &self.draft), (Some(a), Some(d)) if !edit::same(a, d))
    }

    /// Try the draft: the helper undoes it unless it is kept.
    fn apply(&mut self, cx: &mut HostCx) {
        let (Some(limits), Some(draft)) = (self.frame.as_ref().and_then(|f| f.limits()), &self.draft) else {
            cx.toast("The helper isn't running, so nothing can be applied.");
            return;
        };
        match limits.check(draft) {
            Ok(()) => self.link.send(Cmd::Apply(draft.clone())),
            Err(e) => cx.toast(&format!("Can't apply: {e}.")),
        }
    }

    fn save_profiles(&mut self, cx: &mut HostCx) {
        if let Err(e) = self.profiles.save() {
            cx.toast(&format!("The profiles weren't saved: {e}"));
        }
    }

    /// The profile action `rest` is ("apply.2"), carried out.
    fn profile(&mut self, rest: &str, cx: &mut HostCx) {
        let Some((what, i)) = rest.rsplit_once('.').and_then(|(w, i)| Some((w, i.parse::<usize>().ok()?))) else { return };
        let Some(p) = self.profiles.list.get(i).cloned() else { return };
        match what {
            "apply" => {
                self.draft = Some(p.tune);
                self.apply(cx);
            }
            "load" => {
                self.draft = Some(p.tune);
                cx.toast(&format!("\u{201C}{}\u{201D} is on the pages: change what you like, then Apply.", p.name));
            }
            "update" => cx.request(dialogs::save_over(self, i).expect("it is there")),
            "rename" => cx.request(dialogs::rename(self, i).expect("it is there")),
            "delete" => cx.request(dialogs::delete(self, i).expect("it is there")),
            _ => {}
        }
    }
}

impl Host for App {
    type Editor = Editor;
    type AreaState = ();

    fn editors(&self) -> &[Editor] {
        &EDITORS
    }

    fn editor_label(&self, _editor: Editor) -> &str {
        "Afterglow"
    }

    fn title(&self) -> String {
        "Afterglow".to_owned()
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        false
    }

    fn title_menus(&self) -> &[(&str, &str)] {
        &[("Afterglow", "afterglow")]
    }

    fn menu(&self, name: &str) -> Option<Menu> {
        if let Some(i) = name.strip_prefix("profile.").and_then(|i| i.parse::<usize>().ok()) {
            let p = self.profiles.list.get(i)?;
            let item = |label: &str, what: &str| MenuItem::new(label, Action::new(&format!("profile.{what}.{i}")));
            return Some(Menu::new(&p.name, vec![item("Load onto the Pages", "load"), item("Save the Current Settings Here", "update").enabled(self.draft.is_some()), MenuItem::separator(), item("Rename…", "rename"), item("Delete…", "delete")]));
        }
        (name == "afterglow").then(|| {
            let here = self.frame.as_ref().is_some_and(|f| f.limits().is_some());
            Menu::new(
                "Afterglow",
                vec![
                    MenuItem::new("Apply", Action::new("tune.apply")).enabled(here && self.dirty()),
                    MenuItem::new("Discard Changes", Action::new("tune.discard")).enabled(self.dirty()),
                    MenuItem::new("Reset to Stock", Action::new("tune.reset")).enabled(here),
                    MenuItem::separator(),
                    MenuItem::new("Save as a Profile…", Action::new("profile.new")).enabled(self.draft.is_some()),
                    MenuItem::new("Ask the Helper Again", Action::new("helper.retry")),
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
        let profiles = self.profiles.list.iter().enumerate().map(|(i, p)| (format!("profile.apply.{i}"), format!("Apply \u{201C}{}\u{201D}", p.name)));
        let rest = [("tune.apply", "Apply"), ("tune.discard", "Discard Changes"), ("tune.reset", "Reset to Stock"), ("profile.new", "Save as a Profile…"), ("helper.retry", "Ask the Helper Again"), (actions::QUIT, "Quit")];
        pages.chain(profiles).chain(rest.into_iter().map(|(id, label)| (id.to_owned(), label.to_owned()))).filter(|(_, label)| label.to_lowercase().contains(&q)).collect()
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action)
    }

    fn draw_body(&mut self, _editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        self.pick_up(&mut cx.host());
        let width = ui.m.px(sidebar::WIDTH);
        ui.columns(&[width, FILL], |ui, col| match col {
            0 => sidebar::draw(self, ui),
            _ => {
                let m = ui.m;
                let bar_h = m.px(bar::HEIGHT);
                ui.push_id(self.page.id());
                pages::draw(self, ui, (ui.remaining_height() - bar_h - m.gap).max(m.px(100.0)));
                ui.pop_id();
                let r = ui.alloc(Vec2::new(FILL, bar_h));
                // Out to the window's right and bottom edges, past the body's padding.
                bar::draw(self, ui, Rect::new(r.min, r.max + Vec2::splat(m.pad)));
            }
        });
        let mut host = cx.host();
        for action in std::mem::take(&mut self.todo) {
            self.run(&action, &mut host);
        }
        if let Some((name, at)) = self.menu_at.take() {
            host.request(ShellRequest::MenuAt(name, at));
        }
        false
    }

    fn draw_item(&mut self, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
        dialogs::draw(self, key, ui, cx)
    }

    fn dialog_ready(&self, action: &Action) -> bool {
        dialogs::ready(self, &action.id)
    }

    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        let id = action.id.as_str();
        if let Some(page) = id.strip_prefix("page.").and_then(Page::from_id) {
            self.page = page;
            return;
        }
        if let Some(rest) = id.strip_prefix("profile.").filter(|r| r.contains('.')) {
            return self.profile(rest, cx);
        }
        match id {
            "tune.apply" => self.apply(cx),
            "tune.discard" => self.draft = self.applied().cloned(),
            "tune.keep" => self.link.send(Cmd::Keep),
            "tune.revert" => self.link.send(Cmd::Revert),
            "tune.reset" | "stock.apply" => {
                if let Some(stock) = self.stock() {
                    self.draft = Some(stock);
                    self.link.send(Cmd::Reset);
                }
            }
            "stock.load" => {
                if let Some(stock) = self.stock() {
                    self.draft = Some(stock);
                    cx.toast("Stock is on the pages: change what you like, then Apply.");
                }
            }
            "helper.retry" => self.link.send(Cmd::Now),
            "profile.new" if self.draft.is_some() => cx.request(dialogs::new_profile(self)),
            "profile.new" => {}
            "profile.new-go" => {
                let name = profiles::tidy(&self.asking.name);
                if let (Some(tune), true) = (self.draft.clone(), self.profiles.can_name(&name, None)) {
                    self.profiles.list.push(Profile { name: name.clone(), tune });
                    self.save_profiles(cx);
                    cx.toast(&format!("Saved \u{201C}{name}\u{201D}."));
                }
            }
            "profile.rename-go" => {
                let name = profiles::tidy(&self.asking.name);
                if let Some(i) = self.asking.target.filter(|i| self.profiles.can_name(&name, Some(*i)) && *i < self.profiles.list.len()) {
                    self.profiles.list[i].name = name;
                    self.save_profiles(cx);
                }
            }
            "profile.update-go" => {
                if let (Some(i), Some(tune)) = (self.asking.target.filter(|i| *i < self.profiles.list.len()), self.draft.clone()) {
                    self.profiles.list[i].tune = tune;
                    self.save_profiles(cx);
                    cx.toast(&format!("Saved over \u{201C}{}\u{201D}.", self.profiles.list[i].name));
                }
            }
            "profile.delete-go" => {
                if let Some(i) = self.asking.target.filter(|i| *i < self.profiles.list.len()) {
                    let gone = self.profiles.list.remove(i);
                    self.save_profiles(cx);
                    cx.toast(&format!("Deleted \u{201C}{}\u{201D}.", gone.name));
                }
            }
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
