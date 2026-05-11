//! Inspector panel — selected clip's metadata + editable speed / scale /
//! volume sliders. Drag interactions are handled by the main loop via the
//! hit-test rects returned from `field_at`.

use crate::chrome;
use crate::layout::PANEL_PAD;
use crate::playback;
use crate::project::{Project, Track, TrackKind};
use lntrn_render::{Painter, Rect, TextRenderer};

const ROW_H: f32 = 28.0;
const ROW_GAP: f32 = 6.0;
const SLIDER_W: f32 = 130.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InspectorField {
    Speed,
    Scale,
    OffsetX,
    OffsetY,
    Opacity,
    Volume,
}

pub struct FieldHit {
    pub field: InspectorField,
    pub rect: Rect,
    pub min: f32,
    pub max: f32,
}

/// Drag-zone hit-test. Returns the field under (px,py) plus its slider range.
pub fn field_at(project: &Project, r: &Rect, px: f32, py: f32, s: f32) -> Option<FieldHit> {
    if !r.contains(px, py) {
        return None;
    }
    let clip = project.selected_clip_ref()?;
    let track_kind = project
        .track_by_id(clip.track_id)
        .map(|t| t.kind)
        .unwrap_or(TrackKind::Video);
    let layout = compute_field_layout(r, s, track_kind);
    for hit in layout {
        if hit.rect.contains(px, py) {
            return Some(hit);
        }
    }
    None
}

fn compute_field_layout(r: &Rect, s: f32, kind: TrackKind) -> Vec<FieldHit> {
    let pad = PANEL_PAD * s;
    let row_h = ROW_H * s;
    let slider_w = SLIDER_W * s;
    let slider_x = r.x + r.w - slider_w - pad;
    let mut y = r.y + 110.0 * s; // below media header
    let mut out = Vec::new();
    let mut push = |field: InspectorField, min: f32, max: f32, yp: &mut f32| {
        let rect = Rect::new(slider_x, *yp, slider_w, row_h);
        out.push(FieldHit {
            field,
            rect,
            min,
            max,
        });
        *yp += row_h + ROW_GAP * s;
    };
    push(InspectorField::Speed, 0.1, 4.0, &mut y);
    if kind == TrackKind::Video {
        push(InspectorField::Scale, 0.1, 4.0, &mut y);
        push(InspectorField::OffsetX, -0.5, 0.5, &mut y);
        push(InspectorField::OffsetY, -0.5, 0.5, &mut y);
        push(InspectorField::Opacity, 0.0, 1.0, &mut y);
    } else {
        push(InspectorField::Volume, 0.0, 2.0, &mut y);
    }
    out
}

pub fn draw(
    p: &mut Painter,
    t: &mut TextRenderer,
    r: &Rect,
    project: &Project,
    s: f32,
    sw: u32,
    sh: u32,
) {
    p.rect_filled(*r, 0.0, chrome::PANEL);
    let pad = PANEL_PAD * s;
    let x = r.x + pad;
    let y = r.y + pad;
    let w = r.w;
    let accent = chrome::accent();
    let text = chrome::text();
    let text_dim = chrome::text_dim();

    t.queue("I N S P E C T O R", 16.0 * s, x, y, accent, w, sw, sh);
    p.rect_filled(
        Rect::new(x, y + 24.0 * s, r.w - pad * 2.0, 1.0 * s),
        0.0,
        chrome::BORDER,
    );

    let Some(clip) = project.selected_clip_ref() else {
        t.queue(
            "(no clip selected)",
            14.0 * s,
            x,
            y + 44.0 * s,
            text_dim,
            w,
            sw,
            sh,
        );
        return;
    };
    let Some(media) = project.media_by_id(clip.media_id) else {
        return;
    };
    let track = project.track_by_id(clip.track_id);
    let kind = track.map(|t| t.kind).unwrap_or(TrackKind::Video);
    let track_label = track.map(Track::label).unwrap_or_else(|| "?".into());

    // Header block: media name + source path + timing
    let name_y = y + 40.0 * s;
    t.queue(&media.name, 16.0 * s, x, name_y, text, w, sw, sh);

    let path_str = media.path.display().to_string();
    let truncated = if path_str.len() > 42 {
        format!("…{}", &path_str[path_str.len() - 40..])
    } else {
        path_str
    };
    t.queue(
        &truncated,
        11.0 * s,
        x,
        name_y + 22.0 * s,
        text_dim,
        w,
        sw,
        sh,
    );

    let fps = if media.fps > 0.0 { media.fps } else { 30.0 };
    let in_tc = playback::format_timecode(clip.media_in, fps);
    let out_tc = playback::format_timecode(clip.media_out(), fps);
    let meta = format!("{}  •  IN {}  •  OUT {}", track_label, in_tc, out_tc);
    t.queue(&meta, 11.0 * s, x, name_y + 42.0 * s, text_dim, w, sw, sh);

    // Sliders
    let layout = compute_field_layout(r, s, kind);
    for hit in &layout {
        let label = match hit.field {
            InspectorField::Speed => format!("Speed   {:.2}x", clip.speed),
            InspectorField::Scale => format!("Scale   {:.2}x", clip.transform.scale),
            InspectorField::OffsetX => format!("Off X   {:+.2}", clip.transform.offset_x),
            InspectorField::OffsetY => format!("Off Y   {:+.2}", clip.transform.offset_y),
            InspectorField::Opacity => format!("Opacity {:.0}%", clip.transform.opacity * 100.0),
            InspectorField::Volume => format!("Volume  {:.0}%", clip.volume * 100.0),
        };
        let label_y = hit.rect.y + (hit.rect.h - 12.0 * s) * 0.5;
        t.queue(&label, 12.0 * s, x, label_y, text, w, sw, sh);

        let value = match hit.field {
            InspectorField::Speed => clip.speed,
            InspectorField::Scale => clip.transform.scale,
            InspectorField::OffsetX => clip.transform.offset_x,
            InspectorField::OffsetY => clip.transform.offset_y,
            InspectorField::Opacity => clip.transform.opacity,
            InspectorField::Volume => clip.volume,
        };
        draw_slider(p, hit.rect, hit.min, hit.max, value, s);
    }
}

fn draw_slider(p: &mut Painter, r: Rect, min: f32, max: f32, value: f32, s: f32) {
    p.rect_filled(r, 4.0 * s, chrome::INPUT_BG);
    p.rect_stroke_sdf(r, 4.0 * s, 1.0 * s, chrome::BORDER);
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    let knob_w = 6.0 * s;
    let track_x = r.x + 4.0 * s;
    let track_w = r.w - 8.0 * s - knob_w;
    let fill_w = track_w * t + knob_w;
    p.rect_filled(
        Rect::new(r.x, r.y, fill_w + 4.0 * s, r.h),
        4.0 * s,
        chrome::ACTIVE,
    );
    p.rect_filled(
        Rect::new(track_x + track_w * t, r.y + 3.0 * s, knob_w, r.h - 6.0 * s),
        2.0 * s,
        chrome::accent(),
    );
}

/// Convert a cursor x inside a slider rect to a clamped value in [min,max].
pub fn slider_value_for_cursor(hit: &FieldHit, px: f32, s: f32) -> f32 {
    let knob_w = 6.0 * s;
    let track_x = hit.rect.x + 4.0 * s;
    let track_w = hit.rect.w - 8.0 * s - knob_w;
    let frac = ((px - track_x) / track_w).clamp(0.0, 1.0);
    hit.min + frac * (hit.max - hit.min)
}
