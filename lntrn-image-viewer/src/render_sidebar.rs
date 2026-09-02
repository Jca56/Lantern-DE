//! Sidebar drawing: header (folder name, names toggle, collapse), the ".."
//! and folder rows, the image tile grid, scrollbar, and the resize grip.
//! Geometry comes from `SidebarLayout` so hit-testing in input.rs agrees.

use std::path::Path;

use lntrn_render::{Color, Painter, Rect, TextRenderer, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, Scrollbar, TextLabel};

use crate::canvas::sidebar::{SidebarEntry, SidebarState};
use crate::canvas::sidebar_layout::{SidebarLayout, NAME_H};
use crate::{
    ZONE_SIDEBAR_ITEM_BASE, ZONE_SIDEBAR_NAMES, ZONE_SIDEBAR_RESIZE, ZONE_SIDEBAR_SCROLLBAR,
    ZONE_SIDEBAR_TOGGLE,
};

/// Per-mode presentation knobs for the shared sidebar.
#[derive(Clone, Copy)]
pub struct SidebarFlavor<'p> {
    /// Show the "+" add-to-canvas badge on hovered tiles (canvas mode).
    pub add_badge: bool,
    /// Tile to outline as the currently open image (viewer mode).
    pub current: Option<&'p Path>,
}

/// Draws the whole sidebar. Returns the rect of the hovered tile's "+" badge,
/// if any — the caller paints it on the overlay layer so it sits above the
/// thumbnail texture.
#[allow(clippy::too_many_arguments)]
pub fn draw_sidebar<'a>(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    sb: &'a SidebarState,
    layout: &SidebarLayout,
    visible: &[usize],
    flavor: SidebarFlavor<'_>,
    tex_draws: &mut Vec<TextureDraw<'a>>,
    palette: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) -> Option<Rect> {
    let side = layout.side;
    painter.rect_filled(side, 0.0, palette.sidebar);
    let edge_x = side.x + side.w;

    if sb.collapsed {
        let st = input.add_zone(ZONE_SIDEBAR_TOGGLE, side);
        if st.is_hovered() {
            painter.rect_filled(side, 0.0, Color::WHITE.with_alpha(0.05));
        }
        let px = FontSize::Small.px() * s;
        let strip = Rect::new(side.x, side.y + 8.0 * s, side.w, px + 16.0 * s);
        centered_label(text, "▶", &strip, px, palette.text_secondary, false, sw, sh);
        painter.line(
            edge_x,
            side.y,
            edge_x,
            side.y + side.h,
            1.0,
            palette.muted.with_alpha(0.25),
        );
        return None;
    }

    draw_header(painter, text, input, sb, layout, palette, s, sw, sh);
    let badge = draw_slots(
        painter, text, input, sb, layout, visible, flavor, tex_draws, palette, s, sw, sh,
    );

    // Scrollbar.
    let bar = Scrollbar::new(&layout.rows_vp, layout.content_h, sb.scroll.offset);
    let bar_state = input.add_zone(ZONE_SIDEBAR_SCROLLBAR, bar.hover_zone());
    bar.draw(painter, bar_state, palette);

    // Resize grip — registered last so it wins over tiles under the band.
    let grip_state = input.add_zone(ZONE_SIDEBAR_RESIZE, layout.grip);
    if sb.resizing || grip_state.is_hovered() {
        painter.line(
            edge_x,
            side.y,
            edge_x,
            side.y + side.h,
            3.0 * s,
            palette.accent.with_alpha(0.9),
        );
    } else {
        painter.line(
            edge_x,
            side.y,
            edge_x,
            side.y + side.h,
            1.0,
            palette.muted.with_alpha(0.25),
        );
    }
    badge
}

