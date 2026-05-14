//! Square toggle button that floats under the left view arrow. Toggles the
//! desktop clock widget on/off by writing `~/.lantern/config/desktop-widgets.json`.

use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect};
use serde::{Deserialize, Serialize};

use crate::view_arrows::{button_rect as arrow_rect, Side, BTN_W};

/// Square button — matches the arrow width.
pub const BTN_SIZE: f32 = BTN_W;
/// Vertical gap between the arrow's bottom edge and this button.
pub const GAP_BELOW_ARROW: f32 = 14.0;
const RADIUS: f32 = 14.0;
const BG_RGB: (u8, u8, u8) = (24, 24, 24);
const BG_ALPHA_IDLE: f32 = 0.55;
const BG_ALPHA_HOVER: f32 = 0.90;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

/// Compute the button rect (physical pixels) anchored beneath the left arrow.
pub fn button_rect(panel: Rect, scale: f32) -> Rect {
    let arrow = arrow_rect(panel, scale, Side::Left);
    let size = BTN_SIZE * scale;
    let gap = GAP_BELOW_ARROW * scale;
    Rect::new(arrow.x, arrow.y + arrow.h + gap, size, size)
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
    let radius = RADIUS * scale;
    let bg_a = if hovered { BG_ALPHA_HOVER } else { BG_ALPHA_IDLE };
    let bg = Color::from_rgb8(BG_RGB.0, BG_RGB.1, BG_RGB.2).with_alpha(bg_a * alpha);
    painter.rect_filled(r, radius, bg);

    let accent = Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2);
    if hovered {
        painter.rect_stroke_sdf(r, radius, 2.0 * scale, accent.with_alpha(0.65 * alpha));
    }

    // Clock-face icon. Color tracks state: on = accent, off = dim white.
    let icon_color = if clock_on {
        accent.with_alpha(0.95 * alpha)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.40 * alpha)
    };
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let face_radius = (r.w / 2.0) - 6.0 * scale;
    let stroke = 2.0 * scale;
    painter.circle_stroke(cx, cy, face_radius, stroke, icon_color);
    // Hour hand — points to 1 o'clock-ish for visual interest.
    let hour_len = face_radius * 0.5;
    let hh_x = cx + hour_len * 0.5;
    let hh_y = cy - hour_len * 0.85;
    painter.line_round(cx, cy, hh_x, hh_y, stroke, icon_color);
    // Minute hand — points to 12 (straight up).
    let min_len = face_radius * 0.75;
    painter.line_round(cx, cy, cx, cy - min_len, stroke, icon_color);
    // Center pip.
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
