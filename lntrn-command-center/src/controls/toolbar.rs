//! Persisted toolbar layout — which widgets live in which zone and in
//! what order. Lives at `~/.lantern/config/command-center/toolbar.json`.
//!
//! Three ordered zones (Left / Middle / Right) plus an explicit
//! `disabled` list. A widget appears in exactly one of the four lists.
//! Storing `disabled` explicitly (rather than inferring it) lets us tell
//! "the user turned this off" apart from "this widget is brand new" — new
//! widgets added in a later build are auto-placed in their default zone.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::tile::Zone;
use super::TileId;

/// Min / max for the universal per-widget size multiplier.
pub const SIZE_MIN: f32 = 0.6;
pub const SIZE_MAX: f32 = 1.6;

/// Min / max for the universal per-widget extra spacing (logical px),
/// split as padding on both sides so it gaps the widget from its neighbours.
pub const SPACE_MIN: f32 = 0.0;
pub const SPACE_MAX: f32 = 80.0;

/// Where the clock's date sits relative to the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatePos {
    Below,
    Left,
    Right,
}

impl Default for DatePos {
    fn default() -> Self {
        DatePos::Below
    }
}

/// Clock-specific options (ignored by other widgets).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ClockOpts {
    #[serde(default = "yes")]
    pub show_date: bool,
    #[serde(default)]
    pub date_pos: DatePos,
    #[serde(default)]
    pub hour24: bool,
    #[serde(default)]
    pub seconds: bool,
}

fn yes() -> bool {
    true
}

impl Default for ClockOpts {
    fn default() -> Self {
        Self { show_date: true, date_pos: DatePos::Below, hour24: false, seconds: false }
    }
}

/// Per-widget options: a universal `size` plus any widget-specific extras.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WidgetOpts {
    #[serde(default = "one")]
    pub size: f32,
    /// Extra empty space (logical px) padded around the widget to gap it
    /// from its neighbours.
    #[serde(default)]
    pub space: f32,
    #[serde(default)]
    pub clock: ClockOpts,
}

fn one() -> f32 {
    1.0
}

impl Default for WidgetOpts {
    fn default() -> Self {
        Self { size: 1.0, space: 0.0, clock: ClockOpts::default() }
    }
}

#[derive(Debug, Clone)]
pub struct ToolbarLayout {
    pub left: Vec<TileId>,
    pub middle: Vec<TileId>,
    pub right: Vec<TileId>,
    pub disabled: Vec<TileId>,
    /// Per-widget options keyed by config key. Missing = defaults.
    pub options: HashMap<String, WidgetOpts>,
}

/// On-disk shape: zones as lists of widget config keys.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ToolbarFile {
    #[serde(default)]
    left: Vec<String>,
    #[serde(default)]
    middle: Vec<String>,
    #[serde(default)]
    right: Vec<String>,
    #[serde(default)]
    disabled: Vec<String>,
    #[serde(default)]
    options: HashMap<String, WidgetOpts>,
}

impl Default for ToolbarLayout {
    /// Reproduces the original hard-coded bar: connectivity + clock on
    /// the left, the system-stats cluster on the right. The middle zone
    /// starts empty (the now-playing controls float below the bar, not in
    /// the row) — it's there for the user to drag widgets into.
    fn default() -> Self {
        Self {
            left: vec![
                TileId::Workspace,
                TileId::Clock,
                TileId::Audio,
                TileId::Brightness,
                TileId::Wifi,
                TileId::Bluetooth,
            ],
            middle: Vec::new(),
            right: vec![
                TileId::Gaming,
                TileId::Disk,
                TileId::Gpu,
                TileId::Network,
                TileId::Temp,
                TileId::SysMon,
                TileId::Battery,
            ],
            disabled: Vec::new(),
            options: HashMap::new(),
        }
    }
}

impl ToolbarLayout {
    pub fn load() -> Self {
        let parsed: Option<ToolbarFile> = path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok());
        let Some(f) = parsed else {
            return Self::default();
        };

