//! Two tiny sliders that live below the bar when CC is collapsed.
//! Transparency → `[windows] background_opacity`, Blur → `[windows] blur_intensity`,
//! both in `~/.lantern/config/lantern.toml`. Compositor mtime-caches the
//! file so writes here update window blur/opacity on the next frame.
//!
//! The block's position comes from the outer-zone layout
//! ([`crate::outer_zones`]) — every function takes the slot rect the
//! layout allocated rather than computing one from the panel.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

const TRACK_W: f32 = 240.0;
const TRACK_H: f32 = 10.0;
const KNOB_R: f32 = 11.0;
const ROW_H: f32 = 32.0;
const ROW_GAP: f32 = 8.0;
/// Snap step — slider locks to multiples of 5% (0.00, 0.05, …, 1.00).
const SNAP_STEP: f32 = 0.05;

/// Logical footprint of the whole two-slider block, used by the outer
/// zone layout to size this widget's slot.
pub const BLOCK_W: f32 = TRACK_W;
pub const BLOCK_H: f32 = ROW_H * 2.0 + ROW_GAP;

const TRACK_RGB: (u8, u8, u8) = (90, 90, 90);
const FILL_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const KNOB_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slider {
    Transparency,
    Blur,
}

/// Hover + drag state for the two bar sliders. Values themselves are
/// fetched from / written to lantern.toml on demand — this keeps it the
/// authoritative source so external edits (lntrn-system-settings) flow
/// back in immediately.
#[derive(Debug, Default, Clone, Copy)]
pub struct BarSliderState {
    pub hover: Option<Slider>,
    pub dragging: Option<Slider>,
}

fn row_rect(slot: Rect, scale: f32, idx: usize) -> Rect {
    let y = slot.y + idx as f32 * (ROW_H + ROW_GAP) * scale;
    Rect::new(slot.x, y, TRACK_W * scale, ROW_H * scale)
}

fn track_rect(slot: Rect, scale: f32, idx: usize) -> Rect {
    let row = row_rect(slot, scale, idx);
    let cy = row.y + row.h / 2.0;
    Rect::new(
        row.x,
        cy - TRACK_H * scale / 2.0,
        TRACK_W * scale,
        TRACK_H * scale,
    )
}

fn slider_index(s: Slider) -> usize {
    match s {
        Slider::Transparency => 0,
        Slider::Blur => 1,
    }
}

/// Snap a 0..1 value to the nearest SNAP_STEP multiple.
fn snap(v: f32) -> f32 {
    let stepped = (v / SNAP_STEP).round() * SNAP_STEP;
    stepped.clamp(0.0, 1.0)
}

fn read_value(s: Slider) -> f32 {
    // Both sliders read "right = more of the thing" semantics:
    //   Transparency row → background_opacity directly (right = opaque)
    //   Blur row         → blur_intensity directly      (right = max blur)
    match s {
        Slider::Transparency => crate::lantern_toml::read_windows_f32("background_opacity", 1.0),
        Slider::Blur => crate::lantern_toml::read_windows_f32("blur_intensity", 0.8),
    }
}

fn write_value(s: Slider, v: f32) {
    let v = snap(v);
    // Skip the write if the snapped value is already what's on disk —
    // dragging across the same 5% bucket fires many motion events per
    // second and we don't want each one to bump lantern.toml's mtime.
    let current = snap(read_value(s));
    if (current - v).abs() < f32::EPSILON {
        return;
    }
    match s {
        Slider::Transparency => crate::lantern_toml::write_windows_f32("background_opacity", v),
        Slider::Blur => crate::lantern_toml::write_windows_f32("blur_intensity", v),
    }
}

/// Hit-test both sliders. Returns the one under the cursor (track region
/// only — labels aren't clickable).
pub fn hit_test(slot: Rect, scale: f32, phys_x: f32, phys_y: f32) -> Option<Slider> {
    for (i, s) in [Slider::Transparency, Slider::Blur].iter().enumerate() {
        let track = track_rect(slot, scale, i);
        // Wide hit area: vertically expand by knob radius so users can
        // grab the knob even if it overshoots the track.
        let knob_pad = KNOB_R * scale;
        let hit = Rect::new(
            track.x - knob_pad,
            track.y - knob_pad,
            track.w + knob_pad * 2.0,
            track.h + knob_pad * 2.0,
        );
        if phys_x >= hit.x && phys_x <= hit.x + hit.w && phys_y >= hit.y && phys_y <= hit.y + hit.h
        {
            return Some(*s);
        }
    }
    None
}

/// Map cursor x to a 0..1 value on the named slider's track.
pub fn value_at_cursor(slot: Rect, scale: f32, slider: Slider, phys_x: f32) -> f32 {
    let track = track_rect(slot, scale, slider_index(slider));
    ((phys_x - track.x) / track.w).clamp(0.0, 1.0)
}

/// Commit a new value to lantern.toml. Compositor will pick it up on
/// the next frame via its mtime cache.
pub fn set_value(slider: Slider, v: f32) {
    write_value(slider, v);
}

/// Draw both sliders. Alpha typically wired to the bar-mode visibility
/// so they fade in/out with the rest of the collapsed chrome.
pub fn draw(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    slot: Rect,
    scale: f32,
    alpha: f32,
    state: BarSliderState,
    _surface_w: u32,
    _surface_h: u32,
) {
    if alpha < 0.01 {
        return;
    }

    for (i, slider) in [Slider::Transparency, Slider::Blur].iter().enumerate() {
        let track = track_rect(slot, scale, i);
        let v = snap(read_value(*slider));
        let fill_w = track.w * v;
        let hovered_or_dragging = state.hover == Some(*slider) || state.dragging == Some(*slider);

        // Track (full width, dim).
        let track_r = (TRACK_H * scale) / 2.0;
        painter.rect_filled(
            track,
            track_r,
            Color::from_rgb8(TRACK_RGB.0, TRACK_RGB.1, TRACK_RGB.2).with_alpha(0.45 * alpha),
        );
        // Fill up to the value, gold accent.
        if fill_w > 0.0 {
            painter.rect_filled(
                Rect::new(track.x, track.y, fill_w, track.h),
                track_r,
                Color::from_rgb8(FILL_RGB.0, FILL_RGB.1, FILL_RGB.2).with_alpha(0.95 * alpha),
            );
        }
        // Knob — sits on the snapped value position so visual feedback
        // matches what got written.
        let knob_cx = track.x + fill_w;
        let knob_cy = track.y + track.h / 2.0;
        let knob_r = KNOB_R * scale * if hovered_or_dragging { 1.10 } else { 1.0 };
        painter.circle_filled(
            knob_cx,
            knob_cy,
            knob_r,
            Color::from_rgb8(KNOB_RGB.0, KNOB_RGB.1, KNOB_RGB.2).with_alpha(alpha),
        );
    }
}
