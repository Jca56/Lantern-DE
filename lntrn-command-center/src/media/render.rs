//! Now-playing controls, rendered as a floating card in the below-bar
//! band. Only shown while the bar is collapsed AND something is playing.
//! Position comes from the outer-zone layout ([`crate::outer_zones`]) —
//! the card rect is handed in rather than computed from the panel.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{Media, MediaButton, PlaybackStatus};

/// Card size (logical px) — the outer zone layout sizes this widget's
/// slot from these.
pub const CARD_W: f32 = 480.0;
pub const CARD_H: f32 = 62.0;
const CARD_PAD: f32 = 18.0;
const RADIUS: f32 = 16.0;

const BTN: f32 = 32.0;
const BTN_GAP: f32 = 8.0;
const TEXT_GAP: f32 = 14.0;
const TITLE_FONT: f32 = 18.0;
const ARTIST_FONT: f32 = 14.0;
const LINE_GAP: f32 = 2.0;
const PROGRESS_H: f32 = 5.0;
const TEXT_PROGRESS_GAP: f32 = 6.0;

const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const CARD_RGB: (u8, u8, u8) = (24, 24, 24);

/// True when the floating card should be present: a track exists to show.
pub fn is_active(media: &Media) -> bool {
    media.track.is_some()
}

struct Geom {
    prev: Rect,
    play: Rect,
    next: Rect,
    text_x: f32,
    text_w: f32,
    title_y: f32,
    artist_y: f32,
    progress: Rect,
}

/// Lay out the card's parts inside its padded content area.
fn geom(card: Rect, scale: f32) -> Geom {
    let pad = CARD_PAD * scale;
    let x = card.x + pad;
    let y = card.y;
    let w = card.w - pad * 2.0;
    let h = card.h;

    let btn = BTN * scale;
    let gap = BTN_GAP * scale;
    let btn_y = y + (h - btn) / 2.0;
    let prev = Rect::new(x, btn_y, btn, btn);
    let play = Rect::new(x + btn + gap, btn_y, btn, btn);
    let next = Rect::new(x + 2.0 * (btn + gap), btn_y, btn, btn);

    let text_x = next.x + btn + TEXT_GAP * scale;
    let text_w = (x + w - text_x).max(1.0);

    let title_font = TITLE_FONT * scale;
    let artist_font = ARTIST_FONT * scale;
    let line_gap = LINE_GAP * scale;
    let progress_h = PROGRESS_H * scale;
    let tp_gap = TEXT_PROGRESS_GAP * scale;
    let group_h = title_font + line_gap + artist_font + tp_gap + progress_h;
    let top = y + (h - group_h) / 2.0;
    let title_y = top;
    let artist_y = top + title_font + line_gap;
    let progress = Rect::new(text_x, artist_y + artist_font + tp_gap, text_w, progress_h);

    Geom { prev, play, next, text_x, text_w, title_y, artist_y, progress }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_floating(
    painter: &mut Painter,
    text: &mut TextRenderer,
    media: &Media,
    card: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    if alpha < 0.01 {
        return;
    }
    let Some(track) = &media.track else {
        return;
    };

    // Floating plate so it reads as a control over the desktop.
    painter.shadow(card, RADIUS * scale, 18.0 * scale, Color::BLACK.with_alpha(0.35 * alpha), 0.0, 4.0 * scale);
    painter.rect_filled(card, RADIUS * scale, Color::from_rgb8(CARD_RGB.0, CARD_RGB.1, CARD_RGB.2).with_alpha(0.78 * alpha));
    painter.rect_stroke_sdf(card, RADIUS * scale, 1.0 * scale, Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha));

    let g = geom(card, scale);
    let icon = Color::rgba(1.0, 1.0, 1.0, 0.92 * alpha);
    let hov = media.hover;

    button_bg(painter, g.prev, alpha, hov == Some(MediaButton::Previous));
    draw_prev(painter, g.prev, icon);
    button_bg(painter, g.play, alpha, hov == Some(MediaButton::PlayPause));
    if matches!(track.status, PlaybackStatus::Playing) {
        draw_pause(painter, g.play, icon);
    } else {
        draw_play(painter, g.play, icon);
    }
    button_bg(painter, g.next, alpha, hov == Some(MediaButton::Next));
    draw_next(painter, g.next, icon);

    let title_font = TITLE_FONT * scale;
    let artist_font = ARTIST_FONT * scale;
    let white = Color::rgba(1.0, 1.0, 1.0, alpha);
    let secondary = Color::rgba(1.0, 1.0, 1.0, 0.6 * alpha);
    let title = truncate(text, &track.title, title_font, g.text_w);
    text.queue(&title, title_font, g.text_x, g.title_y, white, g.text_w, surface_w, surface_h);
    if !track.artist.is_empty() {
        let artist = truncate(text, &track.artist, artist_font, g.text_w);
        text.queue(&artist, artist_font, g.text_x, g.artist_y, secondary, g.text_w, surface_w, surface_h);
    }

    let p = g.progress;
    painter.rect_filled(p, p.h * 0.5, Color::rgba(1.0, 1.0, 1.0, 0.14 * alpha));
    if track.length_us > 0 {
        let frac = (media.effective_position_us() as f32 / track.length_us as f32).clamp(0.0, 1.0);
        let fill_w = (p.w * frac).max(p.h);
        let accent = Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha);
        painter.rect_filled(Rect::new(p.x, p.y, fill_w, p.h), p.h * 0.5, accent);
    }
}

