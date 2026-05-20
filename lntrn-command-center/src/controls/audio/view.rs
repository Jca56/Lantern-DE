//! Inline tile + click-expand panel rendering and hit-testing.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

use super::icons::{draw_mic, draw_speaker, draw_speaker_colored};
use super::{Audio, Direction};

// ── Inline tile constants ───────────────────────────────────────────────────

const ICON_SIZE: f32 = 22.0;
const ICON_BAR_GAP: f32 = 10.0;
const BAR_WIDTH: f32 = 120.0;
const BAR_HEIGHT: f32 = 8.0;
const BAR_TRACK_RGB: (u8, u8, u8) = (60, 60, 60);
/// The expanded view's slider fill is gold (matches "lines = gold"),
/// but the inline tile keeps a tiny white bar so the row reads as a
/// neutral status strip without competing accents.
const BAR_FILL_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);

/// Logical px the audio tile asks for in the row layout — speaker icon
/// + small gap + 120pt bar.
pub const TILE_WIDTH: f32 = 22.0 + 10.0 + 120.0;

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    audio: &Audio,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    _surface_w: u32,
    _surface_h: u32,
    lit: bool,
) {
    if !audio.is_present() {
        return;
    }

    let icon_size = ICON_SIZE * scale;
    let icon_bar_gap = ICON_BAR_GAP * scale;
    let bar_w = BAR_WIDTH * scale;
    let bar_h = BAR_HEIGHT * scale;

    let group_x = layout.x;

    let icon_color = if lit {
        Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(alpha)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha)
    };
    let icon_y = layout.y + (layout.h - icon_size) / 2.0;
    draw_speaker_colored(painter, group_x, icon_y, icon_size, icon_size, audio.is_muted(), icon_color);

    // Volume bar.
    let bar_x = group_x + icon_size + icon_bar_gap;
    let bar_y = layout.y + (layout.h - bar_h) / 2.0;
    let radius = bar_h * 0.5;

    // Track.
    painter.rect_filled(
        Rect::new(bar_x, bar_y, bar_w, bar_h),
        radius,
        Color::from_rgb8(BAR_TRACK_RGB.0, BAR_TRACK_RGB.1, BAR_TRACK_RGB.2)
            .with_alpha(alpha),
    );

    // Fill — proportional to volume, clamped at 100% (anything over is
    // boost territory and the inline visual just sits at full).
    let v = if audio.is_muted() { 0.0 } else { audio.volume().min(1.0) };
    if v > 0.0 {
        let raw = bar_w * v;
        let fill_w = raw.max(bar_h);
        painter.rect_filled(
            Rect::new(bar_x, bar_y, fill_w, bar_h),
            radius,
            Color::from_rgb8(BAR_FILL_RGB.0, BAR_FILL_RGB.1, BAR_FILL_RGB.2)
                .with_alpha(alpha),
        );
    }

    // Drag knob at the fill end — sits on top of the bar so the inline
    // slider reads as something you can grab.
    let knob_r = bar_h * 1.2;
    let knob_cx = bar_x + bar_w * v;
    let knob_cy = bar_y + bar_h / 2.0;
    let knob_color = if audio.is_muted() {
        Color::from_rgb8(BAR_TRACK_RGB.0, BAR_TRACK_RGB.1, BAR_TRACK_RGB.2).with_alpha(alpha)
    } else {
        Color::from_rgb8(BAR_FILL_RGB.0, BAR_FILL_RGB.1, BAR_FILL_RGB.2).with_alpha(alpha)
    };
    painter.circle_filled(knob_cx, knob_cy, knob_r, knob_color);
}

// ── Inline hit-testing ──────────────────────────────────────────────────────

/// Distinct hit zones inside the inline audio tile.
#[derive(Debug, Clone, Copy)]
pub enum InlineHit {
    SpeakerIcon,
    VolumeBar,
}

/// Physical-pixel rect of the volume bar inside the inline audio tile.
pub fn inline_bar_rect(layout: &TileLayout, scale: f32) -> Rect {
    let icon_size = ICON_SIZE * scale;
    let icon_bar_gap = ICON_BAR_GAP * scale;
    let bar_w = BAR_WIDTH * scale;
    let bar_h = BAR_HEIGHT * scale;
    let bar_x = layout.x + icon_size + icon_bar_gap;
    let bar_y = layout.y + (layout.h - bar_h) / 2.0;
    Rect::new(bar_x, bar_y, bar_w, bar_h)
}

