use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    FontSize, FoxPalette, GradientStrip, InteractionState, ScrollArea, Scrollbar, TextInput,
    TextLabel,
};

/// Blend accent toward warning to get a warmer amber-gold for selection highlights.
/// Pure accent (rgb 200,134,10) reads orange at low alpha on dark bg.
pub fn selection_tint(palette: &FoxPalette) -> Color {
    let a = palette.accent;
    let w = palette.warning;
    Color {
        r: a.r * 0.7 + w.r * 0.3,
        g: a.g * 0.7 + w.g * 0.3,
        b: a.b * 0.7 + w.b * 0.3,
        a: 1.0,
    }
}

use crate::{
    app::{App, ViewMode},
    fs::FileEntry,
    layout::{icon_size, item_size, sidebar_w},
};

// ── Gradient separators ─────────────────────────────────────────────────────

pub fn draw_gradient_h(painter: &mut Painter, palette: &FoxPalette, x: f32, y: f32, width: f32, s: f32) {
    let mut bar = GradientStrip::new(x, y, width);
    bar.height = 4.0 * s;
    bar.colors = palette.file_manager_gradient_stops();
    bar.draw(painter);
}

pub fn draw_gradient_v(painter: &mut Painter, palette: &FoxPalette, x: f32, y: f32, height: f32, s: f32) {
    let colors = palette.file_manager_gradient_stops();
    let w = 4.0 * s;
    let segments = height.max(1.0).ceil() as usize;
    let step = height / segments as f32;
    for i in 0..segments {
        let sy = y + i as f32 * step;
        let sh = if i + 1 == segments { y + height - sy } else { step };
        let t = i as f32 / segments as f32;
        let color = sample_gradient_5(&colors, t);
        painter.rect_filled(Rect::new(x, sy, w, sh), 0.0, color);
    }
}

