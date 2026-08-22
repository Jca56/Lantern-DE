//! Theme presets: snapshot a slice of `lantern.toml` (appearance + window
//! manager + windows + cursor) into a named file in `~/.lantern/themes/`,
//! then apply that file later to instantly switch the whole visual rig.
//!
//! ## Storage
//!
//! Each preset = one TOML file under `~/.lantern/themes/`, e.g.
//! `ocean-blue.toml`. The filename stem is the *slug*; the human-readable
//! name lives inside the file as a top-level `name` key. Each section is
//! optional — only keys actually set in the theme file get overlaid onto
//! the live config when applied.
//!
//! ## Active theme
//!
//! The slug of the currently-applied theme is tracked in
//! `appearance.active_theme`. We don't try to detect "live config drifted
//! from theme" — once applied, the slug stays set even if the user later
//! tweaks individual sliders. The "Update from current" context menu
//! item is how the user re-syncs a theme to their tweaks.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{AppearanceConfig, InputConfig, LanternConfig, WindowsConfig, WmConfig};

// ── Subset structs (all fields optional) ────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearancePreset {
    pub theme: Option<String>,
    pub accent: Option<String>,
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub wallpaper: Option<String>,
    pub background_color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WmPreset {
    pub border_width: Option<u32>,
    pub border_color: Option<String>,
    pub titlebar_height: Option<u32>,
    pub gap: Option<u32>,
    pub corner_radius: Option<u32>,
    pub focus_follows_mouse: Option<bool>,
    pub focus_glow: Option<bool>,
    pub focus_glow_color: Option<String>,
    pub focus_glow_intensity: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowsPreset {
    pub blur_intensity: Option<f32>,
    pub blur_tint: Option<f32>,
    pub blur_tint_color: Option<String>,
    pub blur_darken: Option<f32>,
    pub background_opacity: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorPreset {
    pub cursor_size: Option<u32>,
    pub cursor_theme: Option<String>,
}

/// Per-monitor wallpaper entry inside a theme. `name` matches a head from
/// wlr-output-management (e.g. "DP-1", "HDMI-A-1").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonitorWallpaper {
    pub name: String,
    pub wallpaper: String,
}

// ── Theme file ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeFile {
    pub name: String,
    pub appearance: AppearancePreset,
    pub window_manager: WmPreset,
    pub windows: WindowsPreset,
    pub input: CursorPreset,
    /// Optional per-output wallpapers. When present, each entry overrides the
    /// matching `[[monitors]].wallpaper` in lantern.toml on apply.
    #[serde(
        default,
        rename = "monitor_wallpaper",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub monitor_wallpapers: Vec<MonitorWallpaper>,
}

#[derive(Debug, Clone)]
pub struct ThemePreset {
    pub slug: String,
    pub file: ThemeFile,
}

// Convenience for callers building menus from a preset reference.

impl ThemePreset {
    pub fn name(&self) -> &str {
        &self.file.name
    }
    pub fn wallpaper(&self) -> Option<&str> {
        self.file
            .appearance
            .wallpaper
            .as_deref()
            .filter(|s| !s.is_empty())
    }
    /// Accent hex string fallback chain: appearance.accent → border_color → "#C8860A".
    pub fn accent_hex(&self) -> &str {
        self.file
            .appearance
            .accent
            .as_deref()
            .or(self.file.window_manager.border_color.as_deref())
            .unwrap_or("#C8860A")
    }
}

// ── Filesystem ──────────────────────────────────────────────────────────────

pub fn themes_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".lantern/themes")
}

fn ensure_dir() -> io::Result<()> {
    let dir = themes_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(())
}

pub fn list_themes() -> Vec<ThemePreset> {
    let dir = themes_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip the order manifest (and any other dot/underscore files we use
        // for bookkeeping).
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('_'))
            .unwrap_or(false)
        {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(slug) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut file) = toml::from_str::<ThemeFile>(&text) else {
            continue;
        };
        if file.name.is_empty() {
            file.name = slug.clone();
        }
        out.push(ThemePreset { slug, file });
    }

    // Sort by stored order; themes not in the order list go to the end,
    // alphabetized (so new themes don't randomly insert in the middle).
    let order = load_order();
    let order_idx = |slug: &str| order.iter().position(|s| s == slug);
    out.sort_by(|a, b| match (order_idx(&a.slug), order_idx(&b.slug)) {
        (Some(ai), Some(bi)) => ai.cmp(&bi),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.file.name.to_lowercase().cmp(&b.file.name.to_lowercase()),
    });
    out
}

// ── Ordering manifest ──────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct OrderFile {
    order: Vec<String>,
}

fn order_path() -> PathBuf {
    themes_dir().join("_order.toml")
}

fn load_order() -> Vec<String> {
    let path = order_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let parsed: OrderFile = toml::from_str(&text).unwrap_or_default();
    parsed.order
}

