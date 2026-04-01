use lntrn_render::{Color, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::{App, VisMode, VIS_BARS};
use crate::{Gpu, ZONE_CANVAS, ZONE_PLAY_PAUSE, ZONE_SEEK_BAR, ZONE_VOLUME, ZONE_VOL_SLIDER};

pub struct ControlRects {
    pub seek: Rect,
    pub vol_slider: Rect,
    // Circular seek arc parameters for angle-based hit testing
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
            VisMode::RadialBars => draw_radial_bars(painter, &app.vis_bars, canvas, s),
            VisMode::ConcentricRings => draw_concentric_rings(painter, &app.vis_bars, canvas, s),
        }
    } else if let Some(tex) = &app.video_texture {
        if app.video_width > 0 && app.video_height > 0 {
            let fit = aspect_fit(app.video_width, app.video_height, canvas);
            let mut draw = TextureDraw::new(tex, fit.x, fit.y, fit.w, fit.h);
            draw.clip = Some([canvas.x, canvas.y, canvas.w, canvas.h]);
            tex_draws.push(draw);
        }
    }

    // ── Circular seek arc (follows visualizer curvature) ─────────
    let vis_cx = canvas.x + canvas.w * 0.5;
    let vis_cy = canvas.y + canvas.h * 0.5;
    let max_dim = canvas.w.min(canvas.h);
    let seek_radius = max_dim * 0.52;

    let arc_half_angle = 70.0_f32.to_radians();
    let arc_start = std::f32::consts::FRAC_PI_2 - arc_half_angle; // bottom-right
    let arc_sweep = arc_half_angle * 2.0; // to bottom-left
    let arc_end = arc_start + arc_sweep;

    // Arc endpoint positions (for placing controls)
    // Left end = end of arc (bottom-left), Right end = start of arc (bottom-right)
    let left_x = vis_cx + seek_radius * arc_end.cos();
    let left_y = vis_cy + seek_radius * arc_end.sin();
    let right_x = vis_cx + seek_radius * arc_start.cos();
    let right_y = vis_cy + seek_radius * arc_start.sin();

    // Hit zone: thin crescent that follows the arc shape
    // Only covers from the arc's endpoint height down to the bottom of the arc
    let hit_pad = 12.0 * s;
    let endpoint_y = vis_cy + seek_radius * arc_start.sin(); // Y of endpoints (same for both sides)
    let arc_top_y = endpoint_y - hit_pad;
    let arc_bot_y = vis_cy + seek_radius + hit_pad;
    // X range: only as wide as the endpoint positions + padding
    let arc_left_x = left_x - hit_pad;
    let arc_right_x = right_x + hit_pad;
    let seek_rect = Rect::new(arc_left_x, arc_top_y, arc_right_x - arc_left_x, arc_bot_y - arc_top_y);
    let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_rect);
    let seek_val = if app.seeking { app.seek_value } else { app.progress_fraction() };

    draw_circular_seek(
        painter, palette, s,
        vis_cx, vis_cy, seek_radius,
        arc_start, arc_sweep, seek_val,
        seek_state.is_hovered() || seek_state.is_active(),
    );

    // ── Controls at arc endpoints ───────────────────────────────────
    let font = FontSize::Body;
    let ctrl_offset = 14.0 * s;

    // Play/pause button — left of the left arc endpoint
    let pp_size = 36.0 * s;
    let pp_x = left_x - pp_size - ctrl_offset;
    let pp_y = left_y - pp_size * 0.5;
    let pp_rect = Rect::new(pp_x, pp_y, pp_size, pp_size);
    let pp_state = input.add_zone(ZONE_PLAY_PAUSE, pp_rect);

    let pp_bg = if pp_state.is_hovered() {
        palette.surface_2.with_alpha(0.6)
    } else {
        palette.surface_2.with_alpha(0.3)
    };
    painter.rect_filled(pp_rect, 8.0 * s, pp_bg);

    let pp_icon = if app.is_playing() { "\u{23F8}" } else { "\u{25B6}" };
    let icon_w = text.measure_width(pp_icon, font.px());
    TextLabel::new(
        pp_icon,
        pp_rect.x + (pp_size - icon_w) * 0.5,
        pp_rect.y + (pp_size - font.px()) * 0.5,
    )
    .size(font)
    .color(palette.text)
    .draw(text, ctx.width(), ctx.height());

    // Time label — just right of play/pause, still left of arc
    let time_str = format!(
        "{} / {}",
        App::format_time(app.position_ns),
        App::format_time(app.duration_ns),
    );
    let time_x = pp_x - text.measure_width(&time_str, font.px()) - 10.0 * s;
    let time_y = left_y - font.px() * 0.5;
    TextLabel::new(&time_str, time_x, time_y)
        .size(font)
        .color(palette.text)
        .draw(text, ctx.width(), ctx.height());

    // Volume button — right of the right arc endpoint
    let vol_str = format!("Vol {}%", (app.volume * 100.0).round() as u32);
    let vol_w = text.measure_width(&vol_str, font.px());
    let vol_x = right_x + ctrl_offset;
    let vol_y = right_y - font.px() * 0.5;
    let vol_btn_rect = Rect::new(vol_x - 6.0 * s, right_y - controls_h * 0.5, vol_w + 12.0 * s, controls_h);
    let vol_state = input.add_zone(ZONE_VOLUME, vol_btn_rect);

    if vol_state.is_hovered() {
        painter.rect_filled(vol_btn_rect, 4.0 * s, palette.surface_2.with_alpha(0.3));
    }
    TextLabel::new(&vol_str, vol_x, vol_y)
        .size(font)
        .color(palette.text)
        .draw(text, ctx.width(), ctx.height());

    // ── Volume slider popup ─────────────────────────────────────────
    let mut vol_slider_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    if app.vol_showing {
        let popup_w = 36.0 * s;
        let popup_h = 160.0 * s;
        let popup_x = vol_btn_rect.x + (vol_btn_rect.w - popup_w) * 0.5;
        let popup_y = vol_btn_rect.y - popup_h - 8.0 * s;
        let popup_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);

        // Background
        painter.rect_filled(popup_rect, 8.0 * s, palette.surface);
        painter.rect_filled(
            Rect::new(popup_rect.x + 1.0, popup_rect.y + 1.0, popup_rect.w - 2.0, popup_rect.h - 2.0),
            8.0 * s,
            palette.surface_2.with_alpha(0.3),
        );

        // Slider track area (vertical)
        let track_margin = 10.0 * s;
        let track_x = popup_x + track_margin;
        let track_y = popup_y + track_margin;
        let track_w = popup_w - track_margin * 2.0;
        let track_h = popup_h - track_margin * 2.0;
        vol_slider_rect = Rect::new(track_x, track_y, track_w, track_h);

        let vs_state = input.add_zone(ZONE_VOL_SLIDER, vol_slider_rect);

        // Track background
        painter.rect_filled(
            Rect::new(track_x + track_w * 0.5 - 2.0 * s, track_y, 4.0 * s, track_h),
            2.0 * s,
            palette.surface_2.with_alpha(0.5),
        );

        // Fill (from bottom up)
        let fill_h = track_h * app.volume as f32;
        let fill_y = track_y + track_h - fill_h;
        painter.rect_filled(
            Rect::new(track_x + track_w * 0.5 - 2.0 * s, fill_y, 4.0 * s, fill_h),
            2.0 * s,
            palette.accent,
        );

        // Knob
        let knob_r = 7.0 * s;
        let knob_y = fill_y;
        let knob_color = if vs_state.is_hovered() || app.vol_dragging {
            palette.accent.lighten(0.2)
        } else {
            palette.accent
        };
        painter.circle_filled(track_x + track_w * 0.5, knob_y, knob_r, knob_color);
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
        seek_cx: vis_cx,
        seek_cy: vis_cy,
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