fn sample_gradient_5(colors: &[Color; 5], t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let stops = [0.0_f32, 0.25, 0.50, 0.75, 1.0];
    for i in 0..4 {
        if t <= stops[i + 1] {
            let local = (t - stops[i]) / (stops[i + 1] - stops[i]);
            return lerp_color(colors[i], colors[i + 1], local);
        }
    }
    colors[4]
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

// ── Sidebar place icons ─────────────────────────────────────────────────────

fn draw_place_icon(painter: &mut Painter, name: &str, cx: f32, cy: f32, color: Color, s: f32) {
    let sw = 1.5 * s; // stroke width
    let u = s; // unit scale
    match name {
        "Home" => {
            painter.line(cx - 7.0*u, cy + 1.0*u, cx, cy - 7.0*u, sw, color);
            painter.line(cx, cy - 7.0*u, cx + 7.0*u, cy + 1.0*u, sw, color);
            painter.rect_stroke(Rect::new(cx - 5.0*u, cy + 1.0*u, 10.0*u, 7.0*u), 0.0, sw, color);
        }
        "Desktop" => {
            painter.rect_stroke(Rect::new(cx - 7.0*u, cy - 6.0*u, 14.0*u, 10.0*u), 1.0*u, sw, color);
            painter.line(cx, cy + 4.0*u, cx, cy + 7.0*u, sw, color);
            painter.line(cx - 4.0*u, cy + 7.0*u, cx + 4.0*u, cy + 7.0*u, sw, color);
        }
        "Documents" => {
            painter.rect_stroke(Rect::new(cx - 5.0*u, cy - 7.0*u, 10.0*u, 14.0*u), 0.0, sw, color);
            painter.line(cx + 5.0*u, cy - 7.0*u, cx + 5.0*u, cy - 3.0*u, sw * 0.75, color);
            painter.line(cx + 1.0*u, cy - 7.0*u, cx + 5.0*u, cy - 3.0*u, sw * 0.75, color);
            painter.line(cx - 3.0*u, cy - 1.0*u, cx + 3.0*u, cy - 1.0*u, 1.0*u, color);
            painter.line(cx - 3.0*u, cy + 2.0*u, cx + 3.0*u, cy + 2.0*u, 1.0*u, color);
            painter.line(cx - 3.0*u, cy + 5.0*u, cx + 1.0*u, cy + 5.0*u, 1.0*u, color);
        }
        "Downloads" => {
            painter.line(cx, cy - 6.0*u, cx, cy + 2.0*u, sw, color);
            painter.line(cx - 4.0*u, cy - 2.0*u, cx, cy + 2.0*u, sw, color);
            painter.line(cx + 4.0*u, cy - 2.0*u, cx, cy + 2.0*u, sw, color);
            painter.line(cx - 6.0*u, cy + 3.0*u, cx - 6.0*u, cy + 7.0*u, sw, color);
            painter.line(cx - 6.0*u, cy + 7.0*u, cx + 6.0*u, cy + 7.0*u, sw, color);
            painter.line(cx + 6.0*u, cy + 3.0*u, cx + 6.0*u, cy + 7.0*u, sw, color);
        }
        "Music" => {
            painter.line(cx - 2.0*u, cy - 6.0*u, cx - 2.0*u, cy + 4.0*u, sw, color);
            painter.circle_filled(cx - 4.0*u, cy + 5.0*u, 3.0*u, color);
            painter.line(cx - 2.0*u, cy - 6.0*u, cx + 4.0*u, cy - 4.0*u, sw, color);
            painter.line(cx + 4.0*u, cy - 4.0*u, cx + 4.0*u, cy + 1.0*u, sw, color);
            painter.circle_filled(cx + 2.0*u, cy + 2.0*u, 2.5*u, color);
        }
        "Pictures" => {
            painter.rect_stroke(Rect::new(cx - 7.0*u, cy - 5.0*u, 14.0*u, 12.0*u), 0.0, sw, color);
            painter.line(cx - 4.0*u, cy + 5.0*u, cx - 1.0*u, cy, sw, color);
            painter.line(cx - 1.0*u, cy, cx + 2.0*u, cy + 3.0*u, sw, color);
            painter.line(cx + 2.0*u, cy + 3.0*u, cx + 5.0*u, cy - 1.0*u, sw, color);
            painter.circle_filled(cx + 3.0*u, cy - 2.0*u, 2.0*u, color);
        }
        "Videos" => {
            painter.rect_stroke(Rect::new(cx - 7.0*u, cy - 5.0*u, 14.0*u, 12.0*u), 1.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 3.0*u, cx - 2.0*u, cy + 5.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 3.0*u, cx + 4.0*u, cy + 1.0*u, sw, color);
            painter.line(cx + 4.0*u, cy + 1.0*u, cx - 2.0*u, cy + 5.0*u, sw, color);
        }
        "Trash" => {
            // Lid
            painter.line(cx - 6.0*u, cy - 5.0*u, cx + 6.0*u, cy - 5.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 7.0*u, cx + 2.0*u, cy - 7.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 7.0*u, cx - 2.0*u, cy - 5.0*u, sw * 0.75, color);
            painter.line(cx + 2.0*u, cy - 7.0*u, cx + 2.0*u, cy - 5.0*u, sw * 0.75, color);
            // Body (tapered)
            painter.line(cx - 5.0*u, cy - 4.0*u, cx - 4.0*u, cy + 7.0*u, sw, color);
            painter.line(cx + 5.0*u, cy - 4.0*u, cx + 4.0*u, cy + 7.0*u, sw, color);
            painter.line(cx - 4.0*u, cy + 7.0*u, cx + 4.0*u, cy + 7.0*u, sw, color);
            // Ribs
            painter.line(cx - 2.0*u, cy - 2.0*u, cx - 2.0*u, cy + 5.0*u, sw * 0.75, color);
            painter.line(cx, cy - 2.0*u, cx, cy + 5.0*u, sw * 0.75, color);
            painter.line(cx + 2.0*u, cy - 2.0*u, cx + 2.0*u, cy + 5.0*u, sw * 0.75, color);
        }
        _ => {
            painter.rect_filled(Rect::new(cx - 8.0*u, cy - 2.0*u, 16.0*u, 10.0*u), 2.0*u, color);
            painter.rect_filled(Rect::new(cx - 8.0*u, cy - 4.0*u, 8.0*u, 4.0*u), 1.0*u, color);
        }
    }
}

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
            .selection(app.path_selection)
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

// ── Sidebar ─────────────────────────────────────────────────────────────────

pub fn draw_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    app: &App,
    sidebar_rect: Rect,
    hovered: &[bool],
    drive_hovered: &[bool],
    dragging: bool,
    screen: (u32, u32),
    s: f32,
) {
    let sw = sidebar_w(s);
    let r = 10.0 * s;

    // Sidebar with rounded bottom-left corner
    painter.rect_filled(sidebar_rect, r, palette.sidebar);
    painter.rect_filled(Rect::new(sidebar_rect.x, sidebar_rect.y, r, r), 0.0, palette.sidebar);
    painter.rect_filled(Rect::new(sidebar_rect.x + sidebar_rect.w - r, sidebar_rect.y, r, r), 0.0, palette.sidebar);
    painter.rect_filled(Rect::new(sidebar_rect.x + sidebar_rect.w - r, sidebar_rect.y + sidebar_rect.h - r, r, r), 0.0, palette.sidebar);

    // Vertical gradient divider on right edge
    draw_gradient_v(painter, palette, sw - 4.0 * s, sidebar_rect.y, sidebar_rect.h, s);

    // "PLACES" header
    let mut sy = sidebar_rect.y + 12.0 * s;
    TextLabel::new("PLACES", 14.0 * s, sy)
        .size(FontSize::Custom(20.0 * s))
        .color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    sy += 30.0 * s;

    let item_h = 40.0 * s;
    let places = app.sidebar_places();
    for (index, place) in places.iter().enumerate() {
        let item_rect = Rect::new(4.0 * s, sy, sw - 12.0 * s, item_h);
        let is_active = app.is_active_place(index);
        let is_hovered = hovered.get(index).copied().unwrap_or(false);

        let icon_color = if dragging && is_hovered {
            palette.accent
        } else if is_active {
            palette.accent
        } else if is_hovered {
            palette.text
        } else {
            palette.text_secondary.with_alpha(0.75)
        };
        draw_place_icon(painter, &place.name, 19.0 * s, sy + item_h * 0.5, icon_color, s);

        let text_color = palette.text;
        TextLabel::new(&place.name, 38.0 * s, sy + (item_h - 24.0 * s) * 0.5)
            .size(FontSize::Custom(24.0 * s))
            .color(text_color)
            .max_width(sw - 56.0 * s)
            .draw(text, screen.0, screen.1);

        sy += item_h;

        // Thin divider between items (not after last)
        if index + 1 < places.len() {
            painter.rect_filled(
                Rect::new(14.0 * s, sy, sw - 32.0 * s, 1.5 * s),
                0.0,
                Color::WHITE.with_alpha(0.12),
            );
        }
    }

    // ── Drives / Devices section ────────────────────────────────────
    if !app.drives.is_empty() {
        sy += 20.0 * s;

        // "DEVICES" header
        TextLabel::new("DEVICES", 14.0 * s, sy)
            .size(FontSize::Custom(20.0 * s))
            .color(palette.text_secondary)
            .draw(text, screen.0, screen.1);
        sy += 30.0 * s;

        let drive_item_h = 64.0 * s;
        for (index, drive) in app.drives.iter().enumerate() {
            let item_rect = Rect::new(4.0 * s, sy, sw - 12.0 * s, drive_item_h);
            let is_hovered = drive_hovered.get(index).copied().unwrap_or(false);

            // Drive icon (simple disk shape)
            let icon_color = if is_hovered { palette.text } else { palette.text_secondary.with_alpha(0.75) };
            let icx = 19.0 * s;
            let icy = sy + 18.0 * s;
            draw_drive_icon(painter, icx, icy, icon_color, s);

            // Drive name
            let text_color = if is_hovered { palette.text } else { palette.text };
            TextLabel::new(&drive.name, 38.0 * s, sy + 4.0 * s)
                .size(FontSize::Custom(22.0 * s))
                .color(text_color)
                .max_width(sw - 56.0 * s)
                .draw(text, screen.0, screen.1);

            // Usage text: "XXX GB free of YYY GB"
            let usage_text = format!("{} free of {}", drive.free_display(), drive.total_display());
            TextLabel::new(&usage_text, 38.0 * s, sy + 32.0 * s)
                .size(FontSize::Custom(16.0 * s))
                .color(palette.muted)
                .max_width(sw - 56.0 * s)
                .draw(text, screen.0, screen.1);

            // Usage bar
            let bar_x = 38.0 * s;
            let bar_y = sy + 52.0 * s;
            let bar_w = sw - 52.0 * s;
            let bar_h = 6.0 * s;
            let frac = drive.usage_fraction();

            // Track
            painter.rect_filled(
                Rect::new(bar_x, bar_y, bar_w, bar_h),
                bar_h * 0.5, palette.surface_2,
            );
            // Fill
            let fill_w = (bar_w * frac).max(bar_h);
            let fill_color = if frac > 0.9 {
                palette.danger
            } else if frac > 0.75 {
                palette.warning
            } else {
                palette.accent
            };
            painter.rect_filled(
                Rect::new(bar_x, bar_y, fill_w, bar_h),
                bar_h * 0.5, fill_color,
            );

            sy += drive_item_h;

            // Divider between drives
            if index + 1 < app.drives.len() {
                painter.rect_filled(
                    Rect::new(14.0 * s, sy, sw - 32.0 * s, 1.5 * s),
                    0.0,
                    Color::WHITE.with_alpha(0.12),
                );
            }
        }
    }
}

