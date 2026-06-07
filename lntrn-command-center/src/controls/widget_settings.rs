//! Per-widget settings popover (opened by the ⚙ badge in edit mode).
//! Every widget gets a universal Size slider; the Clock adds date + time
//! format controls. Pure geometry + drawing + hit-testing — the click
//! handler applies the resulting [`Hit`] to the widget's options.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::toolbar::{DatePos, WidgetOpts, SIZE_MAX, SIZE_MIN, SPACE_MAX, SPACE_MIN};
use super::{Controls, TileId};

// logical px
const POP_W: f32 = 300.0;
const PAD: f32 = 14.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 34.0;
const ROW_GAP: f32 = 6.0;
const TOP_GAP: f32 = 12.0;
const RADIUS: f32 = 14.0;
const LABEL_W: f32 = 76.0;
const TRACK_H: f32 = 8.0;
const KNOB_R: f32 = 9.0;
const PILL_W: f32 = 60.0;
const PILL_H: f32 = 26.0;
const FONT: f32 = 15.0;

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const PLATE_RGB: (u8, u8, u8) = (24, 24, 24);

/// A control hit inside the popover.
#[derive(Debug, Clone, Copy)]
pub enum Hit {
    /// A slider grabbed — caller seeks to cursor + begins a drag.
    Slider(WidgetSlider),
    ToggleDate,
    SetDatePos(DatePos),
    Toggle24h,
    ToggleSeconds,
}

/// Which universal slider is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetSlider {
    Size,
    Space,
}

struct PopLayout {
    rect: Rect,
    size_track: Rect,
    space_track: Rect,
    date_toggle: Option<Rect>,
    date_seg: Option<[Rect; 3]>, // Below, Left, Right
    h24: Option<Rect>,
    seconds: Option<Rect>,
}

fn is_clock(id: TileId) -> bool {
    id == TileId::Clock
}

fn layout(id: TileId, controls: &Controls, panel: Rect, scale: f32) -> PopLayout {
    let pop_w = POP_W * scale;
    // Rows: Size + Space always; clock adds Date / Date-pos / Format.
    let body_rows = if is_clock(id) { 5 } else { 2 };
    let row_step = (ROW_H + ROW_GAP) * scale;
    let pop_h = PAD * scale + TITLE_H * scale + body_rows as f32 * row_step + PAD * scale;

    // Centered below the bar — a stable anchor so the size slider doesn't
    // slide out from under the cursor as the widget resizes live.
    let _ = controls;
    let cx = panel.x + panel.w / 2.0;
    let min_x = panel.x + 8.0 * scale;
    let max_x = panel.x + panel.w - 8.0 * scale - pop_w;
    let x = (cx - pop_w / 2.0).clamp(min_x, max_x.max(min_x));
    let y = panel.y + panel.h + TOP_GAP * scale;
    let rect = Rect::new(x, y, pop_w, pop_h);

    let inner_x = x + PAD * scale;
    let inner_w = pop_w - PAD * 2.0 * scale;
    let label_w = LABEL_W * scale;
    let ctrl_x = inner_x + label_w;
    let ctrl_w = inner_w - label_w;
    let body_top = y + PAD * scale + TITLE_H * scale;
    let row_y = |i: usize| body_top + i as f32 * row_step;

    // Row 0: size slider. Row 1: space slider.
    let track_h = TRACK_H * scale;
    let track_at = |i: usize| Rect::new(ctrl_x, row_y(i) + (ROW_H * scale - track_h) / 2.0, ctrl_w, track_h);
    let size_track = track_at(0);
    let space_track = track_at(1);

    let (date_toggle, date_seg, h24, seconds) = if is_clock(id) {
        let pill_w = PILL_W * scale;
        let pill_h = PILL_H * scale;
        // Row 2: date on/off pill (right-aligned).
        let dt = Rect::new(
            x + pop_w - PAD * scale - pill_w,
            row_y(2) + (ROW_H * scale - pill_h) / 2.0,
            pill_w,
            pill_h,
        );
        // Row 3: date-position segmented (3 equal segments across ctrl_w).
        let seg_w = ctrl_w / 3.0;
        let seg_y = row_y(3) + (ROW_H * scale - pill_h) / 2.0;
        let seg = [
            Rect::new(ctrl_x, seg_y, seg_w, pill_h),
            Rect::new(ctrl_x + seg_w, seg_y, seg_w, pill_h),
            Rect::new(ctrl_x + seg_w * 2.0, seg_y, seg_w, pill_h),
        ];
        // Row 4: two pills (24h, Seconds).
        let r4y = row_y(4) + (ROW_H * scale - pill_h) / 2.0;
        let h = Rect::new(ctrl_x, r4y, pill_w, pill_h);
        let s = Rect::new(ctrl_x + pill_w + 12.0 * scale, r4y, pill_w + 18.0 * scale, pill_h);
        (Some(dt), Some(seg), Some(h), Some(s))
    } else {
        (None, None, None, None)
    };

    PopLayout { rect, size_track, space_track, date_toggle, date_seg, h24, seconds }
}