/// Which transport button (if any) is under the cursor, given the card rect.
pub fn floating_button_hit(card: Rect, scale: f32, px: f32, py: f32) -> Option<MediaButton> {
    let g = geom(card, scale);
    let hit = |r: &Rect| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h;
    if hit(&g.prev) {
        Some(MediaButton::Previous)
    } else if hit(&g.play) {
        Some(MediaButton::PlayPause)
    } else if hit(&g.next) {
        Some(MediaButton::Next)
    } else {
        None
    }
}

// ─── button glyphs ───────────────────────────────────────────────────────────

fn button_bg(painter: &mut Painter, r: Rect, alpha: f32, hovered: bool) {
    // Brighter, slightly larger disc on hover so the button lights up
    // under the cursor.
    let (fill_a, grow) = if hovered { (0.22, 1.06) } else { (0.08, 1.0) };
    painter.circle_filled(
        r.x + r.w / 2.0,
        r.y + r.h / 2.0,
        r.w / 2.0 * grow,
        Color::rgba(1.0, 1.0, 1.0, fill_a * alpha),
    );
}

fn draw_play(painter: &mut Painter, r: Rect, col: Color) {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let s = r.h * 0.24;
    let pts = [(cx - s * 0.75, cy - s), (cx - s * 0.75, cy + s), (cx + s, cy)];
    painter.polygon(&pts, col);
}

fn draw_pause(painter: &mut Painter, r: Rect, col: Color) {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let bw = r.w * 0.12;
    let bh = r.h * 0.44;
    let gap = r.w * 0.14;
    painter.rect_filled(Rect::new(cx - gap / 2.0 - bw, cy - bh / 2.0, bw, bh), bw * 0.3, col);
    painter.rect_filled(Rect::new(cx + gap / 2.0, cy - bh / 2.0, bw, bh), bw * 0.3, col);
}

fn draw_next(painter: &mut Painter, r: Rect, col: Color) {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let s = r.h * 0.22;
    let pts = [(cx - s * 1.1, cy - s), (cx - s * 1.1, cy + s), (cx + s * 0.3, cy)];
    painter.polygon(&pts, col);
    let bw = r.w * 0.10;
    painter.rect_filled(Rect::new(cx + s * 0.45, cy - s, bw, s * 2.0), bw * 0.3, col);
}

fn draw_prev(painter: &mut Painter, r: Rect, col: Color) {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let s = r.h * 0.22;
    let pts = [(cx + s * 1.1, cy - s), (cx + s * 1.1, cy + s), (cx - s * 0.3, cy)];
    painter.polygon(&pts, col);
    let bw = r.w * 0.10;
    painter.rect_filled(Rect::new(cx - s * 0.45 - bw, cy - s, bw, s * 2.0), bw * 0.3, col);
}

/// Truncate `s` with an ellipsis so it fits `max_w` at `font` px.
fn truncate(text: &mut TextRenderer, s: &str, font: f32, max_w: f32) -> String {
    if text.measure_width(s, font) <= max_w {
        return s.to_string();
    }
    let ell = "…";
    let mut end = s.len();
    while end > 0 {
        if !s.is_char_boundary(end) {
            end -= 1;
            continue;
        }
        let trial = format!("{}{}", &s[..end], ell);
        if text.measure_width(&trial, font) <= max_w {
            return trial;
        }
        end -= 1;
    }
    ell.to_string()
}
