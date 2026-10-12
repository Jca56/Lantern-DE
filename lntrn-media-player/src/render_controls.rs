//! The bottom controls overlay: scrim, seek bar with timestamps, the
//! prev / play-pause / next cluster, loop-mode and fullscreen toggles —
//! and the geometric icons they're drawn with.

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::{App, LoopMode};
use crate::{ZONE_FULLSCREEN, ZONE_LOOP, ZONE_NEXT, ZONE_PLAY_PAUSE, ZONE_PREV, ZONE_SEEK_BAR};

/// Controls always overlay the bottom of the `canvas`: a darkening scrim, a
/// full-width seek bar with timestamps, a centered prev/play/next cluster, and
/// a fullscreen toggle at the right. Returns the seek-bar track rect for drag
/// mapping in `session.rs`.
#[allow(clippy::too_many_arguments)]
pub fn draw_controls_overlay(
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    ctx: &lntrn_render::GpuContext,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    app: &App,
    s: f32,
    fade: f32,
    canvas: Rect,
    fullscreen: bool,
) -> Rect {
    // Cap how much the controls grow on big windows: full scale up to a
    // reference width, then clamped so a maximized 1440p window doesn't get
    // cartoonishly large buttons. Still respects DPI scale `s`.
    let ref_w = 1280.0 * s;
    let us = s * (canvas.w / ref_w).clamp(0.85, 1.6);

    // Overlay band pinned to the bottom of the canvas.
    let band_h = (140.0 * us).min(canvas.h * 0.45);
    let band = Rect::new(canvas.x, canvas.y + canvas.h - band_h, canvas.w, band_h);

    // Darkening scrim over video so the controls stay legible on any frame.
    // Stacked translucent slabs growing downward fake a top→bottom gradient
    // (darkest at the very bottom where the buttons sit).
    if app.video_mode() {
        let layers = 6;
        for i in 0..layers {
            // Slab i covers the bottom (i+1)/layers of the band; stacking them
            // makes the alpha accumulate toward the bottom.
            let frac = (i + 1) as f32 / layers as f32;
            let lh = band_h * frac;
            painter.rect_filled(
                Rect::new(band.x, band.y + band_h - lh, band.w, lh),
                0.0,
                Color::rgba(0.0, 0.0, 0.0, 0.12 * fade),
            );
        }
    }

    let icon_color = Color::rgba(1.0, 1.0, 1.0, fade);
    let accent = palette.accent.with_alpha(fade);
    let lantern_gold = Color::from_rgb8(250, 180, 0).with_alpha(fade);
    let muted = Color::rgba(1.0, 1.0, 1.0, 0.35 * fade);
    let time_font = FontSize::Custom((26.0 * us).clamp(20.0, 40.0));

    let pad_x = 32.0 * us;
    let seek_h = 12.0 * us;

    // ── Seek row (top of the band) ───────────────────────────────────
    let seek_cy = band.y + 38.0 * us;
    let cur_time = App::format_time(app.position_ns);
    let dur_str = App::format_time(app.duration_ns);
    let time_color = Color::rgba(1.0, 1.0, 1.0, 0.9 * fade);
    let ctw = text.measure_width(&cur_time, time_font.px());
    let dw = text.measure_width(&dur_str, time_font.px());
    let text_y = seek_cy - time_font.px() * 0.5;

    TextLabel::new(&cur_time, band.x + pad_x, text_y)
        .size(time_font)
        .color(time_color)
        .draw(text, ctx.width(), ctx.height());
    TextLabel::new(&dur_str, band.x + band.w - pad_x - dw, text_y)
        .size(time_font)
        .color(time_color)
        .draw(text, ctx.width(), ctx.height());

    let seek_gap = 16.0 * us;
    let seek_left = band.x + pad_x + ctw + seek_gap;
    let seek_right = band.x + band.w - pad_x - dw - seek_gap;
    let seek_w = (seek_right - seek_left).max(0.0);
    let seek_y = seek_cy - seek_h * 0.5;
    let hit_pad = 14.0 * us;
    let seek_hit = Rect::new(
        seek_left - hit_pad,
        seek_y - hit_pad,
        seek_w + hit_pad * 2.0,
        seek_h + hit_pad * 2.0,
    );
    let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_hit);
    let active = seek_state.is_hovered() || seek_state.is_active() || app.seeking;
    let seek_val = if app.seeking {
        app.seek_value
    } else {
        app.progress_fraction()
    };
    draw_seek_bar(
        painter,
        us,
        seek_left,
        seek_y,
        seek_w,
        seek_h,
        seek_val,
        active,
        lantern_gold,
        muted,
    );

    // ── Button row (below the seek row) ──────────────────────────────
    let btn_size = (60.0 * us).clamp(48.0, 84.0);
    let btn_cy = band.y + band_h - btn_size * 0.5 - 18.0 * us;
    let hover = |hovered: bool| if hovered { accent } else { icon_color };

    // Centered transport cluster: prev / play-pause / next.
    let gap = 40.0 * us;
    let cluster_w = btn_size * 3.0 + gap * 2.0;
    let cluster_x = band.x + (band.w - cluster_w) * 0.5;

    let prev_rect = Rect::new(cluster_x, btn_cy - btn_size * 0.5, btn_size, btn_size);
    let pp_rect = Rect::new(
        prev_rect.x + btn_size + gap,
        prev_rect.y,
        btn_size,
        btn_size,
    );
    let next_rect = Rect::new(pp_rect.x + btn_size + gap, pp_rect.y, btn_size, btn_size);

    let prev_state = input.add_zone(ZONE_PREV, prev_rect);
    let pp_state = input.add_zone(ZONE_PLAY_PAUSE, pp_rect);
    let next_state = input.add_zone(ZONE_NEXT, next_rect);

    draw_skip_icon(painter, prev_rect, hover(prev_state.is_hovered()), false);
    if app.is_playing() {
        draw_pause_icon(painter, pp_rect, hover(pp_state.is_hovered()));
    } else {
        draw_play_icon(painter, pp_rect, hover(pp_state.is_hovered()));
    }
    draw_skip_icon(painter, next_rect, hover(next_state.is_hovered()), true);

    // Loop / play-once / continue toggle, left-aligned.
    let loop_rect = Rect::new(band.x + pad_x, btn_cy - btn_size * 0.5, btn_size, btn_size);
    let loop_state = input.add_zone(ZONE_LOOP, loop_rect);
    // The active loop mode is shown in accent so it reads as "on"; hover also
    // highlights. Only the play-once mode is dimmed (it's the do-nothing mode).
    let loop_active = matches!(app.loop_mode, LoopMode::RepeatOne | LoopMode::Continue);
    let loop_color = if loop_state.is_hovered() {
        accent
    } else if loop_active {
        lantern_gold
    } else {
        Color::rgba(1.0, 1.0, 1.0, 0.45 * fade)
    };
    draw_loop_icon(painter, loop_rect, loop_color, app.loop_mode);

    // Fullscreen toggle, right-aligned.
    let fs_rect = Rect::new(
        band.x + band.w - pad_x - btn_size,
        btn_cy - btn_size * 0.5,
        btn_size,
        btn_size,
    );
    let fs_state = input.add_zone(ZONE_FULLSCREEN, fs_rect);
    draw_fullscreen_icon(painter, fs_rect, hover(fs_state.is_hovered()), fullscreen);

    Rect::new(seek_left, seek_y, seek_w, seek_h)
}

