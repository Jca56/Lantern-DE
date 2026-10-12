//! The title bar (solid above video, a hover-reveal overlay above the
//! visualizer) with its window buttons and "View" menu button, and the View
//! dropdown of visualizer color themes.

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::app::App;
use crate::render::fit_text;
use crate::{ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_TITLE_BAR, ZONE_VIEW_MENU};

/// Draw the title bar: window title, a "View" menu button, and Terminal-style
/// circular window controls (minimize / maximize / close). `fade` dims the whole
/// bar for the audio hover-reveal; `reserve` is true when the bar reserves canvas
/// space (video mode) vs. overlaying the bars (audio mode). Returns the View
/// button's rect, which the dropdown hangs from.
#[allow(clippy::too_many_arguments)]
pub fn draw_title_bar(
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    ctx: &lntrn_render::GpuContext,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    app: &App,
    wf: f32,
    title_h: f32,
    s: f32,
    fade: f32,
    reserve: bool,
) -> Rect {
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

    // Minimize — line
    if min_state.is_hovered() {
        painter.circle_filled(min_x, by, btn_r, hover_bg);
    }
    let ic = if min_state.is_hovered() {
        icon_hover
    } else {
        idle
    };
    painter.line(min_x - icon, by, min_x + icon, by, thick, ic);

    // Maximize — square outline
    if max_state.is_hovered() {
        painter.circle_filled(max_x, by, btn_r, hover_bg);
    }
    let ic = if max_state.is_hovered() {
        icon_hover
    } else {
        idle
    };
    painter.rect_stroke_sdf(
        Rect::new(max_x - icon, by - icon, icon * 2.0, icon * 2.0),
        1.5 * s,
        thick,
        ic,
    );

    // Close — X (red on hover)
    let chov = close_state.is_hovered();
    if chov {
        painter.circle_filled(close_x, by, btn_r, close_bg);
    }
    let ic = if chov { close_icon } else { idle };
    painter.line(
        close_x - icon,
        by - icon,
        close_x + icon,
        by + icon,
        thick,
        ic,
    );
    painter.line(
        close_x - icon,
        by + icon,
        close_x + icon,
        by - icon,
        thick,
        ic,
    );

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
    let view_color = if app.view_menu_open || view_state.is_hovered() {
        icon_hover
    } else {
        palette.text.with_alpha(fade)
    };
    TextLabel::new(
        view_label,
        view_x + view_pad,
        (title_h - menu_font.px()) * 0.5,
    )
    .size(menu_font)
    .color(view_color)
    .draw(text, ctx.width(), ctx.height());

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
    let shown = fit_text(text, &title_text, title_px, max_title_w);
    TextLabel::new(&shown, 16.0 * s, (title_h - title_px) * 0.5)
        .size(title_font)
        .color(palette.text.with_alpha(fade))
        .draw(text, ctx.width(), ctx.height());

    view_rect
}

/// Dropdown listing the visualizer color themes as labeled swatch rows. Hangs
/// under `anchor` (the View button), nudged left as needed to stay inside a
/// window `wf` wide. The caller draws it on the topmost layer, so it covers
/// the video and the controls instead of hiding behind them.
#[allow(clippy::too_many_arguments)]
pub fn draw_view_menu(
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    ctx: &lntrn_render::GpuContext,
    input: &mut InteractionContext,
    app: &App,
    anchor: Rect,
    wf: f32,
    s: f32,
) {
    let font = FontSize::Custom((20.0 * s).clamp(15.0, 26.0));
    let row_h = 40.0 * s;
    let count = crate::vis_theme::theme_count();
    let menu_w = anchor.w.max(220.0 * s);
    let pad = 8.0 * s;
    let menu_h = row_h * count as f32 + pad * 2.0;
    let edge = 8.0 * s;
    let menu_x = anchor.x.min(wf - menu_w - edge).max(edge);
    let menu_y = anchor.y + anchor.h + 4.0 * s;

    // Panel background + border.
    painter.rect_filled(
        Rect::new(menu_x, menu_y, menu_w, menu_h),
        10.0 * s,
        Color::from_rgba8(24, 22, 28, 250),
    );
    painter.rect_stroke(
        Rect::new(menu_x, menu_y, menu_w, menu_h),
        10.0 * s,
        1.5 * s,
        Color::from_rgba8(250, 200, 0, 90),
    );

    let sw_size = 22.0 * s;
    for i in 0..count {
        let theme = crate::vis_theme::theme(i);
        let row_y = menu_y + pad + i as f32 * row_h;
        let row = Rect::new(menu_x + pad, row_y, menu_w - pad * 2.0, row_h);
        let st = input.add_zone(crate::ZONE_VIEW_SWATCH_BASE + i as u32, row);
        let selected = i == app.vis_theme_index;
        if st.is_hovered() || selected {
            painter.rect_filled(
                row,
                6.0 * s,
                Color::from_rgba8(250, 200, 0, if selected { 55 } else { 30 }),
            );
        }
        // Swatch: a row of the 5 gradient stops.
        let sw_x = row.x + 8.0 * s;
        let sw_y = row_y + (row_h - sw_size) * 0.5;
        let stub = sw_size / 5.0;
        for (k, &(r, g, b)) in theme.stops.iter().enumerate() {
            painter.rect_filled(
                Rect::new(sw_x + k as f32 * stub, sw_y, stub + 0.5, sw_size),
                0.0,
                Color::from_rgb8(r, g, b),
            );
        }
        painter.rect_stroke(
            Rect::new(sw_x, sw_y, sw_size, sw_size),
            3.0 * s,
            1.0 * s,
            Color::rgba(1.0, 1.0, 1.0, 0.25),
        );
        // Label.
        let label_x = sw_x + sw_size + 12.0 * s;
        let label_color = if selected {
            Color::from_rgb8(250, 200, 0)
        } else {
            Color::from_rgb8(232, 220, 200)
        };
        TextLabel::new(theme.name, label_x, row_y + (row_h - font.px()) * 0.5)
            .size(font)
            .color(label_color)
            .draw(text, ctx.width(), ctx.height());
    }
}
