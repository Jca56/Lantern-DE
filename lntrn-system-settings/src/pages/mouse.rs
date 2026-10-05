//! Pointer, scrolling, clicking and the cursor: its size, which one, and
//! the bundled cursor's colours with a live preview.

use std::path::PathBuf;

use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_image::Image;
use lntrn_math::{Color, Vec2};
use lntrn_ui::Ui;

use crate::config::Config;
use crate::pages::cursor_svg;
use crate::widgets::{hex_color, note, section, slider_int, toggle_string};

/// Pixels the previews are rendered at.
const PREVIEW_PX: u32 = 96;
/// Logical size they are shown at.
const PREVIEW_SIZE: f64 = 60.0;

pub struct CursorEntry {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub image: Option<ImageHandle>,
}

/// Where a rendered picture goes once the GPU has it.
enum Target {
    Default,
    Entry(usize),
}

#[derive(Default)]
pub struct MouseState {
    scanned: bool,
    cursors: Vec<CursorEntry>,
    default_image: Option<ImageHandle>,
    default_key: String,
    default_source: Option<String>,
    pending: Vec<(Target, Image)>,
}

impl MouseState {
    fn scan(&mut self) {
        self.scanned = true;
        self.cursors.clear();
        let Some(dir) = lntrn_sys::dirs::lantern_config().map(|c| c.join("cursors")) else { return };
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("svg" | "png")) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let name = stem.replace(['-', '_'], " ").split_whitespace().map(capitalize).collect::<Vec<_>>().join(" ");
            self.cursors.push(CursorEntry { id: stem.to_owned(), name, path, image: None });
        }
        self.cursors.sort_by_key(|c| c.name.to_lowercase());
        for i in 0..self.cursors.len() {
            if let Some(img) = render_file(&self.cursors[i].path) {
                self.pending.push((Target::Entry(i), img));
            }
        }
    }

    /// Re-render the bundled cursor when its colours change.
    fn refresh_default(&mut self, cfg: &Config) {
        let key = cursor_svg::key(&cfg.input);
        if key == self.default_key {
            return;
        }
        self.default_key = key;
        if self.default_source.is_none() {
            self.default_source = cursor_svg::default_source();
        }
        let Some(src) = &self.default_source else { return };
        if let Some(img) = lntrn_svg::render(&cursor_svg::customize(src, &cfg.input), PREVIEW_PX) {
            self.pending.push((Target::Default, img));
        }
    }

    /// Hand rendered pictures to the GPU. Returns `true` when any went
    /// up, so the frame is rebuilt with them showing.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let any = !self.pending.is_empty();
        for (target, img) in self.pending.drain(..) {
            let slot = match target {
                Target::Default => &mut self.default_image,
                Target::Entry(i) => match self.cursors.get_mut(i) {
                    Some(e) => &mut e.image,
                    None => continue,
                },
            };
            *slot = Some(match slot.take() {
                Some(old) => images.replace(gpu, old, &img),
                None => images.add(gpu, &img),
            });
        }
        any
    }
}

fn capitalize(word: &str) -> String {
    let mut c = word.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn render_file(path: &std::path::Path) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        lntrn_svg::render(std::str::from_utf8(&bytes).ok()?, PREVIEW_PX)
    } else {
        lntrn_image::decode(&bytes).ok()
    }
}

pub fn draw(cfg: &mut Config, st: &mut MouseState, ui: &mut Ui) -> bool {
    if !st.scanned {
        st.scan();
    }
    st.refresh_default(cfg);
    let mut changed = false;

    section(ui, "Pointer");
    changed |= ui.slider("Speed", &mut cfg.input.mouse_speed, -1.0, 1.0, 0.05);
    changed |= ui.toggle("Pointer acceleration", &mut cfg.input.pointer_acceleration);
    note(ui, "On, the pointer moves further the faster the mouse goes; off, it is flat.");
    changed |= ui.toggle("Focus follows mouse", &mut cfg.window_manager.focus_follows_mouse);
    changed |= ui.slider("Scroll speed", &mut cfg.input.scroll_speed, 0.25, 3.0, 0.05);

    section(ui, "Clicking");
    changed |= ui.toggle("Double-click to open files", &mut cfg.input.double_click_to_open);
    changed |= ui.toggle("Ripple on click", &mut cfg.input.click_anim_enabled);
    if cfg.input.click_anim_enabled {
        changed |= ui.slider("Ripple size", &mut cfg.input.click_anim_size, 0.25, 3.0, 0.05);
        changed |= toggle_string(ui, "Ripple has its own colour", &mut cfg.input.click_anim_color, &cfg.input.cursor_outline_color.clone());
        if !cfg.input.click_anim_color.is_empty() {
            changed |= hex_color(ui, "Ripple colour", &mut cfg.input.click_anim_color, Color::hex(0x2563EB));
        } else {
            note(ui, "The ripple takes the cursor's outline colour.");
        }
    }

    section(ui, "Cursor");
    changed |= slider_int(ui, "Size", &mut cfg.input.cursor_size, 16, 128, 1);
    ui.push_id("cursors");
    let is_default = cfg.input.cursor_theme == "default";
    if cursor_row(ui, st.default_image, "Lantern (bundled)", is_default) {
        cfg.input.cursor_theme = "default".to_owned();
        changed = true;
    }
    for i in 0..st.cursors.len() {
        ui.push_index(i);
        let selected = cfg.input.cursor_theme == st.cursors[i].id;
        if cursor_row(ui, st.cursors[i].image, &st.cursors[i].name, selected) {
            cfg.input.cursor_theme = st.cursors[i].id.clone();
            changed = true;
        }
        ui.pop_id();
    }
    ui.pop_id();
    if st.cursors.is_empty() {
        note(ui, "Drop SVG or PNG cursors into ~/.lantern/config/cursors to see them here.");
    }

    if is_default {
        section(ui, "Cursor Colours");
        let inp = &mut cfg.input;
        changed |= hex_color(ui, "Body, light", &mut inp.cursor_body_light, Color::WHITE);
        changed |= hex_color(ui, "Body, dark", &mut inp.cursor_body_dark, Color::hex(0xABABAB));
        changed |= hex_color(ui, "Accent, light", &mut inp.cursor_accent_light, Color::hex(0xFAB414));
        changed |= hex_color(ui, "Accent, dark", &mut inp.cursor_accent_dark, Color::hex(0x9A6300));
        changed |= hex_color(ui, "Outline", &mut inp.cursor_outline_color, Color::hex(0x0A0A0A));
        changed |= ui.slider("Outline width", &mut inp.cursor_outline_scale, 0.0, 3.0, 0.1);
        changed |= ui.slider("Roundness", &mut inp.cursor_corner_radius, 0.0, 1.0, 0.05);
        note(ui, "The preview shows the colours; outline width and roundness show on the real pointer.");
    }
    changed
}

/// A cursor's picture beside its name; `true` when picked.
fn cursor_row(ui: &mut Ui, image: Option<ImageHandle>, name: &str, selected: bool) -> bool {
    let mut picked = false;
    ui.row(|ui| {
        match image {
            Some(h) => {
                ui.image_fit(h, Vec2::new(PREVIEW_SIZE, PREVIEW_SIZE));
            }
            None => {
                ui.alloc(Vec2::new(ui.m.px(PREVIEW_SIZE), ui.m.px(PREVIEW_SIZE)));
            }
        }
        picked = ui.selectable(name, selected).clicked;
    });
    picked
}