/// Map a cursor x on a slider's track to its snapped value.
pub fn slider_value_at(
    id: TileId,
    slider: WidgetSlider,
    controls: &Controls,
    panel: Rect,
    scale: f32,
    px: f32,
) -> f32 {
    let l = layout(id, controls, panel, scale);
    let track = match slider {
        WidgetSlider::Size => l.size_track,
        WidgetSlider::Space => l.space_track,
    };
    let t = ((px - track.x) / track.w).clamp(0.0, 1.0);
    match slider {
        WidgetSlider::Size => {
            let raw = SIZE_MIN + t * (SIZE_MAX - SIZE_MIN);
            ((raw / 0.05).round() * 0.05).clamp(SIZE_MIN, SIZE_MAX)
        }
        WidgetSlider::Space => {
            let raw = SPACE_MIN + t * (SPACE_MAX - SPACE_MIN);
            ((raw / 2.0).round() * 2.0).clamp(SPACE_MIN, SPACE_MAX)
        }
    }
}

pub fn hit(id: TileId, controls: &Controls, panel: Rect, scale: f32, px: f32, py: f32) -> Option<Hit> {
    let l = layout(id, controls, panel, scale);
    let inside = |r: &Rect| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h;
    // Generous vertical pad on the slider tracks so the knob is grabbable.
    let pad = KNOB_R * scale;
    let track_hit = |t: &Rect| Rect::new(t.x - pad, t.y - pad, t.w + pad * 2.0, t.h + pad * 2.0);
    if inside(&track_hit(&l.size_track)) {
        return Some(Hit::Slider(WidgetSlider::Size));
    }
    if inside(&track_hit(&l.space_track)) {
        return Some(Hit::Slider(WidgetSlider::Space));
    }
    if let Some(r) = l.date_toggle {
        if inside(&r) {
            return Some(Hit::ToggleDate);
        }
    }
    if let Some(seg) = l.date_seg {
        for (r, pos) in seg.iter().zip([DatePos::Below, DatePos::Left, DatePos::Right]) {
            if inside(r) {
                return Some(Hit::SetDatePos(pos));
            }
        }
    }
    if let Some(r) = l.h24 {
        if inside(&r) {
            return Some(Hit::Toggle24h);
        }
    }
    if let Some(r) = l.seconds {
        if inside(&r) {
            return Some(Hit::ToggleSeconds);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    id: TileId,
    opts: &WidgetOpts,
    controls: &Controls,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let l = layout(id, controls, panel, scale);
    let r = l.rect;
    painter.shadow(r, RADIUS * scale, 20.0 * scale, Color::BLACK.with_alpha(0.4 * alpha), 0.0, 5.0 * scale);
    painter.rect_filled(r, RADIUS * scale, Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2).with_alpha(0.95 * alpha));
    painter.rect_stroke_sdf(r, RADIUS * scale, 1.0 * scale, Color::rgba(1.0, 1.0, 1.0, 0.12 * alpha));

    let f = FONT * scale;
    let inner_x = r.x + PAD * scale;
    let white = Color::rgba(1.0, 1.0, 1.0, alpha);
    let dim = Color::rgba(1.0, 1.0, 1.0, 0.6 * alpha);

    // Title.
    text.queue(id.display_name(), f, inner_x, r.y + PAD * scale, white, r.w, surface_w, surface_h);

    // Size + Space rows (universal).
    label(text, "Size", inner_x, l.size_track.y, l.size_track.h, f, dim, surface_w, surface_h);
    draw_slider(painter, l.size_track, slider_t(opts.size), scale, alpha);
    label(text, "Space", inner_x, l.space_track.y, l.space_track.h, f, dim, surface_w, surface_h);
    draw_slider(painter, l.space_track, space_t(opts.space), scale, alpha);

    if is_clock(id) {
        let c = &opts.clock;
        if let Some(dt) = l.date_toggle {
            label(text, "Date", inner_x, dt.y, dt.h, f, dim, surface_w, surface_h);
            draw_pill(painter, text, dt, if c.show_date { "On" } else { "Off" }, c.show_date, scale, alpha, surface_w, surface_h);
        }
        if let Some(seg) = l.date_seg {
            label(text, "Date at", inner_x, seg[0].y, seg[0].h, f, dim, surface_w, surface_h);
            let labels = ["Below", "Left", "Right"];
            let active = [DatePos::Below, DatePos::Left, DatePos::Right];
            for ((rr, lbl), pos) in seg.iter().zip(labels).zip(active) {
                draw_pill(painter, text, *rr, lbl, c.date_pos == pos, scale, alpha, surface_w, surface_h);
            }
        }
        if let Some(h) = l.h24 {
            draw_pill(painter, text, h, "24h", c.hour24, scale, alpha, surface_w, surface_h);
        }
        if let Some(s) = l.seconds {
            draw_pill(painter, text, s, "Seconds", c.seconds, scale, alpha, surface_w, surface_h);
        }
    }
}

fn slider_t(size: f32) -> f32 {
    ((size - SIZE_MIN) / (SIZE_MAX - SIZE_MIN)).clamp(0.0, 1.0)
}

fn space_t(space: f32) -> f32 {
    ((space - SPACE_MIN) / (SPACE_MAX - SPACE_MIN)).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
fn label(text: &mut TextRenderer, s: &str, x: f32, row_y: f32, row_h: f32, f: f32, col: Color, sw: u32, sh: u32) {
    text.queue(s, f, x, row_y + (row_h - f) / 2.0, col, 200.0, sw, sh);
}

fn draw_slider(painter: &mut Painter, track: Rect, t: f32, scale: f32, alpha: f32) {
    let r = track.h / 2.0;
    painter.rect_filled(track, r, Color::rgba(1.0, 1.0, 1.0, 0.18 * alpha));
    let fill_w = (track.w * t).max(track.h);
    painter.rect_filled(Rect::new(track.x, track.y, fill_w, track.h), r, accent(alpha));
    let knob_cx = track.x + track.w * t;
    painter.circle_filled(knob_cx, track.y + track.h / 2.0, KNOB_R * scale, Color::rgba(1.0, 1.0, 1.0, alpha));
}

#[allow(clippy::too_many_arguments)]
fn draw_pill(
    painter: &mut Painter,
    text: &mut TextRenderer,
    r: Rect,
    s: &str,
    on: bool,
    scale: f32,
    alpha: f32,
    sw: u32,
    sh: u32,
) {
    let radius = r.h * 0.5;
    if on {
        painter.rect_filled(r, radius, accent(alpha));
    } else {
        painter.rect_filled(r, radius, Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha));
        painter.rect_stroke_sdf(r, radius, 1.0 * scale, Color::rgba(1.0, 1.0, 1.0, 0.18 * alpha));
    }
    let f = FONT * scale;
    let tw = text.measure_width(s, f).min(r.w - 6.0 * scale);
    let col = if on { Color::from_rgb8(20, 16, 6).with_alpha(alpha) } else { Color::rgba(1.0, 1.0, 1.0, 0.9 * alpha) };
    text.queue(s, f, r.x + (r.w - tw) / 2.0, r.y + (r.h - f) / 2.0, col, r.w, sw, sh);
}

fn accent(a: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(a)
}
