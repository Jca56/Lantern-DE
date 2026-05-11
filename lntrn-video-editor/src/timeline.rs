//! Timeline panel — multi-track NLE strip below the preview.
//!
//! Owns rendering of the time ruler, track lanes, clip blocks, and the
//! playhead, plus all hit-testing required by the main loop (which clip
//! is under the cursor, which trim handle, where on the ruler).

use crate::chrome;
use crate::layout::{PANEL_PAD, TIMELINE_HEADER_H, TIMELINE_LABEL_W};
use crate::playback::Playback;
use crate::project::{ClipId, Project, Track, TrackId, TrackKind};
use lntrn_render::{Color, Painter, Rect, TextRenderer};

/// Width of the trim-handle hit zone at each clip edge (logical pixels).
pub const EDGE_HANDLE_W: f32 = 8.0;
/// Width of the mute-toggle hit zone inside the track label gutter.
pub const MUTE_HIT_W: f32 = 18.0;

const TRACK_H_MAX: f32 = 44.0;
const TRACK_H_MIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrimEdge {
    Left,
    Right,
}

fn clip_video_color() -> Color {
    Color::from_rgb8(0x4a, 0x80, 0xb0)
}
fn clip_audio_color() -> Color {
    Color::from_rgb8(0x5f, 0xb8, 0x68)
}

/// Per-track layout info — computed once per frame so render + hit-test agree.
pub struct LaneInfo {
    pub track_id: TrackId,
    pub kind: TrackKind,
    pub mute: bool,
    pub label: String,
    pub rect: Rect,
}

/// Build the list of lanes in display order (video tracks top→bottom by
/// descending index, then audio tracks top→bottom by ascending index).
pub fn build_lanes(project: &Project, r: &Rect, s: f32) -> Vec<LaneInfo> {
    let header_h = TIMELINE_HEADER_H * s;
    let available_h = (r.h - header_h).max(0.0);
    let mut display: Vec<&Track> = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    display.sort_by(|a, b| b.index.cmp(&a.index));
    let mut audio: Vec<&Track> = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .collect();
    audio.sort_by(|a, b| a.index.cmp(&b.index));
    display.extend(audio);

    let n = display.len().max(1) as f32;
    let sep = 1.0 * s;
    let track_h = ((available_h - (n - 1.0) * sep) / n)
        .clamp(TRACK_H_MIN * s, TRACK_H_MAX * s);

    let mut lanes = Vec::with_capacity(display.len());
    let mut y = r.y + header_h;
    for track in display {
        lanes.push(LaneInfo {
            track_id: track.id,
            kind: track.kind,
            mute: track.mute,
            label: track.label(),
            rect: Rect::new(r.x, y, r.w, track_h),
        });
        y += track_h + sep;
    }
    lanes
}

/// Project + active media duration combined into a single "visible" duration
/// used by every timeline pixel→time mapping.
pub fn timeline_visible_duration(project: &Project, playback: &Playback) -> f64 {
    let project_dur = project.timeline_duration();
    if project_dur > 0.0 {
        project_dur.max(playback.duration).max(1.0)
    } else if playback.duration > 0.0 {
        playback.duration
    } else {
        60.0
    }
}

/// Compact tick label — minutes:seconds for short clips, hours:minutes for long.
fn format_ruler_mark(secs: f64) -> String {
    let secs = secs.max(0.0);
    if secs >= 3600.0 {
        let h = (secs / 3600.0) as u32;
        let m = ((secs % 3600.0) / 60.0) as u32;
        format!("{h}:{m:02}h")
    } else {
        let m = (secs / 60.0) as u32;
        let s = (secs % 60.0) as u32;
        format!("{m}:{s:02}")
    }
}

/// Compute a clip's drawn rect within its lane.
fn clip_rect_in_lane(
    lane: &Rect,
    ruler_x: f32,
    ruler_w: f32,
    dur: f64,
    clip_start: f64,
    clip_dur: f64,
    s: f32,
) -> Option<Rect> {
    if clip_dur <= 0.0 || dur <= 0.0 {
        return None;
    }
    let clip_x = ruler_x + (clip_start / dur).clamp(0.0, 1.0) as f32 * ruler_w;
    let raw_w = ((clip_dur / dur) as f32 * ruler_w).max(24.0 * s);
    let clip_w = raw_w.min(ruler_x + ruler_w - clip_x);
    if clip_w <= 8.0 * s {
        return None;
    }
    Some(Rect::new(
        clip_x + 4.0 * s,
        lane.y + 6.0 * s,
        clip_w - 8.0 * s,
        lane.h - 12.0 * s,
    ))
}

