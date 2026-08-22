use lntrn_render::{Color, Rect, TextureDraw};
use lntrn_ui::gpu::{
    draw_window_bg, ContextMenu, FontSize, FoxPalette, InteractionContext, MenuEvent, ScrollArea,
    Scrollbar, TabBar, TextInput, TextLabel, TitleBar,
};

use crate::app::{App, ViewMode};
use crate::icons::{self, IconCache};
use crate::layout::*;
use crate::sections::SidebarHovered;
use crate::sections::*;
use crate::views::{draw_content_list, draw_content_tree};
use crate::{
    Gpu, ZONE_BREADCRUMB_BASE, ZONE_CLOSE, ZONE_CONTENT, ZONE_DRIVE_ITEM_BASE, ZONE_DROP_CANCEL,
    ZONE_DROP_COPY, ZONE_DROP_MOVE, ZONE_FAVORITE_ITEM_BASE, ZONE_FILE_ITEM_BASE, ZONE_MAXIMIZE,
    ZONE_MENU_VIEW, ZONE_MINIMIZE, ZONE_NAV_BACK, ZONE_NAV_FORWARD, ZONE_NAV_PREVIEW_TOGGLE,
    ZONE_NAV_SEARCH, ZONE_NAV_SORT, ZONE_NAV_UP, ZONE_NAV_VIEW_TOGGLE, ZONE_PATH_INPUT,
    ZONE_PREVIEW_RESIZE, ZONE_RENAME_INPUT, ZONE_SCROLLBAR, ZONE_SIDEBAR_DEVICES_HEADER,
    ZONE_SIDEBAR_FAVORITES_HEADER, ZONE_SIDEBAR_FAVORITES_PLUS, ZONE_SIDEBAR_ITEM_BASE,
    ZONE_SIDEBAR_PLACES_HEADER, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE, ZONE_TAB_NEW,
    ZONE_TREE_ITEM_BASE,
};
use lntrn_ui::gpu::controls::{Button, ButtonVariant};

