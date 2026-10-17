//! Apps' icons in the tray (`lntrn-status`'s `apps`): their faces, their
//! pictures, and their menus as the bar shows them. A picture is pixels the app sends
//! (put on the GPU here), or a name: looked up in the app's own folder
//! first (Steam keeps its there), then the status icons the app-icon
//! search doesn't look in, then that search (`lntrn-apps`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_apps::Icons;
use lntrn_image::Image;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_props::Value;
use lntrn_status::apps::{AppItem, Icon, MenuEntry, Status, Toggle};
use lntrn_text::TextStyle;
use lntrn_ui::{Action, ContextMenu, Item, Sense, Ui};

use super::bar::{RADIUS, TrayFrame, TrayPress, wheel};
use super::{Item as Face, Tray};

/// The action a row of an app's menu runs, with its item's `key` and the
/// row's `id`.
pub const CLICK: &str = "tray.app.click";

/// The pictures of apps' icons that came as pixels, by item.
#[derive(Default)]
pub struct AppIcons {
    ready: HashMap<String, (u64, ImageHandle)>,
    wanted: Vec<(String, u64, Image)>,
    /// Where each name was found, by the name and the app's folder.
    found: HashMap<(String, String), String>,
}

impl AppIcons {
    /// `item`'s picture, on the GPU: from the next frame on, the first
    /// time it's asked for.
    pub fn get(&mut self, item: &AppItem, icons: &mut Icons) -> Option<ImageHandle> {
        match &item.icon {
            Icon::None => None,
            Icon::Named { name, theme } => {
                let found = self.found.entry((name.clone(), theme.clone())).or_insert_with(|| lookup(name, theme));
                icons.get(found)
            }
            Icon::Pixmap(p) => match self.ready.get(&item.key) {
                Some((stamp, handle)) if *stamp == p.stamp => Some(*handle),
                _ => {
                    if !self.wanted.iter().any(|(k, s, _)| *k == item.key && *s == p.stamp) {
                        self.wanted.push((item.key.clone(), p.stamp, Image { width: p.width, height: p.height, rgba: p.rgba.to_vec() }));
                    }
                    self.ready.get(&item.key).map(|(_, h)| *h)
                }
            },
        }
    }

    /// Put the pixels asked for on the GPU. Returns whether any went, so
    /// the frame is built again with them.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let any = !self.wanted.is_empty();
        for (key, stamp, image) in std::mem::take(&mut self.wanted) {
            self.ready.insert(key, (stamp, images.add(gpu, &image)));
        }
        any
    }
}

/// What to look `name` up by: a file in the app's own folder `theme`, or
/// a status icon's file, when there is one; else the name, for the
/// app-icon search.
fn lookup(name: &str, theme: &str) -> String {
    if name.starts_with('/') {
        return name.to_owned();
    }
    let sizes = ["scalable", "64x64", "48x48", "32x32", "24x24", "22x22", "16x16"];
    let mut dirs: Vec<PathBuf> = Vec::new();
    if !theme.is_empty() {
        let theme = Path::new(theme);
        dirs.push(theme.to_owned());
        dirs.extend(sizes.iter().flat_map(|s| ["apps", "status", "panel"].map(|kind| theme.join("hicolor").join(s).join(kind))));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    for base in [home.join(".local/share/icons/hicolor"), PathBuf::from("/usr/share/icons/hicolor")] {
        dirs.extend(sizes.iter().flat_map(|s| ["status", "panel"].map(|kind| base.join(s).join(kind))));
    }
    dirs.iter().flat_map(|d| ["svg", "png"].map(|ext| d.join(format!("{name}.{ext}")))).find(|p| p.is_file()).map_or_else(|| name.to_owned(), |p| p.to_string_lossy().into_owned())
}

/// The face of the app whose item is `app`: its picture (its first
/// letter, until that's there), a dot while it needs attention, lit under
/// the pointer and while its menu is up. A click, a middle click and the
/// wheel go to it; a right click asks for its menu. Returns whether the
/// pointer is over it.
#[allow(clippy::too_many_arguments)]
pub(super) fn face(tray: &mut Tray, ui: &mut Ui, face: Rect, g: f64, app: &AppItem, under: Color, icons: &mut Icons, frame: &mut TrayFrame) -> bool {
    let m = ui.m;
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    let r = ui.interact(ui.id("tray-app").with(app.key.as_str()), face, Sense::CLICK);
    frame.pressed |= r.pressed;
    let open = tray.app_menu.as_ref().is_some_and(|menu| menu.key == app.key);
    let fill = match () {
        _ if open => Some(accent.fade(0.22)),
        _ if r.held => Some(under.lerp(Color::WHITE, 0.16)),
        _ if r.hovered => Some(under.lerp(Color::WHITE, 0.11)),
        _ => None,
    };
    if let Some(fill) = fill {
        ui.draw.rounded_rect(face, m.px(RADIUS).min(face.height() * 0.3), fill);
    }
    let pic = Rect::from_center_size(face.center(), Vec2::splat(g)).round();
    match tray.app_icons.get(app, icons) {
        Some(handle) => ui.draw.image(pic, handle, 0.0, Color::WHITE),
        None => {
            ui.draw.rounded_rect(pic, m.px(8.0), Color::WHITE.fade(0.10));
            let letter: String = app.title.chars().next().map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect());
            ui.text_centered(&letter, &TextStyle::new((g * 0.6) as f32).bold(), pic, ink);
        }
    }
    if app.status == Status::NeedsAttention {
        ui.draw.circle(Vec2::new(pic.max.x - g * 0.08, pic.max.y - g * 0.08), g * 0.14, accent);
    }
    let tip = if app.tooltip.is_empty() { &app.title } else { &app.tooltip };
    ui.tooltip(&r, tip);
    if let Some(apps) = &tray.apps {
        if r.clicked {
            apps.activate(&app.key);
        }
        if r.hovered && ui.state.middle_pressed {
            apps.secondary(&app.key);
        }
    }
    if r.hovered && ui.state.right_pressed {
        frame.press = Some(TrayPress::AppMenu(app.key.clone()));
    }
    if r.hovered {
        let notches = wheel(tray, ui, Face::Apps);
        if notches != 0
            && let Some(apps) = &tray.apps
        {
            apps.scroll(&app.key, notches);
        }
    }
    tray.app_faces.push((app.key.clone(), face));
    r.hovered
}

