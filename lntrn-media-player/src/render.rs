use lntrn_render::{Color, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::{App, VisMode, VIS_BARS};
use crate::{Gpu, ZONE_CANVAS, ZONE_PLAY_PAUSE, ZONE_SEEK_BAR, ZONE_VOLUME, ZONE_VOL_SLIDER};

pub struct ControlRects {
    pub seek: Rect,
    pub vol_slider: Rect,
    pub seek_vertical: bool,
    // Circular seek arc parameters (only used when !seek_vertical)
    pub seek_cx: f32,
    pub seek_cy: f32,
    pub seek_arc_start: f32,
    pub seek_arc_sweep: f32,
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

    let controls_h = 48.0 * s;

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

    // ── Seek bar + controls (mode-dependent) ──────────────────────
    let font = FontSize::Body;
    let seek_val = if app.seeking { app.seek_value } else { app.progress_fraction() };
    let time_str = format!("{} / {}", App::format_time(app.position_ns), App::format_time(app.duration_ns));
    let vol_str = format!("Vol {}%", (app.volume * 100.0).round() as u32);

    let mut seek_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    let mut seek_vertical = false;
    let mut arc_cx = 0.0_f32;
    let mut arc_cy = 0.0_f32;
    let mut arc_start = 0.0_f32;
    let mut arc_sweep = 0.0_f32;
    let mut vol_btn_rect = Rect::new(0.0, 0.0, 0.0, 0.0);

    let is_classic = app.audio_only && app.vis_mode == VisMode::ClassicBars;

    if is_classic {
        // ── Vertical seek bar on the left ───────────────────────
        // Must match draw_classic_bars layout: base_y, max_h, bottom_margin
        seek_vertical = true;
        let bottom_margin = 80.0 * s; // room for play/pause + time below
        let bars_base = hf - bottom_margin;
        let bars_max_h = (hf - bottom_margin) * 0.85;
        let bar_x = 16.0 * s;
        let bar_w = 10.0 * s;
        let bar_top = bars_base - bars_max_h;
        let bar_h = bars_max_h;
        seek_rect = Rect::new(bar_x - 8.0 * s, bar_top, bar_w + 16.0 * s, bar_h);
        let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_rect);
        let active = seek_state.is_hovered() || seek_state.is_active();
        draw_vertical_seek(painter, s, bar_x, bar_top, bar_w, bar_h, seek_val, active);

        // Play/pause + time below the seek bar
        let ctrl_x = bar_x + bar_w * 0.5;
        let below_y = bars_base + 8.0 * s;

        let pp_size = 36.0 * s;
        let pp_rect = Rect::new(ctrl_x - pp_size * 0.5, below_y, pp_size, pp_size);
        let pp_state = input.add_zone(ZONE_PLAY_PAUSE, pp_rect);
        let pp_bg = if pp_state.is_hovered() { palette.surface_2.with_alpha(0.6) }
                    else { palette.surface_2.with_alpha(0.3) };
        painter.rect_filled(pp_rect, 8.0 * s, pp_bg);
        let pp_icon = if app.is_playing() { "\u{23F8}" } else { "\u{25B6}" };
        let icon_w = text.measure_width(pp_icon, font.px());
        TextLabel::new(pp_icon, pp_rect.x + (pp_size - icon_w) * 0.5, pp_rect.y + (pp_size - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());

        let tw = text.measure_width(&time_str, font.px());
        TextLabel::new(&time_str, ctrl_x - tw * 0.5, below_y + pp_size + 4.0 * s)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());

        // Volume — bottom right, above the bars baseline
        let vw = text.measure_width(&vol_str, font.px());
        let vol_x = wf - vw - 20.0 * s;
        let vol_y = bars_base + 8.0 * s;
        vol_btn_rect = Rect::new(vol_x - 6.0 * s, vol_y - 8.0 * s, vw + 12.0 * s, 32.0 * s);
        let vol_state = input.add_zone(ZONE_VOLUME, vol_btn_rect);
        if vol_state.is_hovered() {
            painter.rect_filled(vol_btn_rect, 4.0 * s, palette.surface_2.with_alpha(0.3));
        }
        TextLabel::new(&vol_str, vol_x, vol_y)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
    } else {
        // ── Circular arc seek (for rings / video) ───────────────
        let vis_cx = canvas.x + canvas.w * 0.5;
        let vis_cy = canvas.y + canvas.h * 0.5;
        let max_dim = canvas.w.min(canvas.h);
        let seek_radius = max_dim * 0.52;

        let arc_half = 70.0_f32.to_radians();
        arc_start = std::f32::consts::FRAC_PI_2 - arc_half;
        arc_sweep = arc_half * 2.0;
        arc_cx = vis_cx;
        arc_cy = vis_cy;
        let arc_end = arc_start + arc_sweep;

        let left_x = vis_cx + seek_radius * arc_end.cos();
        let left_y = vis_cy + seek_radius * arc_end.sin();
        let right_x = vis_cx + seek_radius * arc_start.cos();
        let right_y = vis_cy + seek_radius * arc_start.sin();

        let vis_outer = max_dim * 0.50;
        let hit_pad = 12.0 * s;
        seek_rect = Rect::new(
            left_x - hit_pad, vis_cy + vis_outer,
            (right_x + hit_pad) - (left_x - hit_pad), vis_cy + seek_radius + hit_pad - (vis_cy + vis_outer),
        );
        let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_rect);
        let active = seek_state.is_hovered() || seek_state.is_active();
        draw_circular_seek(painter, palette, s, vis_cx, vis_cy, seek_radius, arc_start, arc_sweep, seek_val, active);

        let ctrl_offset = 14.0 * s;

        // Play/pause — left of arc
        let pp_size = 36.0 * s;
        let pp_rect = Rect::new(left_x - pp_size - ctrl_offset, left_y - pp_size * 0.5, pp_size, pp_size);
        let pp_state = input.add_zone(ZONE_PLAY_PAUSE, pp_rect);
        let pp_bg = if pp_state.is_hovered() { palette.surface_2.with_alpha(0.6) }
                    else { palette.surface_2.with_alpha(0.3) };
        painter.rect_filled(pp_rect, 8.0 * s, pp_bg);
        let pp_icon = if app.is_playing() { "\u{23F8}" } else { "\u{25B6}" };
        let icon_w = text.measure_width(pp_icon, font.px());
        TextLabel::new(pp_icon, pp_rect.x + (pp_size - icon_w) * 0.5, pp_rect.y + (pp_size - font.px()) * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());

        // Time — left of play/pause
        let tw = text.measure_width(&time_str, font.px());
        TextLabel::new(&time_str, pp_rect.x - tw - 10.0 * s, left_y - font.px() * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());

        // Volume — right of arc
        let vol_w = text.measure_width(&vol_str, font.px());
        let vol_x = right_x + ctrl_offset;
        vol_btn_rect = Rect::new(vol_x - 6.0 * s, right_y - controls_h * 0.5, vol_w + 12.0 * s, controls_h);
        let vol_state = input.add_zone(ZONE_VOLUME, vol_btn_rect);
        if vol_state.is_hovered() {
            painter.rect_filled(vol_btn_rect, 4.0 * s, palette.surface_2.with_alpha(0.3));
        }
        TextLabel::new(&vol_str, vol_x, right_y - font.px() * 0.5)
            .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
    }

    // ── Volume slider popup ─────────────────────────────────────────
    let mut vol_slider_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
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
        seek_vertical,
        seek_cx: arc_cx,
        seek_cy: arc_cy,
        seek_arc_start: arc_start,
        seek_arc_sweep: arc_sweep,
    }
}