fn draw_drive_icon(painter: &mut Painter, cx: f32, cy: f32, color: Color, s: f32) {
    let sw = 1.5 * s;
    let u = s;
    // Simple disk/drive shape
    painter.rect_stroke(
        Rect::new(cx - 8.0*u, cy - 5.0*u, 16.0*u, 10.0*u),
        2.0*u, sw, color,
    );
    // Drive bay lines
    painter.line(cx - 5.0*u, cy - 1.0*u, cx + 5.0*u, cy - 1.0*u, 1.0*u, color);
    painter.line(cx - 5.0*u, cy + 2.0*u, cx + 5.0*u, cy + 2.0*u, 1.0*u, color);
    // Activity LED dot
    painter.circle_filled(cx + 5.0*u, cy - 3.0*u, 1.5*u, color);
}

// ── Content grid ────────────────────────────────────────────────────────────

pub fn draw_content_grid(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    content_rect: Rect,
    entries: &[FileEntry],
    cols: usize,
    area: &ScrollArea,
    hovered: &[bool],
    has_icon: &[bool],
    drag_item: Option<usize>,
    renaming: Option<usize>,
    screen: (u32, u32),
    s: f32,
    zoom: f32,
) {
    let isz = item_size(s, zoom);
    let icsz = icon_size(s, zoom);
    let pad = 8.0 * s;

    painter.rect_filled(content_rect, 0.0, palette.bg);
    area.begin(painter, text);

    let base_y = area.content_y();
    let content_top = content_rect.y;
    let content_bottom = content_rect.y + content_rect.h;

    for (index, entry) in entries.iter().enumerate() {
        let is_dragging = drag_item == Some(index);

        let col = index % cols.max(1);
        let row = index / cols.max(1);
        let x = content_rect.x + pad + col as f32 * (isz + pad);
        let y = base_y + pad + row as f32 * (isz + pad);

        if y + isz < content_top || y > content_bottom {
            continue;
        }

        // Dim the item being dragged but keep it in place (ghost follows cursor)
        let alpha = if is_dragging { 0.3 } else { 1.0 };


        let item_rect = Rect::new(x, y, isz, isz);
        if entry.selected {
            let tint = selection_tint(palette);
            painter.rect_filled(item_rect, 6.0 * s, tint.with_alpha(0.25 * alpha));
            painter.rect_stroke(item_rect, 6.0 * s, 1.0 * s, tint.with_alpha(0.5 * alpha));
        } else if hovered.get(index).copied().unwrap_or(false) {
            painter.rect_filled(item_rect, 6.0 * s, palette.surface_2.with_alpha(0.4));
        }

        // Only draw procedural icons when no texture icon is available
        let label_font = 16.0 * s;
        let content_h = icsz + 2.0 * s + label_font;
        let top_pad = (isz - content_h) * 0.5;
        if !has_icon.get(index).copied().unwrap_or(false) {
            let icon_x = x + (isz - icsz) * 0.5;
            let icon_y = y + top_pad;
            if entry.is_dir {
                let body = Rect::new(icon_x, icon_y + 8.0 * s, icsz, icsz - 12.0 * s);
                painter.rect_filled(body, 4.0 * s, palette.accent.with_alpha(0.3 * alpha));
                painter.rect_stroke(body, 4.0 * s, 1.5 * s, palette.accent.with_alpha(alpha));
                let tab = Rect::new(icon_x, icon_y + 4.0 * s, icsz * 0.45, 8.0 * s);
                painter.rect_filled(tab, 2.0 * s, palette.accent.with_alpha(0.3 * alpha));
                painter.rect_stroke(tab, 2.0 * s, 1.5 * s, palette.accent.with_alpha(alpha));
            } else {
                let page = Rect::new(icon_x + 4.0 * s, icon_y + 2.0 * s, icsz - 8.0 * s, icsz - 4.0 * s);
                painter.rect_filled(page, 3.0 * s, Color::from_rgb8(72, 72, 72).with_alpha(alpha));
                painter.rect_stroke(page, 3.0 * s, 1.5 * s, Color::from_rgb8(102, 102, 102).with_alpha(alpha));
                let fold_x = icon_x + icsz - 16.0 * s;
                let fold_y = icon_y + 2.0 * s;
                painter.rect_filled(Rect::new(fold_x, fold_y, 12.0 * s, 12.0 * s), 0.0, palette.surface.with_alpha(alpha));
                painter.line(fold_x, fold_y, fold_x, fold_y + 12.0 * s, 1.0 * s, Color::from_rgb8(102, 102, 102).with_alpha(alpha));
                painter.line(fold_x, fold_y + 12.0 * s, fold_x + 12.0 * s, fold_y + 12.0 * s, 1.0 * s, Color::from_rgb8(102, 102, 102).with_alpha(alpha));
                let lx = icon_x + 10.0 * s;
                let lw = icsz - 24.0 * s;
                painter.line(lx, icon_y + 20.0*s, lx + lw, icon_y + 20.0*s, 1.5*s, palette.accent.with_alpha(0.6 * alpha));
                painter.line(lx, icon_y + 26.0*s, lx + lw*0.8, icon_y + 26.0*s, 1.5*s, Color::from_rgb8(224, 90, 138).with_alpha(0.6 * alpha));
                painter.line(lx, icon_y + 32.0*s, lx + lw*0.6, icon_y + 32.0*s, 1.5*s, Color::from_rgb8(155, 93, 229).with_alpha(0.6 * alpha));
            }
        }

        let label_y = y + top_pad + icsz + 2.0 * s;
        let min_margin = 2.0 * s;
        let max_label_w = isz - min_margin * 2.0;
        let char_w = label_font * 0.52;

        // Skip label for item being renamed (TextInput draws instead)
        if renaming == Some(index) { continue; }

        // Only draw text if the label is within the visible content area
        if label_y >= content_top && label_y + label_font <= content_bottom {
            let display_name = truncate_with_ellipsis(&entry.name, max_label_w, char_w);
            let est_w = display_name.len() as f32 * char_w;
            let label_x = (x + (isz - est_w) * 0.5).max(x + min_margin);
            TextLabel::new(&display_name, label_x, label_y)
                .size(FontSize::Custom(label_font))
                .color(if entry.selected { palette.accent.with_alpha(alpha) } else { palette.text.with_alpha(alpha) })
                .max_width(max_label_w)
                .draw(text, screen.0, screen.1);
        }

    }

    area.end(painter, text);
}

