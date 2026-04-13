use lntrn_render::{Color, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::{App, LoopMode, VisMode, VIS_BARS};
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CONTROLS_BAR, ZONE_LOOP, ZONE_NEXT, ZONE_PLAY_PAUSE, ZONE_PREV,
    ZONE_SEEK_BAR, ZONE_VOLUME, ZONE_VOL_SLIDER,
};

pub struct ControlRects {
    pub seek: Rect,
    pub vol_slider: Rect,
    pub seek_horizontal: bool,
    pub controls_bar: Rect,
}

/// Render a frame and return control rects (for drag handling in wayland.rs).
pub fn render_frame(
    gpu: &mut Gpu,
    app: &App,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    scale: f32,
    _maximized: bool,
) -> ControlRects {
    let Gpu { ctx, painter, text, tex_pass } = gpu;
    let wf = ctx.width() as f32;
    let hf = ctx.height() as f32;
    let s = scale;

    painter.clear();
    input.begin_frame();

    // ── Canvas area (transparent background) ────────────────────────
    let canvas = Rect::new(0.0, 0.0, wf, hf);
    let _canvas_state = input.add_zone(ZONE_CANVAS, canvas);

    let mut tex_draws: Vec<TextureDraw> = Vec::new();

    if app.audio_only {
        match app.vis_mode {
            VisMode::ConcentricRings => draw_concentric_rings(painter, &app.vis_bars, canvas, s),
            VisMode::ClassicBars => draw_classic_bars(painter, &app.vis_bars, canvas, s),
        }
    } else if let Some(tex) = &app.video_texture {
        if app.video_width > 0 && app.video_height > 0 {
            let fit = aspect_fit(app.video_width, app.video_height, canvas);
            let mut draw = TextureDraw::new(tex, fit.x, fit.y, fit.w, fit.h);
            draw.clip = Some([canvas.x, canvas.y, canvas.w, canvas.h]);
            tex_draws.push(draw);
        }
    }

    // ── Controls ────────────────────────────────────────────────────
    let mut seek_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    let mut seek_horizontal = false;
    let mut vol_btn_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    let mut vol_slider_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    let mut controls_bar_rect = Rect::new(0.0, 0.0, 0.0, 0.0);

    if !app.audio_only && app.controls_visible {
        let font = FontSize::Body;
        let seek_val = if app.seeking { app.seek_value } else { app.progress_fraction() };
        let vol_str = format!("Vol {}%", (app.volume * 100.0).round() as u32);

        // ── Horizontal bottom bar ───────────────────────────────
        let bar_h = 52.0 * s;
        let bar_y = hf - bar_h;
        let bar_rect = Rect::new(0.0, bar_y, wf, bar_h);
        controls_bar_rect = bar_rect;
        input.add_zone(ZONE_CONTROLS_BAR, bar_rect);

        // Semi-transparent background
        painter.rect_filled(bar_rect, 0.0, Color::BLACK.with_alpha(0.65));

        let pad = 14.0 * s;
        let btn_size = 36.0 * s;
        let btn_y = bar_y + (bar_h - btn_size) * 0.5;
        let mut cx = pad;

        // Prev button
        let prev_rect = Rect::new(cx, btn_y, btn_size, btn_size);
        let prev_state = input.add_zone(ZONE_PREV, prev_rect);
        let prev_bg = if prev_state.is_hovered() { palette.surface_2.with_alpha(0.6) }
                      else { palette.surface_2.with_alpha(0.3) };
        painter.rect_filled(prev_rect, 6.0 * s, prev_bg);
        let prev_icon = "\u{23EE}";
        let pw = text.measure_width(prev_icon, font.px());
        TextLabel::new(prev_icon, prev_rect.x + (btn_size - pw) * 0.5, prev_rect.y + (btn_size - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
        cx += btn_size + 6.0 * s;

        // Play/Pause button
        let pp_rect = Rect::new(cx, btn_y, btn_size, btn_size);
        let pp_state = input.add_zone(ZONE_PLAY_PAUSE, pp_rect);
        let pp_bg = if pp_state.is_hovered() { palette.surface_2.with_alpha(0.6) }
                    else { palette.surface_2.with_alpha(0.3) };
        painter.rect_filled(pp_rect, 6.0 * s, pp_bg);
        let pp_icon = if app.is_playing() { "\u{23F8}" } else { "\u{25B6}" };
        let ppw = text.measure_width(pp_icon, font.px());
        TextLabel::new(pp_icon, pp_rect.x + (btn_size - ppw) * 0.5, pp_rect.y + (btn_size - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
        cx += btn_size + 6.0 * s;

        // Next button
        let next_rect = Rect::new(cx, btn_y, btn_size, btn_size);
        let next_state = input.add_zone(ZONE_NEXT, next_rect);
        let next_bg = if next_state.is_hovered() { palette.surface_2.with_alpha(0.6) }
                      else { palette.surface_2.with_alpha(0.3) };
        painter.rect_filled(next_rect, 6.0 * s, next_bg);
        let next_icon = "\u{23ED}";
        let nw = text.measure_width(next_icon, font.px());
        TextLabel::new(next_icon, next_rect.x + (btn_size - nw) * 0.5, next_rect.y + (btn_size - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
        cx += btn_size + 14.0 * s;

        // Current time
        let cur_time = App::format_time(app.position_ns);
        let ctw = text.measure_width(&cur_time, font.px());
        TextLabel::new(&cur_time, cx, bar_y + (bar_h - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
        cx += ctw + 12.0 * s;

        // -- Right side (work backwards from right edge) --
        // Volume button
        let vw = text.measure_width(&vol_str, font.px());
        let vol_x = wf - pad - vw;
        vol_btn_rect = Rect::new(vol_x - 6.0 * s, btn_y, vw + 12.0 * s, btn_size);
        let vol_state = input.add_zone(ZONE_VOLUME, vol_btn_rect);
        if vol_state.is_hovered() {
            painter.rect_filled(vol_btn_rect, 4.0 * s, palette.surface_2.with_alpha(0.3));
        }
        TextLabel::new(&vol_str, vol_x, bar_y + (bar_h - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());

        // Loop toggle (left of volume)
        let loop_label = app.loop_mode.label();
        let lw = text.measure_width(loop_label, font.px());
        let loop_x = vol_btn_rect.x - lw - 18.0 * s;
        let loop_rect = Rect::new(loop_x - 6.0 * s, btn_y, lw + 12.0 * s, btn_size);
        let loop_state = input.add_zone(ZONE_LOOP, loop_rect);
        let loop_color = match app.loop_mode {
            LoopMode::Off => palette.text_secondary.with_alpha(0.5),
            _ => palette.accent,
        };
        if loop_state.is_hovered() {
            painter.rect_filled(loop_rect, 4.0 * s, palette.surface_2.with_alpha(0.3));
        }
        TextLabel::new(loop_label, loop_x, bar_y + (bar_h - font.px()) * 0.5)
            .size(font).color(loop_color).draw(text, ctx.width(), ctx.height());

        // Duration time (left of loop)
        let dur_str = App::format_time(app.duration_ns);
        let dw = text.measure_width(&dur_str, font.px());
        let dur_x = loop_rect.x - dw - 14.0 * s;
        TextLabel::new(&dur_str, dur_x, bar_y + (bar_h - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());

        // Seek bar (fills space between current time and duration)
        let seek_x = cx;
        let seek_end = dur_x - 12.0 * s;
        let seek_w = (seek_end - seek_x).max(0.0);
        let seek_h = 6.0 * s;
        let seek_cy = bar_y + bar_h * 0.5;
        let seek_top = seek_cy - seek_h * 0.5;
        // Hit zone is taller for easy clicking
        seek_rect = Rect::new(seek_x, bar_y, seek_w, bar_h);
        seek_horizontal = true;
        let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_rect);
        let active = seek_state.is_hovered() || seek_state.is_active();

        draw_horizontal_seek(painter, s, seek_x, seek_top, seek_w, seek_h, seek_val, active);

        // Volume slider popup
        if app.vol_showing {
            let popup_w = 36.0 * s;
            let popup_h = 160.0 * s;
            let popup_x = vol_btn_rect.x + (vol_btn_rect.w - popup_w) * 0.5;
            let popup_y = vol_btn_rect.y - popup_h - 8.0 * s;
            let popup_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
            painter.rect_filled(popup_rect, 8.0 * s, palette.surface);
            painter.rect_filled(
                Rect::new(popup_rect.x + 1.0, popup_rect.y + 1.0, popup_rect.w - 2.0, popup_rect.h - 2.0),
                8.0 * s, palette.surface_2.with_alpha(0.3),
            );
            let track_margin = 10.0 * s;
            let track_x = popup_x + track_margin;
            let track_y = popup_y + track_margin;
            let track_w = popup_w - track_margin * 2.0;
            let track_h = popup_h - track_margin * 2.0;
            vol_slider_rect = Rect::new(track_x, track_y, track_w, track_h);
            let vs_state = input.add_zone(ZONE_VOL_SLIDER, vol_slider_rect);
            painter.rect_filled(
                Rect::new(track_x + track_w * 0.5 - 2.0 * s, track_y, 4.0 * s, track_h),
                2.0 * s, palette.surface_2.with_alpha(0.5),
            );
            let fill_h = track_h * app.volume as f32;
            let fill_y = track_y + track_h - fill_h;
            painter.rect_filled(
                Rect::new(track_x + track_w * 0.5 - 2.0 * s, fill_y, 4.0 * s, fill_h),
                2.0 * s, palette.accent,
            );
            let knob_r = 7.0 * s;
            let knob_color = if vs_state.is_hovered() || app.vol_dragging { palette.accent.lighten(0.2) } else { palette.accent };
            painter.circle_filled(track_x + track_w * 0.5, fill_y, knob_r, knob_color);
        }
    }

    // ── Multi-pass render ───────────────────────────────────────────
    let frame = ctx.begin_frame("Media Player");
    match frame {
        Ok(mut frame) => {
            painter.render_into(ctx, &mut frame, Color::rgba(0.0, 0.0, 0.0, 0.0));
            let view = frame.view().clone();
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_text(ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[media-player] render error: {e}"),
    }

    ControlRects {
        seek: seek_rect,
        vol_slider: vol_slider_rect,
        seek_horizontal,
        controls_bar: controls_bar_rect,
    }
}

// ── Horizontal seek bar ───────────────────────────────────────────────────

fn draw_horizontal_seek(
    painter: &mut lntrn_render::Painter,
    s: f32,
    x: f32, y: f32, w: f32, h: f32,
    value: f32, active: bool,
) {
    let blue = Color::from_rgb8(80, 120, 255);
    let purple = Color::from_rgb8(180, 60, 255);
    let track_color = Color::from_rgba8(255, 255, 255, 30);

    let track_h = if active { h + 2.0 * s } else { h };
    let track_y = y + (h - track_h) * 0.5;
    let corner = track_h * 0.5;
    painter.rect_filled(Rect::new(x, track_y, w, track_h), corner, track_color);

    if value > 0.001 {
        let fill_w = w * value;
        // Draw gradient as segments
        let segments = 32;
        let seg_w = fill_w / segments as f32;
        for i in 0..segments {
            let sx = x + seg_w * i as f32;
            let t = i as f32 / segments as f32;
            let r = blue.r + (purple.r - blue.r) * t;
            let g = blue.g + (purple.g - blue.g) * t;
            let b = blue.b + (purple.b - blue.b) * t;
            let seg_corner = if i == 0 { corner } else if i == segments - 1 { corner } else { 0.0 };
            painter.rect_filled(Rect::new(sx, track_y, seg_w + 0.5, track_h), seg_corner, Color::rgba(r, g, b, 1.0));
        }

        // Thumb
        let thumb_x = x + fill_w;
        let thumb_r = if active { 10.0 * s } else { 7.0 * s };
        let thumb_color = Color::rgba(
            blue.r + (purple.r - blue.r) * value,
            blue.g + (purple.g - blue.g) * value,
            blue.b + (purple.b - blue.b) * value,
            1.0,
        );
        painter.circle_filled(thumb_x, y + h * 0.5, thumb_r, thumb_color);
        painter.circle_stroke(thumb_x, y + h * 0.5, thumb_r, 1.5 * s, Color::BLACK.with_alpha(0.2));
    }
}

// ── Visualizer: Pulsing Concentric Rings ────────────────────────────────────

fn draw_concentric_rings(
    painter: &mut lntrn_render::Painter,
    bars: &[f32],
    canvas: Rect,
    s: f32,
) {
    let cx = canvas.x + canvas.w * 0.5;
    let cy = canvas.y + canvas.h * 0.5;
    let max_dim = canvas.w.min(canvas.h);

    let num_rings = 12;
    let bars_per_ring = VIS_BARS / num_rings;
    let min_radius = max_dim * 0.04;
    let max_radius = max_dim * 0.44;

    let bass_avg = bars.iter().take(4).sum::<f32>() / 4.0;
    let overall_energy: f32 = bars.iter().sum::<f32>() / bars.len() as f32;

    let center_r = min_radius * (0.8 + bass_avg * 1.2);
    let center_hue = 0.58 + bass_avg * 0.12;
    painter.circle_filled(cx, cy, center_r, hue_color(center_hue, 0.6, 0.25 + bass_avg * 0.2));

    if bass_avg > 0.35 {
        let glow_r = center_r * 1.8;
        let glow_a = (bass_avg - 0.35) * 0.25;
        painter.circle_filled(cx, cy, glow_r, hue_color(center_hue, 0.4, 0.7).with_alpha(glow_a));
    }

    for ring in 0..num_rings {
        let start = ring * bars_per_ring;
        let end = (start + bars_per_ring).min(VIS_BARS);
        let avg: f32 = bars[start..end].iter().sum::<f32>() / (end - start) as f32;

        let t = ring as f32 / (num_rings - 1) as f32;
        let base_radius = min_radius + (max_radius - min_radius) * t;
        let pulse = avg * max_dim * 0.06;
        let radius = base_radius + pulse;
        let base_thickness = 2.0 * s;
        let thickness = base_thickness + avg * 14.0 * s;

        let hue = 0.5 + t * 0.45;
        let saturation = 0.6 + avg * 0.35;
        let brightness = 0.6 + avg * 0.35;
        let alpha = 0.15 + avg * 0.8;
        let color = hue_color(hue, saturation, brightness).with_alpha(alpha);

        painter.circle_stroke(cx, cy, radius, thickness, color);

        if avg > 0.3 {
            let glow_alpha = (avg - 0.3) * 0.25;
            let glow_color = hue_color(hue, 0.4, 0.95).with_alpha(glow_alpha);
            painter.circle_stroke(cx, cy, radius, thickness + 10.0 * s, glow_color);
        }

        if avg > 0.7 {
            let flash_a = (avg - 0.7) * 0.5;
            painter.circle_stroke(cx, cy, radius, 1.5 * s, Color::from_rgba8(255, 255, 255, (flash_a * 255.0) as u8));
        }
    }

    let outer_r = max_radius + overall_energy * max_dim * 0.04;
    painter.circle_stroke(cx, cy, outer_r, 1.0 * s, Color::from_rgba8(255, 255, 255, 12));
}

// ── Visualizer: Classic Bars ────────────────────────────────────────────────

const PASTEL_COLORS: [(u8, u8, u8); 3] = [
    (170, 110, 250), // lavender purple
    (120, 220, 190), // mint green / teal
    (255, 140, 200), // soft pink
];

fn pastel_color(t: f32, boost: f32) -> Color {
    let t = t.clamp(0.0, 1.0) * (PASTEL_COLORS.len() - 1) as f32;
    let idx = (t as usize).min(PASTEL_COLORS.len() - 2);
    let next = idx + 1;
    let frac = t - t.floor();
    let (r0, g0, b0) = PASTEL_COLORS[idx];
    let (r1, g1, b1) = PASTEL_COLORS[next];
    let r = r0 as f32 + (r1 as f32 - r0 as f32) * frac;
    let g = g0 as f32 + (g1 as f32 - g0 as f32) * frac;
    let b = b0 as f32 + (b1 as f32 - b0 as f32) * frac;
    let bright = 1.0 + boost * 0.3;
    Color::from_rgba8(
        (r * bright).min(255.0) as u8,
        (g * bright).min(255.0) as u8,
        (b * bright).min(255.0) as u8,
        255,
    )
}

fn draw_classic_bars(
    painter: &mut lntrn_render::Painter,
    bars: &[f32],
    canvas: Rect,
    s: f32,
) {
    let num_bars = bars.len();
    let gap = 3.0 * s;
    let total_gap = gap * (num_bars - 1) as f32;
    let left_margin = 32.0 * s;
    let right_margin = 32.0 * s;
    let available_w = canvas.w - left_margin - right_margin;
    let bar_w = ((available_w - total_gap) / num_bars as f32).max(4.0 * s);
    let bottom_margin = 32.0 * s;
    let base_y = canvas.y + canvas.h - bottom_margin;
    let max_h = (canvas.h - bottom_margin) * 0.95;

    let border = 3.0 * s;

    for i in 0..num_bars {
        let raw = bars[i];
        let t = i as f32 / num_bars as f32;
        let magnitude = raw;

        let bar_h = (magnitude * max_h).max(3.0 * s);
        let x = canvas.x + left_margin + i as f32 * (bar_w + gap);
        let y = base_y - bar_h;

        let color = pastel_color(t, magnitude);

        // Black border behind
        painter.rect_filled(
            Rect::new(x - border, y - border, bar_w + border * 2.0, bar_h + border * 2.0),
            0.0, Color::BLACK,
        );
        painter.rect_filled(Rect::new(x, y, bar_w, bar_h), 0.0, color);

        if magnitude > 0.5 {
            let glow_a = (magnitude - 0.5) * 0.3;
            painter.rect_filled(
                Rect::new(x - 2.0 * s, y - 2.0 * s, bar_w + 4.0 * s, bar_h + 4.0 * s),
                0.0, color.with_alpha(glow_a),
            );
        }

        let cap_h = 3.0 * s;
        painter.rect_filled(Rect::new(x, y, bar_w, cap_h), 0.0, color.lighten(0.3).with_alpha(0.9));
    }
}

// ── Color helpers ───────────────────────────────────────────────────────────

fn hue_color(hue: f32, saturation: f32, value: f32) -> Color {
    let h = (hue % 1.0) * 6.0;
    let c = value * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = value - c;

    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    Color::from_rgba8(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
        255,
    )
}

// ── Layout helpers ──────────────────────────────────────────────────────────

fn aspect_fit(img_w: u32, img_h: u32, canvas: Rect) -> Rect {
    let scale_w = canvas.w / img_w as f32;
    let scale_h = canvas.h / img_h as f32;
    let scale = scale_w.min(scale_h);
    let w = img_w as f32 * scale;
    let h = img_h as f32 * scale;
    Rect::new(
        canvas.x + (canvas.w - w) * 0.5,
        canvas.y + (canvas.h - h) * 0.5,
        w,
        h,
    )
}
