//! Desktop Settings — top-strip button + popover overlay that toggles
//! the desktop widgets daemon's per-feature config flags (clock,
//! audio visualizer, …).
//!
//! Mirrors the `WidgetsConfig` defined in `lntrn-desktop`. Persists to
//! `~/.lantern/config/desktop-widgets.json`; the desktop daemon
//! watches the file via inotify and reloads on change.

use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use serde::{Deserialize, Serialize};

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const INACTIVE_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const INACTIVE_ALPHA: f32 = 0.35;

// ── Button on the top strip ───────────────────────────────────────────────

/// Hit-rect for the button — lives in the old "clock" slot of the
/// top strip.
pub fn button_rect(panel: Rect, scale: f32) -> Rect {
    crate::view_indicator::desktop_button_rect(panel, scale)
}

pub fn hit_test_button(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    let r = button_rect(panel, scale);
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

/// Draw a small monitor / display glyph in the strip button.
pub fn draw_button(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered: bool,
    popover_open: bool,
) {
    let r = button_rect(panel, scale);
    let color = if hovered || popover_open {
        Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
    } else {
        Color::from_rgb8(INACTIVE_RGB.0, INACTIVE_RGB.1, INACTIVE_RGB.2)
            .with_alpha(INACTIVE_ALPHA * alpha)
    };
    let stroke = 2.0 * scale;
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;

    // Monitor body — rounded rect screen.
    let body_w = r.w * 0.78;
    let body_h = r.w * 0.54;
    let body_x = cx - body_w / 2.0;
    let body_y = cy - body_h / 2.0 - r.w * 0.06;
    let radius = r.w * 0.07;
    painter.rect_stroke_sdf(
        Rect::new(body_x, body_y, body_w, body_h),
        radius,
        stroke,
        color,
    );

    // Stand neck.
    let neck_w = body_w * 0.18;
    let neck_h = r.w * 0.10;
    painter.rect_filled(
        Rect::new(cx - neck_w / 2.0, body_y + body_h, neck_w, neck_h),
        1.0 * scale,
        color,
    );

    // Base.
    let base_w = body_w * 0.55;
    let base_h = r.w * 0.06;
    let base_x = cx - base_w / 2.0;
    let base_y = body_y + body_h + neck_h;
    painter.rect_filled(
        Rect::new(base_x, base_y, base_w, base_h),
        base_h * 0.5,
        color,
    );
}

// ── Full-body page layout & rendering ─────────────────────────────────────

const PAD: f32 = 20.0;
const TITLE_FONT: f32 = 28.0;
const ROW_FONT: f32 = 18.0;
const ROW_H: f32 = 56.0;
const ROW_GAP: f32 = 10.0;
const TOGGLE_W: f32 = 52.0;
const TOGGLE_H: f32 = 28.0;
const KNOB_PAD: f32 = 3.0;

const ROW_BG: (u8, u8, u8) = (40, 36, 32);
const ROW_BORDER_ALPHA: f32 = 0.10;

#[derive(Copy, Clone)]
pub enum PageHit {
    Background,
    ClockToggle,
    VisualizerToggle,
    RainbowToggle,
}

fn clock_row_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = PAD * scale;
    let body_x = panel.x + pad;
    let body_w = panel.w - pad * 2.0;
    let row_y = top_y + pad + TITLE_FONT * scale + 18.0 * scale;
    Rect::new(body_x, row_y, body_w, ROW_H * scale)
}

fn visualizer_row_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let clock_r = clock_row_rect(panel, top_y, scale);
    Rect::new(clock_r.x, clock_r.y + clock_r.h + ROW_GAP * scale, clock_r.w, clock_r.h)
}

fn rainbow_row_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let viz_r = visualizer_row_rect(panel, top_y, scale);
    Rect::new(viz_r.x, viz_r.y + viz_r.h + ROW_GAP * scale, viz_r.w, viz_r.h)
}

fn toggle_rect_in(row: Rect, scale: f32) -> Rect {
    let w = TOGGLE_W * scale;
    let h = TOGGLE_H * scale;
    Rect::new(row.x + row.w - w - 16.0 * scale, row.y + (row.h - h) / 2.0, w, h)
}

pub fn hit_test_page(panel: Rect, top_y: f32, scale: f32, px: f32, py: f32) -> PageHit {
    if point_in(clock_row_rect(panel, top_y, scale), px, py) {
        return PageHit::ClockToggle;
    }
    if point_in(visualizer_row_rect(panel, top_y, scale), px, py) {
        return PageHit::VisualizerToggle;
    }
    if point_in(rainbow_row_rect(panel, top_y, scale), px, py) {
        return PageHit::RainbowToggle;
    }
    PageHit::Background
}

