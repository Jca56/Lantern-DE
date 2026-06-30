use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use anyhow::Result;
use lntrn_ui::gpu::{
    ContextMenu, FoxPalette, InteractionContext, MenuEvent, PopupSurface, ScrollArea, Scrollbar,
};
use wayland_client::{
    protocol::{wl_data_device_manager, wl_surface},
    Connection, EventQueue, QueueHandle,
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1;
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
    update_rubber_band, edge_resize, resize_edge_to_cursor_shape,
};
use crate::{
    ClickAction, Gpu, CTX_NEW_FOLDER_BLUE, CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE,
    CTX_NEW_FOLDER_PURPLE, CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW,
    VIEW_SLIDER_ID, VIEW_SHOW_HIDDEN_ID,
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
    palette: &mut FoxPalette,
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
    let mut last_theme_variant = lntrn_theme::active_variant();
    let mut last_theme_poll = Instant::now();
    let mut last_dir_check = Instant::now();
    let mut last_dir_mtime: Option<std::time::SystemTime> = None;
    let mut last_dir_path = app.current_dir.clone();
    let mut dir_watcher = crate::dir_watch::DirWatcher::new();
    let mut git = crate::git_status::GitStatus::new();
    let mut git_dir = std::path::PathBuf::new();
    let mut last_git_poll = Instant::now();
    let mut last_devices_check = Instant::now();
    let mut last_tab_click: Option<(usize, Instant)> = None;
    // Pinned tab drag reorder state
    let mut tab_drag: Option<usize> = None;          // index of tab being dragged
    let mut tab_drag_press: Option<(usize, f32)> = None; // (tab_idx, press_x) for drag detection
    // Favorite drag reorder state (mirrors tab_drag/tab_drag_press but axis is Y).
    let mut fav_drag: Option<usize> = None;
    let mut fav_drag_press: Option<(usize, f32)> = None;
    // Scrollbar thumb drag: Some(grab_dy) = pointer offset from the thumb top.
    let mut scrollbar_drag: Option<f32> = None;
    // Smooth wheel scrolling: offset eases toward this target each frame.
    // `scroll_anim_last` detects external offset writes (navigation, zoom,
    // scrollbar drag) so the animation yields instead of yanking back.
    let mut scroll_anim: Option<f32> = None;
    let mut scroll_anim_last: f32 = 0.0;

    eprintln!("[fox] entering main loop, size={}x{}", state.width, state.height);

    while state.running {
        // Event dispatch. Animating: short 16ms poll for ~60Hz redraws. Idle:
        // poll up to 500ms so we still wake periodically to live-poll
        // `[appearance].theme` from disk. Crucially we poll() on the wayland
        // fd instead of thread::sleep so input events wake the loop
        // immediately — sleeping made every click/scroll feel ~500ms laggy.
        let timeout_ms: i32 = if needs_anim { 16 } else { 500 };
        if let Err(e) = event_queue.flush() {
            eprintln!("[fox] flush error: {e}");
            break;
        }
        if let Some(guard) = event_queue.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            // Poll the watcher's eventfd alongside the wayland fd so a file
            // landing in the current directory wakes the loop immediately.
            let mut pfds = [
                libc::pollfd { fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: dir_watcher.fd(), events: libc::POLLIN, revents: 0 },
            ];
            let ret = unsafe { libc::poll(pfds.as_mut_ptr(), 2, timeout_ms) };
            // Any revents (POLLIN, but also POLLHUP/POLLERR on compositor
            // death) → read, so connection errors surface in dispatch and
            // the loop exits instead of busy-spinning on a dead fd.
            if ret > 0 && pfds[0].revents != 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
            if ret > 0 && pfds[1].revents & libc::POLLIN != 0 {
                dir_watcher.drain_fd();
            }
        }
        if let Err(e) = event_queue.dispatch_pending(state) {
            eprintln!("[fox] dispatch_pending error: {e}");
            break;
        }
        // Cap rendering at ~60Hz. Pointer motion events fire at ~1000Hz on
        // modern mice; without this cap each event triggered a render and
        // melted the CPU. Sleeping the remaining frame budget lets queued
        // events coalesce into one render per frame.
        let since_last = last_frame.elapsed();
        let frame_budget = Duration::from_millis(16);
        if since_last < frame_budget {
            std::thread::sleep(frame_budget - since_last);
        }
        // Only force a render when something is actually animating. When idle
        // we leave `frame_done` to the event-driven dispatch handlers (pointer
        // motion, keys, configure, etc.) so an untouched window renders ZERO
        // frames instead of a constant 60fps GPU pass — that idle spin was
        // melting the laptop. The 16ms cap above still coalesces high-rate
        // pointer motion into one frame during interaction.
        if needs_anim {
            state.frame_done = true;
        }

        // Theme live-reload poll. The palette is re-resolved every frame
        // already (see `*palette = FoxPalette::current()` below), but we
        // still want a redraw kick when the variant flips so the change is
        // visible even when the user isn't actively interacting.
        if last_theme_poll.elapsed() >= Duration::from_millis(500) {
            last_theme_poll = Instant::now();
            let v = lntrn_theme::active_variant();
            if v != last_theme_variant {
                last_theme_variant = v;
                state.frame_done = true; // force a render this iteration
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

        // Keep the popup backend's scale in sync with the frame scale. The
        // menus re-read `s` on every open, but the backend used to keep its
        // startup snapshot forever — after anything shifted fractional_scale
        // (resize picking up late output info, scale switch, hotplug), popup
        // buffers were allocated from the stale scale and every context menu
        // rendered clipped on the right/bottom.
        if let Some(backend) = &mut state.popup_backend {
            backend.set_scale(s);
        }

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

        // ── Cursor shape (resize edges) ─────────────────────────────────
        if state.pointer_in_surface && pointer_on_popup.is_none() {
            // Preview-pane resize handle takes priority over window-edge resize
            // so the user gets an EW cursor right on the divider.
            let on_preview_handle = app.preview_drag.is_some() || {
                let view = if app.searching && !app.search_buf.is_empty() {
                    crate::app::ViewMode::List
                } else { app.view_mode };
                let supported = matches!(view, crate::app::ViewMode::List | crate::app::ViewMode::Tree);
                if supported && app.preview_open {
                    let full = if app.pick.is_some() {
                        let bottom = hf - crate::pick_bar::PICK_BAR_H * s;
                        crate::layout::content_rect_with_bottom(wf, bottom, s)
                    } else {
                        content_rect(wf, hf, s)
                    };
                    let pw = crate::layout::preview_effective_w(full.w, app.preview_width, true, s);
                    let h_rect = crate::layout::preview_handle_rect(full, pw, s);
                    h_rect.contains(cx, cy)
                } else { false }
            };
            let desired = if on_preview_handle {
                wp_cursor_shape_device_v1::Shape::EwResize
            } else if scrollbar_drag.is_some()
                || input.zone_at(cx, cy) == Some(crate::ZONE_SCROLLBAR)
            {
                // The scrollbar sits inside the resize border — don't flash
                // resize arrows over it.
                wp_cursor_shape_device_v1::Shape::Default
            } else if toplevel.is_some() && !state.maximized {
                let border = 10.0 * s;
                match edge_resize(cx, cy, wf, hf, border) {
                    Some(edge) => resize_edge_to_cursor_shape(edge),
                    None => wp_cursor_shape_device_v1::Shape::Default,
                }
            } else {
                wp_cursor_shape_device_v1::Shape::Default
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.pointer_enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
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

        // ── Scrollbar thumb drag ─────────────────────────────────────────
        if let Some(grab_dy) = scrollbar_drag {
            let content = content_rect(wf, hf, s);
            let total_h = view_content_height(app, content.w, s);
            let bar = Scrollbar::new(&content, total_h, app.scroll_offset);
            app.scroll_offset = bar.offset_for_thumb_y(
                cy - grab_dy + bar.thumb.h * 0.5,
                total_h,
                content.h,
            );
        }

        // ── Rubber band update + edge auto-scroll ────────────────────────
        if state.pointer_in_surface && app.rubber_band_start.is_some() {
            app.rubber_band_end = Some((cx, cy));
            let cr = active_content_rect(app, wf, hf, s);
            let edge_zone = 50.0 * s;
            let max_speed = 1400.0 * s; // physical px / second at full pull
            let mut scroll_delta = 0.0_f32;
            if cy < cr.y + edge_zone {
                let t = ((cr.y + edge_zone - cy) / edge_zone).clamp(0.0, 1.0);
                scroll_delta = -max_speed * t * t * dt;
            } else if cy > cr.y + cr.h - edge_zone {
                let t = ((cy - (cr.y + cr.h - edge_zone)) / edge_zone).clamp(0.0, 1.0);
                scroll_delta = max_speed * t * t * dt;
            }
            if scroll_delta != 0.0 {
                let zoom = app.icon_zoom;
                let total_h = match app.view_mode {
                    crate::app::ViewMode::Grid => {
                        let cols = grid_columns(cr.w, s, zoom);
                        grid_content_height(app.entries.len(), cols, s, zoom)
                    }
                    crate::app::ViewMode::List => list_content_height(app.entries.len(), s, zoom),
                    crate::app::ViewMode::Tree => tree_content_height(app.tree_entries.len(), s, zoom),
                };
                ScrollArea::apply_scroll(&mut app.scroll_offset, scroll_delta, total_h, cr.h);
            }
            update_rubber_band(app, wf, hf, s);
        }

        // ── Preview pane resize drag ────────────────────────────────────
        if let Some((press_x, start_w)) = app.preview_drag {
            // Dragging LEFT widens the pane (handle is on its left edge).
            let delta_px = press_x - cx;
            let new_w = (start_w + delta_px / s)
                .max(crate::layout::PREVIEW_MIN_W)
                .min((wf / s) * crate::layout::PREVIEW_MAX_FRACTION);
            app.preview_width = new_w;
        }

        // ── Drag detection ──────────────────────────────────────────────
        if state.pointer_in_surface && app.drag_item.is_none() && app.drag_tree_item.is_none() {
            if let (Some(idx), Some((px, py))) = (app.pending_open, app.press_pos) {
                let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
                if dist > 5.0 {
                    if app.press_shift {
                        // Shift+Drag: start a rubber-band from the press
                        // position instead of dragging the file. Replaces
                        // any existing selection so the band defines it.
                        app.clear_selection();
                        app.rubber_band_start = Some((px, py));
                        app.rubber_band_end = Some((cx, cy));
                        app.pending_open = None;
                        app.press_pos = None;
                        update_rubber_band(app, wf, hf, s);
                    } else {
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
            }

            // Tree rows arm their own pending slot — indices point into
            // tree_entries, not entries (nested rows have no entries index).
            // Only plain presses arm it, so no shift/rubber-band sub-branch.
            if let (Some(ti), Some((px, py))) = (app.pending_tree_open, app.press_pos) {
                let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
                if dist > 5.0 && ti < app.tree_entries.len() {
                    app.drag_tree_item = Some(ti);
                    app.drag_pos = Some((cx, cy));
                    app.pending_tree_open = None;
                    app.press_pos = None;

                    // Grabbing a selected row drags the whole selection;
                    // anything else (incl. nested rows) drags solo.
                    let path = app.tree_entries[ti].entry.path.clone();
                    let selected = app.selected_paths();
                    state.dnd_paths = if selected.iter().any(|p| p == &path) {
                        selected
                    } else {
                        vec![path]
                    };
                    state.dnd_serial = state.pointer_serial;
                }
            }

            // Favorite drag-to-reorder detection (Y-axis threshold).
            if fav_drag.is_none() {
                if let Some((fav_idx, press_y)) = fav_drag_press {
                    if (cy - press_y).abs() > 5.0 {
                        fav_drag = Some(fav_idx);
                        fav_drag_press = None;
                    }
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
        if (app.drag_item.is_some() || app.drag_tree_item.is_some()) && state.pointer_in_surface {
            app.drag_pos = Some((cx, cy));
        }

        // ── Start Wayland DnD when cursor leaves window during drag ────
        if (app.drag_item.is_some() || app.drag_tree_item.is_some())
            && !state.dnd_active && !state.dnd_paths.is_empty()
        {
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
                    app.drag_tree_item = None;
                    app.drag_pos = None;
                }
            }
        }

        // ── Clean up drag state after Wayland DnD ends ──────────────────
        if !state.dnd_active && state.dnd_paths.is_empty()
            && (app.drag_item.is_some() || app.drag_tree_item.is_some())
            && !state.pointer_in_surface
        {
            app.drag_item = None;
            app.drag_tree_item = None;
            app.drag_pos = None;
        }

        // ── Keyboard ────────────────────────────────────────────────────
        if let Some(key) = state.key_pressed.take() {
            // Super+F11: "rice mode" — hide/show the title bar (window mode
            // only). The compositor deliberately lets Super+F11 fall through;
            // plain F11 still toggles compositor fullscreen.
            const KEY_F11: u32 = 87;
            if state.logo && key == KEY_F11 && !state.desktop_mode {
                use std::sync::atomic::Ordering;
                let hidden = !crate::layout::CHROME_HIDDEN.load(Ordering::Relaxed);
                crate::layout::CHROME_HIDDEN.store(hidden, Ordering::Relaxed);
                if hidden && view_menu.is_open() {
                    if let Some(backend) = &mut state.popup_backend {
                        view_menu.close_popups(backend);
                    }
                }
            } else {
                handle_key(app, settings, context_menu, &mut state.popup_backend, key, state.ctrl, state.shift, &mut state.running);
            }
        }

        // Key repeat (for text editing modes)
        if let Some(key) = state.held_key {
            if (app.renaming.is_some() || app.path_editing || app.save_name_editing || app.searching || app.sudo_prompt.is_some() || app.cloud_login.is_some())
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
        // Wheel detents move a boosted distance and ease toward the target
        // instead of the old rigid 1:1 jump per event.
        const SCROLL_STEP_MULT: f32 = 4.0;
        if state.scroll_delta.abs() > 0.01 {
            let scroll = state.scroll_delta * s * SCROLL_STEP_MULT;
            input.on_scroll(scroll);
            let content = content_rect(wf, hf, s);
            let total_h = view_content_height(app, content.w, s);
            let max = (total_h - content.h).max(0.0);
            let base = scroll_anim.unwrap_or(app.scroll_offset);
            scroll_anim = Some((base + scroll).clamp(0.0, max));
            scroll_anim_last = app.scroll_offset;
            state.scroll_delta = 0.0;
        }
        if let Some(target) = scroll_anim {
            if app.scroll_offset != scroll_anim_last {
                // Someone else moved the scroll since last step — yield.
                scroll_anim = None;
            } else {
                let k = 1.0 - (-dt * 12.0).exp();
                app.scroll_offset += (target - app.scroll_offset) * k;
                if (target - app.scroll_offset).abs() < 0.5 {
                    app.scroll_offset = target;
                    scroll_anim = None;
                }
                scroll_anim_last = app.scroll_offset;
            }
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
                                app.start_drag_drop(
                                    crate::conflict::PasteMode::Cut,
                                    drop.sources,
                                    drop.dest_dir,
                                    drop.reload_tab,
                                );
                            }
                        }
                        ZONE_DROP_COPY => {
                            if let Some(drop) = app.pending_drop.take() {
                                app.start_drag_drop(
                                    crate::conflict::PasteMode::Copy,
                                    drop.sources,
                                    drop.dest_dir,
                                    drop.reload_tab,
                                );
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
                // Scrollbar grab — must win over edge-resize (the bar lives
                // inside the resize border) and the rubber band.
                let mut handled_scrollbar = false;
                if input.zone_at(cx, cy) == Some(crate::ZONE_SCROLLBAR) {
                    let content = content_rect(wf, hf, s);
                    let total_h = view_content_height(app, content.w, s);
                    let bar = Scrollbar::new(&content, total_h, app.scroll_offset);
                    let grab_dy = if cy >= bar.thumb.y && cy <= bar.thumb.y + bar.thumb.h {
                        cy - bar.thumb.y
                    } else {
                        // Track click: jump the thumb to the cursor, then drag.
                        app.scroll_offset = bar.offset_for_thumb_y(cy, total_h, content.h);
                        bar.thumb.h * 0.5
                    };
                    scrollbar_drag = Some(grab_dy);
                    scroll_anim = None;
                    // Take input capture so the thumb draws Pressed/Dragging
                    // and other zones stop hovering during the drag.
                    input.on_left_pressed();
                    handled_scrollbar = true;
                }
                // Edge resize (window mode only)
                let mut handled_resize = false;
                if let Some(toplevel) = toplevel {
                    if !handled_scrollbar {
                        let border = 10.0 * s;
                        if let Some(edge) = edge_resize(cx, cy, wf, hf, border) {
                            if let Some(seat) = &state.seat {
                                toplevel.resize(seat, state.pointer_serial, edge);
                            }
                            handled_resize = true;
                        }
                    }
                }
                if !handled_scrollbar && !handled_resize {
                    let prev_preview_open = app.preview_open;
                    let prev_view = app.view_mode;
                    let prev_places_collapsed = app.places_collapsed;
                    let prev_favorites_collapsed = app.favorites_collapsed;
                    let prev_devices_collapsed = app.devices_collapsed;
                    let prev_favorites_len = app.sidebar_favorites().len();
                    let action = handle_click(
                        input, app, view_menu, context_menu, &mut state.popup_backend,
                        &mut last_tab_click, &mut tab_drag_press, &mut fav_drag_press,
                        wf, s,
                        lntrn_theme::background_opacity(), "",
                        state.ctrl, state.shift,
                    );
                    let mut settings_dirty = false;
                    if app.preview_open != prev_preview_open {
                        settings.preview_open = app.preview_open;
                        settings_dirty = true;
                    }
                    if app.places_collapsed != prev_places_collapsed {
                        settings.places_collapsed = app.places_collapsed;
                        settings_dirty = true;
                    }
                    if app.favorites_collapsed != prev_favorites_collapsed {
                        settings.favorites_collapsed = app.favorites_collapsed;
                        settings_dirty = true;
                    }
                    if app.devices_collapsed != prev_devices_collapsed {
                        settings.devices_collapsed = app.devices_collapsed;
                        settings_dirty = true;
                    }
                    if app.sidebar_favorites().len() != prev_favorites_len {
                        settings.favorites = app.favorites_paths();
                        settings_dirty = true;
                    }
                    // Don't persist the forced Tree view from pick mode — it's
                    // a transient launch decision, not a user preference.
                    if app.view_mode != prev_view && app.pick.is_none() {
                        settings.set_view_mode(app.view_mode);
                        settings_dirty = true;
                    }
                    if settings_dirty { settings.save(); }
                    match action {
                        ClickAction::None => {
                            if let Some(toplevel) = toplevel {
                                // Title bar drag (window mode only)
                                let title_h = crate::layout::title_bar_rect(0.0, s).h;
                                if cy < title_h && !view_menu.is_open() {
                                    if let Some(seat) = &state.seat {
                                        toplevel._move(seat, state.pointer_serial);
                                    }
                                } else if app.pending_open.is_none()
                                    && app.pending_tree_open.is_none()
                                    && app.preview_drag.is_none()
                                    && !app.suppress_rubber_band
                                {
                                    let cr = active_content_rect(app, wf, hf, s);
                                    if cr.contains(cx, cy) {
                                        app.clear_selection();
                                        app.rubber_band_start = Some((cx, cy));
                                        app.rubber_band_end = Some((cx, cy));
                                    }
                                }
                            } else if app.pending_open.is_none()
                                && app.pending_tree_open.is_none()
                                && app.preview_drag.is_none()
                                && !app.suppress_rubber_band
                            {
                                let cr = active_content_rect(app, wf, hf, s);
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
                scrollbar_drag = None;
                if app.rubber_band_start.is_some() {
                    app.rubber_band_start = None;
                    app.rubber_band_end = None;
                }
                if app.preview_drag.take().is_some() {
                    settings.preview_width = app.preview_width;
                    settings.save();
                }
                // Favorite drag release — reorder
                if let Some(src_idx) = fav_drag.take() {
                    let layout = crate::layout::build_sidebar_layout(
                        s,
                        app.sidebar_places().len(),
                        app.sidebar_favorites().len(),
                        app.drives.len(),
                        app.phones.len(),
                        app.places_collapsed,
                        app.favorites_collapsed,
                        app.devices_collapsed,
                    );
                    if let Some((_, cy)) = input.cursor() {
                        // Target slot is whichever favorite row the cursor is
                        // currently over. Off-row releases are a no-op.
                        let target = layout.favorite_items.iter()
                            .position(|r| cy >= r.y && cy < r.y + r.h);
                        if let Some(target_idx) = target {
                            if target_idx != src_idx && src_idx < app.sidebar_favorites().len() {
                                app.reorder_favorite(src_idx, target_idx);
                                settings.favorites = app.favorites_paths();
                                settings.save();
                            }
                        }
                    }
                }
                fav_drag_press = None;
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
                } else if app.drag_item.is_some() || app.drag_tree_item.is_some() {
                    // Internal drop — sources from whichever drag kind is
                    // live. Grabbing a selected item drags the whole
                    // selection; anything else drags solo.
                    let sources: Vec<std::path::PathBuf> = if let Some(drag_idx) = app.drag_item.take() {
                        let selected = app.selected_paths();
                        if selected.is_empty() || !app.entries[drag_idx].selected {
                            vec![app.entries[drag_idx].path.clone()]
                        } else {
                            selected
                        }
                    } else if let Some(ti) = app.drag_tree_item.take() {
                        if ti < app.tree_entries.len() {
                            let path = app.tree_entries[ti].entry.path.clone();
                            let selected = app.selected_paths();
                            if selected.iter().any(|p| p == &path) { selected } else { vec![path] }
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };
                    if !sources.is_empty() {
                        let prev_fav_len = app.sidebar_favorites().len();
                        handle_drop(app, input, wf, hf, s, sources);
                        if app.sidebar_favorites().len() != prev_fav_len {
                            settings.favorites = app.favorites_paths();
                            settings.save();
                        }
                    }
                    app.pending_open = None;
                    app.pending_tree_open = None;
                    app.drag_pos = None;
                    state.dnd_paths.clear();
                } else if let Some(ti) = app.pending_tree_open.take() {
                    // Deferred tree-row action: the press armed a potential
                    // drag instead of acting; no drag started, so act now.
                    if ti < app.tree_entries.len() {
                        let te = &app.tree_entries[ti];
                        let path = te.entry.path.clone();
                        if te.entry.is_dir {
                            app.toggle_tree_expand(path);
                        } else {
                            let ext = path.extension()
                                .and_then(|e| e.to_str())
                                .map(|s| s.to_lowercase())
                                .unwrap_or_default();
                            if let Some(a) = crate::desktop::default_app_for_extension(&ext) {
                                crate::desktop::launch_app(&a.exec, &path);
                            } else {
                                std::thread::spawn(move || {
                                    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                                });
                            }
                        }
                    }
                } else if let Some(idx) = app.pending_open.take() {
                    if app.press_ctrl {
                        // Ctrl+Click toggle already applied at press time —
                        // do nothing on release so the toggle sticks.
                    } else if app.press_shift {
                        // Shift+Click finalized as a range-select (anchor → idx).
                        let anchor = app.selection_anchor.unwrap_or(idx);
                        app.select_range(anchor, idx);
                        app.selection_anchor = Some(idx);
                    } else {
                        app.on_item_click(idx);
                    }
                }
                app.press_shift = false;
                app.press_ctrl = false;
                app.press_pos = None;
                app.suppress_rubber_band = false;
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
            // Re-style from the live palette so a theme/accent change in
            // System Settings shows up without restarting the file manager.
            // (handle_right_click re-applies the scale before opening.)
            context_menu.set_style(crate::wayland::context_menu_style(palette));
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
        // In window mode use the system-wide [windows].background_opacity so
        // every Lantern app honors a single source of truth. Desktop mode
        // keeps its own setting because that surface is the icon canvas
        // floating over the wallpaper, not a window.
        let opacity = if state.desktop_mode {
            settings.desktop_bg_opacity
        } else {
            lntrn_theme::background_opacity()
        };
        // Re-resolve every frame so System Settings → Appearance changes
        // (theme variant + accent) take effect without relaunching.
        *palette = FoxPalette::current();
        let render_palette = palette.with_bg_opacity(opacity);
        let inline_evt = crate::render::render_frame(
            gpu, app, input, icon_cache, file_info, &git,
            &render_palette, s, state.maximized, view_menu, context_menu,
            tab_drag, fav_drag, opacity, state.desktop_mode,
        );
        // Handle inline context menu events (desktop mode)
        if let Some(evt) = inline_evt {
            if matches!(evt, MenuEvent::Action(_)) {
                context_menu.close();
            }
            if let MenuEvent::SliderChanged { id: crate::CTX_ICON_SIZE, value } = evt {
                apply_icon_zoom(app, value, wf, hf, s);
            } else {
                handle_ctx_event(app, settings, context_menu, &mut state.popup_backend, open_with_apps, file_info, toplevel, state.maximized, &mut state.running, evt);
            }
        }

        // ── Draw & render popup surfaces (window mode) ─────────────────
        if let Some(backend) = &mut state.popup_backend {
            // View menu popup
            if let Some(evt) = view_menu.draw_popups(backend) {
                if let MenuEvent::SliderChanged { id, value } = evt {
                    if id == VIEW_SLIDER_ID {
                        apply_icon_zoom(app, value, wf, hf, s);
                    }
                } else if let MenuEvent::CheckboxToggled { id, checked } = evt {
                    if id == VIEW_SHOW_HIDDEN_ID {
                        app.show_hidden = checked;
                        settings.show_hidden = checked;
                        app.reload();
                    } else if id == crate::VIEW_SHOW_TITLEBAR_ID {
                        // Live toggle + persisted as the open-time default.
                        // (Super+F11 stays a transient toggle, like the
                        // terminal's rice mode.)
                        crate::layout::CHROME_HIDDEN.store(
                            !checked,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        settings.show_titlebar = checked;
                        settings.save();
                        // Hiding the bar removes the View label the menu is
                        // anchored to — close it along with the bar.
                        if !checked {
                            view_menu.close_popups(backend);
                        }
                    } else if id == crate::VIEW_SOLID_DIVIDERS_ID {
                        crate::sections::SOLID_DIVIDERS.store(
                            checked,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        settings.solid_dividers = checked;
                        settings.save();
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
                if let MenuEvent::SliderChanged { id: crate::CTX_ICON_SIZE, value } = evt {
                    apply_icon_zoom(app, value, wf, hf, s);
                } else {
                    handle_ctx_event(app, settings, context_menu, &mut state.popup_backend, open_with_apps, file_info, toplevel, state.maximized, &mut state.running, evt);
                }
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

        // Only request the next frame callback while animating. The callback
        // handler sets `frame_done = true`, so re-arming it every frame would
        // keep waking the loop for a redraw forever even when idle. When still,
        // input/dispatch events drive the next render instead.
        if needs_anim {
            surface.frame(qh, ());
        }
        surface.commit();

        // Poll search results from background thread
        app.poll_search();
        app.poll_op_progress();
        // Drain deferred icon-cache invalidations queued by the Properties
        // icon picker. We can't mutate icon_cache during render_frame
        // (tex_draws still borrows it), so we apply changes between frames.
        if !app.pending_icon_apply.is_empty() {
            let pending = std::mem::take(&mut app.pending_icon_apply);
            for (folder, icon) in pending {
                match icon {
                    Some(path) => crate::icons::set_folder_icon(&folder, &path),
                    None => crate::icons::clear_folder_icon(&folder),
                }
                icon_cache.invalidate(&folder);
            }
        }
        // Pre-warm SVG thumbnails for the icon picker, if open. Picker
        // cell rects come from the previous frame's render — so the very
        // first frame after opening shows empty cells, then thumbnails
        // populate on the next frame (~16ms).
        if let Some(ref props) = app.properties {
            for (path, _, _, _, _) in &props.picker_cell_rects {
                icon_cache.ensure_svg_path(path, &gpu.ctx, &gpu.tex_pass);
            }
        }

        // ── Auto-refresh ─────────────────────────────────────────────
        // Primary: inotify on the current dir → debounced instant reload.
        dir_watcher.watch(&app.current_dir);
        if dir_watcher.take_due_reload() {
            app.reload();
            // Keep the mtime tracker in sync so the fallback poll below
            // doesn't schedule a redundant second reload.
            last_dir_mtime = std::fs::metadata(&app.current_dir)
                .and_then(|m| m.modified()).ok();
            git.refresh(&app.current_dir);
        }

        // ── Git badges/branch ────────────────────────────────────────
        git.poll();
        if app.current_dir != git_dir {
            git_dir = app.current_dir.clone();
            last_git_poll = Instant::now();
            git.refresh(&app.current_dir);
        } else if git.in_repo() && last_git_poll.elapsed() >= Duration::from_secs(5) {
            // Commits/stages from a terminal change git state without any
            // fs event in the viewed dir — cheap periodic re-scan, repos only.
            last_git_poll = Instant::now();
            git.refresh(&app.current_dir);
        }
        // Fallback: dir-mtime poll every 3s, for filesystems without
        // inotify delivery (sshfs and friends).
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

        // ── Devices: poll for hot-plugged USB drives + phones every 2s ──
        if last_devices_check.elapsed() >= Duration::from_secs(2) {
            last_devices_check = Instant::now();
            app.refresh_drives();
            app.refresh_phones();
        }

        needs_anim = view_menu.is_open() || context_menu.is_open()
            || scroll_anim.is_some()
            || scrollbar_drag.is_some()
            || app.drag_item.is_some() || app.drag_tree_item.is_some()
            || app.rubber_band_start.is_some()
            || state.held_key.is_some()
            || app.search_rx.is_some()
            || tab_drag.is_some()
            || fav_drag.is_some()
            || app.preview_drag.is_some()
            || app.op_progress.is_some()
            || state.dnd_active
            || icon_cache.has_pending()
            || dir_watcher.reload_pending()
            || app.quick_look.as_ref().is_some_and(|ql| ql.loading());
    }

    Ok(())
}

/// Total scrollable content height for the active view mode. Mirrors the
/// geometry render.rs feeds its ScrollArea/Scrollbar, including the search
/// results list (taller rows + header).
fn view_content_height(app: &App, content_w: f32, s: f32) -> f32 {
    let zoom = app.icon_zoom;
    if app.searching && !app.search_buf.is_empty() {
        return app.search_results.len() as f32 * crate::layout::search_list_row_h(s, zoom)
            + 32.0 * crate::layout::list_zoom_multiplier(zoom) * s;
    }
    match app.view_mode {
        crate::app::ViewMode::Grid => {
            let cols = grid_columns(content_w, s, zoom);
            grid_content_height(app.entries.len(), cols, s, zoom)
        }
        crate::app::ViewMode::List => list_content_height(app.entries.len(), s, zoom),
        crate::app::ViewMode::Tree => tree_content_height(app.tree_entries.len(), s, zoom),
    }
}

/// Apply a live icon-zoom change (View menu or right-click menu slider):
/// set the zoom and re-clamp the scroll offset against the new grid height.
fn apply_icon_zoom(app: &mut App, value: f32, wf: f32, hf: f32, s: f32) {
    app.icon_zoom = value;
    let content = content_rect(wf, hf, s);
    ScrollArea::apply_scroll(
        &mut app.scroll_offset, 0.0,
        grid_content_height(app.entries.len(), grid_columns(content.w, s, value), s, value),
        content.h,
    );
}

/// Content rect with the preview pane subtracted (if it's open + this view
/// supports it). Used for hit-testing the rubber-band selection so the band
/// doesn't start inside the info pane.
fn active_content_rect(app: &App, wf: f32, hf: f32, s: f32) -> lntrn_render::Rect {
    let full = if app.pick.is_some() {
        let bottom = hf - crate::pick_bar::PICK_BAR_H * s;
        crate::layout::content_rect_with_bottom(wf, bottom, s)
    } else {
        content_rect(wf, hf, s)
    };
    let view = if app.searching && !app.search_buf.is_empty() {
        crate::app::ViewMode::List
    } else {
        app.view_mode
    };
    let preview_supported = matches!(view, crate::app::ViewMode::List | crate::app::ViewMode::Tree);
    let preview_w = if preview_supported {
        crate::layout::preview_effective_w(full.w, app.preview_width, app.preview_open, s)
    } else {
        0.0
    };
    lntrn_render::Rect::new(full.x, full.y, full.w - preview_w, full.h)
}
