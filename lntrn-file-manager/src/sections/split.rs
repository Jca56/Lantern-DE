//! Split-view rendering: the unfocused pane (nav bar + content + scrollbar),
//! the divider handle, and the split toggle icon. The FOCUSED pane renders
//! through the normal single-pane path in render.rs with pane-sized rects;
//! only the unfocused side goes through here, reading parked state and
//! registering the `ZONE_P2_*` zone family (clicks there focus the pane).

use lntrn_render::{Color, Painter, Rect, TextRenderer, GpuContext};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, ScrollArea, Scrollbar, TextLabel};
use lntrn_render::TexturePass;

use crate::app::{DirectoryTab, PaneView, ViewMode};
use crate::icons::IconCache;
use crate::layout::*;
use crate::{
    ZONE_P2_BACK, ZONE_P2_CONTENT, ZONE_P2_FILE_BASE, ZONE_P2_FORWARD, ZONE_P2_PATH,
    ZONE_P2_SCROLLBAR, ZONE_P2_SEARCH, ZONE_P2_SORT, ZONE_P2_TREE_BASE, ZONE_P2_UP,
    ZONE_P2_VIEW_TOGGLE, ZONE_SPLIT_TOGGLE,
};

use super::icons::{draw_sort_icon, draw_view_mode_icon};
use super::{breadcrumb_segments, draw_scrollbar, truncate_with_ellipsis};

/// Split-view toggle icon: two panes side by side.
pub fn draw_split_toggle_icon(painter: &mut Painter, r: Rect, color: Color, s: f32) {
    let m = 0.24;
    let outer = Rect::new(
        r.x + r.w * m,
        r.y + r.h * m,
        r.w * (1.0 - 2.0 * m),
        r.h * (1.0 - 2.0 * m),
    );
    painter.rect_stroke(outer, 2.0 * s, 1.8 * s, color);
    // Center divider line
    painter.rect_filled(
        Rect::new(outer.x + outer.w * 0.5 - 0.9 * s, outer.y, 1.8 * s, outer.h),
        0.0,
        color,
    );
}

/// The draggable divider between panes: a subtle bar with grip dots.
pub fn draw_split_divider(painter: &mut Painter, r: Rect, hovered: bool, pal: &FoxPalette, s: f32) {
    let line_w = 2.0 * s;
    let line = Rect::new(r.x + (r.w - line_w) * 0.5, r.y, line_w, r.h);
    let color = if hovered {
        pal.accent.with_alpha(0.8)
    } else {
        Color::WHITE.with_alpha(0.10)
    };
    painter.rect_filled(line, line_w * 0.5, color);
    // Grip dots at the vertical center
    let dot_color = if hovered { pal.accent } else { pal.text_secondary.with_alpha(0.5) };
    let cx = r.x + r.w * 0.5;
    let cy = r.y + r.h * 0.5;
    for i in -1..=1 {
        painter.circle_filled(cx, cy + i as f32 * 8.0 * s, 1.8 * s, dot_color);
    }
}

fn nav_arrow_color(enabled: bool, hovered: bool, pal: &FoxPalette) -> Color {
    if enabled {
        if hovered { pal.text } else { pal.text_secondary }
    } else {
        pal.muted.with_alpha(0.4)
    }
}

fn draw_back_arrow(painter: &mut Painter, r: Rect, color: Color, s: f32) {
    let bm = 0.22;
    painter.line(r.x + r.w * (1.0 - bm), r.y + r.h * bm, r.x + r.w * bm, r.center_y(), 2.0 * s, color);
    painter.line(r.x + r.w * bm, r.center_y(), r.x + r.w * (1.0 - bm), r.y + r.h * (1.0 - bm), 2.0 * s, color);
}

fn draw_forward_arrow(painter: &mut Painter, r: Rect, color: Color, s: f32) {
    let bm = 0.22;
    painter.line(r.x + r.w * bm, r.y + r.h * bm, r.x + r.w * (1.0 - bm), r.center_y(), 2.0 * s, color);
    painter.line(r.x + r.w * (1.0 - bm), r.center_y(), r.x + r.w * bm, r.y + r.h * (1.0 - bm), 2.0 * s, color);
}

fn draw_up_arrow(painter: &mut Painter, r: Rect, color: Color, s: f32) {
    let bm = 0.22;
    painter.line(r.x + r.w * bm, r.center_y(), r.center_x(), r.y + r.h * bm, 2.0 * s, color);
    painter.line(r.center_x(), r.y + r.h * bm, r.x + r.w * (1.0 - bm), r.center_y(), 2.0 * s, color);
}

/// Everything the unfocused pane needs from the caller.
pub struct InactivePane<'a> {
    pub tab: &'a DirectoryTab,
    pub view: &'a PaneView,
    /// True when this pane is the right column (it hosts the split-close
    /// button and has no tab bar above its content).
    pub is_right: bool,
    pub pane_x: f32,
    pub pane_w: f32,
    /// Scroll offset (copied in; the updated value is returned).
    pub scroll: f32,
    pub zoom: f32,
    /// A drag is in flight — compute hover from the raw cursor so drop
    /// targets highlight (InteractionContext suppresses hover mid-drag).
    pub dragging: bool,
}

