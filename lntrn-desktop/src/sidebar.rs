use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::{
    app::App,
    layout::sidebar_w,
    sections::draw_gradient_v,
};

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
    ox: f32,
) {
    let sw = sidebar_w(s);
    let r = 10.0 * s;
    let sx = sidebar_rect.x; // base x from the (already-offset) rect

    // Sidebar with rounded bottom-left corner
    painter.rect_filled(sidebar_rect, r, palette.sidebar);
    painter.rect_filled(Rect::new(sidebar_rect.x, sidebar_rect.y, r, r), 0.0, palette.sidebar);
    painter.rect_filled(Rect::new(sidebar_rect.x + sidebar_rect.w - r, sidebar_rect.y, r, r), 0.0, palette.sidebar);
    painter.rect_filled(Rect::new(sidebar_rect.x + sidebar_rect.w - r, sidebar_rect.y + sidebar_rect.h - r, r, r), 0.0, palette.sidebar);

    // Vertical gradient divider on right edge
    draw_gradient_v(painter, palette, sx + sw - 4.0 * s, sidebar_rect.y, sidebar_rect.h, s, 0.0);

    // "PLACES" header
    let mut sy = sidebar_rect.y + 12.0 * s;
    TextLabel::new("PLACES", sx + 14.0 * s, sy)
        .size(FontSize::Custom(20.0 * s))
        .color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    sy += 30.0 * s;

    let item_h = 40.0 * s;
    let places = app.sidebar_places();
    for (index, place) in places.iter().enumerate() {
        let item_rect = Rect::new(sx + 4.0 * s, sy, sw - 12.0 * s, item_h);
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
        draw_place_icon(painter, &place.name, sx + 19.0 * s, sy + item_h * 0.5, icon_color, s);

        let text_color = palette.text;
        TextLabel::new(&place.name, sx + 38.0 * s, sy + (item_h - 24.0 * s) * 0.5)
            .size(FontSize::Custom(24.0 * s))
            .color(text_color)
            .max_width(sw - 56.0 * s)
            .draw(text, screen.0, screen.1);

        sy += item_h;

        // Thin divider between items (not after last)
        if index + 1 < places.len() {
            painter.rect_filled(
                Rect::new(sx + 14.0 * s, sy, sw - 32.0 * s, 1.5 * s),
                0.0,
                Color::WHITE.with_alpha(0.12),
            );
        }
    }

    // ── Drives / Devices section ────────────────────────────────────
    if !app.drives.is_empty() {
        sy += 20.0 * s;

        // "DEVICES" header
        TextLabel::new("DEVICES", sx + 14.0 * s, sy)
            .size(FontSize::Custom(20.0 * s))
            .color(palette.text_secondary)
            .draw(text, screen.0, screen.1);
        sy += 30.0 * s;

        let drive_item_h = 64.0 * s;
        for (index, drive) in app.drives.iter().enumerate() {
            let item_rect = Rect::new(sx + 4.0 * s, sy, sw - 12.0 * s, drive_item_h);
            let is_hovered = drive_hovered.get(index).copied().unwrap_or(false);

            // Drive icon (simple disk shape)
            let icon_color = if is_hovered { palette.text } else { palette.text_secondary.with_alpha(0.75) };
            let icx = sx + 19.0 * s;
            let icy = sy + 18.0 * s;
            draw_drive_icon(painter, icx, icy, icon_color, s);

            // Drive name
            let text_color = if is_hovered { palette.text } else { palette.text };
            TextLabel::new(&drive.name, sx + 38.0 * s, sy + 4.0 * s)
                .size(FontSize::Custom(22.0 * s))
                .color(text_color)
                .max_width(sw - 56.0 * s)
                .draw(text, screen.0, screen.1);

            // Usage text: "XXX GB free of YYY GB"
            let usage_text = format!("{} free of {}", drive.free_display(), drive.total_display());
            TextLabel::new(&usage_text, sx + 38.0 * s, sy + 32.0 * s)
                .size(FontSize::Custom(16.0 * s))
                .color(palette.muted)
                .max_width(sw - 56.0 * s)
                .draw(text, screen.0, screen.1);

            // Usage bar
            let bar_x = sx + 38.0 * s;
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
                    Rect::new(sx + 14.0 * s, sy, sw - 32.0 * s, 1.5 * s),
                    0.0,
                    Color::WHITE.with_alpha(0.12),
                );
            }
        }
    }
}