        let mut seen: HashSet<TileId> = HashSet::new();
        let mut left = keys_to_ids(&f.left, &mut seen);
        let mut middle = keys_to_ids(&f.middle, &mut seen);
        let mut right = keys_to_ids(&f.right, &mut seen);
        let mut disabled = keys_to_ids(&f.disabled, &mut seen);

        // Any arrangeable widget the saved file never mentioned is brand
        // new — drop it into whatever zone the default layout wants, so a
        // freshly-added widget shows up instead of silently vanishing.
        let def = Self::default();
        for &id in super::arrangeable_widgets() {
            if seen.contains(&id) {
                continue;
            }
            match def.zone_of(id) {
                Some(Zone::Left) => left.push(id),
                Some(Zone::Middle) => middle.push(id),
                Some(Zone::Right) => right.push(id),
                None => disabled.push(id),
            }
        }

        Self { left, middle, right, disabled, options: f.options }
    }

    pub fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = ToolbarFile {
            left: ids_to_keys(&self.left),
            middle: ids_to_keys(&self.middle),
            right: ids_to_keys(&self.right),
            disabled: ids_to_keys(&self.disabled),
            options: self.options.clone(),
        };
        let Ok(body) = serde_json::to_vec_pretty(&file) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, &body).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }

    /// This widget's options (a copy; defaults if none saved).
    pub fn opts(&self, id: TileId) -> WidgetOpts {
        self.options.get(id.config_key()).copied().unwrap_or_default()
    }

    /// Mutable handle to this widget's options, inserting defaults first.
    pub fn opts_mut(&mut self, id: TileId) -> &mut WidgetOpts {
        self.options
            .entry(id.config_key().to_string())
            .or_insert_with(WidgetOpts::default)
    }

    /// Which zone a widget is in, or `None` if it's disabled.
    pub fn zone_of(&self, id: TileId) -> Option<Zone> {
        if self.left.contains(&id) {
            Some(Zone::Left)
        } else if self.middle.contains(&id) {
            Some(Zone::Middle)
        } else if self.right.contains(&id) {
            Some(Zone::Right)
        } else {
            None
        }
    }

    /// The ordered widget list for a zone.
    pub fn zone(&self, zone: Zone) -> &[TileId] {
        match zone {
            Zone::Left => &self.left,
            Zone::Middle => &self.middle,
            Zone::Right => &self.right,
        }
    }

    fn zone_mut(&mut self, zone: Zone) -> &mut Vec<TileId> {
        match zone {
            Zone::Left => &mut self.left,
            Zone::Middle => &mut self.middle,
            Zone::Right => &mut self.right,
        }
    }

    /// Remove a widget from wherever it currently lives (any zone or the
    /// disabled list).
    fn detach(&mut self, id: TileId) {
        self.left.retain(|&w| w != id);
        self.middle.retain(|&w| w != id);
        self.right.retain(|&w| w != id);
        self.disabled.retain(|&w| w != id);
    }

    /// Move `id` into `zone` at `index` (clamped). Used by drag-drop.
    pub fn move_to(&mut self, id: TileId, zone: Zone, index: usize) {
        self.detach(id);
        let list = self.zone_mut(zone);
        let idx = index.min(list.len());
        list.insert(idx, id);
    }

    /// Disable a widget (move it to the tray).
    pub fn disable(&mut self, id: TileId) {
        self.detach(id);
        self.disabled.push(id);
    }
}

fn keys_to_ids(keys: &[String], seen: &mut HashSet<TileId>) -> Vec<TileId> {
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        if let Some(id) = TileId::from_config_key(k) {
            if seen.insert(id) {
                out.push(id);
            }
        }
    }
    out
}

fn ids_to_keys(ids: &[TileId]) -> Vec<String> {
    ids.iter().map(|id| id.config_key().to_string()).collect()
}

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".lantern/config/command-center/toolbar.json");
    Some(p)
}
