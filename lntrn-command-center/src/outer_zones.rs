//! Outer-chrome zones — six drag targets wrapped around the collapsed
//! bar (Top Left / Middle / Right above it, Bottom Left / Middle / Right
//! below it) and the persisted layout mapping each floating widget to a
//! zone. Mirrors the in-bar toolbar zone system (`controls::toolbar`):
//! ordered lists per zone, saved to
//! `~/.lantern/config/command-center/outer_layout.json`, rearranged by
//! dragging in toolbar edit mode.
//!
//! Geometry lives here so rendering and hit-testing always agree. Some
//! widgets are zone-restricted: the slider block only fits the bottom
//! corners, the media card only the bottom row.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use lntrn_render::Rect;
use serde::{Deserialize, Serialize};

use crate::view_indicator::{
    BTN_GAP, BTN_SIZE, DOT_DIAMETER, DOT_GAP, GEAR_SIZE, RESTART_SIZE, SIDE_PAD, STRIP_OFFSET,
};

/// Gap between the bar's bottom edge and the bottom band (the old
/// slider/media `TOP_GAP`, kept so default positions don't move).
pub const BOTTOM_BAND_GAP: f32 = 12.0;
/// Bottom band height — sized to the tallest bottom widget (the
/// transparency/blur slider block).
pub const BOTTOM_BAND_H: f32 = crate::bar_sliders::BLOCK_H;
/// Top band height — covers the strip buttons with breathing room.
pub const TOP_BAND_H: f32 = 44.0;

// ── Widget + zone identities ───────────────────────────────────────────────

/// One draggable widget in the outer chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuterId {
    /// The red ✕ — closes/restarts the daemon.
    Close,
    /// Gear — Command Center settings page.
    Settings,
    /// Desktop-widgets popover button.
    Desktop,
    /// Claude usage button.
    Usage,
    Emojis,
    Clipboard,
    Notes,
    /// The view-switcher dots (move as one unit).
    Dots,
    /// Transparency + blur slider block (one unit).
    Sliders,
    /// Floating now-playing media card.
    Media,
}

