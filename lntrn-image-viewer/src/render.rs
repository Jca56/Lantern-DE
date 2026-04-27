use lntrn_render::{Color, Painter, Rect, TextPass, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel, TitleBar};

use crate::app::App;
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_NAV_PREV, ZONE_NAV_NEXT,
    ZONE_SHUFFLE,
};

pub fn render_frame(
    gpu: &mut Gpu,
    app: &App,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    scale: f32,
) {
    let Gpu { ctx, painter, text, tex_pass } = gpu;
    let wf = ctx.width() as f32;
    let hf = ctx.height() as f32;
    let s = scale;

    painter.clear();
    input.begin_frame();

    let title_h = 36.0 * s;
    let status_h = 28.0 * s;

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

    // Shuffle toggle — sits just left of the minimize button.
    let min_rect = TitleBar::new(title_rect).scale(s).minimize_button_rect();
    let shuffle_rect = Rect::new(min_rect.x - min_rect.w, title_rect.y, min_rect.w, title_rect.h);
    let shuffle_state = input.add_zone(ZONE_SHUFFLE, shuffle_rect);
    let shuffle_hovered = shuffle_state.is_hovered();
    if app.shuffle {
        painter.rect_filled(shuffle_rect, 0.0, palette.accent.with_alpha(0.18));
    } else if shuffle_hovered {
        painter.rect_filled(shuffle_rect, 0.0, Color::WHITE.with_alpha(0.06));
    }
    let icon_color = if app.shuffle {
        palette.accent
    } else if shuffle_hovered {
        Color::from_rgba8(255, 255, 255, 230)
    } else {
        Color::from_rgba8(236, 236, 236, 200)
    };
    draw_shuffle_icon(painter, shuffle_rect, icon_color, s);

    // ── Canvas area (image display) ─────────────────────────────────
    let canvas = Rect::new(0.0, title_h, wf, hf - title_h - status_h);
    painter.rect_filled(canvas, 0.0, Color::from_rgb8(18, 18, 18));

    let _canvas_state = input.add_zone(ZONE_CANVAS, canvas);

    // Build texture draw list for image
    let mut tex_draws: Vec<TextureDraw> = Vec::new();

    if let Some(img) = &app.image {
        let fit_zoom = (canvas.w / img.width as f32).min(canvas.h / img.height as f32);
        let display_zoom = fit_zoom * app.zoom;
        let draw_w = img.width as f32 * display_zoom;
        let draw_h = img.height as f32 * display_zoom;
        let draw_x = canvas.x + (canvas.w - draw_w) * 0.5 + app.pan_x;
        let draw_y = canvas.y + (canvas.h - draw_h) * 0.5 + app.pan_y;

        let mut draw = TextureDraw::new(&img.texture, draw_x, draw_y, draw_w, draw_h);
        draw.clip = Some([canvas.x, canvas.y, canvas.w, canvas.h]);
        tex_draws.push(draw);
    }

    // ── Navigation arrows ─────────────────────────────────────────
    if app.dir_files.len() > 1 {
        let btn_w = 40.0 * s;
        let btn_h = 60.0 * s;
        let btn_y = canvas.y + (canvas.h - btn_h) * 0.5;
        let margin = 12.0 * s;

        let prev_rect = Rect::new(margin, btn_y, btn_w, btn_h);
        let next_rect = Rect::new(wf - margin - btn_w, btn_y, btn_w, btn_h);

        let prev_state = input.add_zone(ZONE_NAV_PREV, prev_rect);
        let next_state = input.add_zone(ZONE_NAV_NEXT, next_rect);

        let prev_alpha = if prev_state.is_hovered() { 0.7 } else { 0.35 };
        let next_alpha = if next_state.is_hovered() { 0.7 } else { 0.35 };

        painter.rect_filled(prev_rect, 10.0 * s, palette.surface.with_alpha(prev_alpha));
        painter.rect_filled(next_rect, 10.0 * s, palette.surface.with_alpha(next_alpha));

        let arrow_size = FontSize::Heading;
        let arrow_y = btn_y + (btn_h - arrow_size.px()) * 0.5;

        let prev_label = "◀";
        let prev_w = text.measure_width(prev_label, arrow_size.px());
        TextLabel::new(prev_label, prev_rect.x + (btn_w - prev_w) * 0.5, arrow_y)
            .size(arrow_size)
            .color(palette.text.with_alpha(prev_alpha + 0.2))
            .draw(text, ctx.width(), ctx.height());

        let next_label = "▶";
        let next_w = text.measure_width(next_label, arrow_size.px());
        TextLabel::new(next_label, next_rect.x + (btn_w - next_w) * 0.5, arrow_y)
            .size(arrow_size)
            .color(palette.text.with_alpha(next_alpha + 0.2))
            .draw(text, ctx.width(), ctx.height());
    }

    // ── Status bar ──────────────────────────────────────────────────
    let status_rect = Rect::new(0.0, hf - status_h, wf, status_h);
    painter.rect_filled(status_rect, 0.0, palette.surface);

    let status_y = status_rect.y + 4.0 * s;
    TextLabel::new(&app.status_text, 8.0 * s, status_y)
        .size(FontSize::Small)
        .color(palette.text)
        .draw(text, ctx.width(), ctx.height());

    if let Some(img) = &app.image {
        let fit_zoom = (canvas.w / img.width as f32).min(canvas.h / img.height as f32);
        let pct = (fit_zoom * app.zoom * 100.0).round() as u32;
        let info = format!("{} — {}%", app.dimensions_text, pct);
        let info_w = text.measure_width(&info, FontSize::Small.px());
        TextLabel::new(&info, wf - info_w - 8.0 * s, status_y)
            .size(FontSize::Small)
            .color(palette.text)
            .draw(text, ctx.width(), ctx.height());
    }

    // ── Multi-pass render ───────────────────────────────────────────
    let frame = ctx.begin_frame("Image Viewer");
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
        Err(e) => eprintln!("[image-viewer] render error: {e}"),
    }
}

// ── Shuffle icon ────────────────────────────────────────────────────────────

fn draw_shuffle_icon(painter: &mut Painter, rect: Rect, color: Color, s: f32) {
    let cx = rect.center_x();
    let cy = rect.center_y();
    let half_w = 9.0 * s;
    let half_h = 6.0 * s;
    let stroke = 2.0 * s;
    let l = cx - half_w;
    let r = cx + half_w;
    let t = cy - half_h;
    let b = cy + half_h;
    let bend = 1.5 * s;

    // Path A: top-left ── ╲ to bottom-right
    painter.line(l, t, cx - bend, t, stroke, color);
    painter.line(cx - bend, t, cx + bend, b, stroke, color);
    painter.line(cx + bend, b, r, b, stroke, color);

    // Path B: bottom-left ── ╱ to top-right
    painter.line(l, b, cx - bend, b, stroke, color);
    painter.line(cx - bend, b, cx + bend, t, stroke, color);
    painter.line(cx + bend, t, r, t, stroke, color);

    // Arrow tips at right ends
    let tip = 3.0 * s;
    painter.line(r, t, r - tip, t + tip * 0.5, stroke, color);
    painter.line(r, t, r - tip * 0.5, t + tip * 0.9, stroke, color);
    painter.line(r, b, r - tip, b - tip * 0.5, stroke, color);
    painter.line(r, b, r - tip * 0.5, b - tip * 0.9, stroke, color);
}
