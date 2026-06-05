//! HDR controls for the Display panel: an on/off pill toggle and (when on) an
//! SDR-brightness slider. Drawn after the Scale row in `monitor_settings`, but
//! only for outputs the compositor reports as HDR-capable.
//!
//! Kept in its own module so `monitor_settings.rs` stays focused and under the
//! file-size limit. Uses `lntrn_render` primitives + `FoxPalette` like the rest
//! of the Display panel; text is 18pt to stay readable.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

// Zone IDs — continue the monitor_settings range (res/refresh/scale use 1100-2).
pub const ZONE_HDR_TOGGLE: u32 = 1103;
pub const ZONE_SDR_SLIDER: u32 = 1104;
pub const ZONE_HDR_KEEP: u32 = 1105;

const PAD: f32 = 24.0;
const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const LABEL_W: f32 = 160.0;

// SDR brightness slider bounds (nits). 203 is the BT.2408 reference white.
pub const SDR_MIN: u32 = 80;
pub const SDR_MAX: u32 = 300;

/// What the HDR row produced this frame, so the caller can update state + mark
/// the settings dirty. The toggle is handled via the click router; the slider
/// reports a live drag value here (drag, not click-once).
pub struct HdrRowResult {
    pub consumed_h: f32,
    /// `Some(nits)` while the user is dragging the SDR-brightness slider.
    pub dragged_sdr_nits: Option<u32>,
}

/// Draw the HDR toggle row plus, when enabled, the SDR-brightness slider.
#[allow(clippy::too_many_arguments)]
pub fn draw_hdr_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    hdr_on: bool,
    sdr_nits: u32,
    max_nits: u32,
    pending_secs_left: Option<u32>,
    x: f32,
    y: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> HdrRowResult {
    let pad = PAD * s;
    let lsz = LABEL_SIZE * s;
    let row_h = ROW_H * s;
    let label_x = x + pad;
    let ctrl_x = label_x + LABEL_W * s;

    let mut cy = y;
    let mut dragged_sdr_nits = None;

    // ── HDR toggle row ─────────────────────────────────────────────
    text.queue("HDR", lsz, label_x, cy + (row_h - lsz) / 2.0, fox.text, LABEL_W * s, sw, sh);

    // Pill toggle: rounded track + sliding knob.
    let track_w = 56.0 * s;
    let track_h = 30.0 * s;
    let track_rect = Rect::new(ctrl_x, cy + (row_h - track_h) / 2.0, track_w, track_h);
    let toggle_zone = ix.add_zone(ZONE_HDR_TOGGLE, track_rect);
    let track_color = if hdr_on { fox.accent } else { fox.surface_2 };
    painter.rect_filled(track_rect, track_h / 2.0, track_color);
    if toggle_zone.is_hovered() {
        painter.rect_stroke_sdf(track_rect, track_h / 2.0, 1.5 * s, fox.muted);
    }
    let knob_r = track_h / 2.0 - 4.0 * s;
    let knob_cx = if hdr_on {
        track_rect.x + track_w - track_h / 2.0
    } else {
        track_rect.x + track_h / 2.0
    };
    let knob_cy = track_rect.y + track_h / 2.0;
    painter.circle_filled(knob_cx, knob_cy, knob_r, fox.text);

    // State label to the right of the pill.
    let state_label = if hdr_on { "On" } else { "Off" };
    text.queue(
        state_label,
        lsz,
        track_rect.x + track_w + 14.0 * s,
        cy + (row_h - lsz) / 2.0,
        fox.text_secondary,
        120.0 * s,
        sw,
        sh,
    );
    cy += row_h;

    // ── SDR brightness slider (only when HDR is on) ────────────────
    if hdr_on {
        let cap = if max_nits > 0 {
            format!("SDR brightness  ({} nits, display peak {})", sdr_nits, max_nits)
        } else {
            format!("SDR brightness  ({} nits)", sdr_nits)
        };
        text.queue(&cap, lsz * 0.85, label_x, cy + 4.0 * s, fox.text_secondary, 420.0 * s, sw, sh);

        let slider_y = cy + row_h * 0.6;
        let track_x = label_x;
        let track_w2 = 360.0 * s;
        let knob_r2 = 9.0 * s;
        let track = Rect::new(track_x, slider_y, track_w2, 1.0);

        // Hit zone spans the whole track + knob travel.
        let hit = Rect::new(
            track_x - knob_r2,
            slider_y - knob_r2 - 4.0 * s,
            track_w2 + knob_r2 * 2.0,
            knob_r2 * 2.0 + 8.0 * s,
        );
        let slider_zone = ix.add_zone(ZONE_SDR_SLIDER, hit);

        // Live drag: while this zone owns capture, follow the cursor.
        let display_nits = if slider_zone.is_active() {
            if let Some(t) = ix.drag_fraction_x(&track) {
                let nits = (SDR_MIN as f32 + t * (SDR_MAX - SDR_MIN) as f32).round() as u32;
                let nits = nits.clamp(SDR_MIN, SDR_MAX);
                dragged_sdr_nits = Some(nits);
                nits
            } else {
                sdr_nits
            }
        } else {
            sdr_nits
        };

        // Track + filled portion up to the knob.
        painter.line(track_x, slider_y, track_x + track_w2, slider_y, 3.0 * s, fox.surface_2);
        let t = ((display_nits.clamp(SDR_MIN, SDR_MAX) - SDR_MIN) as f32) / ((SDR_MAX - SDR_MIN) as f32);
        let knob_x = track_x + t * track_w2;
        painter.line(track_x, slider_y, knob_x, slider_y, 3.0 * s, fox.accent);
        painter.circle_filled(knob_x, slider_y, knob_r2, fox.accent);
        painter.circle_stroke(knob_x, slider_y, knob_r2, 1.5 * s, fox.text);

        cy += row_h;
    }

    // ── "Keep HDR?" confirmation banner (auto-revert countdown) ────
    if let Some(secs) = pending_secs_left {
        let banner_h = row_h * 1.1;
        let banner = Rect::new(label_x, cy, 480.0 * s, banner_h);
        painter.rect_filled(banner, 8.0 * s, fox.surface_2);
        painter.rect_stroke_sdf(banner, 8.0 * s, 1.5 * s, fox.warning);

        let msg = format!("Keep HDR? Reverting in {secs}s if your screen looks wrong…");
        text.queue(
            &msg,
            lsz * 0.9,
            banner.x + 14.0 * s,
            banner.y + (banner_h - lsz * 0.9) / 2.0,
            fox.text,
            340.0 * s,
            sw,
            sh,
        );

        // "Keep" button on the right of the banner.
        let btn_w = 96.0 * s;
        let btn_h = 34.0 * s;
        let btn = Rect::new(
            banner.x + banner.w - btn_w - 10.0 * s,
            banner.y + (banner_h - btn_h) / 2.0,
            btn_w,
            btn_h,
        );
        let keep_zone = ix.add_zone(ZONE_HDR_KEEP, btn);
        let btn_bg = if keep_zone.is_hovered() { fox.success } else { fox.accent };
        painter.rect_filled(btn, 6.0 * s, btn_bg);
        let klsz = lsz * 0.95;
        text.queue(
            "Keep",
            klsz,
            btn.x + (btn_w - klsz * 2.2) / 2.0,
            btn.y + (btn_h - klsz) / 2.0,
            fox.text,
            btn_w,
            sw,
            sh,
        );

        cy += banner_h + 8.0 * s;
    }

    HdrRowResult {
        consumed_h: cy - y,
        dragged_sdr_nits,
    }
}