#[allow(clippy::too_many_arguments)]
fn draw_header(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    sb: &SidebarState,
    layout: &SidebarLayout,
    palette: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let header = layout.header;
    painter.rect_filled(header, 0.0, palette.surface);
    let px = FontSize::Label.px() * s;
    let btn_w = 44.0 * s;

    // Collapse toggle (rightmost, clear of the resize grip).
    let toggle = Rect::new(
        header.x + header.w - btn_w - 6.0 * s,
        header.y,
        btn_w,
        header.h,
    );
    let tg = input.add_zone(ZONE_SIDEBAR_TOGGLE, toggle);
    if tg.is_hovered() {
        painter.rect_filled(toggle, 6.0 * s, Color::WHITE.with_alpha(0.06));
    }
    centered_label(
        text,
        "◀",
        &toggle,
        px,
        palette.text_secondary,
        false,
        sw,
        sh,
    );

    // Filenames on/off.
    let names = Rect::new(
        toggle.x - btn_w,
        header.y + 8.0 * s,
        btn_w,
        header.h - 16.0 * s,
    );
    let ns = input.add_zone(ZONE_SIDEBAR_NAMES, names);
    if sb.show_names {
        painter.rect_filled(names, 6.0 * s, palette.accent.with_alpha(0.18));
    }
    if ns.is_hovered() {
        painter.rect_filled(names, 6.0 * s, Color::WHITE.with_alpha(0.06));
    }
    let names_color = if sb.show_names {
        palette.accent
    } else {
        palette.text_secondary
    };
    centered_label(text, "Aa", &names, px, names_color, true, sw, sh);

    // Current folder name.
    let dir_name = sb
        .current_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    TextLabel::new(
        &dir_name,
        header.x + 12.0 * s,
        header.y + (header.h - px) * 0.5,
    )
    .size(FontSize::Custom(px))
    .bold()
    .color(palette.text)
    .max_width((names.x - header.x - 20.0 * s).max(20.0))
    .draw(text, sw, sh);
}

#[allow(clippy::too_many_arguments)]
fn draw_slots<'a>(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    sb: &'a SidebarState,
    layout: &SidebarLayout,
    visible: &[usize],
    flavor: SidebarFlavor<'_>,
    tex_draws: &mut Vec<TextureDraw<'a>>,
    palette: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) -> Option<Rect> {
    let rows_vp = layout.rows_vp;
    let rows_clip = [rows_vp.x, rows_vp.y, rows_vp.w, rows_vp.h];
    let scroll = sb.scroll.offset;
    let mut badge = None;

    painter.push_clip(rows_vp);
    text.push_clip(rows_clip);
    for &slot in visible {
        let r = layout.slot_rect(slot, scroll);
        let Some(zone_rect) = r.intersect(&rows_vp) else {
            continue;
        };
        let hovered = input
            .add_zone(ZONE_SIDEBAR_ITEM_BASE + slot as u32, zone_rect)
            .is_hovered();

        if layout.is_parent(slot) {
            if hovered {
                painter.rect_filled(r, 6.0 * s, Color::WHITE.with_alpha(0.06));
            }
            let px = FontSize::Label.px() * s;
            TextLabel::new("⬑  ..", r.x + 10.0 * s, r.y + (r.h - px) * 0.5)
                .size(FontSize::Custom(px))
                .color(palette.text_secondary)
                .draw(text, sw, sh);
            continue;
        }
        let Some(entry) = layout.entry_index(slot).and_then(|i| sb.entries.get(i)) else {
            continue;
        };
        if entry.is_dir {
            draw_dir_row(painter, text, entry, &r, hovered, palette, s, sw, sh);
        } else {
            draw_tile(
                painter, text, sb, layout, entry, &r, hovered, flavor, tex_draws, rows_clip,
                palette, s, sw, sh,
            );
            if hovered && flavor.add_badge {
                badge = Some(layout.add_badge_rect(&r));
            }
        }
    }
    text.pop_clip();
    painter.pop_clip();
    badge
}

#[allow(clippy::too_many_arguments)]
fn draw_dir_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    entry: &SidebarEntry,
    r: &Rect,
    hovered: bool,
    palette: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) {
    if hovered {
        painter.rect_filled(*r, 6.0 * s, Color::WHITE.with_alpha(0.06));
    }
    let icon = 30.0 * s;
    let ix = r.x + 8.0 * s;
    let iy = r.y + (r.h - icon) * 0.5;
    draw_folder_icon(painter, ix, iy, icon, palette);
    let px = FontSize::Label.px() * s;
    let nx = ix + icon + 10.0 * s;
    TextLabel::new(&entry.name, nx, r.y + (r.h - px) * 0.5)
        .size(FontSize::Custom(px))
        .color(palette.text)
        .max_width((r.x + r.w - nx - 8.0 * s).max(20.0))
        .draw(text, sw, sh);
}

