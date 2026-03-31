use lntrn_render::{Color, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, Slider, TextLabel, TitleBar};

use crate::app::{App, VisMode, VIS_BARS};
use crate::{Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_PLAY_PAUSE, ZONE_SEEK_BAR};

/// Render a frame and return the seek bar rect (for drag handling in wayland.rs).
pub fn render_frame(
    gpu: &mut Gpu,
    app: &App,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    scale: f32,
) -> Rect {
    let Gpu { ctx, painter, text, tex_pass } = gpu;
    let wf = ctx.width() as f32;
    let hf = ctx.height() as f32;
    let s = scale;

    painter.clear();
    input.begin_frame();

    let title_h = 36.0 * s;
    let controls_h = 64.0 * s;

    // ── Background ──────────────────────────────────────────────────
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, palette.bg);

    // ── Title bar ───────────────────────────────────────────────────
    let title_rect = Rect::new(0.0, 0.0, wf, title_h);
    let close_state = input.add_zone(ZONE_CLOSE, TitleBar::new(title_rect).scale(s).close_button_rect());
    let max_state = input.add_zone(ZONE_MAXIMIZE, TitleBar::new(title_rect).scale(s).maximize_button_rect());
    let min_state = input.add_zone(ZONE_MINIMIZE, TitleBar::new(title_rect).scale(s).minimize_button_rect());

    TitleBar::new(title_rect)
        .scale(s)
        .close_hovered(close_state.is_hovered())
        .maximize_hovered(max_state.is_hovered())
        .minimize_hovered(min_state.is_hovered())
        .draw(painter, palette);

    // ── Canvas area ─────────────────────────────────────────────────
    let canvas = Rect::new(0.0, title_h, wf, hf - title_h - controls_h);
    painter.rect_filled(canvas, 0.0, Color::from_rgb8(18, 18, 18));
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

    // ── Controls bar ────────────────────────────────────────────────
    let ctrl_rect = Rect::new(0.0, hf - controls_h, wf, controls_h);
    painter.rect_filled(ctrl_rect, 0.0, palette.surface);

    // Play/pause button
    let pp_size = 40.0 * s;
    let pp_x = 12.0 * s;
    let pp_y = ctrl_rect.y + (controls_h - pp_size) * 0.5;
    let pp_rect = Rect::new(pp_x, pp_y, pp_size, pp_size);
    let pp_state = input.add_zone(ZONE_PLAY_PAUSE, pp_rect);

    let pp_bg = if pp_state.is_hovered() {
        palette.surface_2.with_alpha(0.8)
    } else {
        palette.surface_2.with_alpha(0.4)
    };
    painter.rect_filled(pp_rect, 8.0 * s, pp_bg);

    let pp_icon = if app.is_playing() { "\u{23F8}" } else { "\u{25B6}" };
    let icon_size = FontSize::Body;
    let icon_w = text.measure_width(pp_icon, icon_size.px());
    TextLabel::new(
        pp_icon,
        pp_rect.x + (pp_size - icon_w) * 0.5,
        pp_rect.y + (pp_size - icon_size.px()) * 0.5,
    )
    .size(icon_size)
    .color(palette.text)
    .draw(text, ctx.width(), ctx.height());

    // Time label
    let time_str = format!(
        "{} / {}",
        App::format_time(app.position_ns),
        App::format_time(app.duration_ns),
    );
    let time_x = pp_x + pp_size + 12.0 * s;
    let time_y = ctrl_rect.y + (controls_h - FontSize::Small.px()) * 0.5;
    let time_w = text.measure_width(&time_str, FontSize::Small.px());
    TextLabel::new(&time_str, time_x, time_y)
        .size(FontSize::Small)
        .color(palette.text)
        .draw(text, ctx.width(), ctx.height());

    // Volume label (right side)
    let vol_str = format!("Vol {}%", (app.volume * 100.0).round() as u32);
    let vol_w = text.measure_width(&vol_str, FontSize::Small.px());
    let vol_x = wf - vol_w - 12.0 * s;
    TextLabel::new(&vol_str, vol_x, time_y)
        .size(FontSize::Small)
        .color(palette.text)
        .draw(text, ctx.width(), ctx.height());

    // Seek bar (between time and volume)
    let seek_x = time_x + time_w + 16.0 * s;
    let seek_w = vol_x - seek_x - 16.0 * s;
    let seek_h = 40.0 * s;
    let seek_y = ctrl_rect.y + (controls_h - seek_h) * 0.5;
    let seek_rect = Rect::new(seek_x, seek_y, seek_w.max(0.0), seek_h);

    let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_rect);
    let seek_val = if app.seeking { app.seek_value } else { app.progress_fraction() };
    Slider::new(seek_rect)
        .value(seek_val)
        .hovered(seek_state.is_hovered())
        .active(seek_state.is_active())
        .draw(painter, palette);

    // ── Multi-pass render ───────────────────────────────────────────
    let frame = ctx.begin_frame("Media Player");
    match frame {
        Ok(mut frame) => {
            painter.render_into(ctx, &mut frame, palette.bg.with_alpha(0.0));
            let view = frame.view().clone();
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_text(ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[media-player] render error: {e}"),
    }

    seek_rect
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