// ── Circular seek arc ──────────────────────────────────────────────────────

fn draw_circular_seek(
    painter: &mut lntrn_render::Painter,
    palette: &FoxPalette,
    s: f32,
    cx: f32, cy: f32, radius: f32,
    start_angle: f32, sweep: f32,
    value: f32, active: bool,
) {
    // Track arc (dim background showing the full arc path)
    let track_w = 10.0 * s;
    let track_color = palette.text_secondary.with_alpha(0.15);
    painter.arc(
        cx, cy, radius + track_w * 0.5,
        start_angle, sweep,
        track_w, radius - track_w * 0.5,
        track_color,
    );
    // Rounded caps on the track
    let cap_r = track_w * 0.5;
    let trk_start_x = cx + radius * start_angle.cos();
    let trk_start_y = cy + radius * start_angle.sin();
    let trk_end_angle = start_angle + sweep;
    let trk_end_x = cx + radius * trk_end_angle.cos();
    let trk_end_y = cy + radius * trk_end_angle.sin();
    painter.circle_filled(trk_start_x, trk_start_y, cap_r, track_color);
    painter.circle_filled(trk_end_x, trk_end_y, cap_r, track_color);

    // Filled arc with blue→purple gradient, left-to-right
    // Blue at left (value=0 end), purple at right (value=1 end)
    let blue = Color::from_rgb8(80, 120, 255);
    let purple = Color::from_rgb8(180, 60, 255);

    if value > 0.001 {
        let fill_w = if active { 12.0 * s } else { 10.0 * s };
        let fill_amount = sweep * value;
        let fill_start = start_angle + sweep - fill_amount;

        // Draw as segments to create gradient effect
        let segments = 32;
        let seg_sweep = fill_amount / segments as f32;
        for i in 0..segments {
            let seg_start = fill_start + seg_sweep * i as f32;
            // t=0 at left end, t=1 at right end of the filled portion
            let t = i as f32 / segments as f32;
            // Arc goes right-to-left, but fill_start is at left, so invert t
            let t_color = 1.0 - t;
            let r = blue.r + (purple.r - blue.r) * t_color;
            let g = blue.g + (purple.g - blue.g) * t_color;
            let b = blue.b + (purple.b - blue.b) * t_color;
            let color = Color::rgba(r, g, b, 1.0);
            painter.arc(
                cx, cy, radius + fill_w * 0.5,
                seg_start, seg_sweep + 0.005, // tiny overlap to avoid gaps
                fill_w, radius - fill_w * 0.5,
                color,
            );
        }

        // Rounded caps on the fill
        let fill_cap_r = fill_w * 0.5;
        // Left cap (start of fill = left end of arc)
        let fill_left_x = cx + radius * fill_start.cos();
        let fill_left_y = cy + radius * fill_start.sin();
        painter.circle_filled(fill_left_x, fill_left_y, fill_cap_r, blue);
        // Right cap (end of fill = current position)
        let fill_end_angle = fill_start + fill_amount;
        let fill_right_x = cx + radius * fill_end_angle.cos();
        let fill_right_y = cy + radius * fill_end_angle.sin();
        let right_color = Color::rgba(
            blue.r + (purple.r - blue.r) * value,
            blue.g + (purple.g - blue.g) * value,
            blue.b + (purple.b - blue.b) * value,
            1.0,
        );
        painter.circle_filled(fill_right_x, fill_right_y, fill_cap_r, right_color);

        // Glow on filled portion when active
        if active {
            let glow_color = Color::rgba(
                (blue.r + purple.r) * 0.5,
                (blue.g + purple.g) * 0.5,
                (blue.b + purple.b) * 0.5,
                0.1,
            );
            painter.arc(
                cx, cy, radius + 16.0 * s,
                fill_start, fill_amount,
                0.0, radius - 16.0 * s,
                glow_color,
            );
        }
    }

    // Thumb circle — at the boundary between filled and unfilled
    let thumb_angle = start_angle + sweep * (1.0 - value);
    let thumb_x = cx + radius * thumb_angle.cos();
    let thumb_y = cy + radius * thumb_angle.sin();
    let thumb_r = if active { 10.0 * s } else { 8.0 * s };
    // Thumb color matches gradient at current position
    let thumb_color = Color::rgba(
        blue.r + (purple.r - blue.r) * value,
        blue.g + (purple.g - blue.g) * value,
        blue.b + (purple.b - blue.b) * value,
        1.0,
    );
    painter.circle_filled(thumb_x, thumb_y, thumb_r, thumb_color);
    painter.circle_stroke(thumb_x, thumb_y, thumb_r, 1.5 * s, Color::BLACK.with_alpha(0.15));

    if active {
        painter.circle_stroke(thumb_x, thumb_y, thumb_r + 4.0 * s, 2.0 * s, thumb_color.with_alpha(0.3));
    }
}


