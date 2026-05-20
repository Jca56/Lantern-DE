use lntrn_render::{Color, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{draw_window_bg, FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::{App, VIS_BARS};
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE,
    ZONE_NEXT, ZONE_PLAY_PAUSE, ZONE_PREV, ZONE_SEEK_BAR, ZONE_TITLE_BAR,
};

pub struct ControlRects {
    pub seek: Rect,
}

/// Render a frame.
///
/// - `window_h_phys`: bars-area height in physical pixels (the part the
///   compositor sees as the window via set_window_geometry).
/// - `strip_h_phys`: extra surface below the window for hover-revealed
///   controls. 0 when fullscreen/maximized (controls then overlay over bars).
pub fn render_frame(
    gpu: &mut Gpu,
    app: &App,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    opacity: f32,
    scale: f32,
    window_h_phys: f32,
    strip_h_phys: f32,
    _maximized: bool,
) -> ControlRects {
    let Gpu { ctx, painter, text, tex_pass } = gpu;
    let wf = ctx.width() as f32;
    let total_h = ctx.height() as f32;
    let win_h = window_h_phys;
    let strip_h = strip_h_phys;
    let s = scale;
    let corner_r = lntrn_theme::read_config_f32("window_manager", "corner_radius", 20.0) * s;

    painter.clear();
    input.begin_frame();

    let video_mode = !app.audio_only && app.pipeline.is_some();
    let title_h = if video_mode { 32.0 * s } else { 0.0 };

    // ── Window background (palette + opacity from lantern.toml) ───────
    let win_rect = Rect::new(0.0, 0.0, wf, win_h);
    draw_window_bg(painter, win_rect, corner_r, palette, opacity);

    // ── Canvas (bars area or video) ───────────────────────────────────
    let canvas = Rect::new(0.0, title_h, wf, (win_h - title_h).max(0.0));
    let _canvas_state = input.add_zone(ZONE_CANVAS, canvas);

    let mut tex_draws: Vec<TextureDraw> = Vec::new();
    if !video_mode {
        draw_classic_bars(painter, &app.vis_bars, canvas, s);
    } else if let Some(tex) = &app.video_texture {
        if app.video_width > 0 && app.video_height > 0 {
            let fit = aspect_fit(app.video_width, app.video_height, canvas);
            let mut draw = TextureDraw::new(tex, fit.x, fit.y, fit.w, fit.h);
            draw.clip = Some([canvas.x, canvas.y, canvas.w, canvas.h]);
            tex_draws.push(draw);
        }
    }

    // ── Title bar (video mode only) ───────────────────────────────────
    if video_mode {
        draw_title_bar(painter, text, ctx, input, palette, app, wf, title_h, s);
    }

    // ── Controls: strip below window (or overlay in fullscreen) ──────
    let fade = app.controls_alpha.clamp(0.0, 1.0);
    let (controls_rect, overlay_mode) = if strip_h > 0.0 {
        (Rect::new(0.0, win_h, wf, strip_h), false)
    } else {
        // Fullscreen/maximized: overlay over the bottom of the window
        let overlay_h = 72.0 * s;
        (Rect::new(0.0, win_h - overlay_h, wf, overlay_h), true)
    };

    let mut seek_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    if fade > 0.005 {
        seek_rect = draw_controls_strip(
            painter, text, ctx, input, palette, app, s, fade, controls_rect, overlay_mode,
        );
    }
    let _ = controls_rect;

    // ── Multi-pass render ───────────────────────────────────────────
    let frame = ctx.begin_frame("Media Player");
    match frame {
        Ok(mut frame) => {
            // Surface always cleared transparent — window pixels come from
            // draw_window_bg (with opacity), strip stays alpha-0 until controls
            // fade in.
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

    let _ = total_h;
    ControlRects { seek: seek_rect }
}

// ── Hover-reveal controls strip ───────────────────────────────────────────

fn draw_controls_strip(
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    ctx: &lntrn_render::GpuContext,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    app: &App,
    s: f32,
    fade: f32,
    rect: Rect,
    overlay_mode: bool,
) -> Rect {
    if overlay_mode {
        let bg = Color::rgba(0.0, 0.0, 0.0, 0.55 * fade);
        painter.rect_filled(rect, 0.0, bg);
    }

    let time_font = FontSize::Custom(26.0 * s);
    let icon_color = Color::rgba(1.0, 1.0, 1.0, fade);
    let accent = palette.accent.with_alpha(fade);
    let lantern_gold = Color::from_rgb8(250, 180, 0).with_alpha(fade);
    let muted = Color::rgba(1.0, 1.0, 1.0, 0.35 * fade);

    // Layout: icons row sits a bit below center, seek row near bottom of strip.
    let btn_size = 56.0 * s;
    let icons_top_pad = 18.0 * s;
    let icons_cy = rect.y + icons_top_pad + btn_size * 0.5;
    let seek_h = 12.0 * s;
    let seek_row_bottom_pad = 22.0 * s;
    let seek_cy = rect.y + rect.h - seek_row_bottom_pad - seek_h * 0.5;

    // ── Icons row ───────────────────────────────────────────────────
    let gap = 42.0 * s;
    let total_w = btn_size * 3.0 + gap * 2.0;
    let icons_x = rect.x + (rect.w - total_w) * 0.5;

    let prev_rect = Rect::new(icons_x, icons_cy - btn_size * 0.5, btn_size, btn_size);
    let pp_rect = Rect::new(prev_rect.x + btn_size + gap, prev_rect.y, btn_size, btn_size);
    let next_rect = Rect::new(pp_rect.x + btn_size + gap, pp_rect.y, btn_size, btn_size);

    let prev_state = input.add_zone(ZONE_PREV, prev_rect);
    let pp_state = input.add_zone(ZONE_PLAY_PAUSE, pp_rect);
    let next_state = input.add_zone(ZONE_NEXT, next_rect);

    let prev_color = if prev_state.is_hovered() { accent } else { icon_color };
    let pp_color = if pp_state.is_hovered() { accent } else { icon_color };
    let next_color = if next_state.is_hovered() { accent } else { icon_color };

    draw_skip_icon(painter, prev_rect, prev_color, false);
    if app.is_playing() {
        draw_pause_icon(painter, pp_rect, pp_color);
    } else {
        draw_play_icon(painter, pp_rect, pp_color);
    }
    draw_skip_icon(painter, next_rect, next_color, true);

    // ── Seek row ────────────────────────────────────────────────────
    let pad_x = 28.0 * s;
    let cur_time = App::format_time(app.position_ns);
    let dur_str = App::format_time(app.duration_ns);
    let time_color = Color::rgba(1.0, 1.0, 1.0, 0.85 * fade);
    let ctw = text.measure_width(&cur_time, time_font.px());
    let dw = text.measure_width(&dur_str, time_font.px());
    let text_y = seek_cy - time_font.px() * 0.5;

    TextLabel::new(&cur_time, rect.x + pad_x, text_y)
        .size(time_font).color(time_color).draw(text, ctx.width(), ctx.height());
    TextLabel::new(&dur_str, rect.x + rect.w - pad_x - dw, text_y)
        .size(time_font).color(time_color).draw(text, ctx.width(), ctx.height());

    let seek_gap = 18.0 * s;
    let seek_left = rect.x + pad_x + ctw + seek_gap;
    let seek_right = rect.x + rect.w - pad_x - dw - seek_gap;
    let seek_w = (seek_right - seek_left).max(0.0);
    let seek_y = seek_cy - seek_h * 0.5;
    // Generous vertical hit-zone for easy click + drag.
    let hit_pad = 14.0 * s;
    let seek_hit = Rect::new(seek_left - hit_pad, seek_y - hit_pad, seek_w + hit_pad * 2.0, seek_h + hit_pad * 2.0);
    let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_hit);
    let active = seek_state.is_hovered() || seek_state.is_active() || app.seeking;

    let seek_val = if app.seeking { app.seek_value } else { app.progress_fraction() };
    draw_seek_bar(painter, s, seek_left, seek_y, seek_w, seek_h, seek_val, active, lantern_gold, muted);

    // Visible-track rect (the bar itself) is what `wayland.rs` uses to map a
    // pointer x → fraction; return that so drag math uses the bar's left/width
    // instead of the inflated hit-zone.
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
    painter.rect_filled(Rect::new(cx - inner_gap * 0.5 - bar_w, y, bar_w, h), 1.5, color);
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
        painter.triangle(inner_left, y_top, inner_left, y_bot, inner_right - bar_w - 2.0, y_mid, color);
        painter.rect_filled(Rect::new(inner_right - bar_w, y_top, bar_w, y_bot - y_top), 1.0, color);
    } else {
        painter.rect_filled(Rect::new(inner_left, y_top, bar_w, y_bot - y_top), 1.0, color);
        painter.triangle(inner_right, y_top, inner_right, y_bot, inner_left + bar_w + 2.0, y_mid, color);
    }
}

// ── Seek bar ────────────────────────────────────────────────────────────

fn draw_seek_bar(
    painter: &mut lntrn_render::Painter,
    s: f32,
    x: f32, y: f32, w: f32, h: f32,
    value: f32, active: bool,
    accent: Color, track: Color,
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
        painter.circle_stroke(x + fill_w, y + h * 0.5, thumb_r, 2.0 * s, Color::rgba(1.0, 1.0, 1.0, accent.a * 0.4));
    }
}

