use lntrn_render::{Color, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, Slider, TextLabel, TitleBar};

use crate::app::App;
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

    // ── Video canvas ────────────────────────────────────────────────
    let canvas = Rect::new(0.0, title_h, wf, hf - title_h - controls_h);
    painter.rect_filled(canvas, 0.0, Color::from_rgb8(18, 18, 18));
    let _canvas_state = input.add_zone(ZONE_CANVAS, canvas);

    // Draw video texture (aspect-fit centered)
    let mut tex_draws: Vec<TextureDraw> = Vec::new();
    if let Some(tex) = &app.video_texture {
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
    let frame = ctx.begin_frame("Video Player");
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
        Err(e) => eprintln!("[video-player] render error: {e}"),
    }

    seek_rect
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