// ── Geometric white icons ────────────────────────────────────────────────

fn draw_play_icon(painter: &mut lntrn_render::Painter, rect: Rect, color: Color) {
    // Filled triangle pointing right, optically centered.
    let pad = rect.w * 0.18;
    let x_left = rect.x + pad + rect.w * 0.08;
    let x_right = rect.x + rect.w - pad;
    let y_top = rect.y + pad;
    let y_bot = rect.y + rect.h - pad;
    let y_mid = rect.y + rect.h * 0.5;
    painter.triangle(x_left, y_top, x_left, y_bot, x_right, y_mid, color);
}

fn draw_pause_icon(painter: &mut lntrn_render::Painter, rect: Rect, color: Color) {
    let pad = rect.w * 0.22;
    let bar_w = rect.w * 0.18;
    let inner_gap = rect.w * 0.14;
    let y = rect.y + pad;
    let h = rect.h - pad * 2.0;
    let cx = rect.x + rect.w * 0.5;
    painter.rect_filled(
        Rect::new(cx - inner_gap * 0.5 - bar_w, y, bar_w, h),
        1.5,
        color,
    );
    painter.rect_filled(Rect::new(cx + inner_gap * 0.5, y, bar_w, h), 1.5, color);
}

/// Skip-prev / skip-next: a triangle plus a thin vertical bar.
fn draw_skip_icon(painter: &mut lntrn_render::Painter, rect: Rect, color: Color, forward: bool) {
    let pad = rect.w * 0.22;
    let bar_w = rect.w * 0.10;
    let y_top = rect.y + pad;
    let y_bot = rect.y + rect.h - pad;
    let y_mid = rect.y + rect.h * 0.5;
    let inner_left = rect.x + pad;
    let inner_right = rect.x + rect.w - pad;

    if forward {
        // Triangle on the left, bar on the right
        painter.triangle(
            inner_left,
            y_top,
            inner_left,
            y_bot,
            inner_right - bar_w - 2.0,
            y_mid,
            color,
        );
        painter.rect_filled(
            Rect::new(inner_right - bar_w, y_top, bar_w, y_bot - y_top),
            1.0,
            color,
        );
    } else {
        painter.rect_filled(
            Rect::new(inner_left, y_top, bar_w, y_bot - y_top),
            1.0,
            color,
        );
        painter.triangle(
            inner_right,
            y_top,
            inner_right,
            y_bot,
            inner_left + bar_w + 2.0,
            y_mid,
            color,
        );
    }
}

