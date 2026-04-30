//! Battery control tile.
//!
//! Inline tile is a horizontal pill that fills left-to-right proportional
//! to charge level. Color zones:
//!
//!   1-24  %  → red
//!   25-49 %  → yellow
//!   50-100 % → green
//!
//! Click-expand shows percentage, charging status, time-to-empty (or
//! time-to-full when charging).
//!
//! Reads from `/sys/class/power_supply/BAT0` (or first BAT* found).
//! Polls every 15s on each render frame; no thread needed at this rate.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

// ── State ───────────────────────────────────────────────────────────────────

const POLL_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

impl Status {
    /// Human-readable label. Currently only used by diagnostics — the
    /// expanded panel signals charging via the bolt and time-remaining
    /// direction instead of a text label.
    #[allow(dead_code)]
    fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "On Battery",
            Self::Full => "Fully Charged",
            Self::NotCharging => "Not Charging",
            Self::Unknown => "Unknown",
        }
    }
}

pub struct Battery {
    sysfs_path: Option<PathBuf>,
    capacity: u32,
    status: Status,
    energy_now_uwh: u64,
    energy_full_uwh: u64,
    power_now_uw: u64,
    /// Current `charge_control_end_threshold` value (0 = unsupported).
    /// 100 means "no limit"; anything < 100 caps charging at that %.
    charge_limit: u32,
    /// Whether the kernel exposes `charge_control_end_threshold` for
    /// this battery. False on most desktops + laptops without firmware
    /// support; the toggle UI hides itself in that case.
    has_charge_limit: bool,
    last_poll: Instant,
}

impl Battery {
    pub fn new() -> Self {
        let sysfs_path = find_battery();
        let has_charge_limit = sysfs_path
            .as_ref()
            .map(|p| p.join("charge_control_end_threshold").exists())
            .unwrap_or(false);
        let mut bat = Self {
            sysfs_path,
            capacity: 0,
            status: Status::Unknown,
            energy_now_uwh: 0,
            energy_full_uwh: 0,
            power_now_uw: 0,
            charge_limit: 100,
            has_charge_limit,
            // Force the first poll on the first tick.
            last_poll: Instant::now() - POLL_INTERVAL,
        };
        bat.tick();
        bat
    }

    /// True when a battery was found at startup. (When false the inline
    /// tile draws nothing — desktop machine, etc.)
    pub fn is_present(&self) -> bool {
        self.sysfs_path.is_some()
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// Re-read sysfs if enough time has passed.
    pub fn tick(&mut self) {
        if self.last_poll.elapsed() < POLL_INTERVAL {
            return;
        }
        self.last_poll = Instant::now();
        let Some(path) = &self.sysfs_path else { return };

        if let Ok(s) = std::fs::read_to_string(path.join("capacity")) {
            self.capacity = s.trim().parse().unwrap_or(0).min(100);
        }
        if let Ok(s) = std::fs::read_to_string(path.join("status")) {
            self.status = match s.trim() {
                "Charging" => Status::Charging,
                "Discharging" => Status::Discharging,
                "Full" => Status::Full,
                "Not charging" => Status::NotCharging,
                _ => Status::Unknown,
            };
        }
        // Optional fields — silently ignore if absent.
        self.energy_now_uwh = read_u64(path.join("energy_now"));
        self.energy_full_uwh = read_u64(path.join("energy_full"));
        self.power_now_uw = read_u64(path.join("power_now"));
        if self.has_charge_limit {
            self.charge_limit = read_u64(path.join("charge_control_end_threshold")) as u32;
        }
    }

    /// Whether the laptop firmware exposes a charge-end threshold we
    /// can write. The toggle UI hides if false.
    pub fn supports_charge_limit(&self) -> bool {
        self.has_charge_limit
    }

    /// Current charge-end threshold (1-100). Reads as 100 when there
    /// is no limit set or no support.
    #[allow(dead_code)] // diagnostic getter; the UI uses `charge_limited`
    pub fn charge_limit(&self) -> u32 {
        self.charge_limit
    }

    /// True when an 80% cap is currently active (any value <100 counts).
    pub fn charge_limited(&self) -> bool {
        self.has_charge_limit && self.charge_limit < 100
    }

    /// Toggle the charge limit between 80% and 100%. No-op if the
    /// kernel doesn't expose `charge_control_end_threshold`.
    pub fn toggle_charge_limit(&mut self) {
        if !self.has_charge_limit {
            return;
        }
        let Some(path) = &self.sysfs_path else { return };
        let new_limit = if self.charge_limited() { 100 } else { 80 };
        let target = path.join("charge_control_end_threshold");
        match std::fs::write(&target, format!("{}\n", new_limit)) {
            Ok(_) => {
                self.charge_limit = new_limit;
                tracing::info!(limit = new_limit, "charge limit updated");
            }
            Err(e) => {
                tracing::warn!(?e, ?target, "failed to write charge limit");
            }
        }
    }

    /// Estimate seconds until full or empty. Returns `None` when we
    /// can't compute (no power_now reading, or steady-state Full/Unknown).
    pub fn time_remaining_secs(&self) -> Option<u64> {
        if self.power_now_uw == 0 {
            return None;
        }
        match self.status {
            Status::Discharging => {
                if self.energy_now_uwh == 0 {
                    return None;
                }
                // hours = energy_now / power_now → seconds = *3600
                Some(self.energy_now_uwh.saturating_mul(3600) / self.power_now_uw)
            }
            Status::Charging => {
                if self.energy_full_uwh <= self.energy_now_uwh {
                    return Some(0);
                }
                let remaining = self.energy_full_uwh - self.energy_now_uwh;
                Some(remaining.saturating_mul(3600) / self.power_now_uw)
            }
            _ => None,
        }
    }
}

fn read_u64(path: PathBuf) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn find_battery() -> Option<PathBuf> {
    let base = Path::new("/sys/class/power_supply");
    let entries = std::fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("BAT") {
            let path = entry.path();
            if path.join("capacity").exists() {
                return Some(path);
            }
        }
    }
    None
}