// ── Vertical seek bar ─────────────────────────────────────────────────────

fn draw_vertical_seek(
    painter: &mut lntrn_render::Painter,
    s: f32,
    x: f32, top: f32, w: f32, h: f32,
    value: f32, active: bool,
) {
    let blue = Color::from_rgb8(80, 120, 255);
    let purple = Color::from_rgb8(180, 60, 255);
    let track_color = Color::from_rgba8(255, 255, 255, 30);

    // Track (full height, rounded)
    let track_w = if active { 12.0 * s } else { 10.0 * s };
    let track_x = x + (w - track_w) * 0.5;
    let corner = track_w * 0.5;
    painter.rect_filled(Rect::new(track_x, top, track_w, h), corner, track_color);

    // Fill (bottom-to-top) with gradient
    if value > 0.001 {
        let fill_h = h * value;
        let fill_y = top + h - fill_h;

        // Draw gradient as segments
        let segments = 32;
        let seg_h = fill_h / segments as f32;
        for i in 0..segments {
            let sy = fill_y + seg_h * i as f32;
            let t = 1.0 - (i as f32 / segments as f32); // bottom=0 (blue), top=1 (purple)
            let r = blue.r + (purple.r - blue.r) * t;
            let g = blue.g + (purple.g - blue.g) * t;
            let b = blue.b + (purple.b - blue.b) * t;
            let seg_corner = if i == 0 { corner } else if i == segments - 1 { corner } else { 0.0 };
            painter.rect_filled(Rect::new(track_x, sy, track_w, seg_h + 0.5), seg_corner, Color::rgba(r, g, b, 1.0));
        }

        // Rounded caps
        let cap_r = track_w * 0.5;
        painter.circle_filled(x + w * 0.5, fill_y, cap_r, purple);
        let bot_color = Color::rgba(
            blue.r + (purple.r - blue.r) * value,
            blue.g + (purple.g - blue.g) * value,
            blue.b + (purple.b - blue.b) * value,
            1.0,
        );
        painter.circle_filled(x + w * 0.5, top + h, cap_r, bot_color);

        // Thumb at the top of the fill
        let thumb_r = if active { 10.0 * s } else { 8.0 * s };
        let thumb_color = Color::rgba(
            blue.r + (purple.r - blue.r) * value,
            blue.g + (purple.g - blue.g) * value,
            blue.b + (purple.b - blue.b) * value,
            1.0,
        );
        painter.circle_filled(x + w * 0.5, fill_y, thumb_r, thumb_color);
        painter.circle_stroke(x + w * 0.5, fill_y, thumb_r, 1.5 * s, Color::BLACK.with_alpha(0.15));
        if active {
            painter.circle_stroke(x + w * 0.5, fill_y, thumb_r + 4.0 * s, 2.0 * s, thumb_color.with_alpha(0.3));
        }
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

    // Bass energy
    let bass_avg = bars.iter().take(4).sum::<f32>() / 4.0;
    let overall_energy: f32 = bars.iter().sum::<f32>() / bars.len() as f32;

    // Breathing center — pulses with bass
    let center_r = min_radius * (0.8 + bass_avg * 1.2);
    let center_hue = 0.58 + bass_avg * 0.12;
    painter.circle_filled(cx, cy, center_r, hue_color(center_hue, 0.6, 0.25 + bass_avg * 0.2));

    // Center glow on strong bass
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

        // Ring pulses outward with magnitude — much more movement
        let pulse = avg * max_dim * 0.06;
        let radius = base_radius + pulse;

        // Thickness responds more dramatically
        let base_thickness = 2.0 * s;
        let thickness = base_thickness + avg * 14.0 * s;

        // Wider color range: warm magenta → cyan → blue → purple
        let hue = 0.5 + t * 0.45;
        let saturation = 0.6 + avg * 0.35;
        let brightness = 0.6 + avg * 0.35;
        let alpha = 0.15 + avg * 0.8;
        let color = hue_color(hue, saturation, brightness).with_alpha(alpha);

        painter.circle_stroke(cx, cy, radius, thickness, color);

        // Glow layer — triggers easier
        if avg > 0.3 {
            let glow_alpha = (avg - 0.3) * 0.25;
            let glow_color = hue_color(hue, 0.4, 0.95).with_alpha(glow_alpha);
            painter.circle_stroke(cx, cy, radius, thickness + 10.0 * s, glow_color);
        }

        // Bright flash ring on big hits
        if avg > 0.7 {
            let flash_a = (avg - 0.7) * 0.5;
            painter.circle_stroke(cx, cy, radius, 1.5 * s, Color::from_rgba8(255, 255, 255, (flash_a * 255.0) as u8));
        }
    }

    // Faint outer boundary that breathes
    let outer_r = max_radius + overall_energy * max_dim * 0.04;
    painter.circle_stroke(cx, cy, outer_r, 1.0 * s, Color::from_rgba8(255, 255, 255, 12));
}

