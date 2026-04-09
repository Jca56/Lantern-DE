//! Drawing functions for each panel of the NLE layout.
//! Colors and patterns match lantern-studio's warm brown/gold theme.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use crate::chrome;
use crate::layout::{Layout, PANEL_PAD};

// Track-specific colors
fn clip_video() -> Color { Color::from_rgb8(0x4a, 0x80, 0xb0) }
fn clip_audio() -> Color { Color::from_rgb8(0x50, 0x90, 0x60) }

/// Draw all panels for the current frame.
pub fn draw_panels(
    p: &mut Painter, t: &mut TextRenderer,
    layout: &Layout, s: f32, sw: u32, sh: u32,
) {
    draw_media_browser(p, t, &layout.media_browser, s, sw, sh);
    // Preview is handled by preview.rs (video texture + timecode)
    draw_properties(p, t, &layout.properties, s, sw, sh);
    draw_timeline(p, t, &layout.timeline, s, sw, sh);
    draw_status_bar(p, t, &layout.status_bar, s, sw, sh);
    draw_dividers(p, layout, s);
}

fn draw_dividers(p: &mut Painter, layout: &Layout, s: f32) {
    p.rect_filled(layout.div_left, 0.0, chrome::BORDER);
    p.rect_filled(layout.div_right, 0.0, chrome::BORDER);
    // Rainbow gradient between upper panels and timeline
    chrome::draw_rainbow_h(
        p, layout.div_h_upper.x, layout.div_h_upper.y,
        layout.div_h_upper.w, s,
    );
    p.rect_filled(layout.div_h_lower, 0.0, chrome::BORDER);
}

// ── Media Browser ──────────────────────────────────────────────────────────

fn draw_media_browser(
    p: &mut Painter, t: &mut TextRenderer,
    r: &Rect, s: f32, sw: u32, sh: u32,
) {
    p.rect_filled(*r, 0.0, chrome::PANEL);

    let pad = PANEL_PAD * s;
    let x = r.x + pad;
    let y = r.y + pad;
    let w = r.w;
    let accent = chrome::accent();
    let text = chrome::text();
    let text_dim = chrome::text_dim();

    // Header
    t.queue("M E D I A", 16.0 * s, x, y, accent, w, sw, sh);

    // Separator
    p.rect_filled(
        Rect::new(x, y + 24.0 * s, r.w - pad * 2.0, 1.0 * s), 0.0, chrome::BORDER,
    );
}

// ── Properties / Inspector ─────────────────────────────────────────────────

fn draw_properties(
    p: &mut Painter, t: &mut TextRenderer,
    r: &Rect, s: f32, sw: u32, sh: u32,
) {
    p.rect_filled(*r, 0.0, chrome::PANEL);

    let pad = PANEL_PAD * s;
    let x = r.x + pad;
    let y = r.y + pad;
    let w = r.w;
    let accent = chrome::accent();
    let text = chrome::text();
    let text_dim = chrome::text_dim();

    // Header
    t.queue("I N S P E C T O R", 16.0 * s, x, y, accent, w, sw, sh);

    // Separator
    p.rect_filled(
        Rect::new(x, y + 24.0 * s, r.w - pad * 2.0, 1.0 * s), 0.0, chrome::BORDER,
    );

    // (empty — populated when a clip is selected)
}

// ── Timeline ───────────────────────────────────────────────────────────────

fn draw_timeline(
    p: &mut Painter, t: &mut TextRenderer,
    r: &Rect, s: f32, sw: u32, sh: u32,
) {
    p.rect_filled(*r, 0.0, chrome::BG);

    let pad = PANEL_PAD * s;
    let x = r.x;
    let y = r.y;
    let w = r.w;
    let text_dim = chrome::text_dim();
    let playhead_color = chrome::accent();

    // Time ruler header
    let header_h = 28.0 * s;
    p.rect_filled(Rect::new(x, y, w, header_h), 0.0, chrome::PANEL);
    p.rect_filled(Rect::new(x, y + header_h - 1.0 * s, w, 1.0 * s), 0.0, chrome::BORDER);

    let label_w = 60.0 * s;
    let ruler_x = x + label_w;
    let ruler_w = w - label_w;

    // Time markers
    let marks = ["0:00", "0:30", "1:00", "1:30", "2:00", "2:30", "3:00"];
    for (i, mark) in marks.iter().enumerate() {
        let mx = ruler_x + (i as f32 / (marks.len() - 1) as f32) * ruler_w;
        p.rect_filled(
            Rect::new(mx, y + header_h - 8.0 * s, 1.0 * s, 8.0 * s),
            0.0, text_dim,
        );
        t.queue(mark, 14.0 * s, mx + 3.0 * s, y + 6.0 * s, text_dim, w, sw, sh);
    }

    // Track lanes (empty — clips drawn from project state)
    let track_y = y + header_h;
    let track_h = 44.0 * s;
    let track_names = ["V1", "A1"];

    for (i, name) in track_names.iter().enumerate() {
        let ty = track_y + i as f32 * (track_h + 1.0 * s);
        let lane = Rect::new(x, ty, w, track_h);

        let bg = if i % 2 == 0 { chrome::PANEL_DARK } else { chrome::INPUT_BG };
        p.rect_filled(lane, 0.0, bg);

        // Track label
        let label_r = Rect::new(x, ty, label_w, track_h);
        p.rect_filled(label_r, 0.0, chrome::PANEL);
        p.rect_filled(
            Rect::new(x + label_w - 1.0 * s, ty, 1.0 * s, track_h), 0.0, chrome::BORDER,
        );
        t.queue(name, 16.0 * s, x + pad, ty + (track_h - 16.0 * s) * 0.5, text_dim, w, sw, sh);

        // Lane separator
        p.rect_filled(Rect::new(x, ty + track_h, w, 1.0 * s), 0.0, chrome::BORDER);
    }

    // Playhead
    let playhead_x = ruler_x;
    let playhead_bot = track_y + track_names.len() as f32 * (track_h + 1.0 * s);
    p.rect_filled(
        Rect::new(playhead_x - 0.5 * s, y, 2.0 * s, playhead_bot - y),
        0.0, playhead_color,
    );
    let diam = 10.0 * s;
    p.rect_filled(
        Rect::new(playhead_x - diam * 0.5, y, diam, diam * 0.7),
        2.0 * s, playhead_color,
    );
}

// ── Status Bar ─────────────────────────────────────────────────────────────

fn draw_status_bar(
    p: &mut Painter, t: &mut TextRenderer,
    r: &Rect, s: f32, sw: u32, sh: u32,
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

    t.queue("1920x1080  30fps", font, tx, text_y, text_dim, w, sw, sh);

    // Duration on right
    let dur = "00:00:00:00";
    let dur_w = font * 0.60 * dur.len() as f32;
    t.queue(dur, font, r.x + r.w - pad - dur_w, text_y, text_dim, w, sw, sh);
}