/// Hit-test a click against the inline audio tile. The icon and the
/// bar each get a generous tile-height hit zone so the targets are
/// easy to land on — the icon owns the left half-tile up to the
/// midpoint of the gap, the bar owns everything to the right of it.
pub fn hit_test_inline(layout: &TileLayout, scale: f32, x: f32, y: f32) -> Option<InlineHit> {
    if y < layout.y || y > layout.y + layout.h {
        return None;
    }
    if x < layout.x || x > layout.x + layout.w {
        return None;
    }
    let split = layout.x + (ICON_SIZE + ICON_BAR_GAP / 2.0) * scale;
    if x < split {
        Some(InlineHit::SpeakerIcon)
    } else {
        Some(InlineHit::VolumeBar)
    }
}

// ── Click-expand panel constants ────────────────────────────────────────────

const VIEW_TOP_PAD: f32 = 20.0;
const SECTION_HEADER_FONT: f32 = 22.0;
const SECTION_HEADER_BOTTOM_GAP: f32 = 10.0;
const SLIDER_PERCENT_FONT: f32 = 36.0;
const SLIDER_PERCENT_GAP: f32 = 16.0;
const SLIDER_HEIGHT: f32 = 12.0;
const SLIDER_BOTTOM_GAP: f32 = 16.0;
const DEVICE_ROW_HEIGHT: f32 = 44.0;
const DEVICE_FONT: f32 = 22.0;
const DEVICE_DOT_SIZE: f32 = 10.0;
const SECTION_GAP: f32 = 28.0;

/// Icon at the left of each slider row (logical px). Click toggles
/// mute for that section.
const ROW_ICON_SIZE: f32 = 28.0;
const ROW_ICON_GAP: f32 = 14.0;
/// Max devices we render per section. The view fits comfortably with
/// 4; if a system has more sinks/sources than that we just show the
/// top 4 (the default is guaranteed to be in the parsed list).
const MAX_DEVICE_ROWS: usize = 4;

/// Retained from the old expanded-panel sizing math; the panel-mode
/// rework made it unused. Keeping for reference.
#[allow(dead_code)]
pub const EXPANDED_HEIGHT: f32 = 0.0;

/// Vertical offset (logical px) inside the audio view at which a
/// section begins. Output is first, then input below it.
fn section_top_logical(dir: Direction) -> f32 {
    let section_h = section_logical_height();
    match dir {
        Direction::Output => VIEW_TOP_PAD,
        Direction::Input => VIEW_TOP_PAD + section_h + SECTION_GAP,
    }
}

/// Logical height of one section (header + slider row + device rows).
fn section_logical_height() -> f32 {
    SECTION_HEADER_FONT
        + SECTION_HEADER_BOTTOM_GAP
        + SLIDER_PERCENT_FONT
        + SLIDER_BOTTOM_GAP
        + DEVICE_ROW_HEIGHT * MAX_DEVICE_ROWS as f32
}

/// Y coordinate (physical px) of the slider track for the given section.
fn slider_top_y(panel_top_y: f32, dir: Direction, scale: f32) -> f32 {
    let section_top = panel_top_y + section_top_logical(dir) * scale;
    section_top + (SECTION_HEADER_FONT + SECTION_HEADER_BOTTOM_GAP) * scale
}

/// Y coordinate (physical px) where the device list for the given
/// section starts (just below the slider row).
fn device_list_top_y_for(panel_top_y: f32, dir: Direction, scale: f32) -> f32 {
    slider_top_y(panel_top_y, dir, scale) + (SLIDER_PERCENT_FONT + SLIDER_BOTTOM_GAP) * scale
}

/// Mute icon (speaker / mic) rect for the given section. Click here
/// toggles mute for that direction.
pub fn icon_rect_for(panel: Rect, panel_top_y: f32, dir: Direction, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let percent_font = SLIDER_PERCENT_FONT * scale;
    let icon_size = ROW_ICON_SIZE * scale;
    let inner_x = panel.x + pad;
    // Vertically center the icon against the slider row (= percent_font tall).
    let row_top = slider_top_y(panel_top_y, dir, scale);
    let icon_y = row_top + (percent_font - icon_size) / 2.0;
    Rect::new(inner_x, icon_y, icon_size, icon_size)
}

