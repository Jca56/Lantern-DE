use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::app::App;
use crate::layout::{sidebar_w, SidebarLayout};

use super::draw_gradient_v;
use super::icons::{draw_drive_icon, draw_phone_icon, draw_place_icon};

// ── Sidebar ─────────────────────────────────────────────────────────────────

pub struct SidebarHovered<'a> {
    pub places: &'a [bool],
    pub favorites: &'a [bool],
    pub drives: &'a [bool],
    pub phones: &'a [bool],
    pub places_header: bool,
    pub favorites_header: bool,
    pub devices_header: bool,
    pub favorites_plus: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn draw_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    app: &App,
    sidebar_rect: Rect,
    layout: &SidebarLayout,
    hov: &SidebarHovered<'_>,
    dragging: bool,
    fav_drag: Option<usize>,
    screen: (u32, u32),
    s: f32,
) {
    let sw = sidebar_w(s);

    // Sidebar bg intentionally not painted — the window bg already covers
    // this region. Painting `palette.sidebar` here would compound the alpha
    // and read as opaque under transparency. The gradient strip on the
    // right edge still provides a clear visual separator from the content.
    draw_gradient_v(painter, palette, sw - 4.0 * s, sidebar_rect.y, sidebar_rect.h, s);

    // ── PLACES section ───────────────────────────────────────────────
    draw_section_header(
        painter, text, palette,
        "PLACES", layout.places_header,
        app.places_collapsed, hov.places_header,
        screen, s,
    );
    if !app.places_collapsed {
        let places = app.sidebar_places();
        for (i, place) in places.iter().enumerate() {
            let r = layout.place_items[i];
            let is_active = app.is_active_place(i);
            let is_hovered = hov.places.get(i).copied().unwrap_or(false);
            draw_place_row(
                painter, text, palette,
                &place.name, r, is_active, is_hovered, dragging,
                screen, s,
            );
            if i + 1 < places.len() {
                draw_divider(painter, r.y + r.h, sw, s);
            }
        }
    }

    // ── FAVORITES section ────────────────────────────────────────────
    draw_section_header(
        painter, text, palette,
        "FAVORITES", layout.favorites_header,
        app.favorites_collapsed, hov.favorites_header,
        screen, s,
    );
    draw_plus_button(painter, palette, layout.favorites_plus, hov.favorites_plus, s);
    if !app.favorites_collapsed {
        let favorites = app.sidebar_favorites();
        if favorites.is_empty() {
            // Hint text when empty so the section reads as intentional.
            let header = layout.favorites_header;
            TextLabel::new(
                "Drag folders here or use \u{002B}",
                14.0 * s,
                header.y + header.h + 6.0 * s,
            )
            .size(FontSize::Custom(16.0 * s))
            .color(palette.muted)
            .max_width(sw - 28.0 * s)
            .draw(text, screen.0, screen.1);
        }
        for (i, fav) in favorites.iter().enumerate() {
            let r = layout.favorite_items[i];
            let is_active = app.is_active_favorite(i);
            let is_hovered = hov.favorites.get(i).copied().unwrap_or(false);
            let is_drag_source = fav_drag == Some(i);
            let is_drop_target = fav_drag.is_some() && fav_drag != Some(i) && is_hovered;
            // Drop-target highlight: a tinted accent pill behind the row.
            if is_drop_target {
                painter.rect_filled(r, 6.0 * s, palette.accent.with_alpha(0.22));
            }
            draw_place_row(
                painter, text, palette,
                &fav.name, r, is_active, is_hovered, dragging,
                screen, s,
            );
            // Drag source: overlay a translucent veil to read as "lifted".
            if is_drag_source {
                painter.rect_filled(r, 0.0, Color::from_rgba8(0, 0, 0, 96));
            }
            if i + 1 < favorites.len() {
                draw_divider(painter, r.y + r.h, sw, s);
            }
        }
    }

    // ── DEVICES section ──────────────────────────────────────────────
    if !layout.has_devices { return; }
    draw_section_header(
        painter, text, palette,
        "DEVICES", layout.devices_header,
        app.devices_collapsed, hov.devices_header,
        screen, s,
    );
    if !app.devices_collapsed {
        for (i, drive) in app.drives.iter().enumerate() {
            let r = layout.drive_items[i];
            let is_hovered = hov.drives.get(i).copied().unwrap_or(false);
            draw_drive_row(painter, text, palette, drive, r, is_hovered, screen, s);
            let has_more = i + 1 < app.drives.len() || !app.phones.is_empty();
            if has_more {
                draw_divider(painter, r.y + r.h, sw, s);
            }
        }
        for (i, phone) in app.phones.iter().enumerate() {
            let r = layout.phone_items[i];
            let is_hovered = hov.phones.get(i).copied().unwrap_or(false);
            draw_phone_row(painter, text, palette, phone, r, is_hovered, screen, s);
            if i + 1 < app.phones.len() {
                draw_divider(painter, r.y + r.h, sw, s);
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn draw_section_header(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    label: &str,
    rect: Rect,
    collapsed: bool,
    hovered: bool,
    screen: (u32, u32),
    s: f32,
) {
    let _ = painter;
    // Section headers follow the theme accent; hover brightens to full text
    // so the collapse affordance still reads.
    let color = if hovered { palette.text } else { palette.accent };
    // Chevron sits to the left of the label so the click target reads as a
    // disclosure toggle on the whole header row.
    let chev_x = 14.0 * s;
    let chev_y = rect.y + rect.h * 0.5;
    draw_chevron(painter, chev_x, chev_y, !collapsed, color, s);
    TextLabel::new(label, 32.0 * s, rect.y + (rect.h - 22.0 * s) * 0.5)
        .size(FontSize::Custom(22.0 * s))
        .color(color)
        .draw(text, screen.0, screen.1);
}

/// ▾ when expanded, ▸ when collapsed. Drawn as filled triangles via three
/// thin rects (we already have rect_filled — saves dragging in a triangle
/// primitive just for this).
fn draw_chevron(painter: &mut Painter, cx: f32, cy: f32, expanded: bool, color: Color, s: f32) {
    let size = 8.0 * s;
    let half = size * 0.5;
    // Approximate triangles with stacked horizontal/vertical bars. Cheap and
    // looks clean at the small sizes we use.
    if expanded {
        // ▾ — pointing down
        for i in 0..5 {
            let row_w = size - i as f32 * (size / 5.0);
            let y = cy - half + i as f32 * (size / 5.0);
            let x = cx - row_w * 0.5;
            painter.rect_filled(Rect::new(x, y, row_w, size / 5.0), 0.0, color);
        }
    } else {
        // ▸ — pointing right
        for i in 0..5 {
            let col_h = size - i as f32 * (size / 5.0);
            let x = cx - half + i as f32 * (size / 5.0);
            let y = cy - col_h * 0.5;
            painter.rect_filled(Rect::new(x, y, size / 5.0, col_h), 0.0, color);
        }
    }
}

fn draw_plus_button(painter: &mut Painter, palette: &FoxPalette, rect: Rect, hovered: bool, s: f32) {
    let bg = if hovered {
        palette.accent.with_alpha(0.18)
    } else {
        Color::WHITE.with_alpha(0.06)
    };
    let fg = if hovered { palette.accent } else { palette.text_secondary };
    painter.rect_filled(rect, rect.h * 0.5, bg);
    let arm = 12.0 * s;
    let thick = 2.5 * s;
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    painter.rect_filled(Rect::new(cx - arm * 0.5, cy - thick * 0.5, arm, thick), thick * 0.5, fg);
    painter.rect_filled(Rect::new(cx - thick * 0.5, cy - arm * 0.5, thick, arm), thick * 0.5, fg);
}

fn draw_divider(painter: &mut Painter, y: f32, sw: f32, s: f32) {
    painter.rect_filled(
        Rect::new(14.0 * s, y, sw - 32.0 * s, 1.5 * s),
        0.0,
        Color::WHITE.with_alpha(0.12),
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_place_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    name: &str,
    rect: Rect,
    is_active: bool,
    is_hovered: bool,
    dragging: bool,
    screen: (u32, u32),
    s: f32,
) {
    let _ = painter;
    let sw = sidebar_w(s);
    let icon_color = if dragging && is_hovered {
        palette.accent
    } else if is_active {
        palette.accent
    } else if is_hovered {
        palette.text
    } else {
        palette.text_secondary.with_alpha(0.75)
    };
    draw_place_icon(painter, name, 19.0 * s, rect.y + rect.h * 0.5, icon_color, s);
    TextLabel::new(name, 38.0 * s, rect.y + (rect.h - 26.0 * s) * 0.5)
        .size(FontSize::Custom(26.0 * s))
        .color(palette.text)
        .max_width(sw - 56.0 * s)
        .draw(text, screen.0, screen.1);
}

fn draw_drive_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    drive: &crate::fs::Drive,
    rect: Rect,
    is_hovered: bool,
    screen: (u32, u32),
    s: f32,
) {
    let sw = sidebar_w(s);
    let icon_color = if is_hovered { palette.text } else { palette.text_secondary.with_alpha(0.75) };
    let icx = 19.0 * s;
    let icy = rect.y + 18.0 * s;
    draw_drive_icon(painter, icx, icy, icon_color, s);

    TextLabel::new(&drive.name, 38.0 * s, rect.y + 4.0 * s)
        .size(FontSize::Custom(22.0 * s))
        .color(palette.text)
        .max_width(sw - 56.0 * s)
        .draw(text, screen.0, screen.1);

    if drive.mounted {
        let usage_text = format!("{} free of {}", drive.free_display(), drive.total_display());
        TextLabel::new(&usage_text, 38.0 * s, rect.y + 32.0 * s)
            .size(FontSize::Custom(16.0 * s))
            .color(palette.muted)
            .max_width(sw - 56.0 * s)
            .draw(text, screen.0, screen.1);

        let bar_x = 38.0 * s;
        let bar_y = rect.y + 52.0 * s;
        let bar_w = sw - 52.0 * s;
        let bar_h = 6.0 * s;
        let frac = drive.usage_fraction();
        painter.rect_filled(Rect::new(bar_x, bar_y, bar_w, bar_h), bar_h * 0.5, palette.surface_2);
        let fill_w = (bar_w * frac).max(bar_h);
        let fill_color = if frac > 0.9 {
            palette.danger
        } else if frac > 0.75 {
            palette.warning
        } else {
            palette.accent
        };
        painter.rect_filled(Rect::new(bar_x, bar_y, fill_w, bar_h), bar_h * 0.5, fill_color);
    } else {
        let subtitle = format!("Tap to mount \u{00B7} {} \u{00B7} {}", drive.total_display(), drive.fstype);
        TextLabel::new(&subtitle, 38.0 * s, rect.y + 32.0 * s)
            .size(FontSize::Custom(16.0 * s))
            .color(palette.accent)
            .max_width(sw - 56.0 * s)
            .draw(text, screen.0, screen.1);
    }
}

fn draw_phone_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    phone: &crate::fs::Phone,
    rect: Rect,
    is_hovered: bool,
    screen: (u32, u32),
    s: f32,
) {
    let sw = sidebar_w(s);
    let icon_color = if is_hovered { palette.text } else { palette.text_secondary.with_alpha(0.75) };
    draw_phone_icon(painter, 19.0 * s, rect.y + 24.0 * s, icon_color, s);

    TextLabel::new(&phone.name, 38.0 * s, rect.y + 8.0 * s)
        .size(FontSize::Custom(22.0 * s))
        .color(palette.text)
        .max_width(sw - 56.0 * s)
        .draw(text, screen.0, screen.1);

    let status = if crate::fs::is_path_mounted(&phone.mount_point) {
        "Connected"
    } else {
        "Tap to open"
    };
    TextLabel::new(status, 38.0 * s, rect.y + 34.0 * s)
        .size(FontSize::Custom(16.0 * s))
        .color(palette.muted)
        .max_width(sw - 56.0 * s)
        .draw(text, screen.0, screen.1);
}