/// Render a full frame.
pub fn render_frame(
    gpu: &mut Gpu,
    app: &mut App,
    input: &mut InteractionContext,
    icon_cache: &mut IconCache,
    file_info: &mut crate::file_info::FileInfoCache,
    git: &crate::git_status::GitStatus,
    palette: &FoxPalette,
    scale: f32,
    maximized: bool,
    view_menu: &mut ContextMenu,
    context_menu: &mut ContextMenu,
    tab_drag: Option<usize>,
    fav_drag: Option<usize>,
    bg_opacity: f32,
    desktop_mode: bool,
) -> Option<MenuEvent> {
    let Gpu {
        ctx,
        painter,
        text,
        tex_pass,
    } = gpu;

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
    icon_cache.poll_thumbs(ctx, tex_pass);
    // Quick Look texture upload happens here, before any texture borrows
    // are taken for draw lists.
    if let Some(ql) = &mut app.quick_look {
        ql.poll_upload(ctx, tex_pass);
    }
    // Split view geometry: (left_x, left_w, right_x, right_w) + focused side.
    let split_geom = app
        .split
        .as_ref()
        .map(|sp| (split_pane_cols(wf, sp.ratio, s), sp.focused));
    // The focused pane's content column (single-pane: the whole content area).
    let full_content = app.active_content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    // When searching, show search results in list mode
    let is_searching = app.searching && !app.search_buf.is_empty();
    let entries: &[crate::fs::FileEntry] = if is_searching {
        &app.search_results
    } else {
        &app.entries
    };
    let view_mode = if is_searching {
        ViewMode::List
    } else {
        app.view_mode
    };
    // Preview pane: only shown in List/Tree (and during search, which renders
    // as List). Split view claims the width — preview stays off until then.
    let preview_supported =
        matches!(view_mode, ViewMode::List | ViewMode::Tree) && app.split.is_none();
    let preview_w_px = if preview_supported {
        preview_effective_w(full_content.w, app.preview_width, app.preview_open, s)
    } else {
        0.0
    };
    // Clamp the persisted logical width back to a valid range when in use, so
    // future toggles remember the constrained value.
    if preview_w_px > 0.0 {
        app.preview_width = (preview_w_px / s).max(PREVIEW_MIN_W);
    }
    let content = if preview_w_px > 0.0 {
        Rect::new(
            full_content.x,
            full_content.y,
            full_content.w - preview_w_px,
            full_content.h,
        )
    } else {
        full_content
    };

    // Grid-specific vars (used for grid mode + icon loading)
    let cols = grid_columns(content.w, s, zoom);
    let total_content_h = match view_mode {
        ViewMode::Grid => grid_content_height(entries.len(), cols, s, zoom),
        ViewMode::List => {
            let rh = if is_searching {
                search_list_row_h(s, zoom)
            } else {
                list_row_h(s, zoom)
            };
            entries.len() as f32 * rh + 32.0 * list_zoom_multiplier(zoom) * s // +header
        }
        ViewMode::Tree => tree_content_height(app.tree_entries.len(), s, zoom),
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
            let row_h = if is_searching {
                search_list_row_h(s, zoom)
            } else {
                list_row_h(s, zoom)
            };
            let hdr_h = 32.0 * list_zoom_multiplier(zoom) * s;
            for i in 0..entries.len() {
                let y = base_y + hdr_h + i as f32 * row_h;
                if y + row_h >= content.y && y <= content.y + content.h {
                    icon_cache.get_or_load(&entries[i], ctx, tex_pass);
                }
            }
            (0..entries.len())
                .map(|i| {
                    let y = base_y + hdr_h + i as f32 * row_h;
                    y + row_h >= content.y
                        && y <= content.y + content.h
                        && icon_cache.has_icon(&entries[i])
                })
                .collect()
        }
        ViewMode::Tree => {
            let row_h = tree_row_h(s, zoom);
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
                    y + row_h >= content.y
                        && y <= content.y + content.h
                        && icon_cache.has_icon(&tree_entries[i].entry)
                })
                .collect()
        }
    };

    // ── Window background ─────────────────────────────────────────────
    // Borders are drawn by the compositor now via [window_manager].border_width
    // and border_color. draw_window_bg paints `pal.bg.with_alpha(opacity)`
    // and, if a window gradient is configured in System Settings, layers it
    // on top with per-stop alphas (transparent stops reveal the solid bg
    // underneath, not the wallpaper).
    let corner_r = if desktop_mode { 0.0 } else { 18.0 * s };
    let win_rect = Rect::new(0.0, 0.0, wf, hf);
    draw_window_bg(painter, win_rect, corner_r, pal, bg_opacity);

    // ── Title bar (window mode only, hidden in rice mode) ────────────
    if !desktop_mode && !crate::layout::chrome_hidden() {
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
            .transparent_bg(true)
            .close_hovered(close_s.is_hovered())
            .maximize_hovered(max_s.is_hovered())
            .minimize_hovered(min_s.is_hovered())
            .draw(painter, pal);

        // "View" menu label in title bar
        {
            let tb_content = TitleBar::new(tb_rect).scale(s).content_rect();
            let font = 22.0 * s;
            let pad_h = 12.0 * s;
            let label_w = font * 0.52 * 4.0 + pad_h * 2.0;
            let label_h = font + 8.0 * s;
            let label_x = tb_content.x + 10.0 * s;
            let label_y = tb_content.y + (tb_content.h - label_h) * 0.5;
            let label_rect = Rect::new(label_x, label_y, label_w, label_h);
            let view_state = input.add_zone(ZONE_MENU_VIEW, label_rect);
            let is_open = view_menu.is_open();
            let r = 6.0 * s;

            if is_open || view_state.is_hovered() {
                painter.rect_filled(
                    label_rect,
                    r,
                    pal.accent.with_alpha(if is_open { 0.15 } else { 0.10 }),
                );
            }
            let color = if is_open { pal.accent } else { pal.text };
            TextLabel::new("View", label_x + pad_h, label_y + (label_h - font) * 0.5)
                .size(FontSize::Custom(font))
                .color(color)
                .draw(text, w, h);
        }
    }

    // ── Top gradient strip (hidden in rice mode along with the bar) ──
    if !crate::layout::chrome_hidden() {
        let tb_h = title_bar_rect(wf, s).h;
        draw_gradient_h(painter, pal, 0.0, tb_h, wf, s);
    }

    // ── Nav bar ───────────────────────────────────────────────────────
    // In split mode the focused pane's nav bar shrinks to its column (cloud
    // and preview buttons drop out — a zero-width rect skips both zone and
    // draw), but keeps the standard zone IDs so click handling is unchanged.
    let active_col = split_geom.map(|((lx, lw, rx, rw), focused)| match focused {
        crate::app::PaneSide::Left => (lx, lw),
        crate::app::PaneSide::Right => (rx, rw),
    });
    let (nav_rect, vt_rect, back_rect, fwd_rect, up_rect, cloud_rect, p_rect) =
        if let Some((px, pw)) = active_col {
            let is_right = matches!(split_geom, Some((_, crate::app::PaneSide::Right)));
            (
                pane_nav_bar_rect(px, pw, s),
                pane_view_toggle_rect(px, s),
                pane_back_rect(px, s),
                pane_forward_rect(px, s),
                pane_up_rect(px, s),
                Rect::new(0.0, 0.0, 0.0, 0.0),
                pane_path_rect(px, pw, is_right, s),
            )
        } else {
            (
                nav_bar_rect(wf, s),
                view_toggle_rect(s),
                back_button_rect(s),
                forward_button_rect(s),
                up_button_rect(s),
                crate::layout::cloud_button_rect(s),
                path_rect(wf, s),
            )
        };
    let vt_state = input.add_zone(ZONE_NAV_VIEW_TOGGLE, vt_rect);
    let back_state = input.add_zone(ZONE_NAV_BACK, back_rect);
    let fwd_state = input.add_zone(ZONE_NAV_FORWARD, fwd_rect);
    let up_state = input.add_zone(ZONE_NAV_UP, up_rect);
    let cloud_state = if cloud_rect.w > 0.0 {
        input.add_zone(crate::ZONE_NAV_CLOUD, cloud_rect)
    } else {
        lntrn_ui::gpu::InteractionState::Idle
    };
    let mut breadcrumb_hovered = Vec::new();
    let path_hovered;
    if !app.path_editing && !app.searching {
        // Register breadcrumb segment zones
        let segments = crate::sections::breadcrumb_segments(&app.current_dir, s);
        let font = 22.0 * s;
        let char_w = font * 0.45;
        let sep_w = 14.0 * s;
        let pad_x = 6.0 * s;
        let seg_width = |name: &str| -> f32 { name.len() as f32 * char_w + pad_x * 2.0 };

        // Compute overflow skip
        let total_w: f32 = segments
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                if i > 0 {
                    seg_width(name) + sep_w
                } else {
                    seg_width(name)
                }
            })
            .sum();
        let mut skip = 0;
        if total_w > p_rect.w {
            let ellipsis_w = seg_width("...") + sep_w;
            for (i, _) in segments.iter().enumerate() {
                let remaining: f32 = segments[i..]
                    .iter()
                    .enumerate()
                    .map(|(j, (n, _))| {
                        if j > 0 {
                            seg_width(n) + sep_w
                        } else {
                            seg_width(n)
                        }
                    })
                    .sum();
                if ellipsis_w + remaining <= p_rect.w {
                    break;
                }
                skip = i + 1;
            }
        }

        app.breadcrumb_skip = skip;

        let mut cx = p_rect.x + 4.0 * s;
        if skip > 0 {
            cx += seg_width("...") + sep_w;
        }
        for (i, (name, _)) in segments.iter().enumerate() {
            if i < skip {
                continue;
            }
            if i > skip {
                cx += sep_w;
            }
            let sw = seg_width(name);
            let seg_rect = Rect::new(cx, p_rect.y + 2.0 * s, sw, p_rect.h - 4.0 * s);
            let zone_id = ZONE_BREADCRUMB_BASE + (i - skip) as u32;
            let state = input.add_zone(zone_id, seg_rect);
            breadcrumb_hovered.push(state.is_hovered());
            cx += sw;
        }
        // Register remaining empty area for clicking into edit mode
        let remaining_w = (p_rect.x + p_rect.w) - cx;
        if remaining_w > 0.0 {
            let empty_rect = Rect::new(cx, p_rect.y, remaining_w, p_rect.h);
            input.add_zone(ZONE_PATH_INPUT, empty_rect);
        }
        path_hovered = false;
    } else {
        let zone = input.add_zone(ZONE_PATH_INPUT, p_rect);
        path_hovered = zone.is_hovered();
    };
    let preview_btn_rect = if active_col.is_some() {
        Rect::new(0.0, 0.0, 0.0, 0.0)
    } else {
        preview_toggle_rect(wf, s)
    };
    // Only register the preview toggle zone when the active view supports it,
    // so it can't be clicked in Grid mode.
    let preview_btn_state = if preview_supported {
        input.add_zone(ZONE_NAV_PREVIEW_TOGGLE, preview_btn_rect)
    } else {
        lntrn_ui::gpu::InteractionState::Idle
    };
    let (sort_rect, srch_rect) = if let Some((px, pw)) = active_col {
        (pane_sort_rect(px, pw, s), pane_search_rect(px, pw, s))
    } else {
        (sort_button_rect(wf, s), search_button_rect(wf, s))
    };
    let sort_state = input.add_zone(ZONE_NAV_SORT, sort_rect);
    let srch_state = input.add_zone(ZONE_NAV_SEARCH, srch_rect);

    // Split toggle: top-right of the window — the single nav bar, or the
    // right pane's nav when split is on and the right pane is focused (the
    // unfocused right pane registers it in render_inactive_pane instead).
    let split_btn_rect = match split_geom {
        None => Some(split_toggle_rect(wf, s)),
        Some(((_, _, rx, rw), crate::app::PaneSide::Right)) => {
            Some(pane_split_toggle_rect(rx, rw, s))
        }
        Some((_, crate::app::PaneSide::Left)) => None,
    };
    if let Some(sb_rect) = split_btn_rect {
        let sb_hov = input
            .add_zone(crate::ZONE_SPLIT_TOGGLE, sb_rect)
            .is_hovered();
        let active = app.split.is_some();
        let color = if active {
            pal.accent
        } else if sb_hov {
            pal.text
        } else {
            pal.text_secondary
        };
        if sb_hov || active {
            let bg = if active {
                pal.accent.with_alpha(0.15)
            } else {
                pal.surface_2.with_alpha(0.5)
            };
            painter.rect_filled(sb_rect, 4.0 * s, bg);
        }
        crate::sections::draw_split_toggle_icon(painter, sb_rect, color, s);
    }
    draw_nav_bar(
        painter,
        text,
        pal,
        app,
        nav_rect,
        vt_rect,
        vt_state.is_hovered(),
        back_rect,
        back_state.is_hovered(),
        fwd_rect,
        fwd_state.is_hovered(),
        up_rect,
        up_state.is_hovered(),
        cloud_rect,
        cloud_state.is_hovered(),
        p_rect,
        path_hovered,
        &breadcrumb_hovered,
        preview_btn_rect,
        preview_btn_state.is_hovered(),
        preview_supported,
        sort_rect,
        sort_state.is_hovered(),
        srch_rect,
        srch_state.is_hovered(),
        (w, h),
        s,
    );

    // Focused-pane indicator: accent underline beneath the focused pane's
    // nav bar so it's obvious which pane keyboard/actions apply to.
    if active_col.is_some() {
        painter.rect_filled(
            Rect::new(
                nav_rect.x + 6.0 * s,
                nav_rect.y + nav_rect.h - 2.0 * s,
                nav_rect.w - 12.0 * s,
                2.0 * s,
            ),
            1.0 * s,
            pal.accent.with_alpha(0.7),
        );
    }

    draw_gradient_h(painter, pal, 0.0, nav_rect.y + nav_rect.h, wf, s);

    // ── Tab bar (always the LEFT pane's — the right pane has no tabs) ──
    let tab_rect = if let Some(((lx, lw, _, _), _)) = split_geom {
        pane_tab_bar_rect(lx, lw, s)
    } else {
        tab_bar_rect(wf, s)
    };
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
        .transparent_bg(true)
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
            if let Some(target) = tab_rects
                .iter()
                .position(|r| r.contains(cursor_x, r.y + r.h * 0.5))
            {
                if target != src_idx && target < app.tabs.len() && app.tabs[target].pinned {
                    let ind_x = tab_rects[target].x;
                    let ind_y = tab_rects[target].y + 4.0 * s;
                    let ind_h = tab_rects[target].h - 8.0 * s;
                    painter.rect_filled(
                        Rect::new(ind_x - 1.5 * s, ind_y, 3.0 * s, ind_h),
                        1.5 * s,
                        pal.accent,
                    );
                }
            }
        }
    }

    // Drop-target highlight on tabs while dragging files
    if app.drag_item.is_some() || app.drag_tree_item.is_some() {
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
    let favorites = app.sidebar_favorites();
    let sb_layout = build_sidebar_layout(
        s,
        places.len(),
        favorites.len(),
        app.drives.len(),
        app.phones.len(),
        app.places_collapsed,
        app.favorites_collapsed,
        app.devices_collapsed,
    );

    // Section header zones (toggle collapse on click).
    let places_header_hov = input
        .add_zone(ZONE_SIDEBAR_PLACES_HEADER, sb_layout.places_header)
        .is_hovered();
    let favorites_header_hov = input
        .add_zone(ZONE_SIDEBAR_FAVORITES_HEADER, sb_layout.favorites_header)
        .is_hovered();
    let favorites_plus_hov = input
        .add_zone(ZONE_SIDEBAR_FAVORITES_PLUS, sb_layout.favorites_plus)
        .is_hovered();
    let devices_header_hov = if sb_layout.has_devices {
        input
            .add_zone(ZONE_SIDEBAR_DEVICES_HEADER, sb_layout.devices_header)
            .is_hovered()
    } else {
        false
    };

    let mut sidebar_hovered = Vec::with_capacity(places.len());
    for (i, item_rect) in sb_layout.place_items.iter().enumerate() {
        let zone_id = ZONE_SIDEBAR_ITEM_BASE + i as u32;
        sidebar_hovered.push(input.add_zone(zone_id, *item_rect).is_hovered());
    }
    let mut favorite_hovered = Vec::with_capacity(favorites.len());
    for (i, item_rect) in sb_layout.favorite_items.iter().enumerate() {
        let zone_id = ZONE_FAVORITE_ITEM_BASE + i as u32;
        favorite_hovered.push(input.add_zone(zone_id, *item_rect).is_hovered());
    }
    let mut drive_hovered = Vec::with_capacity(app.drives.len());
    for (i, item_rect) in sb_layout.drive_items.iter().enumerate() {
        let zone_id = ZONE_DRIVE_ITEM_BASE + i as u32;
        drive_hovered.push(input.add_zone(zone_id, *item_rect).is_hovered());
    }
    let mut phone_hovered = Vec::with_capacity(app.phones.len());
    for (i, item_rect) in sb_layout.phone_items.iter().enumerate() {
        let zone_id = crate::ZONE_PHONE_ITEM_BASE + i as u32;
        phone_hovered.push(input.add_zone(zone_id, *item_rect).is_hovered());
    }
    let dragging = app.drag_item.is_some() || app.drag_tree_item.is_some();
    let hov = SidebarHovered {
        places: &sidebar_hovered,
        favorites: &favorite_hovered,
        drives: &drive_hovered,
        phones: &phone_hovered,
        places_header: places_header_hov,
        favorites_header: favorites_header_hov,
        devices_header: devices_header_hov,
        favorites_plus: favorites_plus_hov,
    };
    draw_sidebar(
        painter,
        text,
        pal,
        app,
        sidebar,
        &sb_layout,
        &hov,
        dragging,
        fav_drag,
        (w, h),
        s,
    );

    // ── Content area ───────────────────────────────────────────────────
    input.add_zone(ZONE_CONTENT, content);

    match view_mode {
        ViewMode::Grid => {
            let mut item_hovered = Vec::with_capacity(entries.len());
            for i in 0..entries.len() {
                let item_rect = file_item_rect(i, cols, content.x, base_y, s, zoom);
                // Tight hitbox around the icon+label — the cell padding stays
                // click-through so right-clicks between items hit empty space.
                let hit_rect = crate::layout::item_hit_rect(item_rect, s, zoom);
                let zone_id = ZONE_FILE_ITEM_BASE + i as u32;
                let hovered = if let Some(clipped) = hit_rect.intersect(&content) {
                    input.add_zone(zone_id, clipped).is_hovered()
                } else {
                    false
                };
                item_hovered.push(hovered);
            }
            draw_content_grid(
                painter,
                text,
                pal,
                content,
                entries,
                cols,
                &scroll_area,
                &item_hovered,
                &has_icon,
                app.drag_item,
                app.renaming,
                git,
                (w, h),
                s,
                zoom,
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
                        .selection(app.rename_selection)
                        .focused(true)
                        .scale(s)
                        .draw(painter, text, pal, w, h);
                }
            }
        }
        ViewMode::List => {
            let row_h = if is_searching {
                search_list_row_h(s, zoom)
            } else {
                list_row_h(s, zoom)
            };
            let hdr_h = 32.0 * list_zoom_multiplier(zoom) * s;
            let mut item_hovered = Vec::with_capacity(entries.len());
            for i in 0..entries.len() {
                let y = base_y + hdr_h + i as f32 * row_h;
                // Off-screen rows get no zone (and skip the text measuring
                // the tight rect needs).
                if y + row_h < content.y || y > content.y + content.h {
                    item_hovered.push(false);
                    continue;
                }
                // Tight zone: icon + name only — the size/date columns stay
                // empty space (right-click → folder menu), matching grid
                // view. Search rows keep the full row (no empty-area menu
                // there, and the location line spans the row anyway).
                let row_rect = if is_searching {
                    Rect::new(content.x, y, content.w, row_h)
                } else {
                    crate::views::list_row_hit_rect(text, &entries[i], content, y, row_h, s, zoom)
                };
                let zone_id = ZONE_FILE_ITEM_BASE + i as u32;
                let hovered = if let Some(clipped) = row_rect.intersect(&content) {
                    input.add_zone(zone_id, clipped).is_hovered()
                } else {
                    false
                };
                item_hovered.push(hovered);
            }
            let search_root = if is_searching {
                Some(app.current_dir.as_path())
            } else {
                None
            };
            draw_content_list(
                painter,
                text,
                pal,
                content,
                entries,
                &scroll_area,
                &item_hovered,
                &has_icon,
                app.drag_item,
                app.renaming,
                search_root,
                git,
                (w, h),
                s,
                zoom,
            );

            // Inline rename input (list mode)
            if let Some(rename_idx) = app.renaming {
                if rename_idx < entries.len() {
                    let m = list_zoom_multiplier(zoom);
                    let y = base_y + hdr_h + rename_idx as f32 * row_h;
                    let input_x = content.x + 42.0 * m * s;
                    let input_w = (content.x + content.w - input_x - 12.0 * m * s).max(120.0 * s);
                    let input_h = (row_h - 4.0 * s).max(28.0 * s);
                    let input_y = y + (row_h - input_h) * 0.5;
                    let input_rect = Rect::new(input_x, input_y, input_w, input_h);
                    input.add_zone(ZONE_RENAME_INPUT, input_rect);
                    TextInput::new(input_rect)
                        .text(&app.rename_buf)
                        .cursor_pos(app.rename_cursor)
                        .selection(app.rename_selection)
                        .focused(true)
                        .scale(s)
                        .draw(painter, text, pal, w, h);
                }
            }
        }
        ViewMode::Tree => {
            let row_h = tree_row_h(s, zoom);
            let tree_entries = &app.tree_entries;
            let mut item_hovered = Vec::with_capacity(tree_entries.len());
            for i in 0..tree_entries.len() {
                let y = base_y + i as f32 * row_h;
                if y + row_h < content.y || y > content.y + content.h {
                    item_hovered.push(false);
                    continue;
                }
                // Tight zone: arrow + icon + name only, so the space right of
                // a name is empty (right-click → folder menu), matching grid.
                let row_rect = crate::views::tree_row_hit_rect(
                    text,
                    &tree_entries[i],
                    content,
                    y,
                    row_h,
                    s,
                    zoom,
                );
                let zone_id = ZONE_TREE_ITEM_BASE + i as u32;
                let hovered = if let Some(clipped) = row_rect.intersect(&content) {
                    input.add_zone(zone_id, clipped).is_hovered()
                } else {
                    false
                };
                item_hovered.push(hovered);
            }
            // Determine which tree row (if any) corresponds to the currently
            // renamed file by matching the path. Tree indices don't line up
            // with `app.entries` indices because of expand state.
            let renaming_path = app
                .renaming
                .and_then(|i| app.entries.get(i))
                .map(|e| e.path.clone());
            // Build per-tree-row selected flags by looking up each tree
            // entry's path in both `app.entries` (top-level rows) and
            // `app.pick_tree_selection` (pick-mode nested rows).
            let mut selected_paths: std::collections::HashSet<&std::path::Path> = app
                .entries
                .iter()
                .filter(|e| e.selected)
                .map(|e| e.path.as_path())
                .collect();
            for p in &app.pick_tree_selection {
                selected_paths.insert(p.as_path());
            }
            let tree_selected: Vec<bool> = tree_entries
                .iter()
                .map(|te| selected_paths.contains(te.entry.path.as_path()))
                .collect();
            draw_content_tree(
                painter,
                text,
                pal,
                content,
                tree_entries,
                &scroll_area,
                &item_hovered,
                &has_icon,
                &tree_selected,
                app.drag_tree_item,
                renaming_path.as_deref(),
                (w, h),
                s,
                zoom,
            );

            // Inline rename input (tree mode)
            if let Some(ref path) = renaming_path {
                if let Some((ti, te)) = tree_entries
                    .iter()
                    .enumerate()
                    .find(|(_, t)| t.entry.path == *path)
                {
                    let m = list_zoom_multiplier(zoom);
                    let indent = 28.0 * m * s;
                    let icon_sz = 24.0 * m * s;
                    let row_left =
                        content.x + 8.0 * m * s + te.depth as f32 * indent + 16.0 * m * s;
                    let input_x = row_left + icon_sz + 8.0 * m * s;
                    let y = base_y + ti as f32 * row_h;
                    let input_w = (content.x + content.w - input_x - 12.0 * m * s).max(120.0 * s);
                    let input_h = (row_h - 4.0 * s).max(28.0 * s);
                    let input_y = y + (row_h - input_h) * 0.5;
                    let input_rect = Rect::new(input_x, input_y, input_w, input_h);
                    input.add_zone(ZONE_RENAME_INPUT, input_rect);
                    TextInput::new(input_rect)
                        .text(&app.rename_buf)
                        .cursor_pos(app.rename_cursor)
                        .selection(app.rename_selection)
                        .focused(true)
                        .scale(s)
                        .draw(painter, text, pal, w, h);
                }
            }
        }
    }

    // Scrollbar — the zone is the widened hover band over the whole track,
    // not just the thumb, so it's grabbable and track-clicks page-jump.
    if scroll_area.is_scrollable() {
        let scrollbar = Scrollbar::new(&content, total_content_h, app.scroll_offset);
        input.add_zone(ZONE_SCROLLBAR, scrollbar.hover_zone());
        let sb_state = input.zone_state(ZONE_SCROLLBAR);
        draw_scrollbar(painter, &scrollbar, sb_state, pal);
    }

    // ── Preview pane ───────────────────────────────────────────────────
    let mut preview_thumb_rect: Option<Rect> = None;
    let mut preview_thumb_entry: Option<crate::fs::FileEntry> = None;
    if preview_w_px > 0.0 {
        let pane = preview_pane_rect(full_content, preview_w_px);
        let handle = preview_handle_rect(full_content, preview_w_px, s);
        let handle_state = input.add_zone(ZONE_PREVIEW_RESIZE, handle);
        let entry = crate::preview::preview_entry(entries);
        // Ensure the thumbnail icon is loaded.
        if let Some(e) = entry {
            icon_cache.get_or_load(e, ctx, tex_pass);
        }
        preview_thumb_rect = crate::preview::draw_preview_pane(
            painter,
            text,
            pal,
            file_info,
            pane,
            entry,
            handle,
            handle_state.is_hovered(),
            app.preview_drag.is_some(),
            (w, h),
            s,
        );
        preview_thumb_entry = entry.cloned();
    }

    // ── Status bar / Pick bar ──────────────────────────────────────────
    if app.pick.is_some() {
        let bar_y = hf - crate::pick_bar::PICK_BAR_H * s;
        crate::pick_bar::draw_pick_bar(app, painter, text, pal, input, wf, bar_y, s, (w, h));
    } else {
        let status = status_rect(wf, hf, s);
        // Only surface sync status while the user is actually inside ~/Cloud,
        // so the pill doesn't follow them around the filesystem.
        let in_cloud = app.current_dir.starts_with(crate::cloud::cloud_root());
        let cloud_status = if in_cloud {
            app.cloud_sync.as_ref().map(|h| h.status())
        } else {
            None
        };
        draw_status_bar(
            painter,
            text,
            pal,
            status,
            &app.entries,
            file_info,
            cloud_status,
            app.op_progress.as_ref(),
            git.branch(),
            input,
            (w, h),
            s,
        );
    }

    // ── Inactive split pane + divider ─────────────────────────────────
    // The scroll offset update is deferred to the end of the frame — `app`
    // still has immutable borrows (`entries`) live here.
    let mut p2_scroll_new: Option<f32> = None;
    if let Some(((lx, lw, rx, rw), focused)) = split_geom {
        if let Some((tab, view, _side)) = app.inactive_pane() {
            let is_right = focused == crate::app::PaneSide::Left;
            let (px, pw) = if is_right { (rx, rw) } else { (lx, lw) };
            let dragging = app.drag_item.is_some() || app.drag_tree_item.is_some();
            p2_scroll_new = Some(crate::sections::render_inactive_pane(
                painter,
                text,
                ctx,
                tex_pass,
                input,
                icon_cache,
                git,
                pal,
                crate::sections::InactivePane {
                    tab,
                    view,
                    is_right,
                    pane_x: px,
                    pane_w: pw,
                    scroll: tab.scroll_offset,
                    zoom,
                    dragging,
                },
                hf,
                (w, h),
                s,
            ));
        }
        // Divider handle between the panes.
        let split_ratio = app.split.as_ref().map(|sp| sp.ratio).unwrap_or(0.5);
        let divider = split_divider_rect(wf, hf, split_ratio, s);
        let div_state = input.add_zone(crate::ZONE_SPLIT_DIVIDER, divider);
        let div_active = app
            .split
            .as_ref()
            .map_or(false, |sp| sp.divider_drag.is_some());
        crate::sections::draw_split_divider(
            painter,
            divider,
            div_state.is_hovered() || div_active,
            pal,
            s,
        );
    }

    // ── Rubber band selection overlay ─────────────────────────────────
    if let (Some(start), Some(end)) = (app.rubber_band_start, app.rubber_band_end) {
        draw_rubber_band(painter, pal, start, end, content);
    }

    // ── Drag ghost overlay (shapes, drawn in base pass) ────────────
    let drag_count = if app.drag_item.is_some() {
        let sel = entries.iter().filter(|e| e.selected).count();
        sel.max(1)
    } else if let Some(ti) = app.drag_tree_item {
        // Tree multi-drag only when the grabbed row is part of the
        // selection (selection lives in `entries`, keyed by path).
        match app.tree_entries.get(ti) {
            Some(te)
                if app
                    .entries
                    .iter()
                    .any(|e| e.selected && e.path == te.entry.path) =>
            {
                app.entries.iter().filter(|e| e.selected).count().max(1)
            }
            Some(_) => 1,
            None => 0,
        }
    } else {
        0
    };

    // Count badge for multi-drag
    if drag_count > 1 {
        if let Some((dx, dy)) = app.drag_pos {
            let ghost_sz = 80.0 * s;
            let gx = dx - ghost_sz * 0.5;
            let gy = dy - ghost_sz * 0.5;
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

    // ── Collect texture draws for icons ────────────────────────────────
    let content_clip = [content.x, content.y, content.w, content.h];
    let mut tex_draws: Vec<TextureDraw> = match view_mode {
        ViewMode::Grid => (0..entries.len())
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
                let alpha = if app.drag_item.is_some() && entries[i].selected {
                    0.3
                } else {
                    1.0
                };
                let mut draw = TextureDraw::new(tex, dx, dy, dw, dh).opacity(alpha);
                draw.clip = Some(content_clip);
                Some(draw)
            })
            .collect(),
        ViewMode::List => {
            let m = list_zoom_multiplier(zoom);
            let row_h = if is_searching {
                search_list_row_h(s, zoom)
            } else {
                list_row_h(s, zoom)
            };
            let hdr_h = 32.0 * m * s;
            let list_icon_sz = 28.0 * m * s;
            (0..entries.len())
                .filter(|&i| has_icon[i])
                .filter_map(|i| {
                    let y = base_y + hdr_h + i as f32 * row_h;
                    let icon_x = content.x + 8.0 * m * s;
                    let icon_y = y + (row_h - list_icon_sz) * 0.5;
                    let tex = icon_cache.get(&entries[i])?;
                    let (dx, dy, dw, dh) =
                        icons::fit_in_box(tex, icon_x, icon_y, list_icon_sz, list_icon_sz);
                    let alpha = if app.drag_item == Some(i) { 0.3 } else { 1.0 };
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh).opacity(alpha);
                    draw.clip = Some(content_clip);
                    Some(draw)
                })
                .collect()
        }
        ViewMode::Tree => {
            let m = list_zoom_multiplier(zoom);
            let row_h = tree_row_h(s, zoom);
            let tree_indent = 28.0 * m * s;
            let tree_icon_sz = 24.0 * m * s;
            let tree_entries = &app.tree_entries;
            (0..tree_entries.len())
                .filter(|&i| has_icon[i])
                .filter_map(|i| {
                    let te = &tree_entries[i];
                    let y = base_y + i as f32 * row_h;
                    let x_offset = te.depth as f32 * tree_indent;
                    let icon_x = content.x + 8.0 * m * s + x_offset + 16.0 * m * s;
                    let icon_y = y + (row_h - tree_icon_sz) * 0.5;
                    let tex = icon_cache.get(&te.entry)?;
                    let (dx, dy, dw, dh) =
                        icons::fit_in_box(tex, icon_x, icon_y, tree_icon_sz, tree_icon_sz);
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh);
                    draw.clip = Some(content_clip);
                    Some(draw)
                })
                .collect()
        }
    };

    // ── Inactive split pane icon draws ────────────────────────────────
    // The unfocused pane's content was painted in render_inactive_pane, but
    // its icon TEXTURES render here — all icons must go through the single
    // tex_draws pass (a second render_pass would clobber the shared
    // instance buffer). Geometry mirrors render_inactive_pane exactly.
    if let (Some(icr), Some((tab, view, _))) =
        (app.inactive_content_rect(wf, hf, s), app.inactive_pane())
    {
        let p2_clip = [icr.x, icr.y, icr.w, icr.h];
        let p2_entries = &tab.entries;
        // Mirror ScrollArea's clamp so icons can't drift from the rows for
        // a frame when the stored offset exceeds the new max.
        let clamp_scroll =
            |total_h: f32| -> f32 { tab.scroll_offset.min((total_h - icr.h).max(0.0)).max(0.0) };
        match view.view_mode {
            ViewMode::Grid => {
                let p2_cols = grid_columns(icr.w, s, zoom);
                let total_h = grid_content_height(p2_entries.len(), p2_cols, s, zoom);
                let p2_base_y = icr.y - clamp_scroll(total_h);
                tex_draws.extend((0..p2_entries.len()).filter_map(|i| {
                    let ir = file_item_rect(i, p2_cols, icr.x, p2_base_y, s, zoom);
                    if ir.intersect(&icr).is_none() {
                        return None;
                    }
                    let icon_x = ir.x + (ir.w - icsz) * 0.5;
                    let label_font = 16.0 * s;
                    let content_h = icsz + 2.0 * s + label_font;
                    let icon_y = ir.y + (ir.h - content_h) * 0.5;
                    let tex = icon_cache.get(&p2_entries[i])?;
                    let (dx, dy, dw, dh) = icons::fit_in_box(tex, icon_x, icon_y, icsz, icsz);
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh);
                    draw.clip = Some(p2_clip);
                    Some(draw)
                }));
            }
            ViewMode::List => {
                let m = list_zoom_multiplier(zoom);
                let row_h = list_row_h(s, zoom);
                let hdr_h = 32.0 * m * s;
                let list_icon_sz = 28.0 * m * s;
                let total_h = p2_entries.len() as f32 * row_h + hdr_h;
                let p2_base_y = icr.y - clamp_scroll(total_h);
                tex_draws.extend((0..p2_entries.len()).filter_map(|i| {
                    let y = p2_base_y + hdr_h + i as f32 * row_h;
                    if y + row_h < icr.y || y > icr.y + icr.h {
                        return None;
                    }
                    let icon_x = icr.x + 8.0 * m * s;
                    let icon_y = y + (row_h - list_icon_sz) * 0.5;
                    let tex = icon_cache.get(&p2_entries[i])?;
                    let (dx, dy, dw, dh) =
                        icons::fit_in_box(tex, icon_x, icon_y, list_icon_sz, list_icon_sz);
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh);
                    draw.clip = Some(p2_clip);
                    Some(draw)
                }));
            }
            ViewMode::Tree => {
                let m = list_zoom_multiplier(zoom);
                let row_h = tree_row_h(s, zoom);
                let tree_indent = 28.0 * m * s;
                let tree_icon_sz = 24.0 * m * s;
                let total_h = tree_content_height(view.tree_entries.len(), s, zoom);
                let p2_base_y = icr.y - clamp_scroll(total_h);
                tex_draws.extend((0..view.tree_entries.len()).filter_map(|i| {
                    let te = &view.tree_entries[i];
                    let y = p2_base_y + i as f32 * row_h;
                    if y + row_h < icr.y || y > icr.y + icr.h {
                        return None;
                    }
                    let icon_x = icr.x + 8.0 * m * s + te.depth as f32 * tree_indent + 16.0 * m * s;
                    let icon_y = y + (row_h - tree_icon_sz) * 0.5;
                    let tex = icon_cache.get(&te.entry)?;
                    let (dx, dy, dw, dh) =
                        icons::fit_in_box(tex, icon_x, icon_y, tree_icon_sz, tree_icon_sz);
                    let mut draw = TextureDraw::new(tex, dx, dy, dw, dh);
                    draw.clip = Some(p2_clip);
                    Some(draw)
                }));
            }
        }
    }

    // ── Play-button overlays for video thumbnails ─────────────────────
    // Recompute the drawn icon rect for any video entry that has a real
    // thumbnail, so we can stamp a play glyph on top after the textures
    // render. Mirrors the geometry of the tex_draws collection above.
    let mut video_overlays: Vec<(f32, f32, f32, f32)> = match view_mode {
        ViewMode::Grid => (0..entries.len())
            .filter(|&i| has_icon[i] && icons::is_video_file(&entries[i].name))
            .filter_map(|i| {
                let ir = file_item_rect(i, cols, content.x, base_y, s, zoom);
                let icon_x = ir.x + (ir.w - icsz) * 0.5;
                let label_font = 16.0 * s;
                let content_h = icsz + 2.0 * s + label_font;
                let top_pad = (ir.h - content_h) * 0.5;
                let icon_y = ir.y + top_pad;
                let tex = icon_cache.get(&entries[i])?;
                Some(icons::fit_in_box(tex, icon_x, icon_y, icsz, icsz))
            })
            .collect(),
        ViewMode::List => {
            let m = list_zoom_multiplier(zoom);
            let row_h = if is_searching {
                search_list_row_h(s, zoom)
            } else {
                list_row_h(s, zoom)
            };
            let hdr_h = 32.0 * m * s;
            let list_icon_sz = 28.0 * m * s;
            (0..entries.len())
                .filter(|&i| has_icon[i] && icons::is_video_file(&entries[i].name))
                .filter_map(|i| {
                    let y = base_y + hdr_h + i as f32 * row_h;
                    let icon_x = content.x + 8.0 * m * s;
                    let icon_y = y + (row_h - list_icon_sz) * 0.5;
                    let tex = icon_cache.get(&entries[i])?;
                    Some(icons::fit_in_box(
                        tex,
                        icon_x,
                        icon_y,
                        list_icon_sz,
                        list_icon_sz,
                    ))
                })
                .collect()
        }
        ViewMode::Tree => {
            let m = list_zoom_multiplier(zoom);
            let row_h = tree_row_h(s, zoom);
            let tree_indent = 28.0 * m * s;
            let tree_icon_sz = 24.0 * m * s;
            let tree_entries = &app.tree_entries;
            (0..tree_entries.len())
                .filter(|&i| has_icon[i] && icons::is_video_file(&tree_entries[i].entry.name))
                .filter_map(|i| {
                    let te = &tree_entries[i];
                    let y = base_y + i as f32 * row_h;
                    let x_offset = te.depth as f32 * tree_indent;
                    let icon_x = content.x + 8.0 * m * s + x_offset + 16.0 * m * s;
                    let icon_y = y + (row_h - tree_icon_sz) * 0.5;
                    let tex = icon_cache.get(&te.entry)?;
                    Some(icons::fit_in_box(
                        tex,
                        icon_x,
                        icon_y,
                        tree_icon_sz,
                        tree_icon_sz,
                    ))
                })
                .collect()
        }
    };
    // Preview pane: a play badge on the large video thumbnail too.
    if let (Some(thumb), Some(entry)) = (preview_thumb_rect, preview_thumb_entry.as_ref()) {
        if icons::is_video_file(&entry.name) {
            if let Some(tex) = icon_cache.get(entry) {
                video_overlays.push(icons::fit_in_box(tex, thumb.x, thumb.y, thumb.w, thumb.h));
            }
        }
    }

    // Drag ghost texture — just the icon, centered on cursor
    let ghost_entry = if let Some(drag_idx) = app.drag_item {
        entries.get(drag_idx)
    } else if let Some(ti) = app.drag_tree_item {
        app.tree_entries.get(ti).map(|te| &te.entry)
    } else {
        None
    };
    if let (Some(entry), Some((dx, dy))) = (ghost_entry, app.drag_pos) {
        let ghost_sz = 80.0 * s;
        let gx = dx - ghost_sz * 0.5;
        let gy = dy - ghost_sz * 0.5;
        if let Some(tex) = icon_cache.get(entry) {
            let (bx, by, bw, bh) = icons::fit_in_box(tex, gx, gy, ghost_sz, ghost_sz);
            tex_draws.push(TextureDraw::new(tex, bx, by, bw, bh));
        }
    }

    // Preview pane thumbnail texture
    if let (Some(thumb), Some(ref entry)) = (preview_thumb_rect, preview_thumb_entry) {
        if let Some(tex) = icon_cache.get(entry) {
            let (bx, by, bw, bh) = icons::fit_in_box(tex, thumb.x, thumb.y, thumb.w, thumb.h);
            tex_draws.push(TextureDraw::new(tex, bx, by, bw, bh));
        }
    }

    // ── Modal overlays (layer 1) ─────────────────────────────────────
    painter.set_layer(1);
    text.set_layer(1);

    // Play-button overlays sit on layer 1 so they paint over the video
    // thumbnail textures (which render after layer-0 painter shapes).
    for (dx, dy, dw, dh) in &video_overlays {
        let cx = dx + dw * 0.5;
        let cy = dy + dh * 0.5;
        // Circle scales with the thumbnail, clamped so tiny list icons
        // still get a legible badge.
        let r = (dw.min(*dh) * 0.28).clamp(7.0 * s, 26.0 * s);
        painter.circle_filled(cx, cy, r, Color::rgba(0.0, 0.0, 0.0, 0.55));
        // White right-pointing triangle, inset within the circle.
        let t = r * 0.5;
        let off = r * 0.12; // nudge right so it looks optically centered
        painter.triangle(
            cx - t * 0.7 + off,
            cy - t,
            cx - t * 0.7 + off,
            cy + t,
            cx + t + off,
            cy,
            Color::rgba(1.0, 1.0, 1.0, 0.95),
        );
    }

    let mut props_tex_draws = Vec::new();
    // Collect props action outside the &mut borrow so we can mutate
    // app.properties / icon_cache after the dialog draws.
    let mut props_close = false;
    let mut props_icon_chosen: Option<(std::path::PathBuf, std::path::PathBuf)> = None;
    let mut props_icon_reset: Option<std::path::PathBuf> = None;
    let mut props_copy_text: Option<String> = None;
    let mut props_icon_rect_data: Option<(crate::fs::FileEntry, (f32, f32, f32, f32))> = None;
    if let Some(ref mut props) = app.properties {
        let evt = crate::properties::draw_properties_dialog(
            props, painter, text, input, pal, wf, hf, s, w, h,
        );
        match evt {
            Some(crate::properties::PropertiesEvent::Close) => {
                props_close = true;
            }
            Some(crate::properties::PropertiesEvent::IconChosen(icon_path)) => {
                props_icon_chosen = Some((props.path.clone(), icon_path));
                props.picker_open = false;
            }
            Some(crate::properties::PropertiesEvent::IconReset) => {
                props_icon_reset = Some(props.path.clone());
                props.picker_open = false;
            }
            Some(crate::properties::PropertiesEvent::CopyText(t)) => {
                props_copy_text = Some(t);
            }
            None => {}
        }
        if let Some((ix, iy, iw, ih)) = props.icon_rect {
            let entry = crate::fs::FileEntry {
                name: props.name.clone(),
                path: props.path.clone(),
                is_dir: props.is_dir,
                size: props.size_bytes,
                modified: None,
                selected: false,
            };
            props_icon_rect_data = Some((entry, (ix, iy, iw, ih)));
        }
        // Picker cell thumbnails: the wayland loop pre-warms the SVG cache
        // (between frames, when icon_cache isn't borrowed by tex_draws).
        // Here we only do immutable lookups.
        for (path, cx, cy, cw, ch) in &props.picker_cell_rects {
            if let Some(tex) = icon_cache.get_svg_path(path) {
                let (bx, by, bw, bh) = icons::fit_in_box(tex, *cx, *cy, *cw, *ch);
                props_tex_draws.push(TextureDraw::new(tex, bx, by, bw, bh));
            }
        }
    }
    // Deferred inactive-pane scroll write-back (see the split pane block —
    // `entries` borrows were still live there).
    if let Some(v) = p2_scroll_new {
        if let Some(scroll) = app.inactive_scroll_mut() {
            *scroll = v;
        }
    }

    if props_close {
        app.properties = None;
    }
    if let Some((folder, icon_path)) = props_icon_chosen {
        // Defer xattr + invalidation to the next frame — icon_cache is
        // still borrowed by tex_draws at this point in the frame.
        app.pending_icon_apply
            .push((folder, Some(icon_path.to_string_lossy().to_string())));
    }
    if let Some(folder) = props_icon_reset {
        app.pending_icon_apply.push((folder, None));
    }
    if let Some(text) = props_copy_text {
        if let Some(clip) = &app.wayland_clipboard {
            clip.set_text(&text);
        }
    }
    if let Some((entry, (ix, iy, iw, ih))) = props_icon_rect_data {
        if let Some(tex) = icon_cache.get(&entry) {
            let (bx, by, bw, bh) = icons::fit_in_box(tex, ix, iy, iw, ih);
            props_tex_draws.push(TextureDraw::new(tex, bx, by, bw, bh));
        }
    }
    if let Some(ref drop) = app.pending_drop {
        draw_drop_modal(drop, painter, text, input, pal, wf, hf, s, w, h);
    }
    if let Some(ref dialog) = app.drive_dialog {
        crate::dialogs::draw(dialog, painter, text, pal, input, (w, h), s);
    }
    if let Some(ref dialog) = app.cloud_login {
        crate::dialogs::draw_cloud_login(dialog, painter, text, pal, input, (w, h), s);
    }
    if let Some(ref dialog) = app.sudo_prompt {
        crate::dialogs::draw_sudo_prompt(dialog, painter, text, pal, input, (w, h), s);
    }
    if let Some(ref dialog) = app.conflict_dialog {
        crate::dialogs::draw_conflict_dialog(dialog, painter, text, pal, input, (w, h), s);
    }

    // ── Quick Look overlay (topmost modal) ─────────────────────────
    if let Some(ref ql) = app.quick_look {
        if let Some(draw) =
            crate::quick_look::draw_quick_look(ql, painter, text, pal, input, wf, hf, s, w, h)
        {
            props_tex_draws.push(draw);
        }
    }

    // ── Inline context menu (desktop mode) ─────────────────────────
    let mut inline_menu_event = None;
    if desktop_mode {
        painter.set_layer(1);
        inline_menu_event = context_menu.draw(painter, text, input, w, h);
        painter.set_layer(0);
    }

    // ── Layered render ────────────────────────────────────────────────
    let frame = ctx.begin_frame("Lantern File Manager");
    match frame {
        Ok(mut frame) => {
            let view = frame.view().clone();

            // Layer 0: base shapes + textures + text
            painter.render_layer(
                0,
                ctx,
                frame.encoder_mut(),
                &view,
                Some(Color::rgba(0.0, 0.0, 0.0, 0.0)),
            );
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(0, ctx, frame.encoder_mut(), &view);

            // Flush so glyphon's prepare() for layer 1 doesn't overwrite layer 0 vertices
            frame.flush(ctx);

            // Layer 1: modal overlay shapes + textures + text
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            if !props_tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &props_tex_draws, None);
            }
            text.render_layer(1, ctx, frame.encoder_mut(), &view);

            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[fox] render error: {e}"),
    }

    inline_menu_event
}