/// Hit-test pointer position against either section's mute icon.
pub fn hit_test_icon(panel: Rect, panel_top_y: f32, scale: f32, x: f32, y: f32) -> Option<Direction> {
    for &dir in &[Direction::Output, Direction::Input] {
        let r = icon_rect_for(panel, panel_top_y, dir, scale);
        if x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h {
            return Some(dir);
        }
    }
    None
}

/// Layout helper: returns the slider's track rect (physical px) for the
/// given section, used by hit testing. The slider sits to the right of
/// the mute icon.
pub fn slider_rect_for(panel: Rect, panel_top_y: f32, dir: Direction, scale: f32) -> Rect {
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
    let slider_y = slider_top_y(panel_top_y, dir, scale) + (percent_font - slider_h) / 2.0;
    Rect::new(slider_x, slider_y, slider_w, slider_h)
}

/// Backwards-compat wrapper for the layershell's left-click hit-test.
/// Defaults to the Output slider since that was the only one before
/// Input was added; the layershell now calls `slider_rect_for` directly
/// with both directions.
#[allow(dead_code)]
pub fn slider_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    slider_rect_for(panel, top_y, Direction::Output, scale)
}

/// Hit-test a click against either device list. Returns the device ID
/// + which direction it belongs to, if any.
pub fn hit_test_device_dir(
    audio: &Audio,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<(Direction, u32)> {
    for &dir in &[Direction::Output, Direction::Input] {
        let list_top = device_list_top_y_for(panel_top_y, dir, scale);
        let row_h = DEVICE_ROW_HEIGHT * scale;
        let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
        let inner_x = panel.x + pad;
        let inner_w = panel.w - pad * 2.0;
        if x < inner_x || x > inner_x + inner_w {
            continue;
        }
        let devices = match dir {
            Direction::Output => audio.sinks(),
            Direction::Input => audio.sources(),
        };
        for (i, dev) in devices.iter().take(MAX_DEVICE_ROWS).enumerate() {
            let row_y = list_top + i as f32 * row_h;
            if y >= row_y && y <= row_y + row_h {
                return Some((dir, dev.id));
            }
        }
    }
    None
}

/// Backwards-compat alias kept around so the layershell keeps building
/// while it migrates to `hit_test_device_dir`.
#[allow(dead_code)]
pub fn hit_test_device(audio: &Audio, panel: Rect, top_y: f32, scale: f32, x: f32, y: f32) -> Option<u32> {
    hit_test_device_dir(audio, panel, top_y, scale, x, y).map(|(_, id)| id)
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    audio: &Audio,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    draw_section(
        painter, text, audio, Direction::Output,
        panel, top_y, scale, alpha, surface_w, surface_h,
    );
    draw_section(
        painter, text, audio, Direction::Input,
        panel, top_y, scale, alpha, surface_w, surface_h,
    );
    top_y + (section_top_logical(Direction::Input) + section_logical_height()) * scale
}

/// Draw one of the two sections (Output or Input) into the audio view.
/// Each section is: header label + slider row (slider + percentage) +
/// device list. Layout math comes from the `_for` helpers above so
/// hit-testing and rendering stay in lockstep.
fn draw_section(
    painter: &mut Painter,
    text: &mut TextRenderer,
    audio: &Audio,
    dir: Direction,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let percent_font = SLIDER_PERCENT_FONT * scale;
    let percent_gap = SLIDER_PERCENT_GAP * scale;
    let percent_w = percent_font * 2.6;
    let header_font = SECTION_HEADER_FONT * scale;
    let header_gap = SECTION_HEADER_BOTTOM_GAP * scale;
    let device_row_h = DEVICE_ROW_HEIGHT * scale;
    let device_font = DEVICE_FONT * scale;
    let dot_size = DEVICE_DOT_SIZE * scale;
    let dot_pad_left = 6.0 * scale;
    let dot_text_gap = 14.0 * scale;

    // Per-direction values.
    let (label, vol, muted, devices) = match dir {
        Direction::Output => (
            "Output",
            audio.volume(),
            audio.is_muted(),
            audio.sinks(),
        ),
        Direction::Input => (
            "Input",
            audio.input_volume(),
            audio.input_muted(),
            audio.sources(),
        ),
    };

    // Slider scale runs 0..1.2 (i.e. up to 120 %) so users can boost
    // quiet sinks. Knob/fill widths divide by 1.2 to map back to the
    // track. The 100 % mark sits at ~83 % of track width.
    let v = if muted { 0.0 } else { vol.min(1.2) };
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted_white = white.with_alpha(0.55 * alpha);

    // ── Section header ────────────────────────────────────────────────────
    let section_top = panel_top_y + section_top_logical(dir) * scale;
    text.queue(
        label,
        header_font,
        inner_x,
        section_top,
        muted_white,
        inner_w,
        surface_w,
        surface_h,
    );

    // ── Mute icon at the left of the slider row ──────────────────────────
    let icon = icon_rect_for(panel, panel_top_y, dir, scale);
    match dir {
        Direction::Output => draw_speaker(painter, icon.x, icon.y, icon.w, icon.h, muted, alpha),
        Direction::Input => draw_mic(painter, icon.x, icon.y, icon.w, icon.h, muted, alpha),
    }

    // ── Slider row ────────────────────────────────────────────────────────
    let track = slider_rect_for(panel, panel_top_y, dir, scale);
    let radius = track.h * 0.5;

    painter.rect_filled(
        track,
        radius,
        Color::from_rgb8(BAR_TRACK_RGB.0, BAR_TRACK_RGB.1, BAR_TRACK_RGB.2).with_alpha(alpha),
    );
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    if v > 0.0 {
        let fill_w = (track.w * (v / 1.2)).max(track.h);
        painter.rect_filled(
            Rect::new(track.x, track.y, fill_w, track.h),
            radius,
            gold.with_alpha(alpha),
        );
    }

    // Knob — white circle at the current position.
    let knob_cx = track.x + track.w * (v / 1.2);
    let knob_cy = track.y + track.h * 0.5;
    let knob_r = track.h * 1.6;
    painter.circle_filled(
        knob_cx,
        knob_cy,
        knob_r,
        white.with_alpha(alpha),
    );

    // Percentage label on the right.
    let pct = (v * 100.0).round() as i32;
    let pct_str = if muted { "Muted".to_string() } else { format!("{}%", pct) };
    let pct_text_w = text.measure_width(&pct_str, percent_font);
    let pct_x = inner_x + inner_w - percent_w + (percent_w - pct_text_w);
    let pct_y = section_top + header_font + header_gap;
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
    let _ = percent_gap; // already baked into slider_rect_for

    // ── Device list ───────────────────────────────────────────────────────
    let list_top = device_list_top_y_for(panel_top_y, dir, scale);
    for (i, dev) in devices.iter().take(MAX_DEVICE_ROWS).enumerate() {
        let row_y = list_top + i as f32 * device_row_h;
        let text_y = row_y + (device_row_h - device_font) / 2.0;
        let label_alpha = if dev.is_default { alpha } else { 0.78 * alpha };
        let label_color = white.with_alpha(label_alpha);

        let dot_cx = inner_x + dot_pad_left + dot_size * 0.5;
        let dot_cy = row_y + device_row_h * 0.5;
        if dev.is_default {
            // Active = filled white circle.
            painter.circle_filled(
                dot_cx,
                dot_cy,
                dot_size * 0.5,
                white.with_alpha(alpha),
            );
        } else {
            // Available = hollow gold ring.
            painter.circle_stroke(
                dot_cx,
                dot_cy,
                dot_size * 0.5,
                1.5 * scale,
                gold.with_alpha(0.55 * alpha),
            );
        }

        let dev_label = truncate_for_width(
            &dev.name,
            &mut *text,
            device_font,
            inner_w - dot_pad_left - dot_size - dot_text_gap,
        );
        text.queue(
            &dev_label,
            device_font,
            dot_cx + dot_size * 0.5 + dot_text_gap,
            text_y,
            label_color,
            inner_w,
            surface_w,
            surface_h,
        );
    }
}

/// Truncate `s` with ellipsis so its width fits `max_w` at `font_size`.
/// The renderer truncates at character boundaries — good enough for
/// our space-separated sink names.
fn truncate_for_width(s: &str, text: &mut TextRenderer, font_size: f32, max_w: f32) -> String {
    if text.measure_width(s, font_size) <= max_w {
        return s.to_string();
    }
    let ellipsis = "…";
    let mut chars: Vec<char> = s.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + ellipsis;
        if text.measure_width(&candidate, font_size) <= max_w {
            return candidate;
        }
    }
    ellipsis.to_string()
}
