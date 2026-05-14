//! Clock-face toggle glyph rendered into the top strip's "Clock" slot.
//! Toggles the desktop clock widget on/off by writing
//! `~/.lantern/config/desktop-widgets.json`.

use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect};
use serde::{Deserialize, Serialize};

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const INACTIVE_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const INACTIVE_ALPHA: f32 = 0.35;

/// Hit-rect for the clock toggle — owned by `view_indicator` so it
/// lines up with the rest of the top strip.
pub fn button_rect(panel: Rect, scale: f32) -> Rect {
    crate::view_indicator::clock_rect(panel, scale)
}

pub fn hit_test(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    let r = button_rect(panel, scale);
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

pub fn draw(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered: bool,
    clock_on: bool,
) {
    let r = button_rect(panel, scale);
    // Match the rest of the top-strip glyphs: white when idle/off,
    // accent when active (clock enabled) or hovered. No background
    // plate — strip is unified.
    let icon_color = if hovered || clock_on {
        Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
    } else {
        Color::from_rgb8(INACTIVE_RGB.0, INACTIVE_RGB.1, INACTIVE_RGB.2)
            .with_alpha(INACTIVE_ALPHA * alpha)
    };
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let face_radius = (r.w / 2.0) - 3.0 * scale;
    let stroke = 2.0 * scale;
    painter.circle_stroke(cx, cy, face_radius, stroke, icon_color);
    // Hour hand — points to ~1 o'clock for visual interest.
    let hour_len = face_radius * 0.5;
    let hh_x = cx + hour_len * 0.5;
    let hh_y = cy - hour_len * 0.85;
    painter.line_round(cx, cy, hh_x, hh_y, stroke, icon_color);
    // Minute hand — straight up.
    let min_len = face_radius * 0.75;
    painter.line_round(cx, cy, cx, cy - min_len, stroke, icon_color);
    painter.circle_filled(cx, cy, stroke * 0.8, icon_color);
}

// ── Config file I/O ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WidgetsConfig {
    #[serde(default = "default_clock_enabled")]
    pub clock_enabled: bool,
}

fn default_clock_enabled() -> bool {
    true
}

impl Default for WidgetsConfig {
    fn default() -> Self {
        Self {
            clock_enabled: default_clock_enabled(),
        }
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/config/desktop-widgets.json")
}

pub fn load() -> WidgetsConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Atomic save: write to tmp, rename into place. The desktop daemon watches
/// IN_MOVED_TO on the config directory so this triggers a single reload event.
pub fn save(cfg: &WidgetsConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_string_pretty(cfg) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Flip the persistent clock_enabled state. Returns the new value.
pub fn toggle_clock() -> bool {
    let mut cfg = load();
    cfg.clock_enabled = !cfg.clock_enabled;
    save(&cfg);
    cfg.clock_enabled
}