// ── Title bar (video mode) ───────────────────────────────────────────────

fn draw_title_bar(
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    ctx: &lntrn_render::GpuContext,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    app: &App,
    wf: f32, title_h: f32, s: f32,
) {
    let font = FontSize::Caption;
    let title_rect = Rect::new(0.0, 0.0, wf, title_h);
    painter.rect_filled(title_rect, 0.0, Color::rgba(0.08, 0.08, 0.10, 1.0));

    let btn_w = title_h;
    let close_rect = Rect::new(wf - btn_w, 0.0, btn_w, title_h);
    let max_rect = Rect::new(wf - btn_w * 2.0, 0.0, btn_w, title_h);
    let min_rect = Rect::new(wf - btn_w * 3.0, 0.0, btn_w, title_h);

    let close_state = input.add_zone(ZONE_CLOSE, close_rect);
    let max_state = input.add_zone(ZONE_MAXIMIZE, max_rect);
    let min_state = input.add_zone(ZONE_MINIMIZE, min_rect);

    let title_drag_rect = Rect::new(0.0, 0.0, wf - btn_w * 3.0, title_h);
    input.add_zone(ZONE_TITLE_BAR, title_drag_rect);

    if min_state.is_hovered() {
        painter.rect_filled(min_rect, 0.0, palette.surface_2.with_alpha(0.4));
    }
    if max_state.is_hovered() {
        painter.rect_filled(max_rect, 0.0, palette.surface_2.with_alpha(0.4));
    }
    if close_state.is_hovered() {
        painter.rect_filled(close_rect, 0.0, Color::rgba(0.85, 0.2, 0.2, 0.9));
    }

    let icon_min = "\u{2013}";
    let icon_max = "\u{25A1}";
    let icon_close = "\u{2715}";
    let cy = (title_h - font.px()) * 0.5;
    let imw = text.measure_width(icon_min, font.px());
    TextLabel::new(icon_min, min_rect.x + (btn_w - imw) * 0.5, cy)
        .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
    let imxw = text.measure_width(icon_max, font.px());
    TextLabel::new(icon_max, max_rect.x + (btn_w - imxw) * 0.5, cy)
        .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());
    let icw = text.measure_width(icon_close, font.px());
    TextLabel::new(icon_close, close_rect.x + (btn_w - icw) * 0.5, cy)
        .size(font).color(palette.text).draw(text, ctx.width(), ctx.height());

    let title_text = if app.file_name.is_empty() {
        "Lantern Media Player".to_string()
    } else {
        app.file_name.clone()
    };
    let max_title_w = (title_drag_rect.w - 16.0 * s).max(0.0);
    let mut shown = title_text.clone();
    while text.measure_width(&shown, font.px()) > max_title_w && shown.len() > 1 {
        shown.pop();
    }
    if shown.len() < title_text.len() && shown.len() > 1 {
        shown.pop();
        shown.push('\u{2026}');
    }
    TextLabel::new(&shown, 8.0 * s, cy)
        .size(font).color(palette.text_secondary).draw(text, ctx.width(), ctx.height());
}

// ── Visualizer: Classic Bars (flush to bottom of canvas) ─────────────────

const PASTEL_COLORS: [(u8, u8, u8); 3] = [
    (170, 110, 250),
    (120, 220, 190),
    (255, 140, 200),
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
    let side_margin = 24.0 * s;
    let available_w = canvas.w - side_margin * 2.0;
    let bar_w = ((available_w - total_gap) / num_bars as f32).max(4.0 * s);
    // Bars sit flush against the bottom of the canvas (= bottom of window geometry).
    let base_y = canvas.y + canvas.h;
    let max_h = canvas.h * 0.95;
    let border = 3.0 * s;

    for i in 0..num_bars {
        let raw = bars[i];
        let t = i as f32 / num_bars as f32;
        let magnitude = raw;

        let bar_h = (magnitude * max_h).max(3.0 * s);
        let x = canvas.x + side_margin + i as f32 * (bar_w + gap);
        let y = base_y - bar_h;

        let color = pastel_color(t, magnitude);

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
    let _ = VIS_BARS;
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
