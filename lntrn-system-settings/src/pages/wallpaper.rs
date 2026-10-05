//! The Wallpaper page: every picture in `~/.lantern/wallpapers` as a
//! tile, the one showing ringed in the accent. A click puts it up (the
//! compositor follows the file); with more than one screen, a strip
//! picks which screen a click is for.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lntrn_app::Waker;
use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_sys::dirs::{self, UserDir};
use lntrn_ui::{Action, AreaCx, CursorIcon, FILL, Sense, ShellRequest, Ui};

use crate::config::Config;
use crate::kit::{self, controls};
use crate::look;
use crate::thumbs::Thumbs;

/// The action a picked file comes back on.
pub const PICK: &str = "wallpaper.pick";
/// The picture formats the compositor shows and we can thumbnail.
const FORMATS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
/// The narrowest a tile gets before a column is dropped, and its shape.
const TILE_MIN: f64 = 230.0;
const TILE_ASPECT: f64 = 16.0 / 10.0;
/// Seconds between looks at the folder for pictures added or removed.
const LOOK_EVERY: f64 = 1.0;

struct Entry {
    path: PathBuf,
    name: String,
    image: Option<ImageHandle>,
    /// Its thumbnail came back empty: the file won't decode here.
    failed: bool,
}

#[derive(Default)]
pub struct WallpaperState {
    entries: Vec<Entry>,
    scanned: bool,
    /// The folder's modified time when it was last read.
    stamp: Option<SystemTime>,
    looked: f64,
    /// Which screen a click is for: 0 is all of them, `i + 1` is
    /// `config.monitors[i]`.
    target: usize,
    thumbs: Thumbs,
    /// Pictures of entries that went away, freed at the next upload.
    dead: Vec<ImageHandle>,
}

fn folder() -> Option<PathBuf> {
    dirs::lantern().map(|l| l.join("wallpapers"))
}

fn is_picture(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| FORMATS.contains(&e.to_ascii_lowercase().as_str()))
}

/// A file name as a title: no extension, separators as spaces.
fn title(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_default()
}

/// The part of a picture `aspect` wide that covers a tile `tile` wide
/// (both width over height) without stretching: centred, the overhang
/// cut off.
fn cover_uv(aspect: f64, tile: f64) -> Rect {
    if aspect > tile {
        let w = tile / aspect;
        Rect::from_xywh((1.0 - w) * 0.5, 0.0, w, 1.0)
    } else {
        let h = aspect / tile;
        Rect::from_xywh(0.0, (1.0 - h) * 0.5, 1.0, h)
    }
}

/// The wallpaper `target` shows: a screen's own, else the global one.
fn current(cfg: &Config, target: usize) -> &str {
    let own = target.checked_sub(1).and_then(|i| cfg.monitors.get(i)).map(|m| m.wallpaper.as_str()).filter(|w| !w.is_empty());
    own.unwrap_or(&cfg.appearance.wallpaper)
}

/// Put `path` up on `target`. For every screen at once the per-screen
/// overrides go, or they would keep covering it.
fn apply(cfg: &mut Config, target: usize, path: &str) {
    match target.checked_sub(1).and_then(|i| cfg.monitors.get_mut(i)) {
        Some(monitor) => monitor.wallpaper = path.to_owned(),
        None => {
            cfg.appearance.wallpaper = path.to_owned();
            for m in &mut cfg.monitors {
                m.wallpaper.clear();
            }
        }
    }
}

impl WallpaperState {
    pub fn set_waker(&mut self, waker: Waker) {
        self.thumbs.set_waker(waker);
    }

    /// Read the folder again on the next draw: a picture from elsewhere
    /// was just put up and belongs in the grid.
    pub fn rescan(&mut self) {
        self.scanned = false;
    }