// ── Visualizer: Classic Bars ────────────────────────────────────────────────

/// Pastel palette: purple → green → pink → blue (4 even stops)
const PASTEL_COLORS: [(u8, u8, u8); 4] = [
    (170, 110, 250), // lavender purple
    (120, 220, 190), // mint green / teal
    (255, 140, 200), // soft pink
    (120, 170, 255), // sky blue
];

fn pastel_color(t: f32, boost: f32) -> Color {
    let t = (t % 1.0) * PASTEL_COLORS.len() as f32;
    let idx = t as usize % PASTEL_COLORS.len();
    let next = (idx + 1) % PASTEL_COLORS.len();
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
    let gap = 4.0 * s;
    let total_gap = gap * (num_bars - 1) as f32;
    let left_margin = 80.0 * s;
    let right_margin = 32.0 * s;
    let available_w = canvas.w - left_margin - right_margin;
    let bar_w = ((available_w - total_gap) / num_bars as f32).max(4.0 * s);
    let bottom_margin = 80.0 * s;
    let base_y = canvas.y + canvas.h - bottom_margin;
    let max_h = (canvas.h - bottom_margin) * 0.85;

    for i in 0..num_bars {
        // Tame the bass: compress low bars so they don't always max out
        let raw = bars[i];
        let t = i as f32 / num_bars as f32;
        // Bass bars (low t) get compressed more
        let bass_compress = 1.0 - (1.0 - t).powi(2) * 0.5; // 0.5 at leftmost, 1.0 at right
        let magnitude = (raw * bass_compress).min(1.0);

        let bar_h = (magnitude * max_h).max(3.0 * s);
        let x = canvas.x + left_margin + i as f32 * (bar_w + gap);
        let y = base_y - bar_h;

        let color = pastel_color(t, magnitude);

        let corner = (bar_w * 0.4).min(6.0 * s);
        painter.rect_filled(Rect::new(x, y, bar_w, bar_h), corner, color);

        if magnitude > 0.5 {
            let glow_a = (magnitude - 0.5) * 0.3;
            painter.rect_filled(
                Rect::new(x - 2.0 * s, y - 2.0 * s, bar_w + 4.0 * s, bar_h + 4.0 * s),
                corner + 2.0 * s, color.with_alpha(glow_a),
            );
        }

        let cap_h = 3.0 * s;
        painter.rect_filled(Rect::new(x, y, bar_w, cap_h), corner, color.lighten(0.3).with_alpha(0.9));
    }
}

// ── Color helpers ───────────────────────────────────────────────────────────

/// Convert HSV (hue 0..1, saturation 0..1, value 0..1) to Color
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
