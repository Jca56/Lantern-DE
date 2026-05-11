//! Top-level panel orchestration. Per-panel drawing lives in dedicated
//! modules (`timeline`, `inspector`, `preview`) so this file stays narrow.
//! Colors and patterns match lantern-studio's warm brown/gold theme.

use crate::chrome;
use crate::inspector;
use crate::layout::{Layout, PANEL_PAD};
use crate::playback::{self, Playback};
use crate::project::{MediaId, Project};
use crate::timeline;
use lntrn_render::{Painter, Rect, TextRenderer};

const MEDIA_ROW_H: f32 = 54.0;
const MEDIA_LIST_TOP: f32 = 36.0;

// Re-exports so existing call sites don't need to update.
pub use crate::timeline::{
    cursor_to_timeline_secs, timeline_clip_at, timeline_clip_edge_at, timeline_visible_duration,
    track_mute_at, TrimEdge,
};

/// Draw all panels for the current frame.
pub fn draw_panels(
    p: &mut Painter,
    t: &mut TextRenderer,
    layout: &Layout,
    project: &Project,
    playback: &Playback,
    s: f32,
    sw: u32,
    sh: u32,
) {
    draw_media_browser(p, t, &layout.media_browser, project, s, sw, sh);
    // Preview is handled by preview.rs (video texture + timecode)
    inspector::draw(p, t, &layout.properties, project, s, sw, sh);
    timeline::draw(p, t, &layout.timeline, project, playback, s, sw, sh);
    draw_status_bar(p, t, &layout.status_bar, project, playback, s, sw, sh);
    draw_dividers(p, layout, s);
}

pub fn media_item_at(project: &Project, r: &Rect, px: f32, py: f32, s: f32) -> Option<MediaId> {
    if !r.contains(px, py) {
        return None;
    }
    let pad = PANEL_PAD * s;
    let row_h = MEDIA_ROW_H * s;
    let start_y = r.y + MEDIA_LIST_TOP * s;
    let x = r.x + pad;
    let w = r.w - pad * 2.0;
    for (i, item) in project.media.iter().enumerate() {
        let y = start_y + i as f32 * (row_h + 6.0 * s);
        let row = Rect::new(x, y, w, row_h);
        if row.contains(px, py) {
            return Some(item.id);
        }
    }
    None
}

fn draw_dividers(p: &mut Painter, layout: &Layout, s: f32) {
    p.rect_filled(layout.div_left, 0.0, chrome::BORDER);
    p.rect_filled(layout.div_right, 0.0, chrome::BORDER);
    chrome::draw_rainbow_h(
        p,
        layout.div_h_upper.x,
        layout.div_h_upper.y,
        layout.div_h_upper.w,
        s,
    );
    p.rect_filled(layout.div_h_lower, 0.0, chrome::BORDER);
}

// ── Media Browser ──────────────────────────────────────────────────────────

fn draw_media_browser(
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

    t.queue("M E D I A", 16.0 * s, x, y, accent, w, sw, sh);
    p.rect_filled(
        Rect::new(x, y + 24.0 * s, r.w - pad * 2.0, 1.0 * s),
        0.0,
        chrome::BORDER,
    );

    let row_h = MEDIA_ROW_H * s;
    let row_gap = 6.0 * s;
    let row_x = x;
    let row_w = r.w - pad * 2.0;
    let mut row_y = r.y + MEDIA_LIST_TOP * s;

    for item in &project.media {
        if row_y + row_h > r.y + r.h - pad {
            break;
        }
        let selected = project.selected_media == Some(item.id);
        let row = Rect::new(row_x, row_y, row_w, row_h);
        let bg = if selected {
            chrome::ACTIVE
        } else {
            chrome::PANEL_DARK
        };
        p.rect_filled(row, 4.0 * s, bg);
        p.rect_stroke_sdf(
            row,
            4.0 * s,
            1.0 * s,
            if selected { accent } else { chrome::BORDER },
        );
        t.queue(
            &item.name,
            14.0 * s,
            row_x + 8.0 * s,
            row_y + 8.0 * s,
            text,
            row_w,
            sw,
            sh,
        );
        let details = format!(
            "{}x{}  {}",
            item.width,
            item.height,
            short_duration(item.duration)
        );
        t.queue(
            &details,
            12.0 * s,
            row_x + 8.0 * s,
            row_y + 30.0 * s,
            text_dim,
            row_w,
            sw,
            sh,
        );
        row_y += row_h + row_gap;
    }
}

fn short_duration(secs: f64) -> String {
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

// ── Status Bar ─────────────────────────────────────────────────────────────

fn draw_status_bar(
    p: &mut Painter,
    t: &mut TextRenderer,
    r: &Rect,
    project: &Project,
    playback: &Playback,
    s: f32,
    sw: u32,
    sh: u32,
) {
    p.rect_filled(*r, 0.0, chrome::PANEL);
    let pad = PANEL_PAD * s;
    let text_y = r.y + (r.h - 14.0 * s) * 0.5;
    let w = r.w;
    let font = 14.0 * s;
    let text_dim = chrome::text_dim();
    let text_col = chrome::text();

    let mut tx = r.x + pad;
    t.queue("v0.1.0", font, tx, text_y, text_dim, w, sw, sh);
    tx += 60.0 * s;

    t.queue("|", font, tx, text_y, chrome::BORDER, w, sw, sh);
    tx += 16.0 * s;

    t.queue("Untitled Project", font, tx, text_y, text_col, w, sw, sh);
    tx += 140.0 * s;

    t.queue("|", font, tx, text_y, chrome::BORDER, w, sw, sh);
    tx += 16.0 * s;

    let media_label = if playback.has_media() {
        format!(
            "{}x{}  {:.2}fps",
            playback.video_width, playback.video_height, playback.fps
        )
    } else {
        "no media".to_string()
    };
    t.queue(&media_label, font, tx, text_y, text_dim, w, sw, sh);

    // Optional export-progress slot fed by main loop via the global slot below.
    if let Some(msg) = crate::export::status_message() {
        tx += 180.0 * s;
        t.queue("|", font, tx, text_y, chrome::BORDER, w, sw, sh);
        tx += 16.0 * s;
        t.queue(&msg, font, tx, text_y, chrome::accent(), w, sw, sh);
    }

    // Position / duration on the right
    let fps = playback.fps.max(1.0);
    let proj_dur = project.timeline_duration();
    let dur_secs = if proj_dur > 0.0 {
        proj_dur
    } else {
        playback.duration
    };
    let pos_tc = playback::format_timecode(playback.timeline_position, fps);
    let dur_tc = playback::format_timecode(dur_secs, fps);
    let dur_label = format!("{pos_tc}  /  {dur_tc}");
    let dur_w = font * 0.60 * dur_label.len() as f32;
    t.queue(
        &dur_label,
        font,
        r.x + r.w - pad - dur_w,
        text_y,
        text_col,
        w,
        sw,
        sh,
    );
}
