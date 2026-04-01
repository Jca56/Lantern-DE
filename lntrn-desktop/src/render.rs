use lntrn_render::{Color, Rect, TextureDraw};
use lntrn_ui::gpu::{
    ContextMenu, FontSize, FoxPalette, GradientStrip, InteractionContext, MenuEvent,
    ScrollArea, Scrollbar, TabBar, TextInput, TextLabel,
};

use crate::app::{App, ViewMode};
use crate::icons::{self, IconCache};
use crate::layout::*;
use crate::sections::*;
use crate::views::{draw_content_list, draw_content_tree};
use crate::{Gpu, DesktopPanel, ZONE_GLOBAL_TAB_BASE,
    ZONE_NAV_VIEW_TOGGLE, ZONE_NAV_BACK, ZONE_NAV_FORWARD, ZONE_NAV_UP, ZONE_NAV_SEARCH,
    ZONE_SIDEBAR_ITEM_BASE, ZONE_FILE_ITEM_BASE, ZONE_CONTENT, ZONE_SCROLLBAR,
    ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE, ZONE_TAB_NEW, ZONE_RENAME_INPUT, ZONE_PATH_INPUT,
    ZONE_DRIVE_ITEM_BASE, ZONE_TREE_ITEM_BASE};

/// Render a full frame.
pub fn render_frame(
    gpu: &mut Gpu,
    app: &mut App,
    input: &mut InteractionContext,
    icon_cache: &mut IconCache,
    file_info: &mut crate::file_info::FileInfoCache,
    palette: &FoxPalette,
    scale: f32,
    view_menu: &mut ContextMenu,
    context_menu: &mut ContextMenu,
    tab_drag: Option<usize>,
    bg_opacity: f32,
    active_panel: DesktopPanel,
) -> (Option<MenuEvent>, Option<MenuEvent>) {
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
    let content = content_rect(wf, hf, s);
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

    // ── Window background (transparent for layer shell) ──────────────
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 0.0, pal.bg.with_alpha(bg_opacity));

    // ── Panel content ───────────────────────────────────────────────
    if active_panel != DesktopPanel::Files {
        // Blank panel — just render the layered output and return
        painter.set_layer(1);
        let ctx_evt = context_menu.draw(painter, text, input, w, h);
        painter.set_layer(0);
        let frame = ctx.begin_frame("Lantern Desktop");
        match frame {
            Ok(mut frame) => {
                let view = frame.view().clone();
                painter.render_layer(0, ctx, frame.encoder_mut(), &view, Some(Color::rgba(0.0, 0.0, 0.0, 0.0)));
                text.render_layer(0, ctx, frame.encoder_mut(), &view);
                frame.submit(&ctx.queue);
            }
            Err(e) => eprintln!("[desktop] render error: {e}"),
        }
        return (ctx_evt, None);
    }

    // ── Files panel: Nav bar ─────────────────────────────────────────
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
    let path_zone = input.add_zone(ZONE_PATH_INPUT, p_rect);
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
        let is_pinned = app.tabs.get(i).map_or(false, |t| t.pinned);
        if app.tabs.len() > 1 && !is_pinned && i < close_rects.len() {
            let zone_id = ZONE_TAB_CLOSE_BASE + i as u32;
            let state = input.add_zone(zone_id, close_rects[i]);
            if state.is_hovered() {
                hovered_close = Some(i);
            }
        }
    }
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

    // Draw pin icons on pinned tabs
    for (i, tab) in app.tabs.iter().enumerate() {
        if tab.pinned && i < close_rects.len() {
            let cr = &close_rects[i];
            painter.rect_filled(*cr, 4.0 * s, pal.surface);
            let pc = pal.accent;
            let cx = cr.x + cr.w * 0.5;
            let cy = cr.y + cr.h * 0.5;
            painter.circle_filled(cx, cy - 3.0 * s, 3.5 * s, pc);
            painter.line(cx, cy - 0.5 * s, cx, cy + 5.0 * s, 1.5 * s, pc);
        }
    }

    // Pinned tab drag indicator
    if let Some(src_idx) = tab_drag {
        if src_idx < tab_rects.len() {
            painter.rect_stroke(tab_rects[src_idx], 4.0 * s, 2.0 * s, pal.accent);
        }
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

    // ── Status bar ──────────────────────────────────────────────────
    {
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

    // Drag ghost texture
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

    // ── Inline menus (layer 1) ─────────────────────────────────────
    painter.set_layer(1);
    text.set_layer(1);
    let ctx_menu_event = context_menu.draw(painter, text, input, w, h);
    let view_menu_event = view_menu.draw(painter, text, input, w, h);
    painter.set_layer(0);
    text.set_layer(0);

    // ── Layered render ────────────────────────────────────────────────
    let frame = ctx.begin_frame("Lantern Desktop");
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

            // Layer 1: menu overlay shapes + text
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(1, ctx, frame.encoder_mut(), &view);

            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[desktop] render error: {e}"),
    }

    (ctx_menu_event, view_menu_event)
}