/// Loop-mode toggle. Continue → next-track chevrons; RepeatOne → a circular
/// repeat ring with a dot; Once → an arrow into a stop bar ("play to end").
fn draw_loop_icon(painter: &mut lntrn_render::Painter, rect: Rect, color: Color, mode: LoopMode) {
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let r = rect.w * 0.30;
    let t = (rect.w * 0.07).max(1.5);

    match mode {
        LoopMode::RepeatOne | LoopMode::Continue => {
            // Open repeat ring (gap at the top-right) with an arrowhead.
            painter.arc(
                cx,
                cy,
                r + t,
                0.55,
                std::f32::consts::TAU - 1.1,
                t,
                r,
                color,
            );
            // Arrowhead at the ring's leading end (top), pointing clockwise.
            let ah = r * 0.55;
            let tipx = cx + r * 0.85;
            let tipy = cy - r * 0.55;
            painter.triangle(
                tipx,
                tipy - ah * 0.5,
                tipx,
                tipy + ah * 0.5,
                tipx + ah,
                tipy,
                color,
            );
            if matches!(mode, LoopMode::RepeatOne) {
                // Center dot = "this one track".
                painter.circle_filled(cx, cy, t * 1.1, color);
            }
        }
        LoopMode::Once => {
            // Right arrow into a stop bar: "play once, then stop".
            let bar_x = rect.x + rect.w * 0.74;
            let bar_top = cy - r;
            painter.rect_filled(Rect::new(bar_x, bar_top, t, r * 2.0), 0.0, color);
            // Shaft + arrowhead pointing at the bar.
            let shaft_x0 = rect.x + rect.w * 0.26;
            painter.rect_filled(
                Rect::new(shaft_x0, cy - t * 0.5, bar_x - shaft_x0 - r * 0.4, t),
                0.0,
                color,
            );
            let ah = r * 0.6;
            let tipx = bar_x - r * 0.1;
            painter.triangle(
                tipx - ah,
                cy - ah * 0.7,
                tipx - ah,
                cy + ah * 0.7,
                tipx,
                cy,
                color,
            );
        }
    }
}

/// Fullscreen toggle: four corner brackets (enter) or inward arrows (exit).
fn draw_fullscreen_icon(
    painter: &mut lntrn_render::Painter,
    rect: Rect,
    color: Color,
    fullscreen: bool,
) {
    let pad = rect.w * 0.28;
    let x0 = rect.x + pad;
    let y0 = rect.y + pad;
    let x1 = rect.x + rect.w - pad;
    let y1 = rect.y + rect.h - pad;
    let len = (x1 - x0) * 0.38;
    let t = (rect.w * 0.07).max(1.5);

    // Each corner is an L drawn from two thin rects. When entering fullscreen
    // the brackets open outward; when exiting they point inward (offset corners).
    let corners = if fullscreen {
        // Inward: brackets sit toward the center.
        [
            (x0 + len, y0, -1.0, 1.0),
            (x1 - len, y0, 1.0, 1.0),
            (x0 + len, y1, -1.0, -1.0),
            (x1 - len, y1, 1.0, -1.0),
        ]
    } else {
        [
            (x0, y0, 1.0, 1.0),
            (x1, y0, -1.0, 1.0),
            (x0, y1, 1.0, -1.0),
            (x1, y1, -1.0, -1.0),
        ]
    };
    for (cx, cy, dx, dy) in corners {
        let hx = if dx > 0.0 { cx } else { cx - len };
        painter.rect_filled(Rect::new(hx, cy - t * 0.5, len, t), 0.0, color);
        let vy = if dy > 0.0 { cy } else { cy - len };
        painter.rect_filled(Rect::new(cx - t * 0.5, vy, t, len), 0.0, color);
    }
}

// ── Seek bar ────────────────────────────────────────────────────────────

fn draw_seek_bar(
    painter: &mut lntrn_render::Painter,
    s: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    value: f32,
    active: bool,
    accent: Color,
    track: Color,
) {
    let track_h = if active { h + 2.0 * s } else { h };
    let track_y = y + (h - track_h) * 0.5;
    let corner = track_h * 0.5;
    painter.rect_filled(Rect::new(x, track_y, w, track_h), corner, track);

    if value > 0.001 {
        let fill_w = (w * value).max(track_h);
        painter.rect_filled(Rect::new(x, track_y, fill_w, track_h), corner, accent);
        let thumb_r = if active { 14.0 * s } else { 10.0 * s };
        painter.circle_filled(x + fill_w, y + h * 0.5, thumb_r, accent);
        // Tiny white outline so the thumb pops on any background.
        painter.circle_stroke(
            x + fill_w,
            y + h * 0.5,
            thumb_r,
            2.0 * s,
            Color::rgba(1.0, 1.0, 1.0, accent.a * 0.4),
        );
    }
}
