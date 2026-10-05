//! The Themes page: saved looks to switch between, then the look itself:
//! window style, accent, background, font, and (from `effects`) the
//! window glow, focus glow, borders and blur.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Action, AreaCx, Dialog, HostCx, ShellRequest, Ui};

use crate::app::App;
use crate::config::Config;
use crate::fonts;
use crate::pages::effects;
use crate::themes::{self, Preset};
use crate::widgets::{choice, hex_color, note, section, toggle_string};

const STYLES: [(&str, &str); 2] = [("lantern", "Lantern"), ("fox-dark", "Fox")];

#[derive(Default)]
pub struct ThemesState {
    pub presets: Vec<Preset>,
    pub loaded: bool,
    /// Font families found on disk, read once.
    pub families: Vec<String>,
}

impl ThemesState {
    fn refresh(&mut self) {
        self.presets = themes::list();
        self.loaded = true;
        if self.families.is_empty() {
            self.families = fonts::families();
        }
    }

    fn active<'a>(&'a self, cfg: &Config) -> Option<&'a Preset> {
        let slug = cfg.appearance.active_theme.as_str();
        (!slug.is_empty()).then(|| self.presets.iter().find(|p| p.slug == slug)).flatten()
    }
}

pub fn draw(cfg: &mut Config, st: &mut ThemesState, name_buf: &mut String, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
    if !st.loaded {
        st.refresh();
    }
    let mut changed = false;

    section(ui, "Saved Themes");
    if st.presets.is_empty() {
        note(ui, "No saved themes yet. Set up a look below and save it.");
    }
    let mut apply: Option<usize> = None;
    ui.push_id("presets");
    for (i, p) in st.presets.iter().enumerate() {
        ui.push_index(i);
        let active = cfg.appearance.active_theme == p.slug;
        let r = ui.selectable(&p.name, active);
        if let Some(accent) = p.accent().and_then(Color::parse_hex) {
            let side = r.rect.height() * 0.5;
            let swatch = Rect::from_center_size(Vec2::new(r.rect.max.x - side, r.rect.center().y), Vec2::new(side, side));
            ui.fill(swatch, accent);
            ui.outline(swatch, ui.m.border, ui.theme.border_dark);
        }
        if r.clicked {
            apply = Some(i);
        }
        ui.pop_id();
    }
    ui.pop_id();
    if let Some(i) = apply {
        themes::apply(&st.presets[i], cfg);
        cx.toast(&format!("Applied {}", st.presets[i].name));
        changed = true;
    }

    let active = st.active(cfg).map(|p| (p.slug.clone(), p.name.clone()));
    ui.row(|ui| {
        if ui.button("Save Current as Theme…").clicked {
            name_buf.clear();
            cx.request(ShellRequest::Dialog(Dialog::confirm("Save Theme", "Save the current look as a theme.", "Save", Action::new("theme.save_new")).content("theme_name")));
        }
        if let Some((slug, name)) = active.clone() {
            if ui.button("Update from Current").clicked {
                cx.request(ShellRequest::Dialog(Dialog::confirm("Update Theme", &format!("Overwrite \"{name}\" with the current look?"), "Update", Action::new("theme.update"))));
            }
            if ui.button("Rename…").clicked {
                *name_buf = name.clone();
                cx.request(ShellRequest::Dialog(Dialog::confirm("Rename Theme", "A new name for the theme.", "Rename", Action::new("theme.rename")).content("theme_name")));
            }
            if ui.button("Move Up").clicked {
                let _ = themes::shift(&slug, -1);
                st.loaded = false;
            }
            if ui.button("Move Down").clicked {
                let _ = themes::shift(&slug, 1);
                st.loaded = false;
            }
            if ui.button("Delete…").clicked {
                cx.request(ShellRequest::Dialog(Dialog::confirm("Delete Theme", &format!("Delete \"{name}\"? The current look stays as it is."), "Delete", Action::new("theme.delete"))));
            }
        }
    });
    if active.is_none() {
        note(ui, "Click a theme to apply it; its buttons appear once one is active.");
    }

    section(ui, "Look");
    let a = &mut cfg.appearance;
    changed |= choice(ui, "Window style", &mut a.theme, &STYLES);
    changed |= hex_color(ui, "Accent", &mut a.accent, Color::hex(0xFFC800));
    changed |= toggle_string(ui, "Custom window background", &mut a.background_color, "#0E0E0E");
    if !a.background_color.is_empty() {
        changed |= hex_color(ui, "Background", &mut a.background_color, Color::hex(0x0E0E0E));
    }
    let families: Vec<&str> = st.families.iter().map(String::as_str).collect();
    let current = fonts::effective_family(&a.font_family);
    let mut index = families.iter().position(|f| *f == current).unwrap_or(0);
    ui.labelled("Font", |ui| {
        if ui.dropdown("font", &mut index, &families) {
            a.font_family = families[index].to_owned();
            changed = true;
        }
    });
    note(ui, "Lantern apps use the new font as they restart.");
    changed |= ui.slider("Font size", &mut a.font_size, 10.0, 32.0, 1.0);

    changed |= effects::window_glow(cfg, ui);
    changed |= effects::focus_glow(cfg, ui);
    changed |= effects::borders(cfg, ui);
    changed |= effects::blur(cfg, ui);
    changed
}

/// The dialog buttons' actions.
pub fn run(app: &mut App, id: &str, cx: &mut HostCx) {
    let name = app.name_buf.trim().to_owned();
    let active = app.config.appearance.active_theme.clone();
    let result = match id {
        "theme.save_new" => {
            if name.is_empty() {
                cx.toast("A theme needs a name");
                return;
            }
            themes::save_new(&name, &app.config).map(|slug| {
                app.config.appearance.active_theme = slug;
                app.mark_dirty(0.0);
                format!("Saved {name}")
            })
        }
        "theme.update" => {
            let shown = app.themes.presets.iter().find(|p| p.slug == active).map(|p| p.name.clone()).unwrap_or_else(|| active.clone());
            themes::update(&active, &shown, &app.config).map(|()| format!("Updated {shown}"))
        }
        "theme.rename" => {
            if name.is_empty() {
                cx.toast("A theme needs a name");
                return;
            }
            themes::rename(&active, &name).map(|()| format!("Renamed to {name}"))
        }
        "theme.delete" => themes::delete(&active).map(|()| {
            app.config.appearance.active_theme.clear();
            app.mark_dirty(0.0);
            "Theme deleted".to_owned()
        }),
        _ => return,
    };
    app.themes.loaded = false;
    match result {
        Ok(msg) => cx.toast(&msg),
        Err(e) => cx.toast(&format!("Theme error: {e}")),
    }
}
