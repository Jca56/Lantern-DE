//! Named looks: a slice of `lantern.toml` (appearance, window manager,
//! effects, cursor, wallpapers) snapshotted into `~/.lantern/themes/
//! <slug>.toml`, applied back later to switch the whole rig at once.
//!
//! The file name stem is the slug; the shown name is its top-level
//! `name` key. Every section is optional: applying overlays only the
//! keys the file has. `_order.toml` keeps the user's tile order.
//! `appearance.active_theme` remembers the slug last applied; it stays
//! set when sliders are tweaked afterwards, and "Update from Current"
//! is how a theme catches up.

use std::io;
use std::path::PathBuf;

use lntrn_data::{Doc, Map, from_doc, to_doc, toml};

use crate::config::{Config, Wallpapers, monitor_wallpapers};

/// A theme file, read.
#[derive(Clone, Debug)]
pub struct Preset {
    pub slug: String,
    pub name: String,
    pub doc: Doc,
}

impl Preset {
    /// The accent the theme sets, for its tile.
    pub fn accent(&self) -> Option<&str> {
        self.doc.path("appearance.accent")?.as_str().filter(|s| !s.is_empty())
    }
}

pub fn dir() -> PathBuf {
    lntrn_sys::dirs::lantern().unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern")).join("themes")
}

fn file_of(slug: &str) -> PathBuf {
    dir().join(format!("{slug}.toml"))
}

fn ensure_dir() -> io::Result<()> {
    std::fs::create_dir_all(dir())
}

fn read_file(path: &std::path::Path) -> Option<Doc> {
    let text = std::fs::read_to_string(path).ok()?;
    match toml::parse(&text) {
        Ok(d) => Some(d),
        Err(e) => {
            lntrn_core::log_warn!("{}: {e}", path.display());
            None
        }
    }
}

fn write_file(slug: &str, doc: &Doc) -> io::Result<()> {
    ensure_dir()?;
    std::fs::write(file_of(slug), toml::write(doc))
}