/// `item`'s menu, its `entries`, as the bar's menu at `at`.
pub fn menu(item: &AppItem, entries: &[MenuEntry], at: Vec2) -> ContextMenu {
    let title = if item.title.is_empty() { "App" } else { &item.title };
    ContextMenu::new(title, at).tab(title, rows(&item.key, entries))
}

/// The rows for `entries` of the item `key`'s menu: a ticked one marked
/// so, one greyed as words alone.
fn rows(key: &str, entries: &[MenuEntry]) -> Vec<Item> {
    entries
        .iter()
        .map(|e| {
            if e.separator {
                return Item::Separator;
            }
            let label = match e.toggle {
                Toggle::Check(true) => format!("✓  {}", e.label),
                Toggle::Radio(true) => format!("●  {}", e.label),
                _ => e.label.clone(),
            };
            if !e.children.is_empty() {
                Item::sub(&label, rows(key, &e.children))
            } else if !e.enabled {
                Item::header(&label)
            } else {
                Item::action(&label, Action::new(CLICK).with("key", Value::Str(key.to_owned())).with("id", Value::Str(e.id.to_string())))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i32, label: &str) -> MenuEntry {
        MenuEntry { id, label: label.into(), separator: false, enabled: true, toggle: Toggle::None, children: Vec::new() }
    }

    #[test]
    fn a_right_click_on_an_apps_face_asks_for_its_menu() {
        use lntrn_ui::testing::Harness;
        let mut h = Harness::new(400.0, 200.0);
        let (mut tray, mut icons) = (Tray::default(), Icons::new(48));
        let app = AppItem { key: ":1.4/StatusNotifierItem".into(), title: "steam".into(), ..AppItem::default() };
        let face = Rect::from_min_size(Vec2::new(100.0, 100.0), Vec2::splat(56.0));
        let mut draw = |h: &mut Harness, tray: &mut Tray| {
            let mut asked = None;
            h.frame(|ui| {
                let mut frame = TrayFrame { rect: face, press: None, pressed: false };
                super::face(tray, ui, face, 30.0, &app, lntrn_kit::look::BG, &mut icons, &mut frame);
                asked = frame.press;
            });
            asked
        };
        h.move_to(face.center());
        assert_eq!(draw(&mut h, &mut tray), None, "only under the pointer, nothing asked");
        assert_eq!(tray.app_face(&app.key), Some(face));
        h.right_press();
        assert_eq!(draw(&mut h, &mut tray), Some(TrayPress::AppMenu(app.key.clone())));
    }

    #[test]
    fn an_apps_menu_reads_as_the_bars() {
        let item = AppItem { key: ":1.4/StatusNotifierItem".into(), title: "Steam".into(), ..AppItem::default() };
        let entries = vec![
            entry(1, "Library"),
            MenuEntry { separator: true, ..entry(2, "") },
            MenuEntry { toggle: Toggle::Check(true), ..entry(3, "Do Not Disturb") },
            MenuEntry { enabled: false, ..entry(4, "Offline") },
            MenuEntry { children: vec![entry(6, "Online")], ..entry(5, "Status") },
        ];
        let rows = rows(&item.key, &entries);
        assert!(matches!(&rows[0], Item::Action { label, action } if label == "Library" && action.id == CLICK));
        assert!(matches!(rows[1], Item::Separator));
        assert!(matches!(&rows[2], Item::Action { label, .. } if label == "✓  Do Not Disturb"));
        assert!(matches!(&rows[3], Item::Header(label) if label == "Offline"));
        assert!(matches!(&rows[4], Item::Sub { label, items } if label == "Status" && items.len() == 1));
    }

    #[test]
    fn a_name_in_the_apps_own_folder_is_found_there() {
        let dir = std::env::temp_dir().join(format!("lntrn-cc-tray-theme-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("hicolor/22x22/status")).unwrap();
        std::fs::write(dir.join("steam_tray.png"), b"png").unwrap();
        std::fs::write(dir.join("hicolor/22x22/status/alert.svg"), b"svg").unwrap();
        let theme = dir.to_string_lossy();
        assert_eq!(lookup("steam_tray", &theme), dir.join("steam_tray.png").to_string_lossy());
        assert_eq!(lookup("alert", &theme), dir.join("hicolor/22x22/status/alert.svg").to_string_lossy());
        assert_eq!(lookup("no-such-icon-anywhere-42", &theme), "no-such-icon-anywhere-42", "the name, for the app-icon search");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
