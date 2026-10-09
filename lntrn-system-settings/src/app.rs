//! The app as the shell sees it: one editor showing a sidebar of pages
//! and the chosen page, menus, the palette, and live saving. Every
//! change marks the config dirty; it is written a moment after the last
//! one, and the compositor picks it up from the file's mtime. The one
//! other editor is the number Identify puts on a monitor, in a layer
//! surface of its own (see `pages::monitors::badge`).

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, FILL, Host, HostCx, Key, KeyConfig, KeyItem, KeyPress, Menu, MenuItem, Modifiers, Shell, Trigger, Ui, actions};

use crate::config::Config;
use crate::look;
use crate::nav::Page;
use crate::pages;
use crate::pages::monitors::{MonitorsState, badge};
use crate::pages::mouse::MouseState;
use crate::pages::wallpaper::WallpaperState;
use crate::sidebar;

pub const APP_ID: &str = "lntrn-system-settings";

/// Seconds after the last change before the file is written.
const SAVE_DELAY: f64 = 0.3;
/// How often, at most, the file is checked for outside changes.
const DISK_CHECK: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Settings,
    /// The number on a monitor, by the monitor's place among them.
    Badge(u8),
}

const EDITORS: [Editor; 1] = [Editor::Settings];

pub struct App {
    pub config: Config,
    pub page: Page,
    pub wallpaper: WallpaperState,
    pub mouse: MouseState,
    pub monitors: MonitorsState,
    /// Font families found on disk, read when Appearance first shows.
    pub fonts: Vec<String>,
    keys: KeyConfig,
    dirty: bool,
    dirty_since: f64,
    last_disk_check: f64,
}

impl App {
    pub fn new(config: Config) -> Self {
        let mut keys = KeyConfig::default();
        keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(Key::Char('q'), Modifiers::CTRL), actions::QUIT));
        keys.bind(CTX_WINDOW, KeyItem::new(Trigger::key(Key::F(3), Modifiers::NONE), actions::PALETTE));
        Self { config, page: Page::Wallpaper, wallpaper: WallpaperState::default(), mouse: MouseState::default(), monitors: MonitorsState::default(), fonts: Vec::new(), keys, dirty: false, dirty_since: 0.0, last_disk_check: 0.0 }
    }

    /// Something changed: write it once the user pauses.
    pub fn mark_dirty(&mut self, now: f64) {
        self.dirty = true;
        self.dirty_since = now;
    }

    fn draw_content(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        ui.push_id(self.page.id());
        if pages::draw(self, ui, cx) {
            self.mark_dirty(ui.now());
        }
        ui.pop_id();
    }

    /// Write a pending change once the user has paused, and pick up
    /// edits made to the file by someone else while we sit clean.
    fn housekeeping(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        let now = ui.now();
        if self.dirty {
            let waited = now - self.dirty_since;
            if waited >= SAVE_DELAY {
                if let Err(e) = self.config.save() {
                    cx.toast(&format!("Not saved: {e}"));
                }
                self.dirty = false;
            } else {
                ui.state.request_redraw_after(SAVE_DELAY - waited);
            }
        } else if now - self.last_disk_check >= DISK_CHECK {
            self.last_disk_check = now;
            if self.config.changed_on_disk() {
                self.config.reload();
            }
        }
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
            Editor::Settings => "Settings",
            Editor::Badge(_) => "Monitor",
        }
    }

    fn editor_id(&self, editor: Editor) -> String {
        match editor {
            Editor::Settings => "Settings".to_owned(),
            Editor::Badge(index) => badge::editor_id(index),
        }
    }

    fn editor_from_id(&self, id: &str) -> Option<Editor> {
        if id == "Settings" { Some(Editor::Settings) } else { badge::index_of(id).map(Editor::Badge) }
    }

    fn pickable(&self, editor: Editor) -> bool {
        editor == Editor::Settings
    }

    fn title(&self) -> String {
        "System Settings".to_owned()
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        false
    }

    fn title_menus(&self) -> &[(&str, &str)] {
        &[("Settings", "settings")]
    }

    fn menu(&self, name: &str) -> Option<Menu> {
        (name == "settings").then(|| {
            Menu::new(
                "Settings",
                vec![
                    MenuItem::new("Reload from Disk", Action::new("app.reload")),
                    MenuItem::new("Open lantern.toml", Action::new("app.open_config")),
                    MenuItem::separator(),
                    MenuItem::pref_toggle("Reduce Motion", "reduce_motion"),
                    MenuItem::pref_toggle("Debug Overlay", "debug_overlay"),
                    MenuItem::separator(),
                    MenuItem::new("Quit", Action::new(actions::QUIT)),
                ],
            )
        })
    }

    fn palette(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_lowercase();
        Page::ALL
            .into_iter()
            .map(|p| (format!("page.{}", p.id()), format!("Go to {}", p.label())))
            .chain([(actions::QUIT.to_owned(), "Quit".to_owned())])
            .filter(|(_, label)| label.to_lowercase().contains(&q))
            .collect()
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action)
    }

    fn draw_body(&mut self, editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        if let Editor::Badge(index) = editor {
            self.monitors.identify.draw(index, ui, cx);
            return false;
        }
        // Whatever page shows: a setup on trial is counted down here.
        if let Some(soon) = self.monitors.tick(&self.config.monitors, ui.now()) {
            ui.state.request_redraw_after(soon);
        }
        let width = ui.m.px(sidebar::WIDTH);
        ui.columns(&[width, FILL], |ui, col| match col {
            0 => sidebar::draw(ui, &mut self.page),
            _ => self.draw_content(ui, cx),
        });
        self.housekeeping(ui, cx);
        false
    }

    fn run(&mut self, action: &Action, cx: &mut HostCx) {
        if let Some(id) = action.id.strip_prefix("page.") {
            if let Some(p) = Page::from_id(id) {
                self.page = p;
            }
            return;
        }
        match action.id.as_str() {
            "app.reload" => {
                self.config.reload();
                self.dirty = false;
                cx.toast("Reloaded lantern.toml");
            }
            "app.open_config" => {
                let _ = std::process::Command::new("xdg-open").arg(crate::config::path()).spawn();
            }
            pages::wallpaper::PICK => {
                let Some(path) = action.arg("path").and_then(|v| v.as_str()) else { return };
                match pages::wallpaper::pick(&mut self.config, &mut self.wallpaper, path) {
                    // Zero: long enough ago that it is written at once.
                    Ok(()) => self.mark_dirty(0.0),
                    Err(why) => cx.toast(why),
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
        self.wallpaper.set_waker(waker.clone());
        self.monitors.set_waker(waker);
    }

    fn after_rebuild(&mut self, gpu: &Gpu, images: &mut Images, shell: &mut Shell<Self>) -> bool {
        let want = self.config.windows.background_opacity.clamp(0.05, 1.0);
        if (shell.opacity - want).abs() > 1e-3 {
            shell.opacity = want;
        }
        // The shell's own parts wear the Lantern look with the desktop's
        // accent, whatever was saved: a new accent shows as it is picked.
        let theme = look::theme(look::accent(&self.config.appearance.accent));
        let restyled = shell.prefs.theme != theme;
        if restyled {
            shell.prefs.theme = theme;
        }
        let cursors = self.mouse.upload(gpu, images);
        let thumbs = self.wallpaper.upload(gpu, images);
        restyled || cursors || thumbs
    }
}
