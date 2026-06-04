use lntrn_render::{Color, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{draw_window_bg, FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::{App, LoopMode, VIS_BARS};
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_FULLSCREEN, ZONE_LOOP, ZONE_MAXIMIZE, ZONE_MINIMIZE,
    ZONE_NEXT, ZONE_PLAY_PAUSE, ZONE_PREV, ZONE_SEEK_BAR, ZONE_TITLE_BAR, ZONE_VIEW_MENU,
};

pub struct ControlRects {
    pub seek: Rect,
}

/// Render a frame.
///
/// - `window_h_phys`: full window height in physical pixels.
/// - `fullscreen`: when true the title bar is hidden so video fills the surface
///   edge-to-edge. Controls always overlay the bottom of the canvas.
pub fn render_frame(
    gpu: &mut Gpu,
    app: &App,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    opacity: f32,
    scale: f32,
    window_h_phys: f32,
    fullscreen: bool,
) -> ControlRects {
    let Gpu { ctx, painter, text, tex_pass } = gpu;
    let wf = ctx.width() as f32;
    let win_h = window_h_phys;
    let s = scale;
    let corner_r = lntrn_theme::read_config_f32("window_manager", "corner_radius", 20.0) * s;

    painter.clear();
    input.begin_frame();

    let video_mode = !app.audio_only && app.pipeline.is_some();
    let bar_h = 52.0 * s;
    // Title bar visibility:
    //   • video mode      → solid bar that reserves canvas space (as before)
    //   • audio mode      → overlays the bars and fades in with hover (no reserve)
    //   • fullscreen      → hidden
    let title_fade = if fullscreen {
        0.0
    } else if video_mode {
        1.0
    } else {
        app.controls_alpha.clamp(0.0, 1.0)
    };
    let reserve_title = video_mode && !fullscreen;
    let title_h = if reserve_title { bar_h } else { 0.0 };

    // ── Window background (palette + opacity from lantern.toml) ───────
    // Fullscreen video paints opaque black so letterbox bars aren't see-through.
    let win_rect = Rect::new(0.0, 0.0, wf, win_h);
    if fullscreen && video_mode {
        painter.rect_filled(win_rect, 0.0, Color::BLACK);
    } else {
        draw_window_bg(painter, win_rect, corner_r, palette, opacity);
    }

    // ── Canvas (bars area or video) ───────────────────────────────────
    let canvas = Rect::new(0.0, title_h, wf, (win_h - title_h).max(0.0));
    let _canvas_state = input.add_zone(ZONE_CANVAS, canvas);

    let mut tex_draws: Vec<TextureDraw> = Vec::new();
    if !video_mode {
        let theme = crate::vis_theme::theme(app.vis_theme_index);
        draw_classic_bars(painter, &app.vis_bars, canvas, s, &theme.stops);
    } else if let Some(tex) = &app.video_texture {
        if app.video_width > 0 && app.video_height > 0 {
            let fit = aspect_fit(app.video_width, app.video_height, canvas);
            let mut draw = TextureDraw::new(tex, fit.x, fit.y, fit.w, fit.h);
            draw.clip = Some([canvas.x, canvas.y, canvas.w, canvas.h]);
            tex_draws.push(draw);
        }
    }

    // ── Title bar (video: solid; audio: hover-reveal overlay) ─────────
    // Drawn on painter layer 1 in audio mode so it composites above the bars;
    // in video mode it's part of the window frame (layer 0).
    if title_fade > 0.005 {
        if !reserve_title {
            painter.set_layer(1);
        }
        draw_title_bar(
            painter, text, ctx, input, palette, app, wf, bar_h, s, title_fade, reserve_title,
        );
        if !reserve_title {
            painter.set_layer(0);
        }
    }

    // ── Controls: always overlay the bottom of the canvas ────────────
    // Pushed to painter layer 1 so they composite ON TOP of the video texture
    // (which is drawn between painter layer 0 and layer 1 below).
    let fade = app.controls_alpha.clamp(0.0, 1.0);
    let mut seek_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    if fade > 0.005 {
        painter.set_layer(1);
        seek_rect = draw_controls_overlay(
            painter, text, ctx, input, palette, app, s, fade, canvas, fullscreen,
        );
    }

    // ── Multi-pass render ───────────────────────────────────────────
    // Order (each pass draws on top of the previous):
    //   1. painter layer 0 — window bg / black letterbox / audio bars / title bar
    //   2. video texture
    //   3. painter layer 1 — bottom controls overlay (scrim, seek bar, buttons)
    //   4. text — timestamps + title (always on top)
    let frame = ctx.begin_frame("Media Player");
    match frame {
        Ok(mut frame) => {
            let view = frame.view().clone();
            painter.render_layer(0, ctx, frame.encoder_mut(), &view, Some(Color::rgba(0.0, 0.0, 0.0, 0.0)));
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_text(ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[media-player] render error: {e}"),
    }

    ControlRects { seek: seek_rect }
}

// ── Bottom controls overlay ───────────────────────────────────────────────

/// Controls always overlay the bottom of the `canvas`: a darkening scrim, a
/// full-width seek bar with timestamps, a centered prev/play/next cluster, and
/// a fullscreen toggle at the right. Returns the seek-bar track rect for drag
/// mapping in `wayland.rs`.
fn draw_controls_overlay(
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
    let video_mode = !app.audio_only && app.pipeline.is_some();
    if video_mode {
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
        .size(time_font).color(time_color).draw(text, ctx.width(), ctx.height());
    TextLabel::new(&dur_str, band.x + band.w - pad_x - dw, text_y)
        .size(time_font).color(time_color).draw(text, ctx.width(), ctx.height());

    let seek_gap = 16.0 * us;
    let seek_left = band.x + pad_x + ctw + seek_gap;
    let seek_right = band.x + band.w - pad_x - dw - seek_gap;
    let seek_w = (seek_right - seek_left).max(0.0);
    let seek_y = seek_cy - seek_h * 0.5;
    let hit_pad = 14.0 * us;
    let seek_hit = Rect::new(seek_left - hit_pad, seek_y - hit_pad, seek_w + hit_pad * 2.0, seek_h + hit_pad * 2.0);
    let seek_state = input.add_zone(ZONE_SEEK_BAR, seek_hit);
    let active = seek_state.is_hovered() || seek_state.is_active() || app.seeking;
    let seek_val = if app.seeking { app.seek_value } else { app.progress_fraction() };
    draw_seek_bar(painter, us, seek_left, seek_y, seek_w, seek_h, seek_val, active, lantern_gold, muted);

    // ── Button row (below the seek row) ──────────────────────────────
    let btn_size = (60.0 * us).clamp(48.0, 84.0);
    let btn_cy = band.y + band_h - btn_size * 0.5 - 18.0 * us;
    let hover = |hovered: bool| if hovered { accent } else { icon_color };

    // Centered transport cluster: prev / play-pause / next.
    let gap = 40.0 * us;
    let cluster_w = btn_size * 3.0 + gap * 2.0;
    let cluster_x = band.x + (band.w - cluster_w) * 0.5;

    let prev_rect = Rect::new(cluster_x, btn_cy - btn_size * 0.5, btn_size, btn_size);
    let pp_rect = Rect::new(prev_rect.x + btn_size + gap, prev_rect.y, btn_size, btn_size);
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
    let fs_rect = Rect::new(band.x + band.w - pad_x - btn_size, btn_cy - btn_size * 0.5, btn_size, btn_size);
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
            painter.arc(cx, cy, r + t, 0.55, std::f32::consts::TAU - 1.1, t, r, color);
            // Arrowhead at the ring's leading end (top), pointing clockwise.
            let ah = r * 0.55;
            let tipx = cx + r * 0.85;
            let tipy = cy - r * 0.55;
            painter.triangle(tipx, tipy - ah * 0.5, tipx, tipy + ah * 0.5, tipx + ah, tipy, color);
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
            painter.rect_filled(Rect::new(shaft_x0, cy - t * 0.5, bar_x - shaft_x0 - r * 0.4, t), 0.0, color);
            let ah = r * 0.6;
            let tipx = bar_x - r * 0.1;
            painter.triangle(tipx - ah, cy - ah * 0.7, tipx - ah, cy + ah * 0.7, tipx, cy, color);
        }
    }
}

/// Fullscreen toggle: four corner brackets (enter) or inward arrows (exit).
fn draw_fullscreen_icon(painter: &mut lntrn_render::Painter, rect: Rect, color: Color, fullscreen: bool) {
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

// ── Title bar (video: solid; audio: hover-reveal overlay) ────────────────

/// Draw the title bar: window title, a "View" menu button, and Terminal-style
/// circular window controls (minimize / maximize / close). `fade` dims the whole
/// bar for the audio hover-reveal; `reserve` is true when the bar reserves canvas
/// space (video mode) vs. overlaying the bars (audio mode).
fn draw_title_bar(
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    ctx: &lntrn_render::GpuContext,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    app: &App,
    wf: f32, title_h: f32, s: f32,
    fade: f32, reserve: bool,
) {
    let title_font = FontSize::Custom((26.0 * s).clamp(20.0, 34.0));

    // In audio (overlay) mode, lay a subtle scrim behind the bar so controls and
    // title stay legible over the dancing bars.
    if !reserve {
        painter.rect_filled(
            Rect::new(0.0, 0.0, wf, title_h),
            0.0,
            Color::rgba(0.0, 0.0, 0.0, 0.32 * fade),
        );
    }

    let cursor = input.cursor();
    let by = title_h * 0.5;
    let btn_r = 11.0 * s;
    let icon = 4.5 * s;
    let thick = 1.8 * s;
    let gap = 34.0 * s;
    let right_pad = 24.0 * s;

    // Circular control buttons pulled in from the right edge (Terminal layout).
    let close_x = wf - right_pad - btn_r;
    let max_x = close_x - gap;
    let min_x = max_x - gap;

    // Generous square hit-zones centered on each circle so clicks land easily.
    let hz = |cx: f32| Rect::new(cx - btn_r * 1.4, by - btn_r * 1.4, btn_r * 2.8, btn_r * 2.8);
    let close_state = input.add_zone(ZONE_CLOSE, hz(close_x));
    let max_state = input.add_zone(ZONE_MAXIMIZE, hz(max_x));
    let min_state = input.add_zone(ZONE_MINIMIZE, hz(min_x));

    let idle = Color::from_rgba8(140, 120, 90, 255).with_alpha(fade);
    let icon_hover = Color::from_rgba8(250, 200, 0, 255).with_alpha(fade);
    let hover_bg = Color::from_rgba8(250, 200, 0, 60).with_alpha(fade);
    let close_bg = Color::rgba(0.85, 0.2, 0.2, 0.45 * fade);
    let close_icon = Color::rgba(1.0, 0.85, 0.85, fade);

    let _ = cursor;
    // Minimize — line
    if min_state.is_hovered() { painter.circle_filled(min_x, by, btn_r, hover_bg); }
    let ic = if min_state.is_hovered() { icon_hover } else { idle };
    painter.line(min_x - icon, by, min_x + icon, by, thick, ic);

    // Maximize — square outline
    if max_state.is_hovered() { painter.circle_filled(max_x, by, btn_r, hover_bg); }
    let ic = if max_state.is_hovered() { icon_hover } else { idle };
    painter.rect_stroke_sdf(
        Rect::new(max_x - icon, by - icon, icon * 2.0, icon * 2.0), 1.5 * s, thick, ic,
    );

    // Close — X (red on hover)
    let chov = close_state.is_hovered();
    if chov { painter.circle_filled(close_x, by, btn_r, close_bg); }
    let ic = if chov { close_icon } else { idle };
    painter.line(close_x - icon, by - icon, close_x + icon, by + icon, thick, ic);
    painter.line(close_x - icon, by + icon, close_x + icon, by - icon, thick, ic);

    // ── "View" menu button (left of the window controls) ──────────────
    let menu_font = FontSize::Custom((22.0 * s).clamp(16.0, 28.0));
    let view_label = "View";
    let vw = text.measure_width(view_label, menu_font.px());
    let view_pad = 14.0 * s;
    let view_w = vw + view_pad * 2.0;
    let view_x = min_x - btn_r * 1.4 - 12.0 * s - view_w;
    let view_rect = Rect::new(view_x, 0.0, view_w, title_h);
    let view_state = input.add_zone(ZONE_VIEW_MENU, view_rect);
    if view_state.is_hovered() || app.view_menu_open {
        painter.rect_filled(view_rect, 6.0 * s, hover_bg);
    }
    let view_color = if app.view_menu_open || view_state.is_hovered() { icon_hover } else { palette.text.with_alpha(fade) };
    TextLabel::new(view_label, view_x + view_pad, (title_h - menu_font.px()) * 0.5)
        .size(menu_font).color(view_color).draw(text, ctx.width(), ctx.height());

    // ── Drag region + title text (everything left of the View button) ──
    let drag_rect = Rect::new(0.0, 0.0, view_x.max(0.0), title_h);
    input.add_zone(ZONE_TITLE_BAR, drag_rect);

    let title_text = if app.file_name.is_empty() {
        "Lantern Media Player".to_string()
    } else {
        app.file_name.clone()
    };
    let title_px = title_font.px();
    let max_title_w = (drag_rect.w - 28.0 * s).max(0.0);
    let mut shown = title_text.clone();
    while text.measure_width(&shown, title_px) > max_title_w && shown.len() > 1 {
        shown.pop();
    }
    if shown.len() < title_text.len() && shown.len() > 1 {
        shown.pop();
        shown.push('\u{2026}');
    }
    TextLabel::new(&shown, 16.0 * s, (title_h - title_px) * 0.5)
        .size(title_font).color(palette.text.with_alpha(fade)).draw(text, ctx.width(), ctx.height());

    // ── View dropdown (color swatches) ────────────────────────────────
    if app.view_menu_open {
        draw_view_menu(painter, text, ctx, input, app, view_x, title_h, view_w, s);
    }
}

/// Dropdown listing the visualizer color themes as labeled swatch rows.
fn draw_view_menu(
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    ctx: &lntrn_render::GpuContext,
    input: &mut InteractionContext,
    app: &App,
    anchor_x: f32, title_h: f32, anchor_w: f32, s: f32,
) {
    let font = FontSize::Custom((20.0 * s).clamp(15.0, 26.0));
    let row_h = 40.0 * s;
    let count = crate::vis_theme::theme_count();
    let menu_w = (anchor_w.max(220.0 * s)).max(220.0 * s);
    let pad = 8.0 * s;
    let menu_h = row_h * count as f32 + pad * 2.0;
    let menu_x = anchor_x;
    let menu_y = title_h + 4.0 * s;

    // Panel background + border.
    painter.rect_filled(Rect::new(menu_x, menu_y, menu_w, menu_h), 10.0 * s, Color::from_rgba8(24, 22, 28, 250));
    painter.rect_stroke(Rect::new(menu_x, menu_y, menu_w, menu_h), 10.0 * s, 1.5 * s, Color::from_rgba8(250, 200, 0, 90));

    let sw_size = 22.0 * s;
    for i in 0..count {
        let theme = crate::vis_theme::theme(i);
        let row_y = menu_y + pad + i as f32 * row_h;
        let row = Rect::new(menu_x + pad, row_y, menu_w - pad * 2.0, row_h);
        let st = input.add_zone(crate::ZONE_VIEW_SWATCH_BASE + i as u32, row);
        let selected = i == app.vis_theme_index;
        if st.is_hovered() || selected {
            painter.rect_filled(row, 6.0 * s, Color::from_rgba8(250, 200, 0, if selected { 55 } else { 30 }));
        }
        // Swatch: a row of the 5 gradient stops.
        let sw_x = row.x + 8.0 * s;
        let sw_y = row_y + (row_h - sw_size) * 0.5;
        let stub = sw_size / 5.0;
        for (k, &(r, g, b)) in theme.stops.iter().enumerate() {
            painter.rect_filled(
                Rect::new(sw_x + k as f32 * stub, sw_y, stub + 0.5, sw_size),
                0.0, Color::from_rgb8(r, g, b),
            );
        }
        painter.rect_stroke(Rect::new(sw_x, sw_y, sw_size, sw_size), 3.0 * s, 1.0 * s, Color::rgba(1.0, 1.0, 1.0, 0.25));
        // Label.
        let label_x = sw_x + sw_size + 12.0 * s;
        let label_color = if selected { Color::from_rgb8(250, 200, 0) } else { Color::from_rgb8(232, 220, 200) };
        TextLabel::new(theme.name, label_x, row_y + (row_h - font.px()) * 0.5)
            .size(font).color(label_color).draw(text, ctx.width(), ctx.height());
    }
}

// ── Visualizer: Classic Bars (flush to bottom of canvas) ─────────────────

/// Color a bar by its frequency position (`t`: 0.0 = bass .. 1.0 = highs) using
/// the active theme's 5-stop gradient, brightening slightly with loudness so
/// peaks pop.
fn bar_color(stops: &[(u8, u8, u8); 5], t: f32, magnitude: f32) -> Color {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let idx = (t as usize).min(stops.len() - 2);
    let next = idx + 1;
    let frac = t - t.floor();
    let (r0, g0, b0) = stops[idx];
    let (r1, g1, b1) = stops[next];
    let r = r0 as f32 + (r1 as f32 - r0 as f32) * frac;
    let g = g0 as f32 + (g1 as f32 - g0 as f32) * frac;
    let b = b0 as f32 + (b1 as f32 - b0 as f32) * frac;
    let bright = 1.0 + magnitude.clamp(0.0, 1.0) * 0.25;
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
    stops: &[(u8, u8, u8); 5],
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
        let magnitude = bars[i];
        let t = i as f32 / (num_bars.saturating_sub(1).max(1)) as f32;

        let bar_h = (magnitude * max_h).max(3.0 * s);
        let x = canvas.x + side_margin + i as f32 * (bar_w + gap);
        let y = base_y - bar_h;

        let color = bar_color(stops, t, magnitude);

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
