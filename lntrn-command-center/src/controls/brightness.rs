//! Brightness control tile.
//!
//! Inline tile is just a small sun icon (no bar). Click-expand shows
//! a single full-width slider for the backlight level.
//!
//! Backend: reads/writes `/sys/class/backlight/{first}/brightness` and
//! `max_brightness` directly. Most laptops ship those files with `666`
//! perms (the `video` group, but the file mode lets anyone in), so no
//! `sudo`/`pkexec`/polkit dance is needed. If the perms are stricter,
//! writes silently fail and the slider is read-only.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

const POLL_INTERVAL: Duration = Duration::from_millis(750);

pub struct Brightness {
    sysfs_dir: Option<PathBuf>,
    /// Current brightness level (raw kernel units, 0..=max).
    level: u32,
    /// Max brightness (cached at startup; never changes mid-session).
    max: u32,
    last_poll: Instant,
}

impl Brightness {
    pub fn new() -> Self {
        let sysfs_dir = find_backlight();
        let max = sysfs_dir
            .as_ref()
            .and_then(|p| read_u32(p.join("max_brightness")))
            .unwrap_or(0);
        let mut b = Self {
            sysfs_dir,
            level: 0,
            max,
            last_poll: Instant::now() - POLL_INTERVAL,
        };
        b.tick();
        b
    }

    /// True when a backlight device was found at startup.
    pub fn is_present(&self) -> bool {
        self.sysfs_dir.is_some() && self.max > 0
    }

    /// Normalized brightness in `[0.0, 1.0]`.
    pub fn fraction(&self) -> f32 {
        if self.max == 0 {
            return 0.0;
        }
        (self.level as f32 / self.max as f32).clamp(0.0, 1.0)
    }

    pub fn tick(&mut self) {
        if self.last_poll.elapsed() < POLL_INTERVAL {
            return;
        }
        self.last_poll = Instant::now();
        let Some(dir) = &self.sysfs_dir else { return };
        if let Some(v) = read_u32(dir.join("brightness")) {
            self.level = v;
        }
    }

    /// Write a new normalized brightness. We never go below ~5 % of max
    /// so the user can't accidentally make the screen totally black.
    pub fn set_fraction(&mut self, frac: f32) {
        let Some(dir) = &self.sysfs_dir else { return };
        if self.max == 0 {
            return;
        }
        let min_frac = 0.05;
        let f = frac.clamp(min_frac, 1.0);
        let new_level = (f * self.max as f32).round() as u32;
        let target = dir.join("brightness");
        match std::fs::write(&target, format!("{}\n", new_level)) {
            Ok(_) => {
                self.level = new_level;
            }
            Err(e) => {
                tracing::warn!(?e, ?target, "failed to write brightness");
            }
        }
    }
}

fn find_backlight() -> Option<PathBuf> {
    let base = Path::new("/sys/class/backlight");
    let entries = std::fs::read_dir(base).ok()?;
    // Pick the first device. Most laptops have exactly one.
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("brightness").exists() && path.join("max_brightness").exists() {
            return Some(path);
        }
    }
    None
}

fn read_u32(path: PathBuf) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

// ── Inline tile ─────────────────────────────────────────────────────────────

/// Sun icon size (logical px). Square.
const ICON_SIZE: f32 = 28.0;
/// Padding on the *left* of the icon so the sun doesn't sit flush against
/// the audio tile's right edge.
const ICON_LEFT_PAD: f32 = 16.0;

/// Logical px the brightness tile asks for in the row layout.
/// Includes the left pad so the tile_pack inter-tile gap doesn't get
/// folded into our own breathing room.
pub const TILE_WIDTH: f32 = ICON_LEFT_PAD + ICON_SIZE;

pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    brightness: &Brightness,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    _surface_w: u32,
    _surface_h: u32,
) {
    if !brightness.is_present() {
        return;
    }

    let icon_size = ICON_SIZE * scale;
    let icon_x = layout.x + ICON_LEFT_PAD * scale;
    let icon_y = layout.y + (layout.h - icon_size) / 2.0;
    draw_sun(painter, icon_x, icon_y, icon_size, icon_size, alpha);
}