pub const ALL_WIDGETS: [OuterId; 10] = [
    OuterId::Close,
    OuterId::Settings,
    OuterId::Desktop,
    OuterId::Usage,
    OuterId::Emojis,
    OuterId::Clipboard,
    OuterId::Notes,
    OuterId::Dots,
    OuterId::Sliders,
    OuterId::Media,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuterZone {
    TopLeft,
    TopMiddle,
    TopRight,
    BottomLeft,
    BottomMiddle,
    BottomRight,
}

pub const ALL_ZONES: [OuterZone; 6] = [
    OuterZone::TopLeft,
    OuterZone::TopMiddle,
    OuterZone::TopRight,
    OuterZone::BottomLeft,
    OuterZone::BottomMiddle,
    OuterZone::BottomRight,
];

impl OuterZone {
    pub fn is_top(self) -> bool {
        matches!(
            self,
            OuterZone::TopLeft | OuterZone::TopMiddle | OuterZone::TopRight
        )
    }

    /// Horizontal third (0 = left, 1 = middle, 2 = right).
    fn column(self) -> usize {
        match self {
            OuterZone::TopLeft | OuterZone::BottomLeft => 0,
            OuterZone::TopMiddle | OuterZone::BottomMiddle => 1,
            OuterZone::TopRight | OuterZone::BottomRight => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OuterZone::TopLeft => "TOP LEFT",
            OuterZone::TopMiddle => "TOP MIDDLE",
            OuterZone::TopRight => "TOP RIGHT",
            OuterZone::BottomLeft => "BOTTOM LEFT",
            OuterZone::BottomMiddle => "BOTTOM MIDDLE",
            OuterZone::BottomRight => "BOTTOM RIGHT",
        }
    }
}

impl OuterId {
    pub fn config_key(self) -> &'static str {
        match self {
            OuterId::Close => "close",
            OuterId::Settings => "settings",
            OuterId::Desktop => "desktop",
            OuterId::Usage => "usage",
            OuterId::Emojis => "emojis",
            OuterId::Clipboard => "clipboard",
            OuterId::Notes => "notes",
            OuterId::Dots => "dots",
            OuterId::Sliders => "sliders",
            OuterId::Media => "media",
        }
    }

    pub fn from_config_key(key: &str) -> Option<Self> {
        ALL_WIDGETS
            .iter()
            .copied()
            .find(|id| id.config_key() == key)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            OuterId::Close => "Close ✕",
            OuterId::Settings => "Settings",
            OuterId::Desktop => "Desktop",
            OuterId::Usage => "Claude Usage",
            OuterId::Emojis => "Emojis",
            OuterId::Clipboard => "Clipboard",
            OuterId::Notes => "Notes",
            OuterId::Dots => "View Dots",
            OuterId::Sliders => "Opacity & Blur",
            OuterId::Media => "Media Player",
        }
    }

    /// Whether this widget may live in `zone`. The slider block is too
    /// wide for the strip and pairs with the corners; the media card
    /// only fits the bottom row.
    pub fn allowed(self, zone: OuterZone) -> bool {
        match self {
            OuterId::Sliders => {
                matches!(zone, OuterZone::BottomLeft | OuterZone::BottomRight)
            }
            OuterId::Media => !zone.is_top(),
            _ => true,
        }
    }

    /// Logical (unscaled) size of the widget's slot.
    pub fn size(self) -> (f32, f32) {
        match self {
            OuterId::Close => (RESTART_SIZE, RESTART_SIZE),
            OuterId::Settings => (GEAR_SIZE, GEAR_SIZE),
            OuterId::Desktop
            | OuterId::Usage
            | OuterId::Emojis
            | OuterId::Clipboard
            | OuterId::Notes => (BTN_SIZE, BTN_SIZE),
            OuterId::Dots => {
                // Visible dots plus a 6 px hit pad on every side so the
                // unit is grabbable without pixel-perfect aim.
                let n = crate::app::PanelView::ALL.len() as f32;
                (
                    n * DOT_DIAMETER + (n - 1.0) * DOT_GAP + 12.0,
                    DOT_DIAMETER + 12.0,
                )
            }
            OuterId::Sliders => (crate::bar_sliders::BLOCK_W, crate::bar_sliders::BLOCK_H),
            OuterId::Media => (crate::media::render::CARD_W, crate::media::render::CARD_H),
        }
    }
}

// ── Persisted layout ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OuterLayout {
    pub top_left: Vec<OuterId>,
    pub top_middle: Vec<OuterId>,
    pub top_right: Vec<OuterId>,
    pub bottom_left: Vec<OuterId>,
    pub bottom_middle: Vec<OuterId>,
    pub bottom_right: Vec<OuterId>,
}

/// On-disk shape: zones as lists of widget config keys.
#[derive(Debug, Default, Serialize, Deserialize)]
struct OuterFile {
    #[serde(default)]
    top_left: Vec<String>,
    #[serde(default)]
    top_middle: Vec<String>,
    #[serde(default)]
    top_right: Vec<String>,
    #[serde(default)]
    bottom_left: Vec<String>,
    #[serde(default)]
    bottom_middle: Vec<String>,
    #[serde(default)]
    bottom_right: Vec<String>,
}

impl Default for OuterLayout {
    /// Reproduces the original hard-coded chrome: Close/Settings/Desktop
    /// on the top-left, dots centered, the page buttons top-right,
    /// sliders bottom-left, media card bottom-middle.
    fn default() -> Self {
        Self {
            top_left: vec![OuterId::Close, OuterId::Settings, OuterId::Desktop],
            top_middle: vec![OuterId::Dots],
            top_right: vec![
                OuterId::Usage,
                OuterId::Emojis,
                OuterId::Clipboard,
                OuterId::Notes,
            ],
            bottom_left: vec![OuterId::Sliders],
            bottom_middle: vec![OuterId::Media],
            bottom_right: Vec::new(),
        }
    }
}

