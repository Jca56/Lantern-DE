//! Pointer, scrolling, clicking and the cursor: its size, which one, and
//! the bundled cursor's colours with a live preview.

use std::path::PathBuf;

use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_image::Image;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, Metrics, Sense, Ui};

use crate::config::Config;
use crate::kit::{self, percent, pixels, times};
use crate::look;
use crate::pages::cursor_svg;

/// Pixels the previews are rendered at.
const PREVIEW_PX: u32 = 128;
/// Logical size they are shown at.
const PREVIEW_SIZE: f64 = 72.0;

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

    let inp = &mut cfg.input;
    kit::caption(ui, "Pointer");
    kit::card(ui, "pointer", |c| {
        changed |= c.slider("Pointer speed", "", &mut inp.mouse_speed, (-1.0, 1.0), 0.05, |v| format!("{v:+.2}"));
        changed |= c.switch("Acceleration", "On, a faster flick travels further. Off, it is flat.", &mut inp.pointer_acceleration);
        changed |= c.switch("Focus follows the pointer", "The window under the pointer takes the keyboard.", &mut cfg.window_manager.focus_follows_mouse);
        changed |= c.slider("Scroll speed", "", &mut inp.scroll_speed, (0.25, 3.0), 0.05, times);
    });

    kit::caption(ui, "Clicking");
    kit::card(ui, "clicking", |c| {
        changed |= c.switch("Double-click to open files", "", &mut inp.double_click_to_open);
        changed |= c.switch("Ripple on click", "", &mut inp.click_anim_enabled);
        if inp.click_anim_enabled {
            changed |= c.slider("Ripple size", "", &mut inp.click_anim_size, (0.25, 3.0), 0.05, times);
            let outline = inp.cursor_outline_color.clone();
            changed |= c.switch_string("Its own colour", "Off, the ripple takes the cursor's outline colour.", &mut inp.click_anim_color, &outline);
            if !inp.click_anim_color.is_empty() {
                changed |= c.color("Ripple colour", "", &mut inp.click_anim_color, Color::hex(0x2563EB));
            }
        }
    });

    kit::caption(ui, "Cursor");
    let is_default = inp.cursor_theme == "default";
    kit::card(ui, "cursor", |c| {
        changed |= c.slider_int("Size", "", &mut inp.cursor_size, (16, 128), 1, pixels);
        let mut tiles = vec![("default", "Lantern", st.default_image)];
        tiles.extend(st.cursors.iter().map(|e| (e.id.as_str(), e.name.as_str(), e.image)));
        let (m, w) = (c.ui.m, c.ui.avail_width());
        let h = cursor_tiles_height(m, w, tiles.len());
        if let Some(id) = c.block(h, |ui, rect| cursor_tiles(ui, rect, &tiles, &inp.cursor_theme)) {
            inp.cursor_theme = id;
            changed = true;
        }
    });
    if st.cursors.is_empty() {
        kit::note(ui, "Drop SVG or PNG cursors into ~/.lantern/config/cursors to see them here.");
    }

    if is_default {
        kit::caption(ui, "Cursor colours");
        kit::card(ui, "colours", |c| {
            changed |= c.color("Body, light", "", &mut inp.cursor_body_light, Color::WHITE);
            changed |= c.color("Body, dark", "", &mut inp.cursor_body_dark, Color::hex(0xABABAB));
            changed |= c.color("Accent, light", "", &mut inp.cursor_accent_light, Color::hex(0xFAB414));
            changed |= c.color("Accent, dark", "", &mut inp.cursor_accent_dark, Color::hex(0x9A6300));
            changed |= c.color("Outline", "", &mut inp.cursor_outline_color, Color::hex(0x0A0A0A));
            changed |= c.slider("Outline width", "", &mut inp.cursor_outline_scale, (0.0, 3.0), 0.1, times);
            changed |= c.slider("Roundness", "", &mut inp.cursor_corner_radius, (0.0, 1.0), 0.05, percent);
        });
        kit::note(ui, "The preview shows the colours. Outline width and roundness show on the real pointer.");
    }
    changed
}

/// A cursor tile's side and the gap between tiles, in logical pixels.
const TILE: f64 = 150.0;
const TILE_GAP: f64 = 12.0;
/// The room around the tiles inside their card.
const TILE_PAD: f64 = 20.0;

fn tiles_across(m: Metrics, width: f64) -> usize {
    let (tile, gap) = (m.px(TILE), m.px(TILE_GAP));
    (((width - m.px(TILE_PAD) * 2.0 + gap) / (tile + gap)).floor() as usize).max(1)
}

/// How tall the block of `count` cursor tiles is in a card `width` wide.
fn cursor_tiles_height(m: Metrics, width: f64, count: usize) -> f64 {
    let rows = count.div_ceil(tiles_across(m, width)).max(1) as f64;
    rows * m.px(TILE) + (rows - 1.0) * m.px(TILE_GAP) + m.px(TILE_PAD) * 2.0
}

/// The cursors as tiles, `(id, name, picture)` each, the one in use
/// ringed. Returns the id of one that was clicked and is not in use.
fn cursor_tiles(ui: &mut Ui, rect: Rect, tiles: &[(&str, &str, Option<ImageHandle>)], current: &str) -> Option<String> {
    let m = ui.m;
    let (side, gap, pad) = (m.px(TILE), m.px(TILE_GAP), m.px(TILE_PAD));
    let across = tiles_across(m, rect.width());
    let small = kit::small_style(ui);
    let radius = m.px(12.0);
    let accent = ui.theme.accent;
    let mut picked = None;
    ui.push_id("cursors");
    for (i, (id, name, image)) in tiles.iter().enumerate() {
        let at = rect.min + Vec2::new(pad + (i % across) as f64 * (side + gap), pad + (i / across) as f64 * (side + gap));
        let tile = Rect::from_min_size(at, Vec2::splat(side));
        let wid = ui.id("tile").with_index(i);
        let mut r = ui.interact(wid, tile, Sense::CLICK);
        ui.focusable(wid, tile);
        ui.key_click(wid, &mut r);
        let in_use = *id == current;
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        if r.clicked && !in_use {
            picked = Some((*id).to_owned());
        }
        ui.draw.rounded_rect(tile, radius, if r.hovered { look::BUTTON.scale_rgb(1.3) } else { look::BUTTON });
        if in_use {
            ui.draw.stroke_rect(tile, m.px(3.0), radius, accent);
        }
        let name_h = small.line_height() as f64 + m.px(12.0);
        if let Some(image) = image {
            let room = Rect::new(tile.min, Vec2::new(tile.max.x, tile.max.y - name_h));
            let fit = m.px(PREVIEW_SIZE) / image.width.max(image.height).max(1) as f64;
            let size = Vec2::new(image.width as f64 * fit, image.height as f64 * fit);
            ui.draw.image(Rect::from_center_size(room.center() + Vec2::new(0.0, m.px(6.0)), size).round(), *image, 0.0, Color::WHITE);
        }
        let label = Rect::new(Vec2::new(tile.min.x + m.px(8.0), tile.max.y - name_h), Vec2::new(tile.max.x - m.px(8.0), tile.max.y - m.px(6.0)));
        ui.text_centered(name, &small, label, if in_use { look::TEXT } else { look::TEXT_DIM });
        ui.focus_ring(wid, tile);
    }
    ui.pop_id();
    picked
}
