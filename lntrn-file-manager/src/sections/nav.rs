use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextInput, TextLabel};

use crate::app::App;

use super::breadcrumb_segments;
use super::icons::{draw_preview_pane_icon, draw_sort_icon, draw_view_mode_icon};

// ── Nav bar ─────────────────────────────────────────────────────────────────

pub fn draw_nav_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    app: &App,
    nav_rect: Rect,
    view_toggle_rect: Rect,
    view_toggle_hovered: bool,
    back_rect: Rect,
    back_hovered: bool,
    forward_rect: Rect,
    forward_hovered: bool,
    up_rect: Rect,
    up_hovered: bool,
    cloud_rect: Rect,
    cloud_hovered: bool,
    path_rect: Rect,
    _path_hovered: bool,
    breadcrumb_hovered: &[bool],
    preview_rect: Rect,
    preview_hovered: bool,
    preview_supported: bool,
    sort_rect: Rect,
    sort_hovered: bool,
    search_rect: Rect,
    search_hovered: bool,
    screen: (u32, u32),
    s: f32,
) {
    // Nav bar bg intentionally not painted — the window bg already covers
    // this region. Stacking `palette.surface` here would compound the alpha
    // and look opaque under transparency.

    // ── View mode toggle icon (changes per mode) ────────────────────────
    let vt_color = if view_toggle_hovered { palette.text } else { palette.text_secondary };
    if view_toggle_hovered {
        painter.rect_filled(view_toggle_rect, 4.0 * s, palette.surface_2.with_alpha(0.5));
    }
    draw_view_mode_icon(painter, app.view_mode, view_toggle_rect, vt_color, s);

    // Vertical divider
    painter.rect_filled(
        Rect::new(view_toggle_rect.x + view_toggle_rect.w + 2.0 * s, nav_rect.y + 12.0 * s, 1.0, 24.0 * s),
        0.0,
        Color::WHITE.with_alpha(0.08),
    );

    // ── Cloud quick-link button ───────────────────────────────────────────
    let cloud_color = if cloud_hovered { palette.text } else { palette.text_secondary };
    if cloud_hovered {
        painter.rect_filled(cloud_rect, 5.0 * s, palette.surface_2.with_alpha(0.5));
    }
    {
        // Cloud silhouette sized for a 44×44 button — ~30% larger than the
        // sidebar version so it reads as the marquee shortcut it is.
        let cx = cloud_rect.center_x();
        let cy = cloud_rect.center_y();
        let u = s * 1.4; // unit scale for icon strokes
        painter.circle_filled(cx - 4.0*u, cy - 1.0*u, 4.0*u, cloud_color);
        painter.circle_filled(cx + 1.0*u, cy - 3.5*u, 5.5*u, cloud_color);
        painter.circle_filled(cx + 5.0*u, cy,         4.0*u, cloud_color);
        painter.rect_filled(Rect::new(cx - 7.0*u, cy - 1.0*u, 14.0*u, 5.0*u), 2.0*u, cloud_color);
    }

    // Vertical divider between cloud and back
    painter.rect_filled(
        Rect::new(cloud_rect.x + cloud_rect.w + 4.0 * s, nav_rect.y + 12.0 * s, 1.0, 24.0 * s),
        0.0,
        Color::WHITE.with_alpha(0.08),
    );

    // ── Back button ────────────────────────────────────────────────────────
    let back_color = if app.can_go_back() {
        if back_hovered { palette.text } else { palette.text_secondary }
    } else {
        palette.muted.with_alpha(0.4)
    };
    let bm = 0.22; // margin ratio within button
    painter.line(
        back_rect.x + back_rect.w * (1.0 - bm), back_rect.y + back_rect.h * bm,
        back_rect.x + back_rect.w * bm, back_rect.center_y(),
        2.0 * s, back_color,
    );
    painter.line(
        back_rect.x + back_rect.w * bm, back_rect.center_y(),
        back_rect.x + back_rect.w * (1.0 - bm), back_rect.y + back_rect.h * (1.0 - bm),
        2.0 * s, back_color,
    );

    // ── Forward button ─────────────────────────────────────────────────────
    let forward_color = if app.can_go_forward() {
        if forward_hovered { palette.text } else { palette.text_secondary }
    } else {
        palette.muted.with_alpha(0.4)
    };
    painter.line(
        forward_rect.x + forward_rect.w * bm, forward_rect.y + forward_rect.h * bm,
        forward_rect.x + forward_rect.w * (1.0 - bm), forward_rect.center_y(),
        2.0 * s, forward_color,
    );
    painter.line(
        forward_rect.x + forward_rect.w * (1.0 - bm), forward_rect.center_y(),
        forward_rect.x + forward_rect.w * bm, forward_rect.y + forward_rect.h * (1.0 - bm),
        2.0 * s, forward_color,
    );

    // ── Up button ──────────────────────────────────────────────────────────
    let up_color = if app.can_go_up() {
        if up_hovered { palette.text } else { palette.text_secondary }
    } else {
        palette.muted.with_alpha(0.4)
    };
    painter.line(
        up_rect.x + up_rect.w * bm, up_rect.center_y(),
        up_rect.center_x(), up_rect.y + up_rect.h * bm,
        2.0 * s, up_color,
    );
    painter.line(
        up_rect.center_x(), up_rect.y + up_rect.h * bm,
        up_rect.x + up_rect.w * (1.0 - bm), up_rect.center_y(),
        2.0 * s, up_color,
    );

    // Vertical divider before path
    painter.rect_filled(
        Rect::new(up_rect.x + up_rect.w + 4.0 * s, nav_rect.y + 12.0 * s, 1.0, 24.0 * s),
        0.0,
        Color::WHITE.with_alpha(0.08),
    );

    // ── Path bar / Search bar ────────────────────────────────────────────
    if app.searching {
        TextInput::new(path_rect)
            .text(&app.search_buf)
            .placeholder("Search files...")
            .cursor_pos(app.search_cursor)
            .focused(true)
            .scale(s)
            .draw(painter, text, palette, screen.0, screen.1);
    } else if app.path_editing {
        TextInput::new(path_rect)
            .text(&app.path_buf)
            .cursor_pos(app.path_cursor)
            .selection(app.path_selection)
            .focused(true)
            .scale(s)
            .draw(painter, text, palette, screen.0, screen.1);
    } else {
        // Breadcrumb path bar
        let segments = breadcrumb_segments(&app.current_dir, s);
        let font = 22.0 * s;
        let char_w = font * 0.45;
        let sep_w = 14.0 * s;
        let pad_x = 6.0 * s;
        let seg_width = |name: &str| -> f32 { name.len() as f32 * char_w + pad_x * 2.0 };
        let text_y = path_rect.y + (path_rect.h - font) * 0.5;

        // Compute overflow skip (must match render.rs exactly)
        let total_w: f32 = segments.iter().enumerate().map(|(i, (name, _))| {
            if i > 0 { seg_width(name) + sep_w } else { seg_width(name) }
        }).sum();
        let mut skip = 0;
        if total_w > path_rect.w {
            let ellipsis_w = seg_width("...") + sep_w;
            for (i, _) in segments.iter().enumerate() {
                let remaining: f32 = segments[i..].iter().enumerate().map(|(j, (n, _))| {
                    if j > 0 { seg_width(n) + sep_w } else { seg_width(n) }
                }).sum();
                if ellipsis_w + remaining <= path_rect.w { break; }
                skip = i + 1;
            }
        }

        let mut cx = path_rect.x + 4.0 * s;

        if skip > 0 {
            TextLabel::new("...", cx + pad_x, text_y)
                .size(FontSize::Custom(font))
                .color(palette.muted)
                .draw(text, screen.0, screen.1);
            cx += seg_width("...");
            let sep_x = cx + (sep_w - char_w) * 0.5;
            TextLabel::new("/", sep_x, text_y)
                .size(FontSize::Custom(font))
                .color(palette.muted.with_alpha(0.3))
                .draw(text, screen.0, screen.1);
            cx += sep_w;
        }

        for (i, (name, _)) in segments.iter().enumerate() {
            if i < skip { continue; }
            if i > skip {
                let sep_x = cx + (sep_w - char_w) * 0.5;
                TextLabel::new("/", sep_x, text_y)
                    .size(FontSize::Custom(font))
                    .color(palette.muted.with_alpha(0.3))
                    .draw(text, screen.0, screen.1);
                cx += sep_w;
            }

            let sw = seg_width(name);
            let is_last = i == segments.len() - 1;
            let hover_idx = i - skip;
            let hovered = breadcrumb_hovered.get(hover_idx).copied().unwrap_or(false);

            if hovered {
                painter.rect_filled(
                    Rect::new(cx, path_rect.y + 2.0 * s, sw, path_rect.h - 4.0 * s),
                    4.0 * s,
                    palette.surface_2.with_alpha(0.4),
                );
            }

            let color = if is_last { palette.text } else { palette.text_secondary };
            TextLabel::new(name, cx + pad_x, text_y)
                .size(FontSize::Custom(font))
                .color(color)
                .draw(text, screen.0, screen.1);

            cx += sw;
        }
    }

    // ── Preview pane toggle ───────────────────────────────────────────────
    if preview_supported {
        let pv_active = app.preview_open;
        let pv_color = if pv_active { palette.accent }
            else if preview_hovered { palette.text }
            else { palette.text_secondary };
        if preview_hovered || pv_active {
            let bg = if pv_active { palette.accent.with_alpha(0.15) } else { palette.surface_2.with_alpha(0.5) };
            painter.rect_filled(preview_rect, 4.0 * s, bg);
        }
        draw_preview_pane_icon(painter, preview_rect, pv_color, s);
    } else {
        // In Grid mode, fade the icon to indicate it's not available.
        let pv_color = palette.muted.with_alpha(0.4);
        draw_preview_pane_icon(painter, preview_rect, pv_color, s);
    }

    // ── Sort button ────────────────────────────────────────────────────────
    let sort_color = if sort_hovered { palette.text } else { palette.text_secondary };
    if sort_hovered {
        painter.rect_filled(sort_rect, 4.0 * s, palette.surface_2.with_alpha(0.5));
    }
    draw_sort_icon(painter, sort_rect, sort_color, app.sort_dir, s);

    // ── Search button ──────────────────────────────────────────────────────
    let search_active = app.searching;
    let search_color = if search_active { palette.accent } else if search_hovered { palette.text } else { palette.text_secondary };
    if search_hovered || search_active {
        let bg = if search_active { palette.accent.with_alpha(0.15) } else { palette.surface_2.with_alpha(0.5) };
        painter.rect_filled(search_rect, 4.0 * s, bg);
    }
    let sx = search_rect.center_x() - 2.0 * s;
    let sy = search_rect.center_y() - 2.0 * s;
    painter.circle_stroke(sx, sy, 6.0 * s, 1.5 * s, search_color);
    painter.line(sx + 4.5 * s, sy + 4.5 * s, sx + 9.0 * s, sy + 9.0 * s, 2.0 * s, search_color);
}
