//! Command Center settings page — real, functional controls.
//!
//! Settings persist to `~/.lantern/config/command-center/settings.toml`
//! as a flat key/value file (manual mini-parser, no `toml` crate).
//! Each change writes the file immediately so they survive daemon
//! restarts.

use std::fs;
use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

// ── Visual constants ───────────────────────────────────────────────────────

const TITLE_FONT: f32 = 30.0;
const SECTION_FONT: f32 = 16.0;
const ROW_FONT: f32 = 18.0;
const VALUE_FONT: f32 = 16.0;
const ROW_H: f32 = 64.0;
const ROW_GAP: f32 = 8.0;
const SECTION_GAP: f32 = 18.0;
const PAD: f32 = 32.0;
const ROW_RADIUS: f32 = 12.0;
const ROW_PAD_X: f32 = 18.0;

// Slider track + knob.
const SLIDER_W: f32 = 220.0;
const SLIDER_H: f32 = 8.0;
const KNOB_R: f32 = 9.0;

// Toggle pill.
const TOGGLE_W: f32 = 48.0;
const TOGGLE_H: f32 = 24.0;
const KNOB_PAD: f32 = 3.0;

const ROW_BG_RGB: (u8, u8, u8) = (24, 24, 24);
const ROW_BG_ALPHA: f32 = 0.55;
const ROW_BORDER_ALPHA: f32 = 0.10;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

// ── Persisted config ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Config {
    /// Panel surface alpha (0.10 → 1.00).
    pub panel_opacity: f32,
    /// When true, every fresh open of the panel starts in collapsed
    /// mode.
    pub open_collapsed: bool,
    /// When false, the mini-dock under the bar is hidden even in
    /// collapsed mode (chevron + clock only).
    pub show_dock_collapsed: bool,
    /// Unified text size (logical px) used by every Command Center
    /// view — terminal cells, Files rows, search input, etc.
    pub text_size: f32,
    /// View-switch slide duration in seconds.
    pub view_anim_duration: f32,
    /// When true, expanding the panel doesn't grow the bar — instead
    /// a separate body window appears below the bar with a small gap.
    /// The bar (controls row) stays put at the top.
    pub panel_split: bool,
    /// Vertical gap (logical px) between the bar and the body window
    /// when split mode is active.
    pub panel_split_gap: f32,
    /// Extra width (logical px) added to the expanded panel. 0 → base
    /// width. User-drivable via the "Expanded width" slider in Settings.
    pub panel_grow_w: f32,
    /// Extra width (logical px) added to the collapsed bar. Same idea,
    /// independent slider so collapsed + expanded can size separately.
    pub bar_grow_w: f32,
}

/// Max bonus width (logical px) the expanded-panel resize slider allows.
/// Matches the upper end of the old 3-step grow cycle so existing
/// comfortable sizes are still reachable.
pub const GROW_W_MAX: f32 = 400.0;

/// Max bonus width (logical px) the collapsed-bar slider allows.
/// Larger than the panel cap so the bar can stretch nearly edge-to-edge
/// on wide monitors (1000 base + 1400 = 2400 logical px).
pub const BAR_GROW_W_MAX: f32 = 1400.0;

impl Default for Config {
    fn default() -> Self {
        Self {
            panel_opacity: 0.92,
            open_collapsed: false,
            show_dock_collapsed: true,
            text_size: 22.0,
            view_anim_duration: 1.20,
            panel_split: false,
            panel_split_gap: 50.0,
            panel_grow_w: 0.0,
            bar_grow_w: 0.0,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(text) => parse(&text),
            Err(_) => {
                let cfg = Self::default();
                cfg.save();
                cfg
            }
        }
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::warn!(?e, "failed to create settings dir");
                return;
            }
        }
        let body = render_text(self);
        if let Err(e) = fs::write(&path, body) {
            tracing::warn!(?e, "failed to save settings");
        }
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut p = PathBuf::from(home);
    p.push(".lantern/config/command-center/settings.toml");
    p
}