/// Render the unfocused pane. Returns the (possibly clamped) scroll offset.
#[allow(clippy::too_many_arguments)]
pub fn render_inactive_pane(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ctx: &GpuContext,
    tex_pass: &TexturePass,
    input: &mut InteractionContext,
    icon_cache: &mut IconCache,
    git: &crate::git_status::GitStatus,
    pal: &FoxPalette,
    p: InactivePane,
    hf: f32,
    screen: (u32, u32),
    s: f32,
) -> f32 {
    let (w, h) = screen;
    let entries = &p.tab.entries;
    let view = p.view;
    let zoom = p.zoom;

    // ── Nav bar ─────────────────────────────────────────────────────────
    let vt_rect = pane_view_toggle_rect(p.pane_x, s);
    let back_rect = pane_back_rect(p.pane_x, s);
    let fwd_rect = pane_forward_rect(p.pane_x, s);
    let up_rect = pane_up_rect(p.pane_x, s);
    let sort_rect = pane_sort_rect(p.pane_x, p.pane_w, s);
    let srch_rect = pane_search_rect(p.pane_x, p.pane_w, s);
    let path_r = pane_path_rect(p.pane_x, p.pane_w, p.is_right, s);

    let vt_hov = input.add_zone(ZONE_P2_VIEW_TOGGLE, vt_rect).is_hovered();
    let back_hov = input.add_zone(ZONE_P2_BACK, back_rect).is_hovered();
    let fwd_hov = input.add_zone(ZONE_P2_FORWARD, fwd_rect).is_hovered();
    let up_hov = input.add_zone(ZONE_P2_UP, up_rect).is_hovered();
    let sort_hov = input.add_zone(ZONE_P2_SORT, sort_rect).is_hovered();
    let srch_hov = input.add_zone(ZONE_P2_SEARCH, srch_rect).is_hovered();
    input.add_zone(ZONE_P2_PATH, path_r);

    let vt_color = if vt_hov { pal.text } else { pal.text_secondary };
    if vt_hov {
        painter.rect_filled(vt_rect, 4.0 * s, pal.surface_2.with_alpha(0.5));
    }
    draw_view_mode_icon(painter, view.view_mode, vt_rect, vt_color, s);

    draw_back_arrow(
        painter, back_rect,
        nav_arrow_color(!p.tab.history_back.is_empty(), back_hov, pal), s,
    );
    draw_forward_arrow(
        painter, fwd_rect,
        nav_arrow_color(!p.tab.history_forward.is_empty(), fwd_hov, pal), s,
    );
    draw_up_arrow(
        painter, up_rect,
        nav_arrow_color(p.tab.path.parent().is_some(), up_hov, pal), s,
    );

    // Static breadcrumb path — muted; interaction comes after a focusing click.
    {
        let segments = breadcrumb_segments(&p.tab.path, s);
        let font = 22.0 * s;
        let char_w = font * 0.45;
        let full: String = segments
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(" / ");
        let shown = truncate_with_ellipsis(&full, path_r.w - 8.0 * s, char_w);
        TextLabel::new(&shown, path_r.x + 4.0 * s, path_r.y + (path_r.h - font) * 0.5)
            .size(FontSize::Custom(font))
            .color(pal.text_secondary)
            .draw(text, w, h);
    }

    let sort_color = if sort_hov { pal.text } else { pal.text_secondary };
    if sort_hov {
        painter.rect_filled(sort_rect, 4.0 * s, pal.surface_2.with_alpha(0.5));
    }
    draw_sort_icon(painter, sort_rect, sort_color, view.sort_dir, s);

    let srch_color = if srch_hov { pal.text } else { pal.text_secondary };
    if srch_hov {
        painter.rect_filled(srch_rect, 4.0 * s, pal.surface_2.with_alpha(0.5));
    }
    let sx = srch_rect.center_x() - 2.0 * s;
    let sy = srch_rect.center_y() - 2.0 * s;
    painter.circle_stroke(sx, sy, 6.0 * s, 1.5 * s, srch_color);
    painter.line(sx + 4.5 * s, sy + 4.5 * s, sx + 9.0 * s, sy + 9.0 * s, 2.0 * s, srch_color);

    if p.is_right {
        let split_rect = pane_split_toggle_rect(p.pane_x, p.pane_w, s);
        let split_hov = input.add_zone(ZONE_SPLIT_TOGGLE, split_rect).is_hovered();
        if split_hov {
            painter.rect_filled(split_rect, 4.0 * s, pal.surface_2.with_alpha(0.5));
        }
        draw_split_toggle_icon(
            painter, split_rect,
            if split_hov { pal.text } else { pal.accent },
            s,
        );
    }

    // ── Content ─────────────────────────────────────────────────────────
    let content = pane_content_rect(p.pane_x, p.pane_w, hf, s, !p.is_right);
    input.add_zone(ZONE_P2_CONTENT, content);

    let cols = grid_columns(content.w, s, zoom);
    let total_h = match view.view_mode {
        ViewMode::Grid => grid_content_height(entries.len(), cols, s, zoom),
        ViewMode::List => {
            entries.len() as f32 * list_row_h(s, zoom) + 32.0 * list_zoom_multiplier(zoom) * s
        }
        ViewMode::Tree => tree_content_height(view.tree_entries.len(), s, zoom),
    };
    let mut scroll = p.scroll;
    let scroll_area = ScrollArea::new(content, total_h, &mut scroll);
    let base_y = scroll_area.content_y();

    // Manual hover during drags (zone hover is suppressed while a drag is
    // captured elsewhere) so cross-pane drop targets light up.
    let cursor = input.cursor();
    let hover_at = |rect: Rect, zone_state_hovered: bool| -> bool {
        if p.dragging {
            cursor.map_or(false, |(cx, cy)| rect.contains(cx, cy))
        } else {
            zone_state_hovered
        }
    };

    match view.view_mode {
        ViewMode::Grid => {
            let mut hovered = Vec::with_capacity(entries.len());
            let mut has_icon = Vec::with_capacity(entries.len());
            for i in 0..entries.len() {
                let ir = file_item_rect(i, cols, content.x, base_y, s, zoom);
                let visible = ir.intersect(&content).is_some();
                if visible {
                    icon_cache.get_or_load(&entries[i], ctx, tex_pass);
                }
                has_icon.push(visible && icon_cache.has_icon(&entries[i]));
                let hit = item_hit_rect(ir, s, zoom);
                let hov = match hit.intersect(&content) {
                    Some(clipped) => {
                        let state = input.add_zone(ZONE_P2_FILE_BASE + i as u32, clipped);
                        hover_at(clipped, state.is_hovered())
                    }
                    None => false,
                };
                hovered.push(hov);
            }
            super::draw_content_grid(
                painter, text, pal, content, entries, cols,
                &scroll_area, &hovered, &has_icon, None, None, git, screen, s, zoom,
            );
        }
        ViewMode::List => {
            let row_h = list_row_h(s, zoom);
            let hdr_h = 32.0 * list_zoom_multiplier(zoom) * s;
            let mut hovered = Vec::with_capacity(entries.len());
            let mut has_icon = Vec::with_capacity(entries.len());
            for i in 0..entries.len() {
                let y = base_y + hdr_h + i as f32 * row_h;
                let visible = y + row_h >= content.y && y <= content.y + content.h;
                if visible {
                    icon_cache.get_or_load(&entries[i], ctx, tex_pass);
                }
                has_icon.push(visible && icon_cache.has_icon(&entries[i]));
                let row_rect = Rect::new(content.x, y, content.w, row_h);
                let hov = match row_rect.intersect(&content) {
                    Some(clipped) if visible => {
                        let state = input.add_zone(ZONE_P2_FILE_BASE + i as u32, clipped);
                        hover_at(clipped, state.is_hovered())
                    }
                    _ => false,
                };
                hovered.push(hov);
            }
            crate::views::draw_content_list(
                painter, text, pal, content, entries,
                &scroll_area, &hovered, &has_icon, None, None, None, git, screen, s, zoom,
            );
        }
        ViewMode::Tree => {
            let row_h = tree_row_h(s, zoom);
            let tree_entries = &view.tree_entries;
            let mut hovered = Vec::with_capacity(tree_entries.len());
            let mut has_icon = Vec::with_capacity(tree_entries.len());
            for i in 0..tree_entries.len() {
                let y = base_y + i as f32 * row_h;
                let visible = y + row_h >= content.y && y <= content.y + content.h;
                if visible {
                    icon_cache.get_or_load(&tree_entries[i].entry, ctx, tex_pass);
                }
                has_icon.push(visible && icon_cache.has_icon(&tree_entries[i].entry));
                if !visible {
                    hovered.push(false);
                    continue;
                }
                let row_rect =
                    crate::views::tree_row_hit_rect(text, &tree_entries[i], content, y, row_h, s, zoom);
                let hov = match row_rect.intersect(&content) {
                    Some(clipped) => {
                        let state = input.add_zone(ZONE_P2_TREE_BASE + i as u32, clipped);
                        hover_at(clipped, state.is_hovered())
                    }
                    None => false,
                };
                hovered.push(hov);
            }
            let selected = vec![false; tree_entries.len()];
            crate::views::draw_content_tree(
                painter, text, pal, content, tree_entries,
                &scroll_area, &hovered, &has_icon, &selected, None, None, screen, s, zoom,
            );
        }
    }

    if scroll_area.is_scrollable() {
        let scrollbar = Scrollbar::new(&content, total_h, scroll);
        input.add_zone(ZONE_P2_SCROLLBAR, scrollbar.hover_zone());
        let sb_state = input.zone_state(ZONE_P2_SCROLLBAR);
        draw_scrollbar(painter, &scrollbar, sb_state, pal);
    }

    scroll
}