/// Draw a stylised sun: filled circle in the middle + 8 short rays
/// around it. Pure shapes — no SVG.
fn draw_sun(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;

    // Center disc — filled.
    let disc_r = w * 0.22;
    painter.circle_filled(cx, cy, disc_r, color);

    // Rays — 8 short lines at 45° intervals from `inner_r` to `outer_r`.
    let inner_r = w * 0.34;
    let outer_r = w * 0.48;
    let stroke = w * 0.10;
    for i in 0..8 {
        let theta = i as f32 * std::f32::consts::FRAC_PI_4;
        let (sin, cos) = theta.sin_cos();
        let x1 = cx + cos * inner_r;
        let y1 = cy + sin * inner_r;
        let x2 = cx + cos * outer_r;
        let y2 = cy + sin * outer_r;
        painter.line_round(x1, y1, x2, y2, stroke, color);
    }
}

// ── Click-expand panel ─────────────────────────────────────────────────────

const VIEW_TOP_PAD: f32 = 32.0;
const SLIDER_HEIGHT: f32 = 12.0;
const SLIDER_PERCENT_FONT: f32 = 36.0;
const SLIDER_PERCENT_GAP: f32 = 16.0;
const ROW_ICON_SIZE: f32 = 28.0;
const ROW_ICON_GAP: f32 = 14.0;

/// Slider track rect (physical px) for hit testing.
pub fn slider_rect(panel: Rect, panel_top_y: f32, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let percent_font = SLIDER_PERCENT_FONT * scale;
    let percent_gap = SLIDER_PERCENT_GAP * scale;
    let percent_w = percent_font * 2.6;
    let icon_size = ROW_ICON_SIZE * scale;
    let icon_gap = ROW_ICON_GAP * scale;

    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let slider_x = inner_x + icon_size + icon_gap;
    let slider_w = inner_w - icon_size - icon_gap - percent_w - percent_gap;
    let slider_h = SLIDER_HEIGHT * scale;
    let row_top = panel_top_y + VIEW_TOP_PAD * scale;
    let slider_y = row_top + (percent_font - slider_h) / 2.0;
    Rect::new(slider_x, slider_y, slider_w, slider_h)
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    brightness: &Brightness,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let percent_font = SLIDER_PERCENT_FONT * scale;
    let percent_gap = SLIDER_PERCENT_GAP * scale;
    let percent_w = percent_font * 2.6;
    let icon_size = ROW_ICON_SIZE * scale;

    let v = brightness.fraction();
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

    let row_top = panel_top_y + VIEW_TOP_PAD * scale;

    // ── Sun icon at the left of the slider row ─────────────────────────────
    let icon_y = row_top + (percent_font - icon_size) / 2.0;
    draw_sun(painter, inner_x, icon_y, icon_size, icon_size, alpha);

    // ── Slider ─────────────────────────────────────────────────────────────
    let track = slider_rect(panel, panel_top_y, scale);
    let radius = track.h * 0.5;

    // Track (dim grey).
    painter.rect_filled(
        track,
        radius,
        Color::from_rgb8(60, 60, 60).with_alpha(alpha),
    );

    // Fill (gold, matches audio).
    if v > 0.0 {
        let fill_w = (track.w * v).max(track.h);
        painter.rect_filled(
            Rect::new(track.x, track.y, fill_w, track.h),
            radius,
            gold.with_alpha(alpha),
        );
    }

    // Knob (white, matches audio).
    let knob_cx = track.x + track.w * v;
    let knob_cy = track.y + track.h * 0.5;
    let knob_r = track.h * 1.6;
    painter.circle_filled(knob_cx, knob_cy, knob_r, white.with_alpha(alpha));

    // Percentage label on the right.
    let pct = (v * 100.0).round() as i32;
    let pct_str = format!("{}%", pct);
    let pct_text_w = text.measure_width(&pct_str, percent_font);
    let pct_x = inner_x + inner_w - percent_w + (percent_w - pct_text_w);
    let pct_y = row_top;
    text.queue(
        &pct_str,
        percent_font,
        pct_x,
        pct_y,
        white.with_alpha(alpha),
        percent_w,
        surface_w,
        surface_h,
    );
    let _ = percent_gap;

    row_top + percent_font
}
