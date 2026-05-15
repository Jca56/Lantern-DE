use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::app::App;
use crate::layout::sidebar_w;

use super::draw_gradient_v;
use super::icons::{draw_drive_icon, draw_phone_icon, draw_place_icon};

// ── Sidebar ─────────────────────────────────────────────────────────────────

pub fn draw_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    app: &App,
    sidebar_rect: Rect,
    hovered: &[bool],
    drive_hovered: &[bool],
    phone_hovered: &[bool],
    dragging: bool,
    screen: (u32, u32),
    s: f32,
) {
    let sw = sidebar_w(s);
    let _ = sidebar_rect;

    // Sidebar bg intentionally not painted — the window bg already covers
    // this region. Painting `palette.sidebar` here would compound the alpha
    // and read as opaque under transparency. The gradient strip on the
    // right edge still provides a clear visual separator from the content.

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
        let _item_rect = Rect::new(4.0 * s, sy, sw - 12.0 * s, item_h);
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
    if !app.drives.is_empty() || !app.phones.is_empty() {
        sy += 20.0 * s;

        // "DEVICES" header
        TextLabel::new("DEVICES", 14.0 * s, sy)
            .size(FontSize::Custom(20.0 * s))
            .color(palette.text_secondary)
            .draw(text, screen.0, screen.1);
        sy += 30.0 * s;

        let drive_item_h = 64.0 * s;
        for (index, drive) in app.drives.iter().enumerate() {
            let _item_rect = Rect::new(4.0 * s, sy, sw - 12.0 * s, drive_item_h);
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

            if drive.mounted {
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
            } else {
                // Unmounted: "Tap to mount · 8.0 GB · vfat"
                let subtitle = format!(
                    "Tap to mount · {} · {}",
                    drive.total_display(),
                    drive.fstype,
                );
                TextLabel::new(&subtitle, 38.0 * s, sy + 32.0 * s)
                    .size(FontSize::Custom(16.0 * s))
                    .color(palette.accent)
                    .max_width(sw - 56.0 * s)
                    .draw(text, screen.0, screen.1);
            }

            sy += drive_item_h;

            // Divider between drives, or before phones
            let has_more = index + 1 < app.drives.len() || !app.phones.is_empty();
            if has_more {
                painter.rect_filled(
                    Rect::new(14.0 * s, sy, sw - 32.0 * s, 1.5 * s),
                    0.0,
                    Color::WHITE.with_alpha(0.12),
                );
            }
        }

        // ── Phones (MTP) ────────────────────────────────────────────
        let phone_item_h = 56.0 * s;
        for (index, phone) in app.phones.iter().enumerate() {
            let is_hovered = phone_hovered.get(index).copied().unwrap_or(false);

            let icon_color = if is_hovered { palette.text } else { palette.text_secondary.with_alpha(0.75) };
            let icx = 19.0 * s;
            let icy = sy + 24.0 * s;
            draw_phone_icon(painter, icx, icy, icon_color, s);

            let text_color = palette.text;
            TextLabel::new(&phone.name, 38.0 * s, sy + 8.0 * s)
                .size(FontSize::Custom(22.0 * s))
                .color(text_color)
                .max_width(sw - 56.0 * s)
                .draw(text, screen.0, screen.1);

            let status = if crate::fs::is_path_mounted(&phone.mount_point) {
                "Connected"
            } else {
                "Tap to open"
            };
            TextLabel::new(status, 38.0 * s, sy + 34.0 * s)
                .size(FontSize::Custom(16.0 * s))
                .color(palette.muted)
                .max_width(sw - 56.0 * s)
                .draw(text, screen.0, screen.1);

            sy += phone_item_h;

            if index + 1 < app.phones.len() {
                painter.rect_filled(
                    Rect::new(14.0 * s, sy, sw - 32.0 * s, 1.5 * s),
                    0.0,
                    Color::WHITE.with_alpha(0.12),
                );
            }
        }
    }
}