fn parse(text: &str) -> Config {
    let mut cfg = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match key {
            "panel_opacity" => {
                if let Ok(v) = value.parse::<f32>() {
                    cfg.panel_opacity = v.clamp(0.10, 1.0);
                }
            }
            "open_collapsed" => cfg.open_collapsed = value == "true",
            "show_dock_collapsed" => cfg.show_dock_collapsed = value == "true",
            "text_size" => {
                if let Ok(v) = value.parse::<f32>() {
                    cfg.text_size = v.clamp(12.0, 32.0);
                }
            }
            // Back-compat: prior versions stored the size as the
            // terminal-specific key. Read it as the unified value so
            // existing settings carry over after upgrade.
            "terminal_font_size" => {
                if let Ok(v) = value.parse::<f32>() {
                    cfg.text_size = v.clamp(12.0, 32.0);
                }
            }
            // Dropped settings — silently ignore the old keys.
            "terminal_output_size" | "files_text_size" | "wifi_backend" => {}
            "view_anim_duration" => {
                if let Ok(v) = value.parse::<f32>() {
                    cfg.view_anim_duration = v.clamp(0.10, 3.0);
                }
            }
            "panel_split" => cfg.panel_split = value == "true",
            "panel_split_gap" => {
                if let Ok(v) = value.parse::<f32>() {
                    cfg.panel_split_gap = v.clamp(0.0, 120.0);
                }
            }
            "panel_grow_w" => {
                if let Ok(v) = value.parse::<f32>() {
                    cfg.panel_grow_w = v.clamp(0.0, GROW_W_MAX);
                }
            }
            "bar_grow_w" => {
                if let Ok(v) = value.parse::<f32>() {
                    cfg.bar_grow_w = v.clamp(0.0, BAR_GROW_W_MAX);
                }
            }
            _ => {}
        }
    }
    cfg
}

fn render_text(c: &Config) -> String {
    format!(
        "# lntrn-command-center settings\n\
         panel_opacity = {:.3}\n\
         open_collapsed = {}\n\
         show_dock_collapsed = {}\n\
         text_size = {:.1}\n\
         view_anim_duration = {:.2}\n\
         panel_split = {}\n\
         panel_split_gap = {:.1}\n\
         panel_grow_w = {:.1}\n\
         bar_grow_w = {:.1}\n",
        c.panel_opacity,
        c.open_collapsed,
        c.show_dock_collapsed,
        c.text_size,
        c.view_anim_duration,
        c.panel_split,
        c.panel_split_gap,
        c.panel_grow_w,
        c.bar_grow_w,
    )
}

// ── Row enumeration ────────────────────────────────────────────────────────

/// Identifies a single setting. Used by the hit-test/draw code to map
/// rows to which field to read/mutate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    PanelOpacity,
    OpenCollapsed,
    ShowDockCollapsed,
    PanelSplit,
    PanelSplitGap,
    TextSize,
    ViewAnimDuration,
    /// Extra width added to the expanded panel.
    PanelGrowW,
    /// Extra width added to the collapsed bar.
    BarGrowW,
}

impl SettingKey {
    /// True for sliders that change the visible panel's geometry while
    /// the user is dragging. Those need deferred commit (apply on
    /// release, not every frame) to avoid the slider track sliding out
    /// from under the cursor as the panel resizes.
    pub fn defers_during_drag(self) -> bool {
        matches!(self, SettingKey::PanelGrowW)
    }
}

