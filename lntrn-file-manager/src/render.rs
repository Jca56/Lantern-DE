use lntrn_render::{Color, Rect, TextureDraw};
use lntrn_ui::gpu::{
    ContextMenu, FontSize, FoxPalette, GradientStrip, InteractionContext, MenuEvent,
    ScrollArea, Scrollbar, TabBar, TextInput, TextLabel, TitleBar,
};

use crate::app::{App, ViewMode};
use crate::icons::{self, IconCache};
use crate::layout::*;
use crate::sections::*;
use crate::views::{draw_content_list, draw_content_tree};
use lntrn_ui::gpu::controls::{Button, ButtonVariant};
use crate::{Gpu, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_MENU_VIEW,
    ZONE_NAV_VIEW_TOGGLE, ZONE_NAV_BACK, ZONE_NAV_FORWARD, ZONE_NAV_UP, ZONE_NAV_SEARCH,
    ZONE_SIDEBAR_ITEM_BASE, ZONE_FILE_ITEM_BASE, ZONE_CONTENT, ZONE_SCROLLBAR,
    ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE, ZONE_TAB_NEW, ZONE_RENAME_INPUT, ZONE_PATH_INPUT,
    ZONE_DRIVE_ITEM_BASE, ZONE_TREE_ITEM_BASE,
    ZONE_DROP_MOVE, ZONE_DROP_COPY, ZONE_DROP_CANCEL};