/// Returns Some(rect) hovered if the cursor is inside any toggle row,
/// for hover styling.
pub fn hovered_row(panel: Rect, top_y: f32, scale: f32, px: f32, py: f32) -> Option<HoverRow> {
    if point_in(clock_row_rect(panel, top_y, scale), px, py) {
        return Some(HoverRow::Clock);
    }
    if point_in(visualizer_row_rect(panel, top_y, scale), px, py) {
        return Some(HoverRow::Visualizer);
    }
    if point_in(rainbow_row_rect(panel, top_y, scale), px, py) {
        return Some(HoverRow::Rainbow);
    }
    None
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum HoverRow {
    Clock,
    Visualizer,
    Rainbow,
}

fn point_in(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

#[allow(clippy::too_many_arguments)]
pub fn draw_page(
    painter: &mut Painter,
    text: &mut TextRenderer,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    cfg: &WidgetsConfig,
    hovered: Option<HoverRow>,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = PAD * scale;
    let body_x = panel.x + pad;
    let body_w = panel.w - pad * 2.0;

    // Page title.
    let title_font = TITLE_FONT * scale;
    text.queue(
        "Desktop Widgets",
        title_font,
        body_x,
        top_y + pad,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
        body_w,
        surface_w,
        surface_h,
    );

    draw_row(
        painter,
        text,
        clock_row_rect(panel, top_y, scale),
        "Clock",
        cfg.clock_enabled,
        hovered == Some(HoverRow::Clock),
        scale,
        alpha,
        surface_w,
        surface_h,
    );
    draw_row(
        painter,
        text,
        visualizer_row_rect(panel, top_y, scale),
        "Audio Visualizer",
        cfg.visualizer_enabled,
        hovered == Some(HoverRow::Visualizer),
        scale,
        alpha,
        surface_w,
        surface_h,
    );
    draw_row(
        painter,
        text,
        rainbow_row_rect(panel, top_y, scale),
        "Rainbow",
        cfg.rainbow_enabled,
        hovered == Some(HoverRow::Rainbow),
        scale,
        alpha,
        surface_w,
        surface_h,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: Rect,
    label: &str,
    on: bool,
    hovered: bool,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let radius = 10.0 * scale;
    let bg_alpha = if hovered { 0.85 } else { 0.55 };
    painter.rect_filled(
        rect,
        radius,
        Color::from_rgb8(ROW_BG.0, ROW_BG.1, ROW_BG.2).with_alpha(bg_alpha * alpha),
    );
    painter.rect_stroke_sdf(
        rect,
        radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(ROW_BORDER_ALPHA * alpha),
    );

    let row_font = ROW_FONT * scale;
    let label_y = rect.y + (rect.h - row_font) / 2.0;
    text.queue(
        label,
        row_font,
        rect.x + 14.0 * scale,
        label_y,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.95 * alpha),
        rect.w * 0.7,
        surface_w,
        surface_h,
    );

    draw_toggle(painter, toggle_rect_in(rect, scale), scale, alpha, on);
}

fn draw_toggle(painter: &mut Painter, r: Rect, scale: f32, alpha: f32, on: bool) {
    let radius = r.h * 0.5;
    let bg = if on {
        Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.95 * alpha)
    } else {
        Color::from_rgb8(0x55, 0x55, 0x55).with_alpha(0.85 * alpha)
    };
    painter.rect_filled(r, radius, bg);
    let knob_d = r.h - 2.0 * KNOB_PAD * scale;
    let knob_y = r.y + KNOB_PAD * scale;
    let knob_x = if on {
        r.x + r.w - KNOB_PAD * scale - knob_d
    } else {
        r.x + KNOB_PAD * scale
    };
    painter.rect_filled(
        Rect::new(knob_x, knob_y, knob_d, knob_d),
        knob_d * 0.5,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.98 * alpha),
    );
}

// ── Persisted config (mirrors lntrn-desktop's WidgetsConfig) ──────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WidgetsConfig {
    #[serde(default = "default_clock_enabled")]
    pub clock_enabled: bool,
    #[serde(default = "default_visualizer_enabled")]
    pub visualizer_enabled: bool,
    #[serde(default)]
    pub rainbow_enabled: bool,
    /// Persisted top-left position from the desktop daemon — preserved
    /// across CC toggles so dragging isn't lost when the user flips the
    /// switch off and on again.
    #[serde(default)]
    pub rainbow_x: Option<f32>,
    #[serde(default)]
    pub rainbow_y: Option<f32>,
}

fn default_clock_enabled() -> bool {
    true
}
fn default_visualizer_enabled() -> bool {
    false
}

impl Default for WidgetsConfig {
    fn default() -> Self {
        Self {
            clock_enabled: default_clock_enabled(),
            visualizer_enabled: default_visualizer_enabled(),
            rainbow_enabled: false,
            rainbow_x: None,
            rainbow_y: None,
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

/// Atomic save: write to .tmp then rename. The desktop daemon watches
/// IN_MOVED_TO so this triggers a single reload event.
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

pub fn toggle_clock() -> WidgetsConfig {
    let mut cfg = load();
    cfg.clock_enabled = !cfg.clock_enabled;
    save(&cfg);
    cfg
}

pub fn toggle_visualizer() -> WidgetsConfig {
    let mut cfg = load();
    cfg.visualizer_enabled = !cfg.visualizer_enabled;
    save(&cfg);
    cfg
}

pub fn toggle_rainbow() -> WidgetsConfig {
    let mut cfg = load();
    cfg.rainbow_enabled = !cfg.rainbow_enabled;
    save(&cfg);
    cfg
}