#[derive(Debug, Clone, Copy)]
enum RowKind {
    Toggle,
    /// Slider: (min, max, unit suffix)
    Slider(f32, f32, &'static str),
}

struct RowDef {
    key: SettingKey,
    label: &'static str,
    kind: RowKind,
}

struct SectionDef {
    title: &'static str,
    rows: &'static [RowDef],
}

const SECTIONS: &[SectionDef] = &[
    SectionDef {
        title: "Appearance",
        rows: &[
            RowDef {
                key: SettingKey::PanelOpacity,
                label: "Panel opacity",
                kind: RowKind::Slider(0.10, 1.0, "%"),
            },
            RowDef {
                key: SettingKey::TextSize,
                label: "Text size",
                kind: RowKind::Slider(12.0, 32.0, "pt"),
            },
            RowDef {
                key: SettingKey::ViewAnimDuration,
                label: "View slide duration",
                kind: RowKind::Slider(0.20, 2.50, "s"),
            },
        ],
    },
    SectionDef {
        title: "Behavior",
        rows: &[
            RowDef {
                key: SettingKey::OpenCollapsed,
                label: "Open in collapsed mode",
                kind: RowKind::Toggle,
            },
            RowDef {
                key: SettingKey::ShowDockCollapsed,
                label: "Show pinned dock when collapsed",
                kind: RowKind::Toggle,
            },
            RowDef {
                key: SettingKey::PanelSplit,
                label: "Split bar and panel into separate windows",
                kind: RowKind::Toggle,
            },
            RowDef {
                key: SettingKey::PanelSplitGap,
                label: "Split gap",
                kind: RowKind::Slider(0.0, 120.0, "px"),
            },
        ],
    },
    SectionDef {
        title: "Size",
        rows: &[
            RowDef {
                key: SettingKey::PanelGrowW,
                label: "Expanded width",
                kind: RowKind::Slider(0.0, GROW_W_MAX, "px"),
            },
            RowDef {
                key: SettingKey::BarGrowW,
                label: "Collapsed width",
                kind: RowKind::Slider(0.0, BAR_GROW_W_MAX, "px"),
            },
        ],
    },
];

// ── Layout + hit-test ──────────────────────────────────────────────────────

/// Per-row layout in physical px so the renderer and the hit-test stay
/// in lockstep without recomputing twice.
#[derive(Debug, Clone, Copy)]
pub struct RowLayout {
    pub key: SettingKey,
    pub rect: Rect,
    pub control: ControlLayout,
}

#[derive(Debug, Clone, Copy)]
pub enum ControlLayout {
    Toggle(Rect),
    /// Slider track rect + min/max range.
    Slider(Rect, f32, f32),
}

/// Build the per-row layouts for the current panel rect. Caller uses
/// this for both drawing and click/drag hit-testing.
pub fn layout(panel: Rect, top_y: f32, scale: f32) -> Vec<RowLayout> {
    let mut out = Vec::new();
    let pad = PAD * scale;
    let body_x = panel.x + pad;
    let body_w = panel.w - pad * 2.0;
    let mut y = top_y + pad + TITLE_FONT * scale + 18.0 * scale;
    let row_h = ROW_H * scale;
    let row_pad_x = ROW_PAD_X * scale;

    for section in SECTIONS {
        y += SECTION_FONT * scale + 10.0 * scale;
        for (i, row) in section.rows.iter().enumerate() {
            let rect = Rect::new(body_x, y, body_w, row_h);
            let control = match row.kind {
                RowKind::Toggle => {
                    let tw = TOGGLE_W * scale;
                    let th = TOGGLE_H * scale;
                    let tx = body_x + body_w - row_pad_x - tw;
                    let ty = y + (row_h - th) / 2.0;
                    ControlLayout::Toggle(Rect::new(tx, ty, tw, th))
                }
                RowKind::Slider(min, max, _) => {
                    let sw = SLIDER_W * scale;
                    let sh = SLIDER_H * scale;
                    let sx = body_x + body_w - row_pad_x - sw;
                    let sy = y + (row_h - sh) / 2.0;
                    ControlLayout::Slider(Rect::new(sx, sy, sw, sh), min, max)
                }
            };
            out.push(RowLayout { key: row.key, rect, control });
            y += row_h;
            if i + 1 < section.rows.len() {
                y += ROW_GAP * scale;
            }
        }
        y += SECTION_GAP * scale;
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub enum SettingHit {
    Toggle(SettingKey),
    SliderSeek(SettingKey, f32),
}

pub fn hit_test(rows: &[RowLayout], px: f32, py: f32) -> Option<SettingHit> {
    for row in rows {
        match row.control {
            ControlLayout::Toggle(r) => {
                if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
                    return Some(SettingHit::Toggle(row.key));
                }
            }
            ControlLayout::Slider(r, min, max) => {
                // Generous y-tolerance so users don't need pixel-perfect aim.
                let y_min = r.y - 16.0;
                let y_max = r.y + r.h + 16.0;
                if px >= r.x && px <= r.x + r.w && py >= y_min && py <= y_max {
                    let t = ((px - r.x) / r.w).clamp(0.0, 1.0);
                    let value = min + (max - min) * t;
                    return Some(SettingHit::SliderSeek(row.key, value));
                }
            }
        }
    }
    None
}

/// Hit-test that *only* matches a slider — used when the user is
/// already mid-drag, so we don't accidentally hand off to a different
/// row when the cursor wanders.
pub fn hit_slider_only(rows: &[RowLayout], key: SettingKey, px: f32) -> Option<f32> {
    for row in rows {
        if row.key != key {
            continue;
        }
        if let ControlLayout::Slider(r, min, max) = row.control {
            let t = ((px - r.x) / r.w).clamp(0.0, 1.0);
            return Some(min + (max - min) * t);
        }
    }
    None
}

pub fn current_value(cfg: &Config, key: SettingKey) -> SettingValue {
    match key {
        SettingKey::PanelOpacity => SettingValue::F(cfg.panel_opacity),
        SettingKey::OpenCollapsed => SettingValue::B(cfg.open_collapsed),
        SettingKey::ShowDockCollapsed => SettingValue::B(cfg.show_dock_collapsed),
        SettingKey::PanelSplit => SettingValue::B(cfg.panel_split),
        SettingKey::PanelSplitGap => SettingValue::F(cfg.panel_split_gap),
        SettingKey::TextSize => SettingValue::F(cfg.text_size),
        SettingKey::ViewAnimDuration => SettingValue::F(cfg.view_anim_duration),
        SettingKey::PanelGrowW => SettingValue::F(cfg.panel_grow_w),
        SettingKey::BarGrowW => SettingValue::F(cfg.bar_grow_w),
    }
}

pub fn apply_value(cfg: &mut Config, key: SettingKey, value: SettingValue) {
    match (key, value) {
        (SettingKey::PanelOpacity, SettingValue::F(v)) => cfg.panel_opacity = v.clamp(0.10, 1.0),
        (SettingKey::OpenCollapsed, SettingValue::B(v)) => cfg.open_collapsed = v,
        (SettingKey::ShowDockCollapsed, SettingValue::B(v)) => cfg.show_dock_collapsed = v,
        (SettingKey::PanelSplit, SettingValue::B(v)) => cfg.panel_split = v,
        (SettingKey::PanelSplitGap, SettingValue::F(v)) => cfg.panel_split_gap = v.clamp(0.0, 120.0),
        (SettingKey::TextSize, SettingValue::F(v)) => cfg.text_size = v.clamp(12.0, 32.0),
        (SettingKey::ViewAnimDuration, SettingValue::F(v)) => cfg.view_anim_duration = v.clamp(0.10, 3.0),
        (SettingKey::PanelGrowW, SettingValue::F(v)) => cfg.panel_grow_w = v.clamp(0.0, GROW_W_MAX),
        (SettingKey::BarGrowW, SettingValue::F(v)) => cfg.bar_grow_w = v.clamp(0.0, BAR_GROW_W_MAX),
        _ => {}
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SettingValue {
    F(f32),
    B(bool),
}

// ── Drawing ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    cfg: &Config,
    pending: Option<(SettingKey, f32)>,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = PAD * scale;
    let body_x = panel.x + pad;
    let body_w = panel.w - pad * 2.0;

    // Title.
    let title_font = TITLE_FONT * scale;
    text.queue(
        "Settings",
        title_font,
        body_x,
        top_y + pad,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
        body_w,
        surface_w,
        surface_h,
    );

    let rows = layout(panel, top_y, scale);
    let mut row_iter = rows.iter().peekable();

    let mut section_y = top_y + pad + title_font + 18.0 * scale;
    for section in SECTIONS {
        if section_y >= panel.y + panel.h {
            break;
        }
        // Section header.
        let sf = SECTION_FONT * scale;
        text.queue(
            section.title,
            sf,
            body_x,
            section_y,
            Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.85 * alpha),
            body_w,
            surface_w,
            surface_h,
        );
        section_y += sf + 10.0 * scale;

        for (i, row_def) in section.rows.iter().enumerate() {
            let Some(layout) = row_iter.next() else { break };
            draw_row(painter, text, layout, row_def, cfg, pending, scale, alpha, surface_w, surface_h);
            section_y = layout.rect.y + layout.rect.h;
            if i + 1 < section.rows.len() {
                section_y += ROW_GAP * scale;
            }
        }
        section_y += SECTION_GAP * scale;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    layout: &RowLayout,
    row_def: &RowDef,
    cfg: &Config,
    pending: Option<(SettingKey, f32)>,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let row_radius = ROW_RADIUS * scale;
    // Background plate.
    painter.rect_filled(
        layout.rect,
        row_radius,
        Color::from_rgb8(ROW_BG_RGB.0, ROW_BG_RGB.1, ROW_BG_RGB.2).with_alpha(ROW_BG_ALPHA * alpha),
    );
    painter.rect_stroke_sdf(
        layout.rect,
        row_radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(ROW_BORDER_ALPHA * alpha),
    );

    let row_font = ROW_FONT * scale;
    let label_x = layout.rect.x + ROW_PAD_X * scale;
    let label_y = layout.rect.y + (layout.rect.h - row_font) / 2.0;
    text.queue(
        row_def.label,
        row_font,
        label_x,
        label_y,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.95 * alpha),
        layout.rect.w * 0.55,
        surface_w,
        surface_h,
    );

    match layout.control {
        ControlLayout::Toggle(r) => {
            let on = matches!(current_value(cfg, layout.key), SettingValue::B(true));
            draw_toggle(painter, r, scale, alpha, on);
        }
        ControlLayout::Slider(r, min, max) => {
            // Tentative drag value (e.g. for the Expanded width slider)
            // takes precedence over the committed config value so the
            // knob tracks the cursor live even when the panel itself
            // hasn't resized yet.
            let pending_value = pending
                .filter(|(k, _)| *k == layout.key)
                .map(|(_, v)| v);
            let value = pending_value.unwrap_or_else(|| {
                match current_value(cfg, layout.key) {
                    SettingValue::F(v) => v,
                    _ => 0.0,
                }
            });
            let unit = match row_def.kind {
                RowKind::Slider(_, _, u) => u,
                _ => "",
            };
            draw_slider(painter, text, r, scale, alpha, value, min, max, unit, surface_w, surface_h);
        }
    }
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

#[allow(clippy::too_many_arguments)]
fn draw_slider(
    painter: &mut Painter,
    text: &mut TextRenderer,
    r: Rect,
    scale: f32,
    alpha: f32,
    value: f32,
    min: f32,
    max: f32,
    unit: &str,
    surface_w: u32,
    surface_h: u32,
) {
    let track_radius = r.h * 0.5;
    let track_bg = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.18 * alpha);
    painter.rect_filled(r, track_radius, track_bg);

    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    let filled_w = r.w * t;
    if filled_w > 0.0 {
        painter.rect_filled(
            Rect::new(r.x, r.y, filled_w, r.h),
            track_radius,
            Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.95 * alpha),
        );
    }

    let knob_r = KNOB_R * scale;
    let knob_cx = r.x + filled_w;
    let knob_cy = r.y + r.h / 2.0;
    painter.circle_filled(
        knob_cx,
        knob_cy,
        knob_r,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
    );

    // Value label to the right of the slider (small).
    let value_str = format_value(value, unit);
    let vf = VALUE_FONT * scale;
    let vw = text.measure_width(&value_str, vf);
    text.queue(
        &value_str,
        vf,
        r.x - vw - 12.0 * scale,
        r.y + (r.h - vf) / 2.0,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.65 * alpha),
        vw,
        surface_w,
        surface_h,
    );
}

fn format_value(value: f32, unit: &str) -> String {
    match unit {
        "%" => format!("{}%", (value * 100.0).round() as i32),
        "pt" => format!("{} pt", value.round() as i32),
        "s" => format!("{:.2} s", value),
        _ => format!("{:.2}", value),
    }
}