/// Render a full frame.
pub fn render_frame(
    gpu: &mut Gpu,
    app: &mut App,
    input: &mut InteractionContext,
    icon_cache: &mut IconCache,
    file_info: &mut crate::file_info::FileInfoCache,
    palette: &FoxPalette,
    scale: f32,
    maximized: bool,
    view_menu: &mut ContextMenu,
    tab_drag: Option<usize>,
    bg_opacity: f32,
) {
    let Gpu { ctx, painter, text, tex_pass } = gpu;

    let w = ctx.width();
    let h = ctx.height();
    let wf = w as f32;
    let hf = h as f32;
    let pal = palette;
    let s = scale;

    painter.clear();
    input.begin_frame();

    // ── Compute content geometry per view mode ─────────────────────────
    icon_cache.ensure_dir(&app.current_dir);
    icon_cache.poll_video_thumbs(ctx, tex_pass);
    let content = if app.pick.is_some() {
        let bottom = hf - crate::pick_bar::PICK_BAR_H * s;
        content_rect_with_bottom(wf, bottom, s)
    } else {
        content_rect(wf, hf, s)
    };
    let zoom = app.icon_zoom;
    // When searching, show search results in list mode
    let is_searching = app.searching && !app.search_buf.is_empty();
    let entries: &[crate::fs::FileEntry] = if is_searching {
        &app.search_results
    } else {
        &app.entries
    };
    let view_mode = if is_searching { ViewMode::List } else { app.view_mode };

    // Grid-specific vars (used for grid mode + icon loading)
    let cols = grid_columns(content.w, s, zoom);
    let total_content_h = match view_mode {
        ViewMode::Grid => grid_content_height(entries.len(), cols, s, zoom),
        ViewMode::List => list_content_height(entries.len(), s) + 32.0 * s, // +header
        ViewMode::Tree => tree_content_height(app.tree_entries.len(), s),
    };
    let scroll_area = ScrollArea::new(content, total_content_h, &mut app.scroll_offset);
    let base_y = scroll_area.content_y();
    let icsz = icon_size(s, zoom);

    // Icon loading for all view modes (visible entries only)
    let has_icon: Vec<bool> = match view_mode {
        ViewMode::Grid => {
            for i in 0..entries.len() {
                let ir = file_item_rect(i, cols, content.x, base_y, s, zoom);
                if ir.intersect(&content).is_some() {
                    icon_cache.get_or_load(&entries[i], ctx, tex_pass);
                }
            }
            (0..entries.len())
                .map(|i| {
                    let ir = file_item_rect(i, cols, content.x, base_y, s, zoom);
                    ir.intersect(&content).is_some() && icon_cache.has_icon(&entries[i])
                })
                .collect()
        }
        ViewMode::List => {
            let row_h = list_row_h(s);
            let hdr_h = 32.0 * s;
            for i in 0..entries.len() {
                let y = base_y + hdr_h + i as f32 * row_h;
                if y + row_h >= content.y && y <= content.y + content.h {
                    icon_cache.get_or_load(&entries[i], ctx, tex_pass);
                }
            }
            (0..entries.len())
                .map(|i| {
                    let y = base_y + hdr_h + i as f32 * row_h;
                    y + row_h >= content.y && y <= content.y + content.h && icon_cache.has_icon(&entries[i])
                })
                .collect()
        }
        ViewMode::Tree => {
            let row_h = tree_row_h(s);
            let tree_entries = &app.tree_entries;
            for i in 0..tree_entries.len() {
                let y = base_y + i as f32 * row_h;
                if y + row_h >= content.y && y <= content.y + content.h {
                    icon_cache.get_or_load(&tree_entries[i].entry, ctx, tex_pass);
                }
            }
            (0..tree_entries.len())
                .map(|i| {
                    let y = base_y + i as f32 * row_h;
                    y + row_h >= content.y && y <= content.y + content.h && icon_cache.has_icon(&tree_entries[i].entry)
                })
                .collect()
        }
    };

    // ── Window background ─────────────────────────────────────────────
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, pal.bg.with_alpha(bg_opacity));

    // ── Title bar ─────────────────────────────────────────────────────
    let tb_rect = title_bar_rect(wf, s);
    let close_rect = TitleBar::new(tb_rect).scale(s).close_button_rect();
    let max_rect = TitleBar::new(tb_rect).scale(s).maximize_button_rect();
    let min_rect = TitleBar::new(tb_rect).scale(s).minimize_button_rect();
    let close_s = input.add_zone(ZONE_CLOSE, close_rect);
    let max_s = input.add_zone(ZONE_MAXIMIZE, max_rect);
    let min_s = input.add_zone(ZONE_MINIMIZE, min_rect);

    TitleBar::new(tb_rect)
        .scale(s)
        .maximized(maximized)
        .close_hovered(close_s.is_hovered())
        .maximize_hovered(max_s.is_hovered())
        .minimize_hovered(min_s.is_hovered())
        .draw(painter, pal);

    // ── "View" menu label in title bar ──────────────────────────────
    {
        let tb_content = TitleBar::new(tb_rect).scale(s).content_rect();
        let font = 22.0 * s;
        let pad_h = 12.0 * s;
        let label_w = font * 0.52 * 4.0 + pad_h * 2.0; // "View" = 4 chars
        let label_h = font + 8.0 * s;
        let label_x = tb_content.x + 10.0 * s;
        let label_y = tb_content.y + (tb_content.h - label_h) * 0.5;
        let label_rect = Rect::new(label_x, label_y, label_w, label_h);
        let view_state = input.add_zone(ZONE_MENU_VIEW, label_rect);
        let is_open = view_menu.is_open();
        let r = 6.0 * s;

        if is_open || view_state.is_hovered() {
            painter.rect_filled(label_rect, r, pal.accent.with_alpha(if is_open { 0.15 } else { 0.10 }));
        }
        let color = if is_open { pal.accent } else { pal.text };
        TextLabel::new("View", label_x + pad_h, label_y + (label_h - font) * 0.5)
            .size(FontSize::Custom(font))
            .color(color)
            .draw(text, w, h);
    }

    // ── Gradient strip ───────────────────────────────────────────────
    let mut grad = GradientStrip::new(0.0, tb_rect.h, wf)
        .colors(pal.file_manager_gradient_stops());
    grad.height = 4.0 * s;
    grad.draw(painter);

    // ── Nav bar ───────────────────────────────────────────────────────
    let nav_rect = nav_bar_rect(wf, s);
    let vt_rect = view_toggle_rect(s);
    let vt_state = input.add_zone(ZONE_NAV_VIEW_TOGGLE, vt_rect);
    let back_rect = back_button_rect(s);
    let back_state = input.add_zone(ZONE_NAV_BACK, back_rect);
    let fwd_rect = forward_button_rect(s);
    let fwd_state = input.add_zone(ZONE_NAV_FORWARD, fwd_rect);
    let up_rect = up_button_rect(s);
    let up_state = input.add_zone(ZONE_NAV_UP, up_rect);
    let p_rect = path_rect(wf, s);
    let path_zone = if app.path_editing {
        input.add_zone(ZONE_PATH_INPUT, p_rect)
    } else {
        input.add_zone(ZONE_PATH_INPUT, p_rect)
    };
    let srch_rect = search_button_rect(wf, s);
    let srch_state = input.add_zone(ZONE_NAV_SEARCH, srch_rect);
    draw_nav_bar(
        painter, text, pal, app,
        nav_rect, vt_rect, vt_state.is_hovered(),
        back_rect, back_state.is_hovered(),
        fwd_rect, fwd_state.is_hovered(),
        up_rect, up_state.is_hovered(),
        p_rect, path_zone.is_hovered(),
        srch_rect, srch_state.is_hovered(),
        (w, h), s,
    );

    draw_gradient_h(painter, pal, 0.0, nav_rect.y + nav_rect.h, wf, s);

    // ── Tab bar ─────────────────────────────────────────────────────
    let tab_rect = tab_bar_rect(wf, s);
    let tab_labels = app.tab_labels();
    let tab_label_refs: Vec<&str> = tab_labels.iter().map(|s| s.as_str()).collect();

    let tab_bar = TabBar::new(tab_rect)
        .tabs(&tab_label_refs)
        .selected(app.current_tab)
        .scale(s)
        .closable(app.tabs.len() > 1);

    let tab_rects = tab_bar.tab_rects();
    let close_rects = tab_bar.close_rects();
    let new_tab_r = tab_bar.new_tab_rect();

    let mut hovered_tab: Option<usize> = None;
    let mut hovered_close: Option<usize> = None;

    for i in 0..tab_labels.len() {
        if i < tab_rects.len() {
            let zone_id = ZONE_TAB_BASE + i as u32;
            let state = input.add_zone(zone_id, tab_rects[i]);
            if state.is_hovered() {
                hovered_tab = Some(i);
            }
        }
        // Don't register close zones for pinned tabs
        let is_pinned = app.tabs.get(i).map_or(false, |t| t.pinned);
        if app.tabs.len() > 1 && !is_pinned && i < close_rects.len() {
            let zone_id = ZONE_TAB_CLOSE_BASE + i as u32;
            let state = input.add_zone(zone_id, close_rects[i]);
            if state.is_hovered() {
                hovered_close = Some(i);
            }
        }
    }
    // During drag, InteractionContext suppresses hover on non-captured zones.
    // Manually detect tab hover from cursor position so the highlight shows.
    if app.drag_item.is_some() {
        if let Some((cx, cy)) = input.cursor() {
            hovered_tab = tab_rects.iter().position(|r| r.contains(cx, cy));
        }
    }
    let new_tab_state = input.add_zone(ZONE_TAB_NEW, new_tab_r);
    let hovered_new = new_tab_state.is_hovered();

    TabBar::new(tab_rect)
        .tabs(&tab_label_refs)
        .selected(app.current_tab)
        .scale(s)
        .closable(app.tabs.len() > 1)
        .hovered_tab(hovered_tab)
        .hovered_close(hovered_close)
        .hovered_new_tab(hovered_new)
        .draw(painter, text, pal, w, h);

    // Draw pin icons on pinned tabs (replace close button with pin)
    for (i, tab) in app.tabs.iter().enumerate() {
        if tab.pinned && i < close_rects.len() {
            let cr = &close_rects[i];
            // Cover the X icon with surface background
            painter.rect_filled(*cr, 4.0 * s, pal.surface);
            // Draw pin icon centered in the close button rect
            let pc = pal.accent;
            let cx = cr.x + cr.w * 0.5;
            let cy = cr.y + cr.h * 0.5;
            // Pin head (circle)
            painter.circle_filled(cx, cy - 3.0 * s, 3.5 * s, pc);
            // Pin needle
            painter.line(cx, cy - 0.5 * s, cx, cy + 5.0 * s, 1.5 * s, pc);
        }
    }

    // Pinned tab drag indicator
    if let Some(src_idx) = tab_drag {
        // Highlight the source tab being dragged
        if src_idx < tab_rects.len() {
            painter.rect_stroke(tab_rects[src_idx], 4.0 * s, 2.0 * s, pal.accent);
        }
        // Show insertion indicator at cursor position
        if let Some((cursor_x, _)) = input.cursor() {
            if let Some(target) = tab_rects.iter().position(|r| r.contains(cursor_x, r.y + r.h * 0.5)) {
                if target != src_idx && target < app.tabs.len() && app.tabs[target].pinned {
                    let ind_x = tab_rects[target].x;
                    let ind_y = tab_rects[target].y + 4.0 * s;
                    let ind_h = tab_rects[target].h - 8.0 * s;
                    painter.rect_filled(
                        Rect::new(ind_x - 1.5 * s, ind_y, 3.0 * s, ind_h),
                        1.5 * s, pal.accent,
                    );
                }
            }
        }
    }

    // Drop-target highlight on tabs while dragging files
    if app.drag_item.is_some() {
        if let Some(ti) = hovered_tab {
            if ti < tab_rects.len() {
                painter.rect_filled(tab_rects[ti], 4.0 * s, pal.accent.with_alpha(0.2));
                painter.rect_stroke(tab_rects[ti], 4.0 * s, 1.5 * s, pal.accent.with_alpha(0.5));
            }
        }
    }

    // ── Sidebar ───────────────────────────────────────────────────────
    let sidebar = sidebar_rect(hf, s);
    let places = app.sidebar_places();
    let mut sidebar_hovered = Vec::with_capacity(places.len());
    for i in 0..places.len() {
        let item_rect = sidebar_item_rect(i, s);
        let zone_id = ZONE_SIDEBAR_ITEM_BASE + i as u32;
        let state = input.add_zone(zone_id, item_rect);
        sidebar_hovered.push(state.is_hovered());
    }
    let num_places = places.len();
    let mut drive_hovered = Vec::with_capacity(app.drives.len());
    for i in 0..app.drives.len() {
        let item_rect = drive_item_rect(i, num_places, s);
        let zone_id = ZONE_DRIVE_ITEM_BASE + i as u32;
        let state = input.add_zone(zone_id, item_rect);
        drive_hovered.push(state.is_hovered());
    }
    let dragging = app.drag_item.is_some();
    draw_sidebar(painter, text, pal, app, sidebar, &sidebar_hovered, &drive_hovered, dragging, (w, h), s);

    // ── Content area ───────────────────────────────────────────────────
    input.add_zone(ZONE_CONTENT, content);

    match view_mode {
        ViewMode::Grid => {
            let mut item_hovered = Vec::with_capacity(entries.len());
            for i in 0..entries.len() {
                let item_rect = file_item_rect(i, cols, content.x, base_y, s, zoom);
                let zone_id = ZONE_FILE_ITEM_BASE + i as u32;
                let hovered = if let Some(clipped) = item_rect.intersect(&content) {
                    input.add_zone(zone_id, clipped).is_hovered()
                } else {
                    false
                };
                item_hovered.push(hovered);
            }
            draw_content_grid(
                painter, text, pal, content, entries, cols,
                &scroll_area, &item_hovered, &has_icon, app.drag_item, app.renaming, (w, h), s, zoom,
            );

            // Inline rename input (grid mode)
            if let Some(rename_idx) = app.renaming {
                if rename_idx < entries.len() {
                    let ir = file_item_rect(rename_idx, cols, content.x, base_y, s, zoom);
                    let icsz = icon_size(s, zoom);
                    let label_font = 16.0 * s;
                    let content_h = icsz + 2.0 * s + label_font;
                    let top_pad = (ir.h - content_h) * 0.5;
                    let input_y = ir.y + top_pad + icsz;
                    let input_h = 36.0 * s;
                    let input_w = (ir.w + 80.0 * s).min(content.x + content.w - (ir.x - 40.0 * s));
                    let input_x = ir.x - 40.0 * s;
                    let input_rect = Rect::new(input_x, input_y, input_w, input_h);
                    input.add_zone(ZONE_RENAME_INPUT, input_rect);
                    TextInput::new(input_rect)
                        .text(&app.rename_buf)
                        .cursor_pos(app.rename_cursor)
                        .focused(true)
                        .scale(s)
                        .draw(painter, text, pal, w, h);
                }
            }
        }
        ViewMode::List => {
            let row_h = list_row_h(s);
            let hdr_h = 32.0 * s;
            let mut item_hovered = Vec::with_capacity(entries.len());
            for i in 0..entries.len() {
                let row_rect = Rect::new(content.x, base_y + hdr_h + i as f32 * row_h, content.w, row_h);
                let zone_id = ZONE_FILE_ITEM_BASE + i as u32;
                let hovered = if let Some(clipped) = row_rect.intersect(&content) {
                    input.add_zone(zone_id, clipped).is_hovered()
                } else {
                    false
                };
                item_hovered.push(hovered);
            }
            draw_content_list(
                painter, text, pal, content, entries,
                &scroll_area, &item_hovered, &has_icon, app.drag_item, app.renaming, (w, h), s,
            );
        }
        ViewMode::Tree => {
            let row_h = tree_row_h(s);
            let tree_entries = &app.tree_entries;
            let mut item_hovered = Vec::with_capacity(tree_entries.len());
            for i in 0..tree_entries.len() {
                let row_rect = Rect::new(content.x, base_y + i as f32 * row_h, content.w, row_h);
                let zone_id = ZONE_TREE_ITEM_BASE + i as u32;
                let hovered = if let Some(clipped) = row_rect.intersect(&content) {
                    input.add_zone(zone_id, clipped).is_hovered()
                } else {
                    false
                };
                item_hovered.push(hovered);
            }
            draw_content_tree(
                painter, text, pal, content, tree_entries,
                &scroll_area, &item_hovered, &has_icon, (w, h), s,
            );
        }
    }

    // Scrollbar
    if scroll_area.is_scrollable() {
        let scrollbar = Scrollbar::new(&content, total_content_h, app.scroll_offset);
        input.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
        let sb_state = input.zone_state(ZONE_SCROLLBAR);
        draw_scrollbar(painter, &scrollbar, sb_state, pal);
    }

    // ── Status bar / Pick bar ──────────────────────────────────────────
    if app.pick.is_some() {
        let bar_y = hf - crate::pick_bar::PICK_BAR_H * s;
        crate::pick_bar::draw_pick_bar(app, painter, text, pal, input, wf, bar_y, s, (w, h));
    } else {
        let status = status_rect(wf, hf, s);
        draw_status_bar(painter, text, pal, status, &app.entries, file_info, (w, h), s);
    }

    // ── Rubber band selection overlay ─────────────────────────────────
    if let (Some(start), Some(end)) = (app.rubber_band_start, app.rubber_band_end) {
        draw_rubber_band(painter, pal, start, end, content);
    }

    // ── Drag ghost overlay (shapes, drawn in base pass) ────────────
    let drag_count = if app.drag_item.is_some() {
        let sel = entries.iter().filter(|e| e.selected).count();
        sel.max(1)
    } else { 0 };

    if let (Some(drag_idx), Some((dx, dy))) = (app.drag_item, app.drag_pos) {
        if drag_idx < entries.len() {
            let ghost_sz = 80.0 * s;
            let gx = dx - ghost_sz * 0.5;
            let gy = dy - ghost_sz * 0.5;

            // Count badge for multi-drag
            if drag_count > 1 {
                let badge_font = 14.0 * s;
                let badge_text = format!("{drag_count}");
                let badge_w = (badge_text.len() as f32 * badge_font * 0.6 + 10.0 * s).max(24.0 * s);
                let badge_h = 22.0 * s;
                let badge_x = gx + ghost_sz - badge_w * 0.5;
                let badge_y = gy - badge_h * 0.3;
                let badge_rect = Rect::new(badge_x, badge_y, badge_w, badge_h);
                painter.rect_filled(badge_rect, badge_h * 0.5, pal.accent);
                let tx = badge_x + (badge_w - badge_text.len() as f32 * badge_font * 0.55) * 0.5;
                let ty = badge_y + (badge_h - badge_font) * 0.5;
                TextLabel::new(&badge_text, tx, ty)
                    .size(FontSize::Custom(badge_font))
                    .color(pal.bg)
                    .draw(text, w, h);
            }
        }
    }

    // ── Collect texture draws for icons ────────────────────────────────
    let content_clip = [content.x, content.y, content.w, content.h];
    let mut tex_draws: Vec<TextureDraw> = match view_mode {
        ViewMode::Grid => {
            (0..entries.len())
                .filter(|&i| has_icon[i])
                .filter_map(|i| {
                    let ir = file_item_rect(i, cols, content.x, base_y, s, zoom);
                    let icon_x = ir.x + (ir.w - icsz) * 0.5;
                    let label_font = 16.0 * s;
                    let content_h = icsz + 2.0 * s + label_font;
                    let top_pad = (ir.h - content_h) * 0.5;
                    let icon_y = ir.y + top_pad;
                    let tex = icon_cache.get(&entries[i])?;
                    let (dx, dy, dw, dh) = icons::fit_in_box(tex, icon_x, icon_y, icsz, icsz);
                    let alpha = if app.drag_item.is_some() && entries[i].selected { 0.3 } else { 1.0 };
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh).opacity(alpha);
                    draw.clip = Some(content_clip);
                    Some(draw)
                })
                .collect()
        }
        ViewMode::List => {
            let row_h = list_row_h(s);
            let hdr_h = 32.0 * s;
            let list_icon_sz = 28.0 * s;
            (0..entries.len())
                .filter(|&i| has_icon[i])
                .filter_map(|i| {
                    let y = base_y + hdr_h + i as f32 * row_h;
                    let icon_x = content.x + 8.0 * s;
                    let icon_y = y + (row_h - list_icon_sz) * 0.5;
                    let tex = icon_cache.get(&entries[i])?;
                    let (dx, dy, dw, dh) = icons::fit_in_box(tex, icon_x, icon_y, list_icon_sz, list_icon_sz);
                    let alpha = if app.drag_item == Some(i) { 0.3 } else { 1.0 };
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh).opacity(alpha);
                    draw.clip = Some(content_clip);
                    Some(draw)
                })
                .collect()
        }
        ViewMode::Tree => {
            let row_h = tree_row_h(s);
            let tree_indent = 28.0 * s;
            let tree_icon_sz = 24.0 * s;
            let tree_entries = &app.tree_entries;
            (0..tree_entries.len())
                .filter(|&i| has_icon[i])
                .filter_map(|i| {
                    let te = &tree_entries[i];
                    let y = base_y + i as f32 * row_h;
                    let x_offset = te.depth as f32 * tree_indent;
                    let icon_x = content.x + 8.0 * s + x_offset + 16.0 * s;
                    let icon_y = y + (row_h - tree_icon_sz) * 0.5;
                    let tex = icon_cache.get(&te.entry)?;
                    let (dx, dy, dw, dh) = icons::fit_in_box(tex, icon_x, icon_y, tree_icon_sz, tree_icon_sz);
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh);
                    draw.clip = Some(content_clip);
                    Some(draw)
                })
                .collect()
        }
    };

    // Drag ghost texture — just the icon, centered on cursor
    if let (Some(drag_idx), Some((dx, dy))) = (app.drag_item, app.drag_pos) {
        if drag_idx < entries.len() {
            let ghost_sz = 80.0 * s;
            let gx = dx - ghost_sz * 0.5;
            let gy = dy - ghost_sz * 0.5;
            if let Some(tex) = icon_cache.get(&entries[drag_idx]) {
                let (bx, by, bw, bh) = icons::fit_in_box(tex, gx, gy, ghost_sz, ghost_sz);
                tex_draws.push(TextureDraw::new(tex, bx, by, bw, bh));
            }
        }
    }

    // ── Modal overlays (layer 1) ─────────────────────────────────────
    painter.set_layer(1);
    text.set_layer(1);
    if let Some(ref props) = app.properties {
        crate::properties::draw_properties_dialog(
            props, painter, text, input, pal,
            wf, hf, s, w, h,
        );
    }
    if let Some(ref drop) = app.pending_drop {
        draw_drop_modal(drop, painter, text, input, pal, wf, hf, s, w, h);
    }

    // ── Layered render ────────────────────────────────────────────────
    let frame = ctx.begin_frame("Lantern File Manager");
    match frame {
        Ok(mut frame) => {
            let view = frame.view().clone();

            // Layer 0: base shapes + textures + text
            painter.render_layer(0, ctx, frame.encoder_mut(), &view, Some(Color::rgba(0.0, 0.0, 0.0, 0.0)));
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(0, ctx, frame.encoder_mut(), &view);

            // Flush so glyphon's prepare() for layer 1 doesn't overwrite layer 0 vertices
            frame.flush(ctx);

            // Layer 1: modal overlay shapes + text
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(1, ctx, frame.encoder_mut(), &view);

            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[fox] render error: {e}"),
    }
}