    /// List the folder's pictures, then any wallpaper in use that lives
    /// somewhere else, keeping the thumbnails already made.
    fn scan(&mut self, cfg: &Config) {
        self.scanned = true;
        let dir = folder();
        self.stamp = dir.as_deref().and_then(|d| std::fs::metadata(d).ok()).and_then(|m| m.modified().ok());
        let mut paths: Vec<PathBuf> = dir
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_picture(p)).collect())
            .unwrap_or_default();
        paths.sort_by_key(|p| title(p).to_lowercase());
        let in_use = std::iter::once(cfg.appearance.wallpaper.as_str()).chain(cfg.monitors.iter().map(|m| m.wallpaper.as_str()));
        for used in in_use.filter(|u| !u.is_empty()).map(PathBuf::from) {
            if !paths.contains(&used) && used.is_file() {
                paths.push(used);
            }
        }
        let mut old = std::mem::take(&mut self.entries);
        for path in paths {
            let kept = old.iter().position(|e| e.path == path).map(|i| old.swap_remove(i));
            self.entries.push(kept.unwrap_or_else(|| Entry { name: title(&path), path, image: None, failed: false }));
        }
        self.dead.extend(old.into_iter().filter_map(|e| e.image));
        let wanted: Vec<(usize, PathBuf)> = self.entries.iter().enumerate().filter(|(_, e)| e.image.is_none() && !e.failed).map(|(i, e)| (i, e.path.clone())).collect();
        let keep: Vec<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
        self.thumbs.request(wanted, &keep);
    }

    /// Scan the first time, and again when the folder has changed.
    fn refresh(&mut self, now: f64, cfg: &Config) {
        if self.scanned && now - self.looked >= LOOK_EVERY {
            self.looked = now;
            let stamp = folder().and_then(|d| std::fs::metadata(d).ok()).and_then(|m| m.modified().ok());
            if stamp != self.stamp {
                self.scanned = false;
            }
        }
        if !self.scanned {
            self.looked = now;
            self.scan(cfg);
        }
    }

    /// Hand the thumbnails that landed to the GPU. Returns `true` when
    /// any did, so the frame is rebuilt with them showing.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        for handle in self.dead.drain(..) {
            images.remove(handle.id);
        }
        let landed = self.thumbs.take();
        for (slot, image) in &landed {
            let Some(entry) = self.entries.get_mut(*slot) else { continue };
            match image {
                Some(image) => entry.image = Some(images.add(gpu, image)),
                None => entry.failed = true,
            }
        }
        !landed.is_empty()
    }
}

/// A file chosen in the browser: put it up, or say why not.
pub fn pick(cfg: &mut Config, st: &mut WallpaperState, path: &str) -> Result<(), &'static str> {
    let file = Path::new(path);
    if !file.is_file() {
        return Err("That file is not there");
    }
    if !is_picture(file) {
        return Err("Not a picture the desktop can show");
    }
    apply(cfg, st.target, path);
    st.rescan();
    Ok(())
}

pub fn draw(cfg: &mut Config, st: &mut WallpaperState, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
    st.refresh(ui.now(), cfg);
    // Keep looking at the folder while the page shows.
    ui.state.request_redraw_after(LOOK_EVERY);
    if st.target > cfg.monitors.len() {
        st.target = 0;
    }
    let mut changed = false;
    if cfg.monitors.len() > 1 {
        kit::card(ui, "screens", |c| {
            let mut options = vec!["All screens"];
            options.extend(cfg.monitors.iter().map(|m| m.name.as_str()));
            c.segmented("Show on", "Which screen a click puts the picture on.", &mut st.target, &options);
        });
    }
    toolbar(ui, st.entries.len(), cx);
    if st.entries.is_empty() {
        kit::note(ui, "No pictures in ~/.lantern/wallpapers yet. Drop some in there, or browse for one.");
        return changed;
    }
    if let Some(path) = grid(ui, st, current(cfg, st.target)) {
        apply(cfg, st.target, &path);
        changed = true;
    }
    changed
}