fn save_order(order: &[String]) -> io::Result<()> {
    ensure_dir()?;
    let file = OrderFile {
        order: order.to_vec(),
    };
    let text =
        toml::to_string_pretty(&file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(order_path(), text)
}

/// Direction of a reorder hop.
#[derive(Debug, Clone, Copy)]
pub enum MoveDir {
    Left,
    Right,
}

/// Move a theme one position left or right in the user's order. Themes not
/// yet in the order list get appended in their current list order before
/// the move. No-op if already at the edge.
pub fn move_theme(slug: &str, dir: MoveDir) -> io::Result<()> {
    // Build a complete order from `list_themes` (so unordered themes get a
    // stable position) and apply the move.
    let mut order: Vec<String> = list_themes().into_iter().map(|t| t.slug).collect();
    let Some(idx) = order.iter().position(|s| s == slug) else {
        return Ok(());
    };
    let new_idx = match dir {
        MoveDir::Left if idx > 0 => idx - 1,
        MoveDir::Right if idx + 1 < order.len() => idx + 1,
        _ => return Ok(()),
    };
    order.swap(idx, new_idx);
    save_order(&order)
}

pub fn save_theme(name: &str, cfg: &LanternConfig) -> io::Result<ThemePreset> {
    ensure_dir()?;
    let slug = unique_slug(&slugify(name));
    let file = capture(name, cfg);
    write_theme_file(&slug, &file)?;
    // Append to the order list so the new tile lands at the end.
    let mut order = load_order();
    if !order.contains(&slug) {
        order.push(slug.clone());
    }
    let _ = save_order(&order);
    Ok(ThemePreset { slug, file })
}

pub fn update_theme(slug: &str, name: &str, cfg: &LanternConfig) -> io::Result<()> {
    ensure_dir()?;
    let file = capture(name, cfg);
    write_theme_file(slug, &file)
}

pub fn rename_theme(slug: &str, new_name: &str) -> io::Result<String> {
    ensure_dir()?;
    // Load existing
    let path = themes_dir().join(format!("{slug}.toml"));
    let text = fs::read_to_string(&path)?;
    let mut file: ThemeFile =
        toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    file.name = new_name.to_string();
    // Slug stays the same to keep references stable.
    write_theme_file(slug, &file)?;
    Ok(slug.to_string())
}

pub fn delete_theme(slug: &str) -> io::Result<()> {
    let path = themes_dir().join(format!("{slug}.toml"));
    if path.exists() {
        fs::remove_file(path)?;
    }
    // Drop from the order list too.
    let mut order = load_order();
    order.retain(|s| s != slug);
    let _ = save_order(&order);
    Ok(())
}

fn write_theme_file(slug: &str, file: &ThemeFile) -> io::Result<()> {
    let path = themes_dir().join(format!("{slug}.toml"));
    let text =
        toml::to_string_pretty(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(path, text)
}

// ── Capture / apply ─────────────────────────────────────────────────────────

fn capture(name: &str, cfg: &LanternConfig) -> ThemeFile {
    // Snapshot every connected monitor that has its own wallpaper set, so a
    // multi-display layout round-trips through "Save theme" / "Apply theme"
    // with each output keeping its own picture.
    let monitor_wallpapers: Vec<MonitorWallpaper> = cfg
        .monitors
        .iter()
        .filter(|m| !m.wallpaper.is_empty())
        .map(|m| MonitorWallpaper {
            name: m.name.clone(),
            wallpaper: m.wallpaper.clone(),
        })
        .collect();

    ThemeFile {
        name: name.to_string(),
        appearance: AppearancePreset {
            theme: Some(cfg.appearance.theme.clone()),
            accent: Some(cfg.appearance.accent.clone()),
            font_family: Some(cfg.appearance.font_family.clone()),
            font_size: Some(cfg.appearance.font_size),
            wallpaper: Some(cfg.appearance.wallpaper.clone()),
            background_color: Some(cfg.appearance.background_color.clone()),
        },
        monitor_wallpapers,
        window_manager: WmPreset {
            border_width: Some(cfg.window_manager.border_width),
            border_color: Some(cfg.window_manager.border_color.clone()),
            titlebar_height: Some(cfg.window_manager.titlebar_height),
            gap: Some(cfg.window_manager.gap),
            corner_radius: Some(cfg.window_manager.corner_radius),
            focus_follows_mouse: Some(cfg.window_manager.focus_follows_mouse),
            focus_glow: Some(cfg.window_manager.focus_glow),
            focus_glow_color: Some(cfg.window_manager.focus_glow_color.clone()),
            focus_glow_intensity: Some(cfg.window_manager.focus_glow_intensity),
        },
        windows: WindowsPreset {
            blur_intensity: Some(cfg.windows.blur_intensity),
            blur_tint: Some(cfg.windows.blur_tint),
            blur_tint_color: Some(cfg.windows.blur_tint_color.clone()),
            blur_darken: Some(cfg.windows.blur_darken),
            background_opacity: Some(cfg.windows.background_opacity),
        },
        input: CursorPreset {
            cursor_size: Some(cfg.input.cursor_size),
            cursor_theme: Some(cfg.input.cursor_theme.clone()),
        },
    }
}

/// Overlay only the Some(_) keys from `preset` onto `cfg`. Anything the theme
/// doesn't specify is left untouched.
pub fn apply_theme(preset: &ThemePreset, cfg: &mut LanternConfig) {
    apply_appearance(&preset.file.appearance, &mut cfg.appearance);
    apply_wm(&preset.file.window_manager, &mut cfg.window_manager);
    apply_windows(&preset.file.windows, &mut cfg.windows);
    apply_cursor(&preset.file.input, &mut cfg.input);
    apply_monitor_wallpapers(&preset.file, cfg);
    cfg.appearance.active_theme = preset.slug.clone();
}

/// Resolve which wallpaper each connected monitor should now show, then write
/// it into the matching `MonitorEntry`. Priority for each monitor:
///   1. Theme's per-monitor wallpaper for this output name.
///   2. Theme's global `appearance.wallpaper` (cascade).
///   3. Whatever was already there (leave untouched).
///
/// Without this, switching themes just edits `appearance.wallpaper` while the
/// per-monitor overrides quietly win at the compositor, so the screen never
/// changes.
fn apply_monitor_wallpapers(theme: &ThemeFile, cfg: &mut LanternConfig) {
    let global = theme
        .appearance
        .wallpaper
        .as_deref()
        .filter(|s| !s.is_empty());

    for monitor in cfg.monitors.iter_mut() {
        let per_output = theme
            .monitor_wallpapers
            .iter()
            .find(|w| w.name == monitor.name)
            .map(|w| w.wallpaper.as_str())
            .filter(|s| !s.is_empty());

        if let Some(wp) = per_output {
            monitor.wallpaper = wp.to_string();
        } else if let Some(wp) = global {
            monitor.wallpaper = wp.to_string();
        }
    }
}

fn apply_appearance(p: &AppearancePreset, c: &mut AppearanceConfig) {
    if let Some(v) = &p.theme {
        c.theme = v.clone();
    }
    if let Some(v) = &p.accent {
        c.accent = v.clone();
    }
    if let Some(v) = &p.font_family {
        c.font_family = v.clone();
    }
    if let Some(v) = p.font_size {
        c.font_size = v;
    }
    if let Some(v) = &p.wallpaper {
        c.wallpaper = v.clone();
    }
    if let Some(v) = &p.background_color {
        c.background_color = v.clone();
    }
}

fn apply_wm(p: &WmPreset, c: &mut WmConfig) {
    if let Some(v) = p.border_width {
        c.border_width = v;
    }
    if let Some(v) = &p.border_color {
        c.border_color = v.clone();
    }
    if let Some(v) = p.titlebar_height {
        c.titlebar_height = v;
    }
    if let Some(v) = p.gap {
        c.gap = v;
    }
    if let Some(v) = p.corner_radius {
        c.corner_radius = v;
    }
    if let Some(v) = p.focus_follows_mouse {
        c.focus_follows_mouse = v;
    }
    if let Some(v) = p.focus_glow {
        c.focus_glow = v;
    }
    if let Some(v) = &p.focus_glow_color {
        c.focus_glow_color = v.clone();
    }
    if let Some(v) = p.focus_glow_intensity {
        c.focus_glow_intensity = v;
    }
}

fn apply_windows(p: &WindowsPreset, c: &mut WindowsConfig) {
    if let Some(v) = p.blur_intensity {
        c.blur_intensity = v;
    }
    if let Some(v) = p.blur_tint {
        c.blur_tint = v;
    }
    if let Some(v) = &p.blur_tint_color {
        c.blur_tint_color = v.clone();
    }
    if let Some(v) = p.blur_darken {
        c.blur_darken = v;
    }
    if let Some(v) = p.background_opacity {
        c.background_opacity = v;
    }
}

fn apply_cursor(p: &CursorPreset, c: &mut InputConfig) {
    if let Some(v) = p.cursor_size {
        c.cursor_size = v;
    }
    if let Some(v) = &p.cursor_theme {
        c.cursor_theme = v.clone();
    }
}

// ── Slug helpers ────────────────────────────────────────────────────────────

fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = true; // suppress leading dashes
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("theme");
    }
    out
}

fn unique_slug(base: &str) -> String {
    let dir = themes_dir();
    if !dir.join(format!("{base}.toml")).exists() {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !dir.join(format!("{candidate}.toml")).exists() {
            return candidate;
        }
    }
    base.to_string()
}