// ── Inline pill ─────────────────────────────────────────────────────────────

const PILL_WIDTH: f32 = 120.0;
const PILL_HEIGHT: f32 = 28.0;
const PILL_LABEL_FONT: f32 = 22.0;
/// Right-margin from the panel's right edge.
const PILL_RIGHT_MARGIN: f32 = 4.0;
/// Gap between the pill and the percentage label.
const PILL_LABEL_GAP: f32 = 10.0;

/// Logical px the battery tile asks for in the row layout. Wide enough
/// for "100% [pill]" with the optional ⚡ to the left of the number.
pub const TILE_WIDTH: f32 = 60.0 /* number / bolt */ + 10.0 + 120.0 + 4.0;
/// Logical px of the lightning bolt's *bounding box* (when charging).
/// The visible glyph fills 80 % of this height (10 % pad top + bottom)
/// so it matches the percentage-label cap-height visually without the
/// polygon kissing the row's edge.
const BOLT_HEIGHT: f32 = 30.0;

/// Track (background) and fill colors per zone, both as 8-bit sRGB
/// triples passed through `Color::from_rgb8` for gamma-correctness.
const PILL_TRACK_RGB: (u8, u8, u8) = (60, 60, 60); // dark grey track
const RED_FILL_RGB: (u8, u8, u8) = (220, 60, 50);
const YELLOW_FILL_RGB: (u8, u8, u8) = (240, 190, 40);
const GREEN_FILL_RGB: (u8, u8, u8) = (50, 200, 100);

fn fill_rgb(capacity: u32) -> (u8, u8, u8) {
    if capacity < 25 {
        RED_FILL_RGB
    } else if capacity < 50 {
        YELLOW_FILL_RGB
    } else {
        GREEN_FILL_RGB
    }
}