/// Map an x-coordinate inside the timeline scrub area back to a time in
/// seconds along the project timeline.
pub fn cursor_to_timeline_secs(
    project: &Project,
    playback: &Playback,
    r: &Rect,
    px: f32,
    s: f32,
) -> Option<f64> {
    let label_w = TIMELINE_LABEL_W * s;
    let ruler_x = r.x + label_w;
    let ruler_w = r.w - label_w;
    if ruler_w <= 0.0 {
        return None;
    }
    let prog = ((px - ruler_x) / ruler_w).clamp(0.0, 1.0);
    let dur = timeline_visible_duration(project, playback);
    Some(prog as f64 * dur)
}

/// Hit-test: clip the cursor is over. Edge handles take priority — the
/// caller should consult `timeline_clip_edge_at` first.
pub fn timeline_clip_at(
    project: &Project,
    playback: &Playback,
    r: &Rect,
    px: f32,
    py: f32,
    s: f32,
) -> Option<ClipId> {
    if !r.contains(px, py) {
        return None;
    }
    let label_w = TIMELINE_LABEL_W * s;
    let ruler_x = r.x + label_w;
    let ruler_w = r.w - label_w;
    if ruler_w <= 0.0 || px < ruler_x {
        return None;
    }
    let dur = timeline_visible_duration(project, playback);
    let lanes = build_lanes(project, r, s);

    for clip in project.timeline_clips.iter().rev() {
        let Some(lane) = lanes.iter().find(|l| l.track_id == clip.track_id) else {
            continue;
        };
        if let Some(rect) = clip_rect_in_lane(
            &lane.rect,
            ruler_x,
            ruler_w,
            dur,
            clip.start,
            clip.duration,
            s,
        ) {
            if rect.contains(px, py) {
                return Some(clip.id);
            }
        }
    }
    None
}

pub fn timeline_clip_edge_at(
    project: &Project,
    playback: &Playback,
    r: &Rect,
    px: f32,
    py: f32,
    s: f32,
) -> Option<(ClipId, TrimEdge)> {
    if !r.contains(px, py) {
        return None;
    }
    let label_w = TIMELINE_LABEL_W * s;
    let ruler_x = r.x + label_w;
    let ruler_w = r.w - label_w;
    if ruler_w <= 0.0 || px < ruler_x {
        return None;
    }
    let edge_w = EDGE_HANDLE_W * s;
    let dur = timeline_visible_duration(project, playback);
    let lanes = build_lanes(project, r, s);

    for clip in project.timeline_clips.iter().rev() {
        let Some(lane) = lanes.iter().find(|l| l.track_id == clip.track_id) else {
            continue;
        };
        let Some(rect) = clip_rect_in_lane(
            &lane.rect,
            ruler_x,
            ruler_w,
            dur,
            clip.start,
            clip.duration,
            s,
        ) else {
            continue;
        };
        if !rect.contains(px, py) {
            continue;
        }
        if rect.w < edge_w * 3.0 {
            continue;
        }
        if px < rect.x + edge_w {
            return Some((clip.id, TrimEdge::Left));
        }
        if px > rect.x + rect.w - edge_w {
            return Some((clip.id, TrimEdge::Right));
        }
    }
    None
}

/// Mute-button hit-test in the track label gutter. Returns the track whose
/// mute toggle was clicked, if any.
pub fn track_mute_at(project: &Project, r: &Rect, px: f32, py: f32, s: f32) -> Option<TrackId> {
    if !r.contains(px, py) {
        return None;
    }
    let label_w = TIMELINE_LABEL_W * s;
    if px > r.x + label_w {
        return None;
    }
    let mute_w = MUTE_HIT_W * s;
    let mute_x = r.x + label_w - mute_w - 4.0 * s;
    if px < mute_x {
        return None;
    }
    for lane in build_lanes(project, r, s) {
        if py >= lane.rect.y && py < lane.rect.y + lane.rect.h {
            return Some(lane.track_id);
        }
    }
    None
}

// ── Drawing ────────────────────────────────────────────────────────────────