fn draw_drop_modal(
    drop: &crate::app::PendingDrop,
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    input: &mut InteractionContext,
    pal: &FoxPalette,
    screen_w: f32, screen_h: f32,
    s: f32, sw: u32, sh: u32,
) {
    let dialog_w = 380.0 * s;
    let pad = 20.0 * s;
    let cr = 12.0 * s;
    let title_font = 20.0 * s;
    let body_font = 16.0 * s;
    let btn_h = 40.0 * s;
    let btn_gap = 10.0 * s;

    // File name for display
    let file_name = if drop.sources.len() == 1 {
        drop.sources[0].file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".into())
    } else {
        format!("{} items", drop.sources.len())
    };
    let dest_dir = drop.dest_dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());

    // Compute dialog height
    let content_h = title_font + pad * 0.5
        + body_font * 2.0 + pad * 0.5  // two lines of body text
        + pad + btn_h;
    let dialog_h = pad * 2.0 + content_h;
    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    // Backdrop
    painter.rect_filled(
        Rect::new(0.0, 0.0, screen_w, screen_h), 0.0,
        Color::rgba(0.0, 0.0, 0.0, 0.55),
    );

    // Shadow + panel
    let panel = Rect::new(dx, dy, dialog_w, dialog_h);
    let shadow = Rect::new(dx - 8.0 * s, dy - 4.0 * s, dialog_w + 16.0 * s, dialog_h + 16.0 * s);
    painter.rect_filled(shadow, cr + 4.0 * s, Color::rgba(0.0, 0.0, 0.0, 0.3));
    painter.rect_filled(panel, cr, pal.surface);
    painter.rect_stroke_sdf(panel, cr, 1.0 * s, pal.muted.with_alpha(0.2));

    let mut cy = dy + pad;

    // Title
    text.queue("Move or Copy?", title_font, dx + pad, cy, pal.text, dialog_w - pad * 2.0, sw, sh);
    cy += title_font + pad * 0.5;

    // Body text
    let line1 = format!("\"{file_name}\"");
    let line2 = format!("→ {dest_dir}/");
    text.queue(&line1, body_font, dx + pad, cy, pal.text, dialog_w - pad * 2.0, sw, sh);
    cy += body_font + 4.0 * s;
    text.queue(&line2, body_font, dx + pad, cy, pal.text_secondary, dialog_w - pad * 2.0, sw, sh);
    cy += body_font + pad;

    // Buttons: [Move] [Copy] [Cancel] — right-aligned
    let btn_w = 90.0 * s;
    let total_btn_w = btn_w * 3.0 + btn_gap * 2.0;
    let btn_x_start = dx + dialog_w - pad - total_btn_w;

    // Move button
    let move_rect = Rect::new(btn_x_start, cy, btn_w, btn_h);
    let move_state = input.add_zone(ZONE_DROP_MOVE, move_rect);
    Button::new(move_rect, "Move")
        .variant(ButtonVariant::Primary)
        .hovered(move_state.is_hovered())
        .pressed(move_state.is_active())
        .draw(painter, text, pal, sw, sh);

    // Copy button
    let copy_rect = Rect::new(btn_x_start + btn_w + btn_gap, cy, btn_w, btn_h);
    let copy_state = input.add_zone(ZONE_DROP_COPY, copy_rect);
    Button::new(copy_rect, "Copy")
        .hovered(copy_state.is_hovered())
        .pressed(copy_state.is_active())
        .draw(painter, text, pal, sw, sh);

    // Cancel button
    let cancel_rect = Rect::new(btn_x_start + (btn_w + btn_gap) * 2.0, cy, btn_w, btn_h);
    let cancel_state = input.add_zone(ZONE_DROP_CANCEL, cancel_rect);
    Button::new(cancel_rect, "Cancel")
        .hovered(cancel_state.is_hovered())
        .pressed(cancel_state.is_active())
        .draw(painter, text, pal, sw, sh);
}