// ── Visualizer: Mirrored Radial Bars ────────────────────────────────────────

fn draw_radial_bars(
    painter: &mut lntrn_render::Painter,
    bars: &[f32],
    canvas: Rect,
    s: f32,
) {
    let cx = canvas.x + canvas.w * 0.5;
    let cy = canvas.y + canvas.h * 0.5;
    let max_dim = canvas.w.min(canvas.h);

    // Bass energy drives the center circle size
    let bass_avg = bars.iter().take(4).sum::<f32>() / 4.0;

    let ring_radius = max_dim * 0.13;
    let max_bar_length = max_dim * 0.30;
    let bar_gap_angle = 0.012;

    let num_bars = bars.len();
    let bar_sweep = (std::f32::consts::TAU / num_bars as f32) - bar_gap_angle;

    // Breathing center circle — pulses with bass
    let center_r = ring_radius - 2.0 * s + bass_avg * max_dim * 0.03;
    let center_color = hue_color(0.75, 0.4, 0.15 + bass_avg * 0.15);
    painter.circle_filled(cx, cy, center_r, center_color);
    painter.circle_stroke(cx, cy, ring_radius, 2.0 * s, Color::from_rgba8(255, 255, 255, 20));

    // Bass glow behind everything
    if bass_avg > 0.3 {
        let glow_a = (bass_avg - 0.3) * 0.15;
        painter.circle_filled(cx, cy, ring_radius + max_dim * 0.05, hue_color(0.75, 0.5, 0.6).with_alpha(glow_a));
    }

    for i in 0..num_bars {
        let magnitude = bars[i];
        if magnitude < 0.02 {
            continue;
        }

        let t = i as f32 / num_bars as f32;
        let angle = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let bar_length = magnitude * max_bar_length;

        // Rainbow color — shifts with magnitude for extra life
        let color = hue_color(t + magnitude * 0.1, 0.85, 0.7 + magnitude * 0.3);

        // Outward bar
        painter.arc(
            cx, cy,
            ring_radius + bar_length,
            angle, bar_sweep,
            0.0,
            ring_radius + 2.0 * s,
            color,
        );

        // Outward glow on strong bars
        if magnitude > 0.5 {
            let glow_len = bar_length * 1.15;
            let glow_color = color.with_alpha((magnitude - 0.5) * 0.4);
            painter.arc(
                cx, cy,
                ring_radius + glow_len,
                angle, bar_sweep,
                0.0,
                ring_radius + bar_length,
                glow_color,
            );
        }

        // Inward bar (shorter, more transparent)
        let inner_length = magnitude * max_bar_length * 0.5;
        let inner_color = color.with_alpha(0.45);
        if ring_radius > inner_length + 4.0 * s {
            painter.arc(
                cx, cy,
                ring_radius - 2.0 * s,
                angle, bar_sweep,
                0.0,
                ring_radius - inner_length,
                inner_color,
            );
        }
    }

    // Outer accent ring
    painter.circle_stroke(cx, cy, ring_radius + 1.0 * s, 1.0 * s, Color::from_rgba8(255, 255, 255, 20));
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
