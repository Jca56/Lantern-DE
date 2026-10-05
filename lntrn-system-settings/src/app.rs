//! The app as the shell sees it: one editor showing a sidebar of pages
//! and the chosen page, menus, the palette, and live saving. Every
//! change marks the config dirty; it is written a moment after the last
//! one, and the compositor picks it up from the file's mtime.

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::AppHost;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, AreaCx, FILL, Host, HostCx, Key, KeyConfig, KeyItem, KeyPress, Menu, MenuItem, Modifiers, Shell, Trigger, Ui, actions};

use crate::config::Config;
use crate::nav::{CATEGORIES, Page};
use crate::pages;
use crate::pages::mouse::MouseState;
use crate::pages::themes::ThemesState;

pub const APP_ID: &str = "lntrn-system-settings";

/// Logical width of the sidebar.
const SIDEBAR_W: f64 = 320.0;
/// Seconds after the last change before the file is written.
const SAVE_DELAY: f64 = 0.3;
/// How often, at most, the file is checked for outside changes.
const DISK_CHECK: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Settings,
}

const EDITORS: [Editor; 1] = [Editor::Settings];

pub struct App {
    pub config: Config,
    pub page: Page,
    pub themes: ThemesState,
    pub mouse: MouseState,
    /// Text typed into the open dialog's field.
    pub name_buf: String,
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
        Self { config, page: Page::Themes, themes: ThemesState::default(), mouse: MouseState::default(), name_buf: String::new(), keys, dirty: false, dirty_since: 0.0, last_disk_check: 0.0 }
    }

    /// Something changed: write it once the user pauses.
    pub fn mark_dirty(&mut self, now: f64) {
        self.dirty = true;
        self.dirty_since = now;
    }

    fn draw_sidebar(&mut self, ui: &mut Ui) {
        let rect = Rect::from_min_size(ui.cursor(), Vec2::new(ui.avail_width(), ui.remaining_height()));
        ui.fill_shaded(rect, ui.theme.header);
        ui.push_id("sidebar");
        ui.space(ui.m.pad);
        let pad = ui.m.pad;
        ui.indent(pad, |ui| {
            for cat in CATEGORIES {
                let pages: Vec<Page> = cat.pages.iter().copied().filter(|p| p.available()).collect();
                if pages.len() == 1 {
                    if ui.tree_leaf(cat.label, self.page == pages[0]).clicked {
                        self.page = pages[0];
                    }
                    continue;
                }
                let inside = pages.contains(&self.page);
                ui.tree_node(cat.label, false, |ui| {
                    for page in &pages {
                        if ui.tree_leaf(page.label(), self.page == *page).clicked {
                            self.page = *page;
                        }
                    }
                });
                let _ = inside;
            }
        });
        ui.pop_id();
    }

    fn draw_content(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        let pad = ui.m.pad;
        ui.indent(pad, |ui| {
            ui.space(pad);
            ui.heading(self.page.label());
            ui.push_id(self.page.id());
            let mut changed = false;
            ui.scroll_area("page", None, |ui| {
                changed = pages::draw(self, ui, cx);
            });
            ui.pop_id();
            if changed {
                self.mark_dirty(ui.now());
            }
        });
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
                self.themes.loaded = false;
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

    fn editor_label(&self, _editor: Editor) -> &str {
        "Settings"
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
            .filter(|p| p.available())
            .map(|p| (format!("page.{}", p.id()), format!("Go to {}", p.label())))
            .chain([(actions::QUIT.to_owned(), "Quit".to_owned())])
            .filter(|(_, label)| label.to_lowercase().contains(&q))
            .collect()
    }

    fn key_hint(&self, action: &Action) -> Option<String> {
        self.keys.hint_for(action)
    }

    fn draw_body(&mut self, _editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        let sidebar = ui.m.px(SIDEBAR_W);
        ui.columns(&[sidebar, FILL], |ui, col| match col {
            0 => self.draw_sidebar(ui),
            _ => self.draw_content(ui, cx),
        });
        self.housekeeping(ui, cx);
        false
    }

    fn draw_item(&mut self, key: &str, ui: &mut Ui, _cx: &mut HostCx) -> bool {
        match key {
            "theme_name" => ui.text_field_hint("Name", &mut self.name_buf, "Theme name").changed,
            _ => false,
        }
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
                self.themes.loaded = false;
                self.dirty = false;
                cx.toast("Reloaded lantern.toml");
            }
            "app.open_config" => {
                let _ = std::process::Command::new("xdg-open").arg(crate::config::path()).spawn();
            }
            id if id.starts_with("theme.") => pages::themes::run(self, id, cx),
            other => cx.toast(&format!("unknown action {other}")),
        }
    }

    fn key(&self, press: KeyPress, _editor: Option<Editor>) -> Option<Action> {
        self.keys.resolve(&[CTX_WINDOW], &press.to_event(), |_| true).map(KeyItem::action)
    }
}

impl AppHost for App {
    fn after_rebuild(&mut self, gpu: &Gpu, images: &mut Images, shell: &mut Shell<Self>) -> bool {
        let want = self.config.windows.background_opacity.clamp(0.05, 1.0);
        if (shell.opacity - want).abs() > 1e-3 {
            shell.opacity = want;
        }
        self.mouse.upload(gpu, images)
    }
}