/// How many pictures there are, and the buttons that find more.
fn toolbar(ui: &mut Ui, count: usize, cx: &mut AreaCx<()>) {
    let m = ui.m;
    let bar = ui.alloc(Vec2::new(FILL, m.px(controls::BUTTON_H)));
    let small = kit::small_style(ui);
    let text = if count == 1 { "1 picture in ~/.lantern/wallpapers".to_owned() } else { format!("{count} pictures in ~/.lantern/wallpapers") };
    ui.text_in_rect(&text, &small, Rect::new(Vec2::new(bar.min.x + m.px(5.0), bar.min.y), bar.max), look::TEXT_DIM);
    let mut right = bar.max.x;
    let mut button = |ui: &mut Ui, text: &str| {
        let w = controls::button_width(ui, text);
        let rect = Rect::new(Vec2::new(right - w, bar.min.y), Vec2::new(right, bar.max.y));
        right -= w + m.px(12.0);
        let id = ui.id(text);
        controls::button(ui, id, rect, text)
    };
    if button(ui, "Browse…") {
        let start = dirs::user_dir(UserDir::Pictures).or_else(dirs::home).unwrap_or_default();
        // The name is only a filter: the browser lists pictures.
        cx.request(ShellRequest::PathDialog { action: Action::new(PICK), save: false, suggest: start.join("*.png").display().to_string() });
    }
    if button(ui, "Open Folder")
        && let Some(dir) = folder()
    {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
    ui.space(m.px(10.0));
}

/// The tiles. Returns the picture that was clicked, when one was and it
/// is not the one already up.
fn grid(ui: &mut Ui, st: &WallpaperState, current: &str) -> Option<String> {
    let m = ui.m;
    let gap = m.px(24.0);
    let small = kit::small_style(ui);
    let name_h = small.line_height() as f64 + m.px(10.0);
    let w = ui.avail_width();
    let cols = (((w + gap) / (m.px(TILE_MIN) + gap)).floor() as usize).max(1);
    let tile_w = ((w - gap * (cols as f64 - 1.0)) / cols as f64).floor();
    let tile_h = (tile_w / TILE_ASPECT).round();
    let pitch = Vec2::new(tile_w + gap, tile_h + name_h + gap);
    let rows = st.entries.len().div_ceil(cols);
    let area = ui.alloc(Vec2::new(FILL, rows as f64 * pitch.y - gap));
    let radius = m.px(14.0);
    let accent = ui.theme.accent;
    let mut picked = None;
    ui.push_id("tiles");
    for (i, entry) in st.entries.iter().enumerate() {
        let at = area.min + Vec2::new((i % cols) as f64 * pitch.x, (i / cols) as f64 * pitch.y);
        let tile = Rect::from_min_size(at, Vec2::new(tile_w, tile_h));
        let id = ui.id("tile").with_index(i);
        let mut r = ui.interact(id, tile, Sense::CLICK);
        ui.focusable(id, tile);
        ui.key_click(id, &mut r);
        let up = Path::new(current) == entry.path;
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        if r.clicked && !up {
            picked = Some(entry.path.display().to_string());
        }

        // A ring just off the picture: the accent on the one that is up.
        let ring = m.px(4.0);
        if up || r.hovered {
            let off = m.px(4.0) + ring;
            ui.draw.stroke_rect(tile.expand(off), ring, radius + off, if up { accent } else { look::TRACK });
        }
        match entry.image {
            Some(image) => ui.draw.image_uv(tile, image, cover_uv(image.aspect(), tile_w / tile_h), radius, Color::WHITE),
            None => {
                ui.draw.rounded_rect(tile, radius, look::CARD);
                ui.draw.stroke_rect(tile, m.px(1.0), radius, look::LINE);
                ui.text_centered(if entry.failed { "No preview" } else { "Loading…" }, &small, tile, look::TEXT_DIM);
            }
        }
        if up {
            let c = Vec2::new(tile.max.x - m.px(26.0), tile.min.y + m.px(26.0));
            let s = m.px(8.0);
            ui.draw.circle(c, m.px(17.0), accent);
            ui.draw.polyline(&[c + Vec2::new(-s, 0.0), c + Vec2::new(-s * 0.25, s * 0.7), c + Vec2::new(s, -s * 0.7)], m.px(3.0), ui.theme.accent_text, false);
        }
        let name = Rect::from_min_size(Vec2::new(tile.min.x, tile.max.y + m.px(6.0)), Vec2::new(tile_w, small.line_height() as f64));
        ui.text_centered(&entry.name, &small, name, if up { look::TEXT } else { look::TEXT_DIM });
        ui.focus_ring(id, tile);
    }
    ui.pop_id();
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Monitor;

    #[test]
    fn a_picture_covers_its_tile_without_stretching() {
        // Wider than the tile: the sides go.
        let uv = cover_uv(2.0, 1.6);
        assert!((uv.width() - 0.8).abs() < 1e-9 && (uv.min.x - 0.1).abs() < 1e-9 && uv.height() == 1.0);
        // Taller: top and bottom go.
        let uv = cover_uv(1.0, 1.6);
        assert!((uv.height() - 0.625).abs() < 1e-9 && uv.width() == 1.0);
        assert_eq!(cover_uv(1.6, 1.6), Rect::from_xywh(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn a_click_is_for_one_screen_or_all_of_them() {
        let mut cfg = Config::empty();
        cfg.appearance.wallpaper = "/a.png".into();
        cfg.monitors = vec![Monitor { name: "eDP-1".into(), wallpaper: String::new() }, Monitor { name: "DP-2".into(), wallpaper: "/b.png".into() }];
        assert_eq!((current(&cfg, 0), current(&cfg, 1), current(&cfg, 2)), ("/a.png", "/a.png", "/b.png"));
        apply(&mut cfg, 1, "/c.png");
        assert_eq!((current(&cfg, 1), cfg.appearance.wallpaper.as_str()), ("/c.png", "/a.png"));
        // For all screens, the overrides go.
        apply(&mut cfg, 0, "/d.png");
        assert!(cfg.monitors.iter().all(|m| m.wallpaper.is_empty()));
        assert_eq!((current(&cfg, 1), current(&cfg, 2)), ("/d.png", "/d.png"));
        assert_eq!(title(Path::new("/w/Fox_in-the Ocean.png")), "Fox in the Ocean");
        assert!(is_picture(Path::new("a.JPG")) && !is_picture(Path::new("clip.mp4")));
    }
}