impl OuterLayout {
    pub fn load() -> Self {
        let parsed: Option<OuterFile> = path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok());
        let Some(f) = parsed else {
            return Self::default();
        };

        let mut seen: HashSet<&'static str> = HashSet::new();
        let mut out = Self {
            top_left: keys_to_ids(&f.top_left, &mut seen),
            top_middle: keys_to_ids(&f.top_middle, &mut seen),
            top_right: keys_to_ids(&f.top_right, &mut seen),
            bottom_left: keys_to_ids(&f.bottom_left, &mut seen),
            bottom_middle: keys_to_ids(&f.bottom_middle, &mut seen),
            bottom_right: keys_to_ids(&f.bottom_right, &mut seen),
        };

        // A widget the saved file never mentioned is brand new — drop it
        // into its default zone so it shows up instead of vanishing.
        let def = Self::default();
        for id in ALL_WIDGETS {
            if seen.contains(id.config_key()) {
                continue;
            }
            let zone = def.zone_of(id).unwrap_or(OuterZone::TopRight);
            out.zone_mut(zone).push(id);
        }
        out
    }

    pub fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = OuterFile {
            top_left: ids_to_keys(&self.top_left),
            top_middle: ids_to_keys(&self.top_middle),
            top_right: ids_to_keys(&self.top_right),
            bottom_left: ids_to_keys(&self.bottom_left),
            bottom_middle: ids_to_keys(&self.bottom_middle),
            bottom_right: ids_to_keys(&self.bottom_right),
        };
        let Ok(body) = serde_json::to_vec_pretty(&file) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, &body).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }

    pub fn zone(&self, zone: OuterZone) -> &[OuterId] {
        match zone {
            OuterZone::TopLeft => &self.top_left,
            OuterZone::TopMiddle => &self.top_middle,
            OuterZone::TopRight => &self.top_right,
            OuterZone::BottomLeft => &self.bottom_left,
            OuterZone::BottomMiddle => &self.bottom_middle,
            OuterZone::BottomRight => &self.bottom_right,
        }
    }

    fn zone_mut(&mut self, zone: OuterZone) -> &mut Vec<OuterId> {
        match zone {
            OuterZone::TopLeft => &mut self.top_left,
            OuterZone::TopMiddle => &mut self.top_middle,
            OuterZone::TopRight => &mut self.top_right,
            OuterZone::BottomLeft => &mut self.bottom_left,
            OuterZone::BottomMiddle => &mut self.bottom_middle,
            OuterZone::BottomRight => &mut self.bottom_right,
        }
    }

    pub fn zone_of(&self, id: OuterId) -> Option<OuterZone> {
        ALL_ZONES.into_iter().find(|&z| self.zone(z).contains(&id))
    }

    /// Move `id` into `zone` at `index` (clamped). Used by drag-drop.
    pub fn move_to(&mut self, id: OuterId, zone: OuterZone, index: usize) {
        for z in ALL_ZONES {
            self.zone_mut(z).retain(|&w| w != id);
        }
        let list = self.zone_mut(zone);
        let idx = index.min(list.len());
        list.insert(idx, id);
    }
}

fn keys_to_ids(keys: &[String], seen: &mut HashSet<&'static str>) -> Vec<OuterId> {
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        if let Some(id) = OuterId::from_config_key(k) {
            if seen.insert(id.config_key()) {
                out.push(id);
            }
        }
    }
    out
}

fn ids_to_keys(ids: &[OuterId]) -> Vec<String> {
    ids.iter().map(|id| id.config_key().to_string()).collect()
}

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".lantern/config/command-center/outer_layout.json");
    Some(p)
}

// ── Geometry ───────────────────────────────────────────────────────────────