pub fn draw(
    p: &mut Painter,
    t: &mut TextRenderer,
    r: &Rect,
    project: &Project,
    playback: &Playback,
    s: f32,
    sw: u32,
    sh: u32,
) {
    p.rect_filled(*r, 0.0, chrome::BG);
    let x = r.x;
    let y = r.y;
    let w = r.w;
    let text_dim = chrome::text_dim();
    let playhead_color = chrome::accent();

    // Time ruler header
    let header_h = TIMELINE_HEADER_H * s;
    p.rect_filled(Rect::new(x, y, w, header_h), 0.0, chrome::PANEL);
    p.rect_filled(
        Rect::new(x, y + header_h - 1.0 * s, w, 1.0 * s),
        0.0,
        chrome::BORDER,
    );

    let label_w = TIMELINE_LABEL_W * s;
    let ruler_x = x + label_w;
    let ruler_w = w - label_w;

    let dur = timeline_visible_duration(project, playback);
    let mark_count = 7usize;
    let mark_step = dur / (mark_count - 1) as f64;
    for i in 0..mark_count {
        let t_secs = i as f64 * mark_step;
        let mx = ruler_x + (i as f32 / (mark_count - 1) as f32) * ruler_w;
        p.rect_filled(
            Rect::new(mx, y + header_h - 8.0 * s, 1.0 * s, 8.0 * s),
            0.0,
            text_dim,
        );
        let label = format_ruler_mark(t_secs);
        t.queue(
            &label,
            14.0 * s,
            mx + 3.0 * s,
            y + 6.0 * s,
            text_dim,
            w,
            sw,
            sh,
        );
    }

    let lanes = build_lanes(project, r, s);
    for (i, lane) in lanes.iter().enumerate() {
        let bg = if i % 2 == 0 {
            chrome::PANEL_DARK
        } else {
            chrome::INPUT_BG
        };
        p.rect_filled(lane.rect, 0.0, bg);

        // Track label gutter
        let label_r = Rect::new(lane.rect.x, lane.rect.y, label_w, lane.rect.h);
        p.rect_filled(label_r, 0.0, chrome::PANEL);
        p.rect_filled(
            Rect::new(
                lane.rect.x + label_w - 1.0 * s,
                lane.rect.y,
                1.0 * s,
                lane.rect.h,
            ),
            0.0,
            chrome::BORDER,
        );
        let pad = PANEL_PAD * s;
        let label_color = if lane.mute { text_dim } else { chrome::text() };
        t.queue(
            &lane.label,
            14.0 * s,
            lane.rect.x + pad,
            lane.rect.y + (lane.rect.h - 14.0 * s) * 0.5,
            label_color,
            w,
            sw,
            sh,
        );

        // Mute toggle on the right edge of the label gutter
        let mute_w = MUTE_HIT_W * s;
        let mute_x = lane.rect.x + label_w - mute_w - 4.0 * s;
        let mute_y = lane.rect.y + (lane.rect.h - mute_w) * 0.5;
        let mute_r = Rect::new(mute_x, mute_y, mute_w, mute_w);
        p.rect_filled(
            mute_r,
            3.0 * s,
            if lane.mute {
                chrome::close_red()
            } else {
                chrome::BUTTON
            },
        );
        p.rect_stroke_sdf(mute_r, 3.0 * s, 1.0 * s, chrome::BORDER);
        let glyph = if lane.mute { "M" } else { "m" };
        t.queue(
            glyph,
            12.0 * s,
            mute_x + 5.0 * s,
            mute_y + 3.0 * s,
            chrome::text(),
            w,
            sw,
            sh,
        );

        // Lane separator
        p.rect_filled(
            Rect::new(lane.rect.x, lane.rect.y + lane.rect.h, w, 1.0 * s),
            0.0,
            chrome::BORDER,
        );
    }

    // Empty-state hint inside the first (V1) video lane
    if project.timeline_clips.is_empty() {
        if let Some(lane) = lanes
            .iter()
            .find(|l| l.kind == TrackKind::Video && l.label == "V1")
        {
            let hint = if project.selected_media.is_some() {
                "Press Enter to add the selected media to the timeline"
            } else {
                "Open a video (File → Open) to start editing"
            };
            let sz = 14.0 * s;
            t.queue(
                hint,
                sz,
                ruler_x + 12.0 * s,
                lane.rect.y + (lane.rect.h - sz) * 0.5,
                text_dim,
                w,
                sw,
                sh,
            );
        }
    }

    // Clips
    for clip in &project.timeline_clips {
        let Some(media) = project.media_by_id(clip.media_id) else {
            continue;
        };
        let Some(lane) = lanes.iter().find(|l| l.track_id == clip.track_id) else {
            continue;
        };
        let Some(rect) = clip_rect_in_lane(
            &lane.rect,
            ruler_x,
            ruler_w,
            dur,
            clip.start,
            clip.duration,
            s,
        ) else {
            continue;
        };
        let selected = project.selected_clip == Some(clip.id);
        let (color, is_audio) = match lane.kind {
            TrackKind::Video => (clip_video_color(), false),
            TrackKind::Audio => (clip_audio_color(), true),
        };
        let dimmed = lane.mute;
        let label = match lane.kind {
            TrackKind::Video => format_clip_label(media.name.as_str(), clip.speed),
            TrackKind::Audio => format!("{} audio", media.name),
        };
        draw_clip_block(p, t, rect, color, &label, selected, is_audio, dimmed, s, sw, sh);
    }

    // Playhead
    let prog = if dur > 0.0 {
        (playback.timeline_position / dur).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let playhead_x = ruler_x + prog * ruler_w;
    let playhead_bot = if let Some(last) = lanes.last() {
        last.rect.y + last.rect.h
    } else {
        y + header_h
    };
    p.rect_filled(
        Rect::new(playhead_x - 0.5 * s, y, 2.0 * s, playhead_bot - y),
        0.0,
        playhead_color,
    );
    let diam = 10.0 * s;
    p.rect_filled(
        Rect::new(playhead_x - diam * 0.5, y, diam, diam * 0.7),
        2.0 * s,
        playhead_color,
    );
}

fn format_clip_label(name: &str, speed: f32) -> String {
    if (speed - 1.0).abs() < 0.005 {
        name.to_string()
    } else {
        format!("{name}  {speed:.2}x")
    }
}

fn draw_clip_block(
    p: &mut Painter,
    t: &mut TextRenderer,
    r: Rect,
    color: Color,
    label: &str,
    selected: bool,
    audio: bool,
    dimmed: bool,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let fill = if dimmed {
        Color::rgba(color.r * 0.45, color.g * 0.45, color.b * 0.45, color.a)
    } else {
        color
    };
    p.rect_filled(r, 4.0 * s, fill);
    p.rect_stroke_sdf(
        r,
        4.0 * s,
        if selected { 2.0 * s } else { 1.0 * s },
        if selected {
            chrome::accent()
        } else {
            chrome::BORDER
        },
    );
    if selected {
        p.rect_filled(Rect::new(r.x, r.y, r.w, 3.0 * s), 4.0 * s, chrome::accent());
    }

    // Trim handles
    let edge_w = EDGE_HANDLE_W * s;
    if r.w >= edge_w * 4.0 {
        let strip = Color::rgba(1.0, 1.0, 1.0, 0.18);
        p.rect_filled(
            Rect::new(r.x + 1.0 * s, r.y + 4.0 * s, 2.0 * s, r.h - 8.0 * s),
            1.0 * s,
            strip,
        );
        p.rect_filled(
            Rect::new(r.x + r.w - 3.0 * s, r.y + 4.0 * s, 2.0 * s, r.h - 8.0 * s),
            1.0 * s,
            strip,
        );
    }

    if audio {
        draw_audio_waveform_hint(p, r, s);
    }

    t.queue(
        label,
        12.0 * s,
        r.x + 8.0 * s,
        r.y + 7.0 * s,
        chrome::text(),
        r.w,
        sw,
        sh,
    );
}

fn draw_audio_waveform_hint(p: &mut Painter, r: Rect, s: f32) {
    let col = Color::rgba(1.0, 1.0, 1.0, 0.35);
    let base_y = r.y + r.h * 0.62;
    let step = 8.0 * s;
    let mut x = r.x + 10.0 * s;
    let max_x = r.x + r.w - 8.0 * s;
    let mut i = 0usize;
    while x < max_x {
        let h = [8.0, 14.0, 10.0, 18.0, 12.0][i % 5] * s;
        p.rect_filled(Rect::new(x, base_y - h * 0.5, 2.0 * s, h), 1.0 * s, col);
        x += step;
        i += 1;
    }
}
