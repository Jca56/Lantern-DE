use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, TextInput};

use crate::app::{App, ViewMode};

// ── View mode icon ──────────────────────────────────────────────────────────

fn draw_view_mode_icon(painter: &mut Painter, mode: ViewMode, r: Rect, color: Color, s: f32) {
    match mode {
        ViewMode::Grid => {
            // 2x2 squares
            let vx = r.x + 8.0 * s;
            let vy = r.y + 8.0 * s;
            let sq = 8.0 * s;
            let gap = 3.0 * s;
            painter.rect_filled(Rect::new(vx, vy, sq, sq), 1.0 * s, color);
            painter.rect_filled(Rect::new(vx + sq + gap, vy, sq, sq), 1.0 * s, color);
            painter.rect_filled(Rect::new(vx, vy + sq + gap, sq, sq), 1.0 * s, color);
            painter.rect_filled(Rect::new(vx + sq + gap, vy + sq + gap, sq, sq), 1.0 * s, color);
        }
        ViewMode::List => {
            // Three horizontal lines with bullet dots
            let lx = r.x + 8.0 * s;
            let ly = r.y + 10.0 * s;
            let lw = 18.0 * s;
            let gap = 6.0 * s;
            for i in 0..3 {
                let y = ly + i as f32 * gap;
                painter.circle_filled(lx + 2.0 * s, y + 1.0 * s, 1.5 * s, color);
                painter.rect_filled(Rect::new(lx + 6.0 * s, y, lw, 2.0 * s), 1.0 * s, color);
            }
        }
        ViewMode::Tree => {
            // Tree structure: vertical line with branches
            let tx = r.x + 10.0 * s;
            let ty = r.y + 8.0 * s;
            let sw = 1.5 * s;
            // Trunk
            painter.line(tx, ty, tx, ty + 18.0 * s, sw, color);
            // Branch 1
            painter.line(tx, ty + 3.0 * s, tx + 8.0 * s, ty + 3.0 * s, sw, color);
            painter.rect_filled(Rect::new(tx + 10.0 * s, ty + 1.0 * s, 8.0 * s, 4.0 * s), 1.0 * s, color);
            // Branch 2
            painter.line(tx, ty + 10.0 * s, tx + 8.0 * s, ty + 10.0 * s, sw, color);
            painter.rect_filled(Rect::new(tx + 10.0 * s, ty + 8.0 * s, 8.0 * s, 4.0 * s), 1.0 * s, color);
            // Branch 3
            painter.line(tx, ty + 17.0 * s, tx + 8.0 * s, ty + 17.0 * s, sw, color);
            painter.rect_filled(Rect::new(tx + 10.0 * s, ty + 15.0 * s, 8.0 * s, 4.0 * s), 1.0 * s, color);
        }
    }
}

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
    path_rect: Rect,
    path_hovered: bool,
    search_rect: Rect,
    search_hovered: bool,
    screen: (u32, u32),
    s: f32,
    _ox: f32,
) {
    painter.rect_filled(nav_rect, 0.0, palette.surface);

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
            .focused(true)
            .scale(s)
            .draw(painter, text, palette, screen.0, screen.1);
    } else {
        let path = app.current_path_display();
        TextInput::new(path_rect)
            .text(&path)
            .placeholder("/")
            .hovered(path_hovered)
            .scale(s)
            .draw(painter, text, palette, screen.0, screen.1);
    }

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