pub fn draw_inline(
    painter: &mut Painter,
    text: &mut TextRenderer,
    battery: &Battery,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    if !battery.is_present() {
        return;
    }

    let pill_w = PILL_WIDTH * scale;
    let pill_h = PILL_HEIGHT * scale;
    let label_font = PILL_LABEL_FONT * scale;
    let right_margin = PILL_RIGHT_MARGIN * scale;
    let label_gap = PILL_LABEL_GAP * scale;

    // Anchor: right edge of the layout slot. The pill sits flush right
    // with `right_margin` breathing room from the panel edge; the
    // percentage label sits to its *left* with `label_gap`.
    let pill_right = layout.x + layout.w - right_margin;
    let pill_x = pill_right - pill_w;
    let pill_y = layout.y + (layout.h - pill_h) / 2.0;

    let radius = pill_h * 0.5;
    let cap = battery.capacity();
    let (fr, fg, fb) = fill_rgb(cap);

    // Track (full pill, dimmed bg).
    painter.rect_filled(
        Rect::new(pill_x, pill_y, pill_w, pill_h),
        radius,
        Color::from_rgb8(PILL_TRACK_RGB.0, PILL_TRACK_RGB.1, PILL_TRACK_RGB.2)
            .with_alpha(alpha),
    );

    // Fill (proportional, left-to-right).
    if cap > 0 {
        // Minimum visible fill width so very low % still shows a sliver.
        let raw = pill_w * (cap as f32 / 100.0);
        let fill_w = raw.max(pill_h);
        painter.rect_filled(
            Rect::new(pill_x, pill_y, fill_w, pill_h),
            radius,
            Color::from_rgb8(fr, fg, fb).with_alpha(alpha),
        );
    }

    // To the LEFT of the pill: number is always shown; the suffix
    // swaps based on charging status — ⚡ when charging, "%" otherwise.
    let charging = matches!(battery.status(), Status::Charging);
    let number_str = format!("{}", cap);
    let percent_str = "%";

    let number_w = text.measure_width(&number_str, label_font);
    let suffix_w = if charging {
        BOLT_HEIGHT * scale * 0.55 // bolt's bounding-box width
    } else {
        text.measure_width(percent_str, label_font)
    };

    let total_w = number_w + suffix_w;
    let group_x = pill_x - label_gap - total_w;
    let label_y = layout.y + (layout.h - label_font) / 2.0;
    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);

    text.queue(
        &number_str,
        label_font,
        group_x,
        label_y,
        white,
        number_w,
        surface_w,
        surface_h,
    );

    if charging {
        let bolt_box_h = BOLT_HEIGHT * scale;
        let bolt_box_w = suffix_w;
        let bolt_x = group_x + number_w;
        let bolt_y = layout.y + (layout.h - bolt_box_h) / 2.0;
        draw_bolt(painter, bolt_x, bolt_y, bolt_box_w, bolt_box_h, alpha);
    } else {
        text.queue(
            percent_str,
            label_font,
            group_x + number_w,
            label_y,
            white,
            suffix_w,
            surface_w,
            surface_h,
        );
    }
}

/// Draw a stylised lightning bolt as two triangles joined at the
/// middle. We can't use `Painter::polygon` because that fan-triangulates
/// from points[0] and requires the polygon to be convex — the bolt's
/// notch makes it concave and the fan paints triangles that overshoot
/// the silhouette. Two explicit triangles render the shape correctly.
///
/// The visible glyph fills 80 % of the bounding box height (10 %
/// padding top + bottom) so it can't kiss the row's edge.
fn draw_bolt(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let pad_y = 0.10;
    let map_y = |py: f32| pad_y + py * (1.0 - 2.0 * pad_y);
    // Helper: bounding-box fraction → pixel coords.
    let pt = |fx: f32, fy: f32| (x + fx * w, y + map_y(fy) * h);

    //   apex ─────────╮
    //                 ╲    upper triangle: apex, midpoint, mid-notch
    //                  ╲
    //   left-elbow ────mid (notch)
    //                  ╲
    //                   ╲  lower triangle: notch, mid, bottom-tip
    //                bottom-tip
    let apex = pt(0.85, 0.0);
    let left_elbow = pt(0.0, 0.55);
    let mid_notch = pt(0.50, 0.50);
    let right_elbow = pt(1.0, 0.45);
    let bottom_tip = pt(0.20, 1.0);

    let bolt_color = Color::from_rgb8(0xff, 0xd0, 0x40).with_alpha(alpha);

    // Upper half: apex (top-right) → left elbow → notch.
    painter.triangle(
        apex.0, apex.1,
        left_elbow.0, left_elbow.1,
        mid_notch.0, mid_notch.1,
        bolt_color,
    );
    // Lower half: notch → bottom tip → right elbow.
    painter.triangle(
        mid_notch.0, mid_notch.1,
        bottom_tip.0, bottom_tip.1,
        right_elbow.0, right_elbow.1,
        bolt_color,
    );
}

// ── Click-expand panel ──────────────────────────────────────────────────────

#[allow(dead_code)] // not used since the panel mode swap; retained for reference.
pub const EXPANDED_HEIGHT: f32 = 130.0;

const DETAIL_PERCENT_FONT: f32 = 64.0;
const DETAIL_TIME_FONT: f32 = 18.0;
const DETAIL_TOP_PAD: f32 = 16.0;
const DETAIL_TIME_GAP: f32 = 4.0;

const TOGGLE_LABEL_FONT: f32 = 18.0;
const TOGGLE_LABEL_GAP: f32 = 12.0;
const TOGGLE_W: f32 = 52.0;
const TOGGLE_H: f32 = 28.0;

