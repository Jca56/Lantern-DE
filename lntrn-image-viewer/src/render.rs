use lntrn_render::{Color, Painter, Rect, TextRenderer, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel, TitleBar};

use std::path::PathBuf;

use crate::app::App;
use crate::canvas::sidebar::SidebarState;
use crate::canvas::sidebar_layout::SidebarLayout;
use crate::render_dialog::draw_viewer_dialog;
use crate::render_info::draw_info_overlay;
use crate::render_sidebar::{draw_sidebar, SidebarFlavor};
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_NAV_NEXT, ZONE_NAV_PREV,
    ZONE_SHUFFLE,
};

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    gpu: &mut Gpu,
    app: &App,
    sb: &mut SidebarState,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    scale: f32,
    dt: f32,
) {
    let Gpu {
        ctx,
        painter,
        text,
        tex_pass,
    } = gpu;
    let wf = ctx.width() as f32;
    let hf = ctx.height() as f32;
    let (sw, sh) = (ctx.width(), ctx.height());
    let s = scale;
    let hidden = app.chrome_hidden;

    // ── Phase A: browser bookkeeping (mutable) ──────────────────────
    sb.poll_thumbs(ctx, tex_pass);
    sb.scroll.tick(dt);
    // Re-clamp the panel width in case the window shrank under it.
    sb.set_width(sb.width, wf / s);
    let show_sidebar = sidebar_reserved_w(app, sb, s) > 0.0;
    let layout = SidebarLayout::compute_in(sb, sidebar_band(app, wf, hf, s), s);
    sb.scroll.clamp_to(layout.content_h, layout.rows_vp.h);
    let visible: Vec<usize> = if show_sidebar && !sb.collapsed {
        layout.visible_slots(sb.scroll.offset)
    } else {
        Vec::new()
    };
    let thumb_paths: Vec<PathBuf> = visible
        .iter()
        .filter_map(|&slot| layout.entry_index(slot))
        .filter_map(|i| sb.entries.get(i))
        .filter(|e| !e.is_dir)
        .map(|e| e.path.clone())
        .collect();
    for p in &thumb_paths {
        sb.request_thumb(p);
    }

    // ── Phase B: draw (immutable borrows) ───────────────────────────
    let sb: &SidebarState = sb;
    painter.clear();
    painter.set_layer(0);
    text.set_layer(0);
    input.begin_frame();

    // ── Background + chrome ─────────────────────────────────────────
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, palette.bg);
    if !hidden {
        draw_title_bar(painter, input, app, palette, wf, s);
    }

    let mut tex_draws: Vec<TextureDraw> = Vec::new();
    if show_sidebar {
        draw_sidebar(
            painter,
            text,
            input,
            sb,
            &layout,
            &visible,
            SidebarFlavor {
                add_badge: false,
                current: app.path.as_deref(),
            },
            &mut tex_draws,
            palette,
            s,
            sw,
            sh,
        );
    }

    // ── Canvas area (image display) ─────────────────────────────────
    let canvas = viewer_canvas(app, sb, wf, hf, s);
    // With the chrome hidden the image area IS the window, so it has to
    // supply the rounded corners the title/status bars normally provide.
    let radius = if hidden { 10.0 * s } else { 0.0 };
    painter.rect_filled(canvas, radius, Color::from_rgb8(18, 18, 18));
    let _canvas_state = input.add_zone(ZONE_CANVAS, canvas);

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

    if !hidden {
        draw_status_bar(painter, text, app, palette, canvas, wf, hf, s, sw, sh);
    }

    // ── Overlay layer: everything that floats above the picture ─────
    painter.set_layer(1);
    text.set_layer(1);

    // ── Navigation arrows ─────────────────────────────────────────
    if app.dir_files.len() > 1 {
        let btn_w = 40.0 * s;
        let btn_h = 60.0 * s;
        let btn_y = canvas.y + (canvas.h - btn_h) * 0.5;
        let margin = 12.0 * s;

        let prev_rect = Rect::new(canvas.x + margin, btn_y, btn_w, btn_h);
        let next_rect = Rect::new(canvas.x + canvas.w - margin - btn_w, btn_y, btn_w, btn_h);

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

    if app.show_info {
        draw_info_overlay(painter, text, app, palette, canvas, s, sw, sh);
    }
    if let Some(dialog) = &app.dialog {
        draw_viewer_dialog(painter, text, input, dialog, palette, wf, hf, s, sw, sh);
    }

    // ── Render passes ───────────────────────────────────────────────
    match ctx.begin_frame("Image Viewer") {
        Ok(mut frame) => {
            let view = frame.view().clone();
            painter.render_layer(
                0,
                ctx,
                frame.encoder_mut(),
                &view,
                Some(palette.bg.with_alpha(0.0)),
            );
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(0, ctx, frame.encoder_mut(), &view);
            frame.flush(ctx);
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(1, ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[image-viewer] render error: {e}"),
    }
}

// ── Text helpers ──────────────────────────────────────────────────────────

/// Shrink `s` to fit within `max_w` pixels by dropping characters from the
/// middle and inserting an ellipsis — keeps both the leading dirs and the
/// filename visible (e.g. `/home/a…/candle.svg`). Returns `s` unchanged if it
/// already fits, or just "…" if even that won't fit.
fn middle_ellipsize(text: &mut TextRenderer, s: &str, fpx: f32, max_w: f32) -> String {
    if text.measure_width(s, fpx) <= max_w {
        return s.to_string();
    }
    let ell = "…";
    if text.measure_width(ell, fpx) > max_w {
        return String::new();
    }
    // Work on chars so we never split a UTF-8 codepoint.
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let build = |keep: usize| -> String {
        let head = keep.div_ceil(2);
        let tail = keep - head;
        let mut out: String = chars[..head].iter().collect();
        out.push_str(ell);
        out.extend(chars[n - tail..].iter());
        out
    };
    // Binary search the largest `keep` (total visible chars, head+tail) that fits.
    let (mut lo, mut hi) = (0usize, n);
    let mut best = String::from(ell);
    while lo <= hi {
        let keep = (lo + hi) / 2;
        let candidate = build(keep);
        if text.measure_width(&candidate, fpx) <= max_w {
            best = candidate;
            lo = keep + 1;
        } else {
            if keep == 0 {
                break;
            }
            hi = keep - 1;
        }
    }
    best
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

/// Sidebar width the viewer reserves on the left. A collapsed browser keeps
/// its thin strip as the affordance to reopen it, except in rice mode where
/// it vanishes so nothing but the picture remains.
pub fn sidebar_reserved_w(app: &App, sb: &SidebarState, s: f32) -> f32 {
    if app.chrome_hidden && sb.collapsed {
        0.0
    } else {
        sb.phys_width(s)
    }
}

/// The vertical strip the browser may occupy: between title and status bar,
/// or the full window height in rice mode.
pub fn sidebar_band(app: &App, wf: f32, hf: f32, s: f32) -> Rect {
    let (title_h, status_h) = chrome_heights(app, s);
    Rect::new(0.0, title_h, wf, (hf - title_h - status_h).max(1.0))
}

/// The image display area: the window minus chrome and browser, or the whole
/// window in rice mode. Render, scroll-zoom, and SVG re-rasterization all
/// derive the canvas from here so they can never disagree.
pub fn viewer_canvas(app: &App, sb: &SidebarState, wf: f32, hf: f32, s: f32) -> Rect {
    let (title_h, status_h) = chrome_heights(app, s);
    let left = sidebar_reserved_w(app, sb, s).min(wf - 1.0).max(0.0);
    Rect::new(
        left,
        title_h,
        (wf - left).max(1.0),
        (hf - title_h - status_h).max(1.0),
    )
}

fn chrome_heights(app: &App, s: f32) -> (f32, f32) {
    if app.chrome_hidden {
        (0.0, 0.0)
    } else {
        (crate::TITLE_H * s, crate::STATUS_H * s)
    }
}

fn draw_title_bar(
    painter: &mut Painter,
    input: &mut InteractionContext,
    app: &App,
    palette: &FoxPalette,
    wf: f32,
    s: f32,
) {
    let title_h = crate::TITLE_H * s;
    // ── Title bar ───────────────────────────────────────────────────
    let title_rect = Rect::new(0.0, 0.0, wf, title_h);
    let close_state = input.add_zone(
        ZONE_CLOSE,
        TitleBar::new(title_rect).scale(s).close_button_rect(),
    );
    let max_state = input.add_zone(
        ZONE_MAXIMIZE,
        TitleBar::new(title_rect).scale(s).maximize_button_rect(),
    );
    let min_state = input.add_zone(
        ZONE_MINIMIZE,
        TitleBar::new(title_rect).scale(s).minimize_button_rect(),
    );

    TitleBar::new(title_rect)
        .scale(s)
        .close_hovered(close_state.is_hovered())
        .maximize_hovered(max_state.is_hovered())
        .minimize_hovered(min_state.is_hovered())
        .draw(painter, palette);

    // Shuffle toggle — sits just left of the minimize button.
    let min_rect = TitleBar::new(title_rect).scale(s).minimize_button_rect();
    let shuffle_rect = Rect::new(
        min_rect.x - min_rect.w,
        title_rect.y,
        min_rect.w,
        title_rect.h,
    );
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
}

#[allow(clippy::too_many_arguments)]
fn draw_status_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    app: &App,
    palette: &FoxPalette,
    canvas: Rect,
    wf: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let status_h = crate::STATUS_H * s;
    // ── Status bar ──────────────────────────────────────────────────
    // Font px scales with `s` so the text grows with the rest of the chrome
    // (title_h/status_h are *s too) instead of staying tiny on scaled outputs.
    let status_font = FontSize::Custom(FontSize::Body.px() * s);
    let fpx = status_font.px();
    let status_rect = Rect::new(0.0, hf - status_h, wf, status_h);
    painter.rect_filled(status_rect, 0.0, palette.surface);

    let pad = 12.0 * s;
    // Vertically centre the text within the bar.
    let status_y = status_rect.y + (status_h - fpx) * 0.5;
    let gap = 16.0 * s; // min gap between path (left) and info (right)

    // Right side: dimensions + zoom %. Drawn first so we know how much width
    // it claims, then the path gets whatever's left.
    let info = {
        let mut parts: Vec<String> = Vec::new();
        if app.slideshow.is_some() {
            parts.push(format!("▶ {}s", app.slideshow_interval.as_secs()));
        }
        if app.dir_files.len() > 1 {
            parts.push(format!("{} / {}", app.dir_index + 1, app.dir_files.len()));
        }
        if let Some(img) = &app.image {
            let fit_zoom = (canvas.w / img.width as f32).min(canvas.h / img.height as f32);
            let pct = (fit_zoom * app.zoom * 100.0).round() as u32;
            parts.push(format!("{} — {}%", app.dimensions_text, pct));
        }
        if parts.is_empty() {
            None
        } else {
            let joined = parts.join("   ·   ");
            let w = text.measure_width(&joined, fpx);
            Some((joined, w))
        }
    };
    let info_w = info.as_ref().map(|(_, w)| *w).unwrap_or(0.0);

    // Available width for the path = bar minus padding, info, and a gap.
    let avail = wf - pad * 2.0 - info_w - if info_w > 0.0 { gap } else { 0.0 };

    // Draw the (middle-ellipsized) path on the left if there's room for it.
    if avail > fpx {
        let left = app.flash_text().unwrap_or(app.status_text.as_str());
        let path = middle_ellipsize(text, left, fpx, avail);
        TextLabel::new(&path, pad, status_y)
            .size(status_font)
            .color(palette.text)
            .draw(text, sw, sh);
    }

    // Draw the info on the right (always — it's small and the more useful bit
    // when space is tight), unless the bar is too narrow for even that.
    if let Some((info, info_w)) = info {
        if info_w <= wf - pad * 2.0 {
            TextLabel::new(&info, wf - info_w - pad, status_y)
                .size(status_font)
                .color(palette.text)
                .draw(text, sw, sh);
        }
    }
}