// ── Rubber band selection ────────────────────────────────────────────────────

pub fn draw_rubber_band(
    painter: &mut Painter,
    palette: &FoxPalette,
    start: (f32, f32),
    end: (f32, f32),
    content_rect: Rect,
) {
    let x0 = start.0.min(end.0).max(content_rect.x);
    let y0 = start.1.min(end.1).max(content_rect.y);
    let x1 = start.0.max(end.0).min(content_rect.x + content_rect.w);
    let y1 = start.1.max(end.1).min(content_rect.y + content_rect.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let r = Rect::new(x0, y0, x1 - x0, y1 - y0);
    painter.rect_filled(r, 2.0, selection_tint(palette).with_alpha(0.18));
    painter.rect_stroke(r, 2.0, 1.0, palette.accent.with_alpha(0.6));
}

// ── Scrollbar ───────────────────────────────────────────────────────────────

pub fn draw_scrollbar(
    painter: &mut Painter,
    scrollbar: &Scrollbar,
    state: InteractionState,
    palette: &FoxPalette,
) {
    scrollbar.draw(painter, state, palette);
}

// ── Status bar ──────────────────────────────────────────────────────────────

pub fn draw_status_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    status_rect: Rect,
    entries: &[FileEntry],
    file_info: &mut crate::file_info::FileInfoCache,
    screen: (u32, u32),
    s: f32,
) {
    let r = 10.0 * s;
    painter.rect_filled(status_rect, r, palette.surface);
    painter.rect_filled(Rect::new(status_rect.x, status_rect.y, r, r), 0.0, palette.surface);
    painter.rect_filled(Rect::new(status_rect.x + status_rect.w - r, status_rect.y, r, r), 0.0, palette.surface);
    painter.rect_filled(Rect::new(0.0, status_rect.y, status_rect.w, 1.0), 0.0, palette.bg);

    let total = entries.len();
    let dirs = entries.iter().filter(|e| e.is_dir).count();
    let files = total - dirs;
    let selected: Vec<&FileEntry> = entries.iter().filter(|e| e.selected).collect();
    let sel_count = selected.len();
    let sel_bytes: u64 = selected.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();

    let font = FontSize::Custom(20.0 * s);
    let dot_sep = " \u{2022} ";
    let mut x = 12.0 * s;
    let y = status_rect.y + 4.0 * s;
    let cw = 9.0 * s; // approximate char width

    let counts = format!("{dirs} folders, {files} files");
    TextLabel::new(&counts, x, y)
        .size(font)
        .color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    x += counts.len() as f32 * cw;

    if sel_count > 0 {
        // Dot separator
        TextLabel::new(dot_sep, x, y)
            .size(font)
            .color(palette.muted.with_alpha(0.5))
            .draw(text, screen.0, screen.1);
        x += 24.0 * s;

        let sel_text = format!("{sel_count} selected");
        TextLabel::new(&sel_text, x, y)
            .size(font)
            .color(palette.accent)
            .draw(text, screen.0, screen.1);
        x += sel_text.len() as f32 * cw + 6.0 * s;

        let size_text = format!("({})", format_bytes(sel_bytes));
        TextLabel::new(&size_text, x, y)
            .size(font)
            .color(palette.muted)
            .draw(text, screen.0, screen.1);
        x += size_text.len() as f32 * cw;

        // Single file selected — show detailed info
        if sel_count == 1 && !selected[0].is_dir {
            let info = file_info.get(&selected[0].path);

            // File type
            TextLabel::new(dot_sep, x, y)
                .size(font)
                .color(palette.muted.with_alpha(0.5))
                .draw(text, screen.0, screen.1);
            x += 24.0 * s;

            TextLabel::new(&info.type_name, x, y)
                .size(font)
                .color(palette.text_secondary)
                .draw(text, screen.0, screen.1);
            x += info.type_name.len() as f32 * cw;

            // Dimensions (images and video)
            if let Some((w, h)) = info.dimensions {
                TextLabel::new(dot_sep, x, y)
                    .size(font)
                    .color(palette.muted.with_alpha(0.5))
                    .draw(text, screen.0, screen.1);
                x += 24.0 * s;

                let dims = format!("{w}\u{00D7}{h}");
                TextLabel::new(&dims, x, y)
                    .size(font)
                    .color(palette.text_secondary)
                    .draw(text, screen.0, screen.1);
                x += dims.len() as f32 * cw;
            }

            // Duration (audio and video)
            if let Some(ref dur) = info.duration {
                TextLabel::new(dot_sep, x, y)
                    .size(font)
                    .color(palette.muted.with_alpha(0.5))
                    .draw(text, screen.0, screen.1);
                x += 24.0 * s;

                TextLabel::new(dur, x, y)
                    .size(font)
                    .color(palette.text_secondary)
                    .draw(text, screen.0, screen.1);
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub fn truncate_with_ellipsis(name: &str, max_w: f32, char_w: f32) -> String {
    let est_w = name.len() as f32 * char_w;
    if est_w <= max_w {
        return name.to_string();
    }
    let ellipsis_w = 3.0 * char_w; // "…"
    let max_chars = ((max_w - ellipsis_w) / char_w).floor().max(1.0) as usize;
    let truncated: String = name.chars().take(max_chars).collect();
    format!("{truncated}\u{2026}")
}

fn format_bytes(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let size_f = size as f64;
    if size_f >= GB {
        format!("{:.1} GB", size_f / GB)
    } else if size_f >= MB {
        format!("{:.1} MB", size_f / MB)
    } else if size_f >= KB {
        format!("{:.0} KB", size_f / KB)
    } else {
        format!("{} B", size)
    }
}