/// Layout helper: returns the toggle switch rect (physical px) for hit
/// testing. The toggle sits at the far right of the row, vertically
/// centered against the big percentage number. The "Limit to 80%"
/// label is placed to its *right* by `draw_expanded`.
pub fn toggle_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let percent_font = DETAIL_PERCENT_FONT * scale;
    let toggle_label_font = TOGGLE_LABEL_FONT * scale;
    let toggle_label_gap = TOGGLE_LABEL_GAP * scale;
    let toggle_w = TOGGLE_W * scale;
    let toggle_h = TOGGLE_H * scale;

    // Toggle is placed so the label "Limit to 80%" fits to its right
    // and the label's right edge aligns with the panel's right edge.
    // We reserve the label width then position the toggle just to its left.
    let label_w = approx_label_width(toggle_label_font);
    let toggle_x = inner_x + inner_w - label_w - toggle_label_gap - toggle_w;
    let row_top = top_y + DETAIL_TOP_PAD * scale;
    let toggle_y = row_top + (percent_font - toggle_h) / 2.0;
    Rect::new(toggle_x, toggle_y, toggle_w, toggle_h)
}

/// Hard-coded label width estimate (in physical px). Used so the toggle
/// rect knows where to anchor relative to the label's right edge
/// without us threading a `TextRenderer` through the rect helper.
/// Slightly generous so we never clip the label.
fn approx_label_width(font_px: f32) -> f32 {
    // "Limit to 80%" — 12 chars; ~0.55 em per char average for our font.
    12.0 * font_px * 0.55
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    battery: &Battery,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;

    let percent_font = DETAIL_PERCENT_FONT * scale;
    let time_font = DETAIL_TIME_FONT * scale;
    let time_gap = DETAIL_TIME_GAP * scale;
    let toggle_label_font = TOGGLE_LABEL_FONT * scale;
    let toggle_label_gap = TOGGLE_LABEL_GAP * scale;

    let y_start = top_y + DETAIL_TOP_PAD * scale;
    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    let muted = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.6 * alpha);

    // ── LEFT column: big % on top, time-remaining underneath ──
    let percent_str = format!("{}%", battery.capacity());
    let percent_w = text.measure_width(&percent_str, percent_font);
    text.queue(
        &percent_str,
        percent_font,
        inner_x,
        y_start,
        white,
        percent_w,
        surface_w,
        surface_h,
    );

    if let Some(secs) = battery.time_remaining_secs() {
        let time_str = format_duration(secs, battery.status());
        text.queue(
            &time_str,
            time_font,
            inner_x,
            y_start + percent_font + time_gap,
            muted,
            percent_w.max(200.0 * scale),
            surface_w,
            surface_h,
        );
    }

    // ── RIGHT: toggle, with "Limit to 80%" to the *right* of it ──
    if battery.supports_charge_limit() {
        let on = battery.charge_limited();
        let t_rect = toggle_rect(panel, top_y, scale);
        draw_toggle(painter, t_rect, on, alpha, scale);

        let label = "Limit to 80%";
        let label_x = t_rect.x + t_rect.w + toggle_label_gap;
        let label_y = y_start + (percent_font - toggle_label_font) / 2.0;
        text.queue(
            label,
            toggle_label_font,
            label_x,
            label_y,
            white,
            300.0 * scale,
            surface_w,
            surface_h,
        );
    }

    y_start + percent_font + time_gap + time_font
}

/// Draw a Switch/iOS-style on/off pill toggle. Track is dimmed when off,
/// accent gold when on; knob slides from left to right when toggled on.
fn draw_toggle(painter: &mut Painter, rect: Rect, on: bool, alpha: f32, scale: f32) {
    let radius = rect.h * 0.5;
    // Track.
    let track_color = if on {
        Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(alpha)
    } else {
        Color::from_rgb8(0x44, 0x44, 0x44).with_alpha(alpha)
    };
    painter.rect_filled(rect, radius, track_color);

    // Knob: white circle inset 2px from the track edge.
    let inset = 3.0 * scale;
    let knob_r = (rect.h - inset * 2.0) * 0.5;
    let knob_cy = rect.y + rect.h * 0.5;
    let knob_cx = if on {
        rect.x + rect.w - inset - knob_r
    } else {
        rect.x + inset + knob_r
    };
    painter.circle_filled(
        knob_cx,
        knob_cy,
        knob_r,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
    );
}

fn format_duration(secs: u64, status: Status) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let prefix = match status {
        Status::Charging => "until full",
        Status::Discharging => "remaining",
        _ => "remaining",
    };
    if hours > 0 {
        format!("{}h {}m {}", hours, minutes, prefix)
    } else {
        format!("{}m {}", minutes, prefix)
    }
}