/// Every theme, in the user's order; new ones alphabetical at the end.
pub fn list() -> Vec<Preset> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if stem.starts_with('_') || path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(doc) = read_file(&path) else { continue };
        let name = doc.get("name").and_then(Doc::as_str).filter(|n| !n.is_empty()).unwrap_or(stem).to_owned();
        out.push(Preset { slug: stem.to_owned(), name, doc });
    }
    let order = load_order();
    let index = |slug: &str| order.iter().position(|s| s == slug);
    out.sort_by(|a, b| match (index(&a.slug), index(&b.slug)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    out
}

fn order_path() -> PathBuf {
    dir().join("_order.toml")
}

fn load_order() -> Vec<String> {
    let Some(doc) = read_file(&order_path()) else { return Vec::new() };
    doc.get("order").and_then(Doc::as_list).map(|l| l.iter().filter_map(|d| d.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn save_order(order: &[String]) -> io::Result<()> {
    ensure_dir()?;
    let mut doc = Doc::map();
    doc.set("order", Doc::List(order.iter().map(|s| Doc::Str(s.clone())).collect()));
    std::fs::write(order_path(), toml::write(&doc))
}

/// Move a theme one place up (`-1`) or down (`+1`) in the tile order.
pub fn shift(slug: &str, delta: i32) -> io::Result<()> {
    let mut order: Vec<String> = list().into_iter().map(|t| t.slug).collect();
    let Some(i) = order.iter().position(|s| s == slug) else { return Ok(()) };
    let j = i as i64 + delta as i64;
    if j < 0 || j >= order.len() as i64 {
        return Ok(());
    }
    order.swap(i, j as usize);
    save_order(&order)
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "theme".to_owned() } else { out }
}

fn unique_slug(base: &str) -> String {
    if !file_of(base).exists() {
        return base.to_owned();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|s| !file_of(s).exists()).unwrap_or_else(|| base.to_owned())
}

/// The keys a theme snapshots from each section.
const APPEARANCE_KEYS: &[&str] = &["theme", "accent", "font_family", "font_size", "wallpaper", "background_color"];
const WINDOWS_KEYS: &[&str] = &["blur_intensity", "blur_tint", "blur_tint_color", "blur_darken", "background_opacity"];
const INPUT_KEYS: &[&str] = &["cursor_size", "cursor_theme"];

fn subset(doc: Doc, keys: &[&str]) -> Doc {
    let Doc::Map(m) = doc else { return Doc::map() };
    Doc::Map(Map(m.0.into_iter().filter(|(k, _)| keys.contains(&k.as_str())).collect()))
}

/// The current look as a theme document.
fn capture(name: &str, cfg: &Config) -> Doc {
    let mut doc = Doc::map();
    doc.set("name", Doc::Str(name.to_owned()));
    doc.set("appearance", subset(to_doc(&cfg.appearance), APPEARANCE_KEYS));
    doc.set("window_manager", to_doc(&cfg.window_manager));
    doc.set("windows", subset(to_doc(&cfg.windows), WINDOWS_KEYS));
    doc.set("input", subset(to_doc(&cfg.input), INPUT_KEYS));
    let wallpapers: Vec<Doc> = monitor_wallpapers()
        .into_iter()
        .map(|(name, wp)| {
            let mut m = Doc::map();
            m.set("name", Doc::Str(name));
            m.set("wallpaper", Doc::Str(wp));
            m
        })
        .collect();
    if !wallpapers.is_empty() {
        doc.set("monitor_wallpapers", Doc::List(wallpapers));
    }
    doc
}

/// Save the current look under a new name; returns its slug.
pub fn save_new(name: &str, cfg: &Config) -> io::Result<String> {
    let slug = unique_slug(&slugify(name));
    write_file(&slug, &capture(name, cfg))?;
    let mut order = load_order();
    if !order.contains(&slug) {
        order.push(slug.clone());
        let _ = save_order(&order);
    }
    Ok(slug)
}

/// Overwrite a theme with the current look, keeping its name.
pub fn update(slug: &str, name: &str, cfg: &Config) -> io::Result<()> {
    write_file(slug, &capture(name, cfg))
}

/// Change the shown name; the slug (and `active_theme`) stay.
pub fn rename(slug: &str, new_name: &str) -> io::Result<()> {
    let mut doc = read_file(&file_of(slug)).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "theme file"))?;
    doc.set("name", Doc::Str(new_name.to_owned()));
    write_file(slug, &doc)
}

pub fn delete(slug: &str) -> io::Result<()> {
    let path = file_of(slug);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let mut order = load_order();
    order.retain(|s| s != slug);
    let _ = save_order(&order);
    Ok(())
}

/// Overlay the theme on the config: only the keys it has change. Its
/// wallpapers reach `[[monitors]]` on the next save.
pub fn apply(preset: &Preset, cfg: &mut Config) {
    let d = &preset.doc;
    if let Some(s) = d.get("appearance") {
        from_doc(&mut cfg.appearance, s);
    }
    if let Some(s) = d.get("window_manager") {
        from_doc(&mut cfg.window_manager, s);
    }
    if let Some(s) = d.get("windows") {
        from_doc(&mut cfg.windows, s);
    }
    if let Some(s) = d.get("input") {
        from_doc(&mut cfg.input, s);
    }
    let per_output = d
        .get("monitor_wallpapers")
        .and_then(Doc::as_list)
        .map(|l| l.iter().filter_map(|m| Some((m.get("name")?.as_str()?.to_owned(), m.get("wallpaper")?.as_str()?.to_owned()))).collect())
        .unwrap_or_default();
    let global = d.path("appearance.wallpaper").and_then(Doc::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    cfg.pending_wallpapers = Some(Wallpapers { global, per_output });
    cfg.appearance.active_theme = preset.slug.clone();
}