fn draw_drop_modal(
    drop: &crate::app::PendingDrop,
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    input: &mut InteractionContext,
    pal: &FoxPalette,
    screen_w: f32,
    screen_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
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
        drop.sources[0]
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".into())
    } else {
        format!("{} items", drop.sources.len())
    };
    let dest_dir = drop
        .dest_dir
        .file_name()
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
        Rect::new(0.0, 0.0, screen_w, screen_h),
        0.0,
        Color::rgba(0.0, 0.0, 0.0, 0.55),
    );

    // Shadow + panel
    let panel = Rect::new(dx, dy, dialog_w, dialog_h);
    let shadow = Rect::new(
        dx - 8.0 * s,
        dy - 4.0 * s,
        dialog_w + 16.0 * s,
        dialog_h + 16.0 * s,
    );
    painter.rect_filled(shadow, cr + 4.0 * s, Color::rgba(0.0, 0.0, 0.0, 0.3));
    painter.rect_filled(panel, cr, pal.surface);
    painter.rect_stroke_sdf(panel, cr, 1.0 * s, pal.muted.with_alpha(0.2));

    let mut cy = dy + pad;

    // Title
    text.queue(
        "Move or Copy?",
        title_font,
        dx + pad,
        cy,
        pal.text,
        dialog_w - pad * 2.0,
        sw,
        sh,
    );
    cy += title_font + pad * 0.5;

    // Body text
    let line1 = format!("\"{file_name}\"");
    let line2 = format!("→ {dest_dir}/");
    text.queue(
        &line1,
        body_font,
        dx + pad,
        cy,
        pal.text,
        dialog_w - pad * 2.0,
        sw,
        sh,
    );
    cy += body_font + 4.0 * s;
    text.queue(
        &line2,
        body_font,
        dx + pad,
        cy,
        pal.text_secondary,
        dialog_w - pad * 2.0,
        sw,
        sh,
    );
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
        .scale(s)
        .draw(painter, text, pal, sw, sh);

    // Copy button
    let copy_rect = Rect::new(btn_x_start + btn_w + btn_gap, cy, btn_w, btn_h);
    let copy_state = input.add_zone(ZONE_DROP_COPY, copy_rect);
    Button::new(copy_rect, "Copy")
        .hovered(copy_state.is_hovered())
        .pressed(copy_state.is_active())
        .scale(s)
        .draw(painter, text, pal, sw, sh);

    // Cancel button
    let cancel_rect = Rect::new(btn_x_start + (btn_w + btn_gap) * 2.0, cy, btn_w, btn_h);
    let cancel_state = input.add_zone(ZONE_DROP_CANCEL, cancel_rect);
    Button::new(cancel_rect, "Cancel")
        .hovered(cancel_state.is_hovered())
        .pressed(cancel_state.is_active())
        .scale(s)
        .draw(painter, text, pal, sw, sh);
}
