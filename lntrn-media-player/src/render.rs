//! Putting a frame together. Three layers, each a painter pass followed by
//! its text, so what's on top really covers what's below:
//!
//!   0. window background, visualizer bars — then the video texture
//!   1. title bar, controls overlay, status message, volume readout
//!   2. the View dropdown
//!
//! Layers only ever go up within a frame (`set_layer` ignores a step back),
//! so the drawing below is strictly in that order.

use std::time::Instant;

use lntrn_render::{Color, Painter, Rect, TextRenderer, TextureDraw};
use lntrn_ui::gpu::{draw_window_bg, FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::App;
use crate::render_controls::draw_controls_overlay;
use crate::render_title::{draw_title_bar, draw_view_menu};
use crate::{Gpu, ZONE_CANVAS};

pub struct ControlRects {
    pub seek: Rect,
}

/// The look read from lantern.toml. Loaded once and again when the file
/// changes — reading it per frame meant parsing the config hundreds of
/// times a second.
pub struct Theme {
    pub palette: FoxPalette,
    pub opacity: f32,
    /// Window corner radius, logical px.
    pub corner_radius: f32,
}

impl Theme {
    pub fn load() -> Self {
        Self {
            palette: FoxPalette::current(),
            opacity: lntrn_theme::background_opacity(),
            corner_radius: lntrn_theme::read_config_f32("window_manager", "corner_radius", 20.0),
        }
    }
}

/// Render a frame.
///
/// - `window_h_phys`: full window height in physical pixels.
/// - `fullscreen`: when true the title bar is hidden so video fills the surface
///   edge-to-edge. Controls always overlay the bottom of the canvas.
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    gpu: &mut Gpu,
    app: &App,
    input: &mut InteractionContext,
    theme: &Theme,
    scale: f32,
    window_h_phys: f32,
    fullscreen: bool,
    now: Instant,
) -> ControlRects {
    let Gpu {
        ctx,
        painter,
        text,
        tex_pass,
    } = gpu;
    let wf = ctx.width() as f32;
    let win_h = window_h_phys;
    let s = scale;
    let palette = &theme.palette;

    painter.clear();
    input.begin_frame();

    let video_mode = app.video_mode();
    let bar_h = 52.0 * s;
    // Title bar visibility:
    //   • video mode      → solid bar that reserves canvas space
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

    // ── Layer 0: window background, bars or video ─────────────────────
    // Fullscreen video paints opaque black so letterbox bars aren't see-through.
    let win_rect = Rect::new(0.0, 0.0, wf, win_h);
    if fullscreen && video_mode {
        painter.rect_filled(win_rect, 0.0, Color::BLACK);
    } else {
        draw_window_bg(painter, win_rect, theme.corner_radius * s, palette, theme.opacity);
    }

    let canvas = Rect::new(0.0, title_h, wf, (win_h - title_h).max(0.0));
    input.add_zone(ZONE_CANVAS, canvas);

    let mut tex_draws: Vec<TextureDraw> = Vec::new();
    if !video_mode {
        let stops = &crate::vis_theme::theme(app.vis_theme_index).stops;
        crate::visualizer::draw_bars(painter, app.visualizer.bars(), canvas, s, stops);
    } else if let Some(tex) = &app.video_texture {
        if app.video_width > 0 && app.video_height > 0 {
            let facing = app.video_orientation;
            let (shown_w, shown_h) = if facing.swaps_axes() {
                (app.video_height, app.video_width)
            } else {
                (app.video_width, app.video_height)
            };
            let fit = aspect_fit(shown_w, shown_h, canvas);
            let mut draw = TextureDraw::new(tex, fit.x, fit.y, fit.w, fit.h)
                .oriented(facing.quarter_turns, facing.mirror);
            draw.clip = Some([canvas.x, canvas.y, canvas.w, canvas.h]);
            tex_draws.push(draw);
        }
    }

    // ── Layer 1: everything that floats over the picture ──────────────
    painter.set_layer(1);
    text.set_layer(1);

    let view_button = (title_fade > 0.005).then(|| {
        draw_title_bar(
            painter,
            text,
            ctx,
            input,
            palette,
            app,
            wf,
            bar_h,
            s,
            title_fade,
            reserve_title,
        )
    });

    // Nothing playing: say why (no media yet, or what went wrong).
    if app.pipeline.is_none() && !app.status_text.is_empty() {
        draw_status(text, ctx.width(), ctx.height(), palette, &app.status_text, canvas, s);
    }

    let fade = app.controls_alpha.clamp(0.0, 1.0);
    let mut seek_rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    if fade > 0.005 {
        seek_rect = draw_controls_overlay(
            painter, text, ctx, input, palette, app, s, fade, canvas, fullscreen,
        );
    }

    let osd = app.volume_osd_alpha(now);
    if osd > 0.005 {
        // Clear of the title bar wherever it currently is.
        let top = if fullscreen { 0.0 } else { bar_h };
        draw_volume_osd(painter, text, ctx.width(), ctx.height(), app.volume, wf, top, s, osd);
    }

    // ── Layer 2: the View dropdown, above the video and the controls ──
    painter.set_layer(2);
    text.set_layer(2);
    if let Some(anchor) = view_button.filter(|_| app.view_menu_open) {
        draw_view_menu(painter, text, ctx, input, app, anchor, wf, s);
    }

    // ── Render passes ───────────────────────────────────────────────
    // Painter then text per layer, the video between layer 0's painter and
    // everything else. The flushes keep one layer's text upload from
    // overwriting the buffer an earlier layer's pass still reads.
    match ctx.begin_frame("Media Player") {
        Ok(mut frame) => {
            let view = frame.view().clone();
            painter.render_layer(
                0,
                ctx,
                frame.encoder_mut(),
                &view,
                Some(Color::rgba(0.0, 0.0, 0.0, 0.0)),
            );
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(0, ctx, frame.encoder_mut(), &view);
            frame.flush(ctx);
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(1, ctx, frame.encoder_mut(), &view);
            frame.flush(ctx);
            painter.render_layer(2, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(2, ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[media-player] render error: {e}"),
    }

    ControlRects { seek: seek_rect }
}

// ── Status message ──────────────────────────────────────────────────────

/// One centered line in the middle of the canvas: "No media loaded", or the
/// reason a file wouldn't play.
fn draw_status(
    text: &mut TextRenderer,
    sw: u32,
    sh: u32,
    palette: &FoxPalette,
    message: &str,
    canvas: Rect,
    s: f32,
) {
    let px = 26.0 * s;
    let shown = fit_text(text, message, px, (canvas.w - 64.0 * s).max(0.0));
    let w = text.measure_width(&shown, px);
    TextLabel::new(
        &shown,
        canvas.x + (canvas.w - w) * 0.5,
        canvas.y + (canvas.h - px) * 0.42,
    )
    .size(FontSize::Custom(px))
    .color(palette.text.with_alpha(0.9))
    .max_width(w + 24.0 * s)
    .draw(text, sw, sh);
}

// ── Volume readout ──────────────────────────────────────────────────────

/// A pill near the top of the window — "Volume 65%" and a level bar — shown
/// for a moment whenever the volume changes.
#[allow(clippy::too_many_arguments)]
fn draw_volume_osd(
    painter: &mut Painter,
    text: &mut TextRenderer,
    sw: u32,
    sh: u32,
    volume: f64,
    wf: f32,
    top: f32,
    s: f32,
    alpha: f32,
) {
    let px = 28.0 * s;
    let pad = 28.0 * s;
    let label = format!("Volume {}%", (volume * 100.0).round() as u32);
    // Sized for the widest label so the pill doesn't twitch as digits change.
    let label_w = text.measure_width("Volume 100%", px);
    let bar_w = 200.0 * s;
    let gap = 22.0 * s;
    let w = (pad * 2.0 + label_w + gap + bar_w).min(wf - 24.0 * s);
    let h = 72.0 * s;
    let pill = Rect::new((wf - w) * 0.5, top + 20.0 * s, w, h);

    painter.rect_filled(pill, h * 0.5, Color::rgba(0.0, 0.0, 0.0, 0.66 * alpha));
    TextLabel::new(&label, pill.x + pad, pill.y + (h - px) * 0.5)
        .size(FontSize::Custom(px))
        .color(Color::rgba(1.0, 1.0, 1.0, alpha))
        .max_width(label_w + 12.0 * s)
        .draw(text, sw, sh);

    let track_h = 10.0 * s;
    let track_x = pill.x + pad + label_w + gap;
    let track_w = (pill.x + pill.w - pad - track_x).max(0.0);
    let track = Rect::new(track_x, pill.y + (h - track_h) * 0.5, track_w, track_h);
    painter.rect_filled(track, track_h * 0.5, Color::rgba(1.0, 1.0, 1.0, 0.28 * alpha));
    let level = volume.clamp(0.0, 1.0) as f32;
    if level > 0.0 && track_w > 0.0 {
        let fill = Rect::new(track.x, track.y, (track_w * level).max(track_h), track_h);
        let gold = Color::from_rgb8(250, 180, 0).with_alpha(alpha);
        painter.rect_filled(fill, track_h * 0.5, gold);
    }
}

// ── Layout helpers ──────────────────────────────────────────────────────────

/// `s` shortened to fit `max_w`, ending in an ellipsis when it had to be cut.
/// Binary search over the character count: a long file name costs a handful
/// of measurements, not one per character.
pub fn fit_text(text: &mut TextRenderer, s: &str, px: f32, max_w: f32) -> String {
    if text.measure_width(s, px) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let build = |keep: usize| -> String {
        let mut out: String = chars[..keep].iter().collect();
        out.push('\u{2026}');
        out
    };
    // Largest prefix whose ellipsized form still fits.
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if text.measure_width(&build(mid), px) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        String::new()
    } else {
        build(lo)
    }
}

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
