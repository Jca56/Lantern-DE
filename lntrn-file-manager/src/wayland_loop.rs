use std::time::{Duration, Instant};

use anyhow::Result;
use lntrn_ui::gpu::{
    ContextMenu, FoxPalette, InteractionContext, MenuEvent, PopupSurface, ScrollArea,
};
use wayland_client::{
    protocol::{wl_data_device_manager, wl_surface},
    Connection, EventQueue, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::wp_viewport;
use wayland_protocols::xdg::shell::client::xdg_toplevel;

use crate::app::App;
use crate::desktop::DesktopApp;
use crate::icons::IconCache;
use crate::layout::{content_rect, grid_columns, grid_content_height, list_content_height, tree_content_height};
use crate::settings::Settings;
use crate::wayland::State;
use crate::wayland_actions::{
    handle_click, handle_ctx_event, handle_drop, handle_key, handle_right_click,
    update_rubber_band, copy_dir_recursive, edge_resize,
};
use crate::{
    ClickAction, Gpu, CTX_NEW_FOLDER_BLUE, CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE,
    CTX_NEW_FOLDER_PURPLE, CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW,
    VIEW_SLIDER_ID, VIEW_OPACITY_SLIDER_ID, VIEW_SHOW_HIDDEN_ID,
    ZONE_DROP_CANCEL, ZONE_DROP_COPY, ZONE_DROP_MOVE,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_loop(
    _conn: &Connection,
    event_queue: &mut EventQueue<State>,
    state: &mut State,
    qh: &QueueHandle<State>,
    surface: &wl_surface::WlSurface,
    toplevel: &Option<xdg_toplevel::XdgToplevel>,
    viewport: &Option<wp_viewport::WpViewport>,
    gpu: &mut Gpu,
    palette: &FoxPalette,
    view_menu: &mut ContextMenu,
    context_menu: &mut ContextMenu,
    open_with_apps: &mut Vec<DesktopApp>,
    app: &mut App,
    input: &mut InteractionContext,
    icon_cache: &mut IconCache,
    file_info: &mut crate::file_info::FileInfoCache,
    settings: &mut Settings,
) -> Result<()> {
    let mut last_frame = Instant::now();
    let mut needs_anim = false;
    let mut last_dir_check = Instant::now();
    let mut last_dir_mtime: Option<std::time::SystemTime> = None;
    let mut last_dir_path = app.current_dir.clone();
    let mut last_tab_click: Option<(usize, Instant)> = None;
    // Pinned tab drag reorder state
    let mut tab_drag: Option<usize> = None;          // index of tab being dragged
    let mut tab_drag_press: Option<(usize, f32)> = None; // (tab_idx, press_x) for drag detection

    eprintln!("[fox] entering main loop, size={}x{}", state.width, state.height);

    while state.running {
        // Event dispatch
        if needs_anim {
            if let Err(e) = event_queue.flush() {
                eprintln!("[fox] flush error: {e}");
                break;
            }
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            if let Err(e) = event_queue.dispatch_pending(state) {
                eprintln!("[fox] dispatch_pending error: {e}");
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
            state.frame_done = true;
        } else {
            if let Err(e) = event_queue.blocking_dispatch(state) {
                eprintln!("[fox] blocking_dispatch error: {e}");
                break;
            }
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32().min(0.05);
        last_frame = now;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.ctx.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            view_menu.set_scale(scale_f);
            context_menu.set_scale(scale_f);
        }

        let wf = gpu.ctx.width() as f32;
        let hf = gpu.ctx.height() as f32;
        let s = scale_f;

        // ── Cursor routing ──────────────────────────────────────────────
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;

        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state.popup_backend.as_ref()?.find_popup_id_by_wl_surface(ps)
        });

        if pointer_on_popup.is_some() {
            input.on_cursor_left();
        } else if state.pointer_in_surface {
            input.on_cursor_moved(cx, cy);
        } else {
            input.on_cursor_left();
        }

        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface { pointer_on_popup } else { None };
            backend.route_cursor(active, cx, cy);
        }

        // Set pointer depth for submenu close logic
        {
            let depth = pointer_on_popup.and_then(|pid| {
                (0..context_menu.popup_count())
                    .find(|&d| context_menu.popup_id_at_depth(d) == Some(pid))
            });
            context_menu.set_pointer_depth(depth);

            let vdepth = pointer_on_popup.and_then(|pid| {
                (0..view_menu.popup_count())
                    .find(|&d| view_menu.popup_id_at_depth(d) == Some(pid))
            });
            view_menu.set_pointer_depth(vdepth);
        }

        // ── Rubber band update ──────────────────────────────────────────
        if state.pointer_in_surface && app.rubber_band_start.is_some() {
            app.rubber_band_end = Some((cx, cy));
            update_rubber_band(app, wf, hf, s);
        }

        // ── Drag detection ──────────────────────────────────────────────
        if state.pointer_in_surface && app.drag_item.is_none() {
            if let (Some(idx), Some((px, py))) = (app.pending_open, app.press_pos) {
                let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
                if dist > 5.0 {
                    app.drag_item = Some(idx);
                    app.drag_pos = Some((cx, cy));
                    app.pending_open = None;
                    app.press_pos = None;

                    // Prepare DnD paths (Wayland DnD starts when cursor leaves window)
                    let paths: Vec<std::path::PathBuf> = {
                        let selected = app.selected_paths();
                        if selected.is_empty() || !app.entries[idx].selected {
                            vec![app.entries[idx].path.clone()]
                        } else {
                            selected
                        }
                    };
                    state.dnd_paths = paths;
                    state.dnd_serial = state.pointer_serial;
                }
            }

            // Pinned tab drag detection
            if tab_drag.is_none() {
                if let Some((tab_idx, press_x)) = tab_drag_press {
                    if (cx - press_x).abs() > 5.0 {
                        tab_drag = Some(tab_idx);
                        tab_drag_press = None;
                    }
                }
            }
        }
        if app.drag_item.is_some() && state.pointer_in_surface {
            app.drag_pos = Some((cx, cy));
        }

        // ── Start Wayland DnD when cursor leaves window during drag ────
        if app.drag_item.is_some() && !state.dnd_active && !state.dnd_paths.is_empty() {
            let raw_cx = state.cursor_x as f32;
            let raw_cy = state.cursor_y as f32;
            let logical_w = state.width as f32;
            let logical_h = state.height as f32;
            if raw_cx < 0.0 || raw_cy < 0.0 || raw_cx > logical_w || raw_cy > logical_h {
                if let (Some(mgr), Some(dd), Some(surf)) = (
                    &state.data_device_manager,
                    &state.data_device,
                    &state.surface,
                ) {
                    let source = mgr.create_data_source(qh, ());
                    source.offer("text/uri-list".to_string());
                    source.offer("text/plain".to_string());
                    source.set_actions(
                        wl_data_device_manager::DndAction::Copy
                        | wl_data_device_manager::DndAction::Move,
                    );
                    dd.start_drag(Some(&source), surf, None, state.dnd_serial);
                    state.dnd_active = true;
                    // Clear internal drag — compositor owns the drag now
                    app.drag_item = None;
                    app.drag_pos = None;
                }
            }
        }

        // ── Clean up drag state after Wayland DnD ends ──────────────────
        if !state.dnd_active && state.dnd_paths.is_empty() && app.drag_item.is_some()
            && !state.pointer_in_surface
        {
            app.drag_item = None;
            app.drag_pos = None;
        }

        // ── Keyboard ────────────────────────────────────────────────────
        if let Some(key) = state.key_pressed.take() {
            handle_key(app, settings, context_menu, &mut state.popup_backend, key, state.ctrl, state.shift, &mut state.running);
        }

        // Key repeat (for text editing modes)
        if let Some(key) = state.held_key {
            if (app.renaming.is_some() || app.path_editing || app.save_name_editing || app.searching)
                && std::time::Instant::now() >= state.repeat_deadline
            {
                handle_key(app, settings, context_menu, &mut state.popup_backend, key, state.ctrl, state.shift, &mut state.running);
                let interval = if state.repeat_started { 30 } else { 300 };
                state.repeat_deadline = std::time::Instant::now()
                    + std::time::Duration::from_millis(interval);
                state.repeat_started = true;
                state.frame_done = true;
            }
        }

        // ── Scroll ──────────────────────────────────────────────────────
        if state.scroll_delta.abs() > 0.01 {
            let scroll = state.scroll_delta * s;
            input.on_scroll(scroll);
            let content = content_rect(wf, hf, s);
            let zoom = app.icon_zoom;
            let total_h = match app.view_mode {
                crate::app::ViewMode::Grid => {
                    let cols = grid_columns(content.w, s, zoom);
                    grid_content_height(app.entries.len(), cols, s, zoom)
                }
                crate::app::ViewMode::List => list_content_height(app.entries.len(), s),
                crate::app::ViewMode::Tree => tree_content_height(app.tree_entries.len(), s),
            };
            ScrollArea::apply_scroll(&mut app.scroll_offset, scroll, total_h, content.h);
            state.scroll_delta = 0.0;
        }

        // ── Left press ──────────────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            if let Some(pid) = pointer_on_popup {
                // Click is on a popup surface — route to popup interaction
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else if app.pending_drop.is_some() {
                // Drop confirmation modal — handle buttons
                if let Some(zone) = input.on_left_pressed() {
                    match zone {
                        ZONE_DROP_MOVE => {
                            if let Some(drop) = app.pending_drop.take() {
                                for src in &drop.sources {
                                    if let Some(name) = src.file_name() {
                                        let dest = drop.dest_dir.join(name);
                                        let _ = std::fs::rename(src, &dest);
                                    }
                                }
                                app.reload();
                                if let Some(tab) = drop.reload_tab {
                                    app.reload_tab(tab);
                                }
                            }
                        }
                        ZONE_DROP_COPY => {
                            if let Some(drop) = app.pending_drop.take() {
                                let sources = drop.sources.clone();
                                let dest_dir = drop.dest_dir.clone();
                                std::thread::spawn(move || {
                                    for src in &sources {
                                        if let Some(name) = src.file_name() {
                                            let dest = dest_dir.join(name);
                                            if src.is_dir() {
                                                copy_dir_recursive(src, &dest);
                                            } else {
                                                let _ = std::fs::copy(src, &dest);
                                            }
                                        }
                                    }
                                });
                                app.reload();
                                if let Some(tab) = drop.reload_tab {
                                    app.reload_tab(tab);
                                }
                            }
                        }
                        ZONE_DROP_CANCEL => {
                            app.pending_drop = None;
                        }
                        _ => {}
                    }
                }
            } else if app.properties.is_some() {
                // Properties dialog is open
                if let Some(zone) = input.on_left_pressed() {
                    if zone == 800 || zone == 801 {
                        // Close button or backdrop
                        app.properties = None;
                    } else if zone >= 810 && zone <= 815 {
                        // Section header toggle
                        if let Some(ref mut props) = app.properties {
                            let idx = (zone - 810) as usize;
                            if idx < props.section_open.len() {
                                props.section_open[idx] = !props.section_open[idx];
                            }
                        }
                    }
                    // zone == 802 (panel body) — do nothing, keep dialog open
                } else {
                    // Click outside any zone — close
                    app.properties = None;
                }
            } else if context_menu.is_open() {
                // Click outside popup — close it
                if let Some(backend) = &mut state.popup_backend {
                    context_menu.close_popups(backend);
                } else {
                    context_menu.close();
                }
            } else if view_menu.is_open() {
                // View menu popup is open — click outside closes it
                if let Some(backend) = &mut state.popup_backend {
                    view_menu.close_popups(backend);
                }
            } else {
                // Edge resize (window mode only)
                let mut handled_resize = false;
                if let Some(toplevel) = toplevel {
                    let border = 10.0 * s;
                    if let Some(edge) = edge_resize(cx, cy, wf, hf, border) {
                        if let Some(seat) = &state.seat {
                            toplevel.resize(seat, state.pointer_serial, edge);
                        }
                        handled_resize = true;
                    }
                }
                if !handled_resize {
                    let action = handle_click(
                        input, app, view_menu, &mut state.popup_backend,
                        &mut last_tab_click, &mut tab_drag_press, s,
                        settings.bg_opacity,
                    );
                    match action {
                        ClickAction::None => {
                            if let Some(toplevel) = toplevel {
                                // Title bar drag (window mode only)
                                let title_h = crate::layout::title_bar_rect(0.0, s).h;
                                if cy < title_h && !view_menu.is_open() {
                                    if let Some(seat) = &state.seat {
                                        toplevel._move(seat, state.pointer_serial);
                                    }
                                } else if app.pending_open.is_none() {
                                    let cr = content_rect(wf, hf, s);
                                    if cr.contains(cx, cy) {
                                        app.clear_selection();
                                        app.rubber_band_start = Some((cx, cy));
                                        app.rubber_band_end = Some((cx, cy));
                                    }
                                }
                            } else if app.pending_open.is_none() {
                                let cr = content_rect(wf, hf, s);
                                if cr.contains(cx, cy) {
                                    app.clear_selection();
                                    app.rubber_band_start = Some((cx, cy));
                                    app.rubber_band_end = Some((cx, cy));
                                }
                            }
                        }
                        ClickAction::Close => {
                            state.running = false;
                        }
                        ClickAction::Minimize => {
                            if let Some(toplevel) = toplevel {
                                toplevel.set_minimized();
                            }
                        }
                        ClickAction::ToggleMaximize => {
                            if let Some(toplevel) = toplevel {
                                if state.maximized {
                                    toplevel.unset_maximized();
                                } else {
                                    toplevel.set_maximized();
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Left release ────────────────────────────────────────────────
        if state.left_released {
            state.left_released = false;
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_released();
                    }
                }
            } else {
                if app.rubber_band_start.is_some() {
                    app.rubber_band_start = None;
                    app.rubber_band_end = None;
                }
                // Pinned tab drag release — reorder
                if let Some(src_idx) = tab_drag.take() {
                    let tab_bar_rect = crate::layout::tab_bar_rect(wf, s);
                    let tab_labels = app.tab_labels();
                    let tab_label_refs: Vec<&str> = tab_labels.iter().map(|s| s.as_str()).collect();
                    let rects = lntrn_ui::gpu::TabBar::new(tab_bar_rect)
                        .tabs(&tab_label_refs)
                        .scale(s)
                        .tab_rects();
                    // Find which tab slot the cursor is over
                    if let Some((cursor_x, _)) = input.cursor() {
                        let target_idx = rects.iter().position(|r| r.contains(cursor_x, r.y + r.h * 0.5))
                            .unwrap_or(src_idx);
                        // Only reorder among pinned tabs
                        if target_idx != src_idx
                            && target_idx < app.tabs.len()
                            && app.tabs[target_idx].pinned
                        {
                            let tab = app.tabs.remove(src_idx);
                            app.tabs.insert(target_idx, tab);
                            // Fix current_tab index
                            if app.current_tab == src_idx {
                                app.current_tab = target_idx;
                            } else if src_idx < app.current_tab && target_idx >= app.current_tab {
                                app.current_tab -= 1;
                            } else if src_idx > app.current_tab && target_idx <= app.current_tab {
                                app.current_tab += 1;
                            }
                        }
                    }
                } else if let Some(drag_idx) = app.drag_item.take() {
                    handle_drop(app, input, wf, hf, s, drag_idx);
                    app.pending_open = None;
                    state.dnd_paths.clear();
                } else if let Some(idx) = app.pending_open.take() {
                    app.on_item_click(idx);
                }
                tab_drag_press = None;
                input.on_left_released();
            }
        }

        // ── Right click ─────────────────────────────────────────────────
        if state.right_clicked {
            state.right_clicked = false;
            // Close existing menus first
            if view_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    view_menu.close_popups(backend);
                }
            }
            if context_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    context_menu.close_popups(backend);
                }
            }
            handle_right_click(app, context_menu, &mut state.popup_backend, input, open_with_apps, wf, hf, s);
        }

        // ── Popup closed by compositor ──────────────────────────────────
        if state.popup_closed {
            state.popup_closed = false;
            if let Some(backend) = &mut state.popup_backend {
                view_menu.close_popups(backend);
                context_menu.close_popups(backend);
            }
        }

        // ── Update menus ────────────────────────────────────────────────
        view_menu.update(dt);
        context_menu.update(dt);

        // ── Begin popup frames ──────────────────────────────────────────
        if let Some(backend) = &mut state.popup_backend {
            backend.begin_frame_all();
        }

        // ── Render ──────────────────────────────────────────────────────
        let opacity = if state.desktop_mode { settings.desktop_bg_opacity } else { settings.bg_opacity };
        let render_palette = palette.with_bg_opacity(opacity);
        let inline_evt = crate::render::render_frame(
            gpu, app, input, icon_cache, file_info,
            &render_palette, s, state.maximized, view_menu, context_menu,
            tab_drag, opacity, state.desktop_mode,
        );
        // Handle inline context menu events (desktop mode)
        if let Some(evt) = inline_evt {
            if matches!(evt, MenuEvent::Action(_)) {
                context_menu.close();
            }
            handle_ctx_event(app, settings, context_menu, &mut state.popup_backend, open_with_apps, file_info, evt);
        }

        // ── Draw & render popup surfaces (window mode) ─────────────────
        if let Some(backend) = &mut state.popup_backend {
            // View menu popup
            if let Some(evt) = view_menu.draw_popups(backend) {
                if let MenuEvent::SliderChanged { id, value } = evt {
                    if id == VIEW_SLIDER_ID {
                        app.icon_zoom = value;
                        let content = content_rect(wf, hf, s);
                        ScrollArea::apply_scroll(
                            &mut app.scroll_offset, 0.0,
                            grid_content_height(app.entries.len(),
                                grid_columns(content.w, s, value), s, value),
                            content.h,
                        );
                    } else if id == VIEW_OPACITY_SLIDER_ID {
                        settings.bg_opacity = value.clamp(0.0, 1.0);
                        settings.save();
                    }
                } else if let MenuEvent::CheckboxToggled { id, checked } = evt {
                    if id == VIEW_SHOW_HIDDEN_ID {
                        app.show_hidden = checked;
                        settings.show_hidden = checked;
                        app.reload();
                    }
                } else if matches!(evt, MenuEvent::Action(_)) {
                    view_menu.close_popups(backend);
                }
            }
            // Right-click context menu popup
            if let Some(evt) = context_menu.draw_popups(backend) {
                if matches!(evt, MenuEvent::Action(_)) {
                    context_menu.close_popups(backend);
                }
                handle_ctx_event(app, settings, context_menu, &mut state.popup_backend, open_with_apps, file_info, evt);
            }
            // Render popup surfaces, injecting folder icon textures for swatch items
            let swatches = context_menu.swatch_rects();
            let root_pid = context_menu.root_popup_id();
            if let Some(backend) = &mut state.popup_backend {
                backend.render_all_except(root_pid.filter(|_| !swatches.is_empty()));

                // Render the root popup with texture icons for swatches
                if !swatches.is_empty() {
                    if let Some(pid) = root_pid {
                        if let Some(ctx) = backend.popup_render(pid) {
                            if let Ok(mut frame) = ctx.gpu.begin_frame("popup") {
                                let view = frame.view().clone();
                                // Pass 1: shapes
                                ctx.painter.render_pass(
                                    &ctx.gpu, frame.encoder_mut(), &view,
                                    lntrn_render::Color::TRANSPARENT,
                                );
                                // Pre-load all folder color textures into cache
                                for &(sid, _, _, _) in &swatches {
                                    let color_name = match sid {
                                        CTX_NEW_FOLDER_RED => "red",
                                        CTX_NEW_FOLDER_ORANGE => "orange",
                                        CTX_NEW_FOLDER_YELLOW => "yellow",
                                        CTX_NEW_FOLDER_GREEN => "green",
                                        CTX_NEW_FOLDER_BLUE => "blue",
                                        CTX_NEW_FOLDER_PURPLE => "purple",
                                        _ => "",
                                    };
                                    icon_cache.get_or_load_folder_color(
                                        color_name, &ctx.gpu, &ctx.tex_pass,
                                    );
                                }
                                // Pass 2: folder icon textures (all loaded, only immutable borrows now)
                                let mut tex_draws = Vec::new();
                                for &(sid, ix, iy, isz) in &swatches {
                                    let color_name = match sid {
                                        CTX_NEW_FOLDER_RED => "red",
                                        CTX_NEW_FOLDER_ORANGE => "orange",
                                        CTX_NEW_FOLDER_YELLOW => "yellow",
                                        CTX_NEW_FOLDER_GREEN => "green",
                                        CTX_NEW_FOLDER_BLUE => "blue",
                                        CTX_NEW_FOLDER_PURPLE => "purple",
                                        _ => "",
                                    };
                                    if let Some(tex) = icon_cache.get_folder_color(color_name) {
                                        let (dx, dy, dw, dh) = crate::icons::fit_in_box(tex, ix, iy, isz, isz);
                                        tex_draws.push(lntrn_render::TextureDraw::new(tex, dx, dy, dw, dh));
                                    }
                                }
                                if !tex_draws.is_empty() {
                                    ctx.tex_pass.render_pass(
                                        &ctx.gpu, frame.encoder_mut(), &view, &tex_draws, None,
                                    );
                                }
                                // Pass 3: text
                                ctx.text.render_queued(&ctx.gpu, frame.encoder_mut(), &view);
                                frame.submit(&ctx.gpu.queue);
                            }
                            backend.commit_popup(pid);
                        }
                    }
                }
            }
        }

        surface.frame(qh, ());
        surface.commit();

        // Poll search results from background thread
        app.poll_search();

        // ── Auto-refresh: check directory mtime every 3 seconds ─────
        if app.current_dir != last_dir_path {
            // Directory changed (navigation) — reset tracker, don't reload
            last_dir_path = app.current_dir.clone();
            last_dir_mtime = std::fs::metadata(&app.current_dir)
                .and_then(|m| m.modified()).ok();
            last_dir_check = Instant::now();
        } else if last_dir_check.elapsed() >= Duration::from_secs(3) {
            last_dir_check = Instant::now();
            let current_mtime = std::fs::metadata(&app.current_dir)
                .and_then(|m| m.modified())
                .ok();
            if current_mtime != last_dir_mtime {
                last_dir_mtime = current_mtime;
                app.reload();
            }
        }

        needs_anim = view_menu.is_open() || context_menu.is_open()
            || app.drag_item.is_some() || app.rubber_band_start.is_some()
            || state.held_key.is_some()
            || app.search_rx.is_some()
            || tab_drag.is_some()
            || state.dnd_active;
    }

    Ok(())
}