/// Compute every present widget's rect (physical px) for this frame.
/// `media_active = false` drops the media card (neighbors close ranks);
/// edit mode passes `true` so the card stays grabbable while idle.
pub fn positions(
    layout: &OuterLayout,
    panel: Rect,
    scale: f32,
    media_active: bool,
) -> Vec<(OuterId, Rect)> {
    let mut out = Vec::new();
    let gap = BTN_GAP * scale;
    for zone in ALL_ZONES {
        let ids: Vec<OuterId> = layout
            .zone(zone)
            .iter()
            .copied()
            .filter(|&id| id != OuterId::Media || media_active)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let total_w: f32 =
            ids.iter().map(|id| id.size().0 * scale).sum::<f32>() + gap * (ids.len() - 1) as f32;
        let third = panel.w / 3.0;
        let col_x = panel.x + third * zone.column() as f32;
        let mut x = match zone.column() {
            0 => panel.x + SIDE_PAD * scale,
            1 => col_x + (third - total_w) / 2.0,
            _ => panel.x + panel.w - SIDE_PAD * scale - total_w,
        };
        for id in ids {
            let (lw, lh) = id.size();
            let (w, h) = (lw * scale, lh * scale);
            // Top band: items center on the strip line above the panel.
            // Bottom band: items hang off the bar's bottom edge.
            let y = if zone.is_top() {
                panel.y - STRIP_OFFSET * scale - h / 2.0
            } else {
                panel.y + panel.h + BOTTOM_BAND_GAP * scale
            };
            out.push((id, Rect::new(x, y, w, h)));
            x += w + gap;
        }
    }
    out
}

/// The rect computed for `id` this frame, if it's present.
pub fn rect_in(positions: &[(OuterId, Rect)], id: OuterId) -> Option<Rect> {
    positions.iter().find(|(w, _)| *w == id).map(|(_, r)| *r)
}

fn contains(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

/// Whether the cursor is over `id`'s slot.
pub fn hit_id(positions: &[(OuterId, Rect)], id: OuterId, px: f32, py: f32) -> bool {
    rect_in(positions, id).is_some_and(|r| contains(r, px, py))
}

/// The widget under the cursor, if any. Used by edit mode to begin drags.
pub fn hit_widget(positions: &[(OuterId, Rect)], px: f32, py: f32) -> Option<OuterId> {
    positions
        .iter()
        .find(|(_, r)| contains(*r, px, py))
        .map(|(id, _)| *id)
}

/// Zone hint rect (physical px) for edit-mode drawing and drop bands.
pub fn zone_rect(zone: OuterZone, panel: Rect, scale: f32) -> Rect {
    let third = panel.w / 3.0;
    let x = panel.x + third * zone.column() as f32;
    if zone.is_top() {
        Rect::new(
            x,
            panel.y - TOP_BAND_H * scale,
            third,
            (TOP_BAND_H - 2.0) * scale,
        )
    } else {
        Rect::new(
            x,
            panel.y + panel.h + 2.0 * scale,
            third,
            (BOTTOM_BAND_GAP + BOTTOM_BAND_H) * scale,
        )
    }
}

/// Resolve where a released outer-widget drag lands: a zone plus an
/// insertion index, or `None` (cancel) when the cursor is outside both
/// bands or the zone is disallowed for this widget.
pub fn resolve_drop(
    layout: &OuterLayout,
    panel: Rect,
    scale: f32,
    id: OuterId,
    cursor: (f32, f32),
) -> Option<(OuterZone, usize)> {
    let (cx, cy) = cursor;
    let zone = ALL_ZONES
        .into_iter()
        .find(|&z| contains(zone_rect(z, panel, scale), cx, cy))?;
    if !id.allowed(zone) {
        return None;
    }
    // Insertion index by comparing against the zone's current items
    // (excluding the dragged widget). Media is included so indexes stay
    // stable whether or not something is playing.
    let pos = positions(layout, panel, scale, true);
    let idx = layout
        .zone(zone)
        .iter()
        .filter(|&&w| w != id)
        .filter_map(|&w| rect_in(&pos, w))
        .filter(|r| r.x + r.w / 2.0 < cx)
        .count();
    Some((zone, idx))
}