#[allow(clippy::too_many_arguments)]
fn draw_tile<'a>(
    painter: &mut Painter,
    text: &mut TextRenderer,
    sb: &'a SidebarState,
    layout: &SidebarLayout,
    entry: &'a SidebarEntry,
    tile: &Rect,
    hovered: bool,
    flavor: SidebarFlavor<'_>,
    tex_draws: &mut Vec<TextureDraw<'a>>,
    rows_clip: [f32; 4],
    palette: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let radius = 8.0 * s;
    let is_current = flavor.current == Some(entry.path.as_path());
    painter.rect_filled(*tile, radius, palette.surface_2.with_alpha(0.55));
    if is_current {
        painter.rect_filled(*tile, radius, palette.accent.with_alpha(0.22));
    }

    let tb = layout.thumb_box(tile);
    let inset = 4.0 * s;
    let inner = Rect::new(
        tb.x + inset,
        tb.y + inset,
        tb.w - inset * 2.0,
        tb.h - inset * 2.0,
    );
    if let Some(tex) = sb.thumb(&entry.path) {
        let (tw, th) = (tex.width as f32, tex.height as f32);
        let k = (inner.w / tw).min(inner.h / th);
        let (dw, dh) = (tw * k, th * k);
        let mut draw = TextureDraw::new(
            tex,
            inner.x + (inner.w - dw) * 0.5,
            inner.y + (inner.h - dh) * 0.5,
            dw,
            dh,
        );
        draw.clip = Some(rows_clip);
        tex_draws.push(draw);
    } else {
        // Still decoding: soft placeholder block.
        painter.rect_filled(inner, radius * 0.75, palette.surface.with_alpha(0.7));
    }

    if is_current {
        painter.rect_stroke(*tile, radius, 3.0 * s, palette.accent);
    } else if hovered {
        painter.rect_stroke(*tile, radius, 2.0 * s, palette.accent.with_alpha(0.85));
    }

    if sb.show_names {
        let px = FontSize::Caption.px() * s;
        let strip_y = tile.y + tile.h - NAME_H * s;
        TextLabel::new(
            &entry.name,
            tile.x + 8.0 * s,
            strip_y + (NAME_H * s - px) * 0.5,
        )
        .size(FontSize::Custom(px))
        .color(palette.text_secondary)
        .max_width((tile.w - 16.0 * s).max(10.0))
        .draw(text, sw, sh);
    }
}

/// The "+" add-to-canvas badge (overlay layer, above the thumbnail).
pub fn draw_add_badge(painter: &mut Painter, b: &Rect, palette: &FoxPalette, s: f32) {
    let (cx, cy, r) = (b.center_x(), b.center_y(), b.w * 0.5);
    painter.circle_filled(cx, cy, r + 2.0 * s, Color::BLACK.with_alpha(0.35));
    painter.circle_filled(cx, cy, r, palette.accent);
    let k = r * 0.45;
    painter.line(cx - k, cy, cx + k, cy, 2.5 * s, Color::WHITE);
    painter.line(cx, cy - k, cx, cy + k, 2.5 * s, Color::WHITE);
}

fn draw_folder_icon(painter: &mut Painter, x: f32, y: f32, size: f32, palette: &FoxPalette) {
    let body = Rect::new(x + size * 0.08, y + size * 0.25, size * 0.84, size * 0.55);
    let tab = Rect::new(x + size * 0.08, y + size * 0.16, size * 0.38, size * 0.18);
    painter.rect_filled(tab, size * 0.06, palette.accent.with_alpha(0.75));
    painter.rect_filled(body, size * 0.08, palette.accent.with_alpha(0.9));
}

#[allow(clippy::too_many_arguments)]
fn centered_label(
    text: &mut TextRenderer,
    label: &str,
    r: &Rect,
    px: f32,
    color: Color,
    bold: bool,
    sw: u32,
    sh: u32,
) {
    let w = text.measure_width(label, px);
    let mut l = TextLabel::new(label, r.x + (r.w - w) * 0.5, r.y + (r.h - px) * 0.5)
        .size(FontSize::Custom(px))
        .color(color)
        .max_width(w + 24.0);
    if bold {
        l = l.bold();
    }
    l.draw(text, sw, sh);
}
