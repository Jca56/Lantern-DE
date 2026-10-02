use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use anyhow::Result;
use lntrn_ui::gpu::{
    ContextMenu, FoxPalette, InteractionContext, MenuEvent, PopupSurface, ScrollArea,
};
use wayland_client::{protocol::wl_surface, Connection, EventQueue, QueueHandle};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1;
use wayland_protocols::wp::viewporter::client::wp_viewport;
use wayland_protocols::xdg::shell::client::xdg_toplevel;

use crate::app::App;
use crate::desktop::DesktopApp;
use crate::icons::IconCache;
use crate::layout::{
    content_rect, grid_columns, grid_content_height, list_content_height, tree_content_height,
};
use crate::settings::Settings;
use crate::wayland::State;
use crate::wayland_actions::{
    edge_resize, handle_click, handle_ctx_event, handle_drop, handle_key, handle_right_click,
    resize_edge_to_cursor_shape, text_entry_active, update_rubber_band,
};
use crate::{
    ClickAction, Gpu, CTX_NEW_FOLDER_BLUE, CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE,
    CTX_NEW_FOLDER_PURPLE, CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW, VIEW_SHOW_HIDDEN_ID,
    VIEW_SLIDER_ID, ZONE_DROP_CANCEL, ZONE_DROP_COPY, ZONE_DROP_MOVE,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_loop(
    conn: &Connection,
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
    // Palette and window opacity come out of lantern.toml. Resolving them
    // re-reads and re-parses that file four times, which used to happen on
    // every rendered frame; now it happens when the file's stamp changes.
    let theme_stamp = || {
        lntrn_theme::lantern_config_path()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| (m.modified().ok(), m.len()))
    };
    let mut last_theme_stamp = theme_stamp();
    let mut bg_opacity = lntrn_theme::background_opacity();
    *palette = FoxPalette::current();
    let mut last_theme_poll = Instant::now();
    // Watchers, git badges, and the results of off-thread work: everything
    // that has to move on while no frame is drawn (housekeeping.rs).
    let mut hk = crate::housekeeping::Housekeeping::new();
    let mut last_tab_click: Option<(usize, Instant)> = None;
    // Pinned tab drag reorder state
    let mut tab_drag: Option<usize> = None; // index of tab being dragged
    let mut tab_drag_press: Option<(usize, f32)> = None; // (tab_idx, press_x) for drag detection
                                                         // Favorite drag reorder state (mirrors tab_drag/tab_drag_press but axis is Y).
    let mut fav_drag: Option<usize> = None;
    let mut fav_drag_press: Option<(usize, f32)> = None;
    // Scrollbar thumb drag: Some(grab_dy) = pointer offset from the thumb top.
    let mut scrollbar_drag: Option<f32> = None;
    // The same for the sidebar's and the Properties dialog's scrollbars.
    let mut aside_drag: Option<crate::scrollbar::AsideDrag> = None;
    // Smooth wheel scrolling: offset eases toward this target each frame.
    // `scroll_anim_last` detects external offset writes (navigation, zoom,
    // scrollbar drag) so the animation yields instead of yanking back.
    let mut scroll_anim: Option<f32> = None;
    let mut scroll_anim_last: f32 = 0.0;

    eprintln!(
        "[fox] entering main loop, size={}x{}",
        state.width, state.height
    );

    loop {
        // Every close path (title bar, Super+Q, the menu, a picker's
        // result) only clears `running`. While a copy or move is in flight
        // the window must not vanish under it: exiting would kill the
        // worker in the middle of a file. `intercept_close` asks the user
        // instead, and `close_ready` lets go once the worker is idle.
        if app.close_ready() {
            break;
        }
        if !state.running {
            if !app.intercept_close() {
                break;
            }
            state.running = true;
            // The question has to be drawn: the compositor hides a window
            // it asked to close until the window commits a new frame.
            state.frame_done = true;
        }
        // Event dispatch. A frame already owed: do not wait at all.
        // Animating: short 16ms poll for ~60Hz redraws. Idle: poll up to
        // 500ms so we still wake periodically to live-poll
        // `[appearance].theme` from disk, and no longer than until the next
        // thing scheduled (a debounced reload, a key repeat), which is
        // looked at without drawing frames while it waits. Crucially we
        // poll() on the wayland fd instead of thread::sleep so input events
        // wake the loop immediately — sleeping made every click/scroll feel
        // ~500ms laggy.
        let timeout_ms: i32 = if state.frame_done {
            0
        } else if needs_anim {
            16
        } else {
            // The zoom slider moves many times a second. Its value is
            // written here, once things are quiet, instead of only at exit
            // (which a session that is killed never reaches).
            if app.pick.is_none() && settings.icon_zoom != app.icon_zoom {
                settings.icon_zoom = app.icon_zoom;
                settings.save();
            }
            let repeat = state.kbd.repeat_at().filter(|_| text_entry_active(app));
            let wake_at = crate::housekeeping::sooner(hk.wake_at(), repeat);
            // A changed file waiting for its second probe (file_info/cache.rs).
            let wake_at = crate::housekeeping::sooner(wake_at, file_info.wake_at());
            crate::housekeeping::poll_timeout_ms(Instant::now(), wake_at, 500)
        };
        match event_queue.flush() {
            Ok(()) => {}
            // A full socket is not a dead connection: the rest goes out on a
            // later flush, once the compositor has drained its end.
            Err(wayland_client::backend::WaylandError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                eprintln!("[fox] flush error: {e}");
                break;
            }
        }
        if let Some(guard) = event_queue.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            // Poll the watchers' eventfds alongside the wayland fd so a file
            // landing in a shown directory wakes the loop immediately, and
            // the wake fd of the off-thread work (bg.rs) so its results do.
            // (A negative fd is skipped by poll.)
            let pollin = |fd| libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let [watch_fd, inactive_watch_fd] = hk.fds();
            let mut pfds = [
                pollin(fd),
                pollin(watch_fd),
                pollin(inactive_watch_fd),
                pollin(crate::bg::wake_fd().unwrap_or(-1)),
            ];
            let nfds = pfds.len() as libc::nfds_t;
            let ret = unsafe { libc::poll(pfds.as_mut_ptr(), nfds, timeout_ms) };
            // Any revents (POLLIN, but also POLLHUP/POLLERR on compositor
            // death) → read, so connection errors surface in dispatch and
            // the loop exits instead of busy-spinning on a dead fd.
            if ret > 0 && pfds[0].revents != 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
            // What woke us is collected below, frame or no frame.
            if ret > 0 && (pfds[1].revents | pfds[2].revents) & libc::POLLIN != 0 {
                hk.drain_fds();
            }
            if ret > 0 && pfds[3].revents & libc::POLLIN != 0 {
                crate::bg::drain_wake();
            }
        }
        if let Err(e) = event_queue.dispatch_pending(state) {
            eprintln!("[fox] dispatch_pending error: {e}");
            break;
        }
        // Only force a render when something is actually animating. When idle
        // we leave `frame_done` to the event-driven dispatch handlers (pointer
        // motion, keys, configure, etc.) and to the housekeeping below, so an
        // untouched window renders ZERO frames instead of a constant 60fps
        // GPU pass — that idle spin was melting the laptop.
        if needs_anim {
            state.frame_done = true;
        }

        // Theme live-reload poll: a change in System Settings → Appearance
        // (variant, accent, background, opacity) lands within half a second,
        // with a redraw kick so it shows even on an untouched window.
        if last_theme_poll.elapsed() >= Duration::from_millis(500) {
            last_theme_poll = Instant::now();
            let stamp = theme_stamp();
            if stamp != last_theme_stamp {
                last_theme_stamp = stamp;
                *palette = FoxPalette::current();
                bg_opacity = lntrn_theme::background_opacity();
                state.frame_done = true; // force a render this iteration
            }
        }
        // A status note that ran out, an archive job that finished on its
        // own thread: neither produces an event, so look on every wake-up.
        if app.idle_tick() {
            state.frame_done = true;
        }
        // Folder changes, git results, finished operations and probes: on
        // every wake-up too, not only when a frame is drawn anyway. In an
        // idle window a new file used to wait for the pointer to move.
        if hk.tick(app, file_info) {
            state.frame_done = true;
        }
        // The paths of a drop onto the window have been read.
        if state.drop_in.poll(app) {
            state.frame_done = true;
        }
        // The Audio section of Properties has a result to show (a save, a
        // decoded cover, a picked image) or its "Saved" note has run out.
        if app
            .properties
            .as_ref()
            .and_then(|p| p.audio.as_ref())
            .is_some_and(|a| a.frame_wanted())
        {
            state.frame_done = true;
        }
        // A held key is due to repeat into a text field.
        if text_entry_active(app)
            && state
                .kbd
                .repeat_at()
                .is_some_and(|due| Instant::now() >= due)
        {
            state.frame_done = true;
        }
        if !state.frame_done {
            continue;
        }
        state.frame_done = false;

        // Cap rendering at ~60Hz. Pointer motion events fire at ~1000Hz on
        // modern mice; without this cap each event triggered a render and
        // melted the CPU. Sleeping the remaining frame budget lets queued
        // events coalesce into one render per frame. (Wake-ups that draw
        // nothing do not sleep: they have no frame to pace.)
        let since_last = last_frame.elapsed();
        let frame_budget = Duration::from_millis(16);
        if since_last < frame_budget {
            std::thread::sleep(frame_budget - since_last);
        }

        let scale_f = state.fractional_scale() as f32;
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32().min(0.05);
        last_frame = now;

        // Handle resize (xdg configure, or a preferred-scale change which
        // alters phys size without any configure)
        if state.configured || state.scale_changed {
            state.configured = false;
            state.scale_changed = false;
            gpu.ctx
                .resize(state.phys_width().max(1), state.phys_height().max(1));
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
            state
                .popup_backend
                .as_ref()?
                .find_popup_id_by_wl_surface(ps)
        });

        if pointer_on_popup.is_some() {
            input.on_cursor_left();
        } else if state.pointer_in_surface {
            input.on_cursor_moved(cx, cy);
        } else {
            input.on_cursor_left();
        }

        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface {
                pointer_on_popup
            } else {
                None
            };
            backend.route_cursor(active, cx, cy);
        }

        // ── Cursor shape (resize edges) ─────────────────────────────────
        if state.pointer_in_surface && pointer_on_popup.is_none() {
            // Preview-pane resize handle takes priority over window-edge resize
            // so the user gets an EW cursor right on the divider.
            let on_preview_handle = app.preview_drag.is_some() || {
                let view = if app.searching && !app.search_buf.is_empty() {
                    crate::app::ViewMode::List
                } else {
                    app.view_mode
                };
                let supported = matches!(
                    view,
                    crate::app::ViewMode::List | crate::app::ViewMode::Tree
                ) && app.split.is_none();
                if supported && app.preview_open {
                    let full = if app.pick.is_some() {
                        let bottom = hf - crate::pick_bar::PICK_BAR_H * s;
                        crate::layout::content_rect_with_bottom(wf, bottom, s)
                    } else {
                        content_rect(wf, hf, s)
                    };
                    let pw = crate::layout::preview_effective_w(full.w, app.preview_width, true, s);
                    let h_rect = crate::layout::preview_handle_rect(full, pw, s);
                    // pw == 0: the pane doesn't fit and isn't drawn.
                    pw > 0.0 && h_rect.contains(cx, cy)
                } else {
                    false
                }
            };
            // Split divider gets the same EW treatment.
            let on_split_divider = app.split.as_ref().map_or(false, |sp| {
                sp.divider_drag.is_some() || {
                    crate::layout::split_divider_rect(wf, hf, sp.ratio, s).contains(cx, cy)
                }
            });
            let desired = if on_preview_handle || on_split_divider {
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
                (0..view_menu.popup_count()).find(|&d| view_menu.popup_id_at_depth(d) == Some(pid))
            });
            view_menu.set_pointer_depth(vdepth);
        }

        // ── Scrollbar thumb drag ─────────────────────────────────────────
        if let Some(grab_dy) = scrollbar_drag {
            let content = active_content_rect(app, wf, hf, s);
            let total_h = view_content_height(app, content.w, s);
            let bar = crate::scrollbar::bar(&content, total_h, app.scroll_offset, s);
            app.scroll_offset =
                bar.offset_for_thumb_y(cy - grab_dy + bar.thumb.h * 0.5, total_h, content.h);
        }
        if let Some(drag) = &aside_drag {
            drag.follow(app, cy, hf, s);
        }
        // The scrollbar of a cloud dialog's list keeps its own hold.
        app.cloud_list_drag(cy, s);

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
                    crate::app::ViewMode::List => {
                        list_content_height(app.entries.len(), s, zoom) + list_header_h(s, zoom)
                    }
                    crate::app::ViewMode::Tree => {
                        tree_content_height(app.tree_entries.len(), s, zoom)
                    }
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

        // ── Split divider drag ──────────────────────────────────────────
        if app
            .split
            .as_ref()
            .map_or(false, |sp| sp.divider_drag.is_some())
        {
            // Stops where the panes do (each keeps a minimum width in
            // pixels, not just a share of the window).
            if let Some(ratio) = crate::layout::split_ratio_at(cx, wf, s) {
                if let Some(sp) = app.split.as_mut() {
                    sp.ratio = ratio;
                }
                app.split_ratio = ratio;
            }
        }

        // ── Drag detection ──────────────────────────────────────────────
        if state.pointer_in_surface && app.drag_item.is_none() && app.drag_tree_item.is_none() {
            // While an earlier drag is still in the compositor's hands (its
            // receiver may yet ask for the paths), a new one would swap the
            // paths under it: the press stays a click. (Not for ever: a
            // receiver that never finishes is given up on.)
            state.expire_drag_out();
            let may_drag = !state.dnd_active;
            if let (Some(idx), Some((px, py))) =
                (app.pending_open.filter(|_| may_drag), app.press_pos)
            {
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
                        // `get`: a reload between press and this frame can
                        // have shrunk the list under the index.
                        let paths: Vec<std::path::PathBuf> = match app.entries.get(idx) {
                            Some(entry) if !entry.selected => vec![entry.path.clone()],
                            Some(entry) => {
                                let selected = app.selected_paths();
                                if selected.is_empty() {
                                    vec![entry.path.clone()]
                                } else {
                                    selected
                                }
                            }
                            None => Vec::new(),
                        };
                        if paths.is_empty() {
                            app.drag_item = None;
                            app.drag_pos = None;
                        }
                        state.dnd_paths = paths;
                        state.dnd_serial = state.pointer_serial;
                    }
                }
            }

            // Tree rows arm their own pending slot — indices point into
            // tree_entries, not entries (nested rows have no entries index).
            // Only plain presses arm it, so no shift/rubber-band sub-branch.
            if let (Some(ti), Some((px, py))) =
                (app.pending_tree_open.filter(|_| may_drag), app.press_pos)
            {
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
            && !state.dnd_active
            && !state.dnd_paths.is_empty()
        {
            let raw_cx = state.cursor_x as f32;
            let raw_cy = state.cursor_y as f32;
            let logical_w = state.width as f32;
            let logical_h = state.height as f32;
            let outside = raw_cx < 0.0 || raw_cy < 0.0 || raw_cx > logical_w || raw_cy > logical_h;
            if outside && state.hand_off_drag(conn, qh) {
                // Clear internal drag — compositor owns the drag now
                app.drag_item = None;
                app.drag_tree_item = None;
                app.drag_pos = None;
                // …and its grab swallows the button release, so finish
                // the press here or the zone capture (and with it all
                // hover feedback) stays stuck until the next click.
                input.on_left_released();
                app.press_shift = false;
                app.press_ctrl = false;
                app.press_pos = None;
                app.suppress_rubber_band = false;
            }
        }

        // ── Clean up drag state after Wayland DnD ends ──────────────────
        if !state.dnd_active
            && state.dnd_paths.is_empty()
            && (app.drag_item.is_some() || app.drag_tree_item.is_some())
            && !state.pointer_in_surface
        {
            app.drag_item = None;
            app.drag_tree_item = None;
            app.drag_pos = None;
        }

        // ── A drag from outside over the window, or dropped on it ───────
        let own_drag: &[std::path::PathBuf] = if state.dnd_active {
            &state.dnd_paths
        } else {
            &[]
        };
        state.drop_in.frame(conn, app, input, own_drag, wf, hf, s);

        // A nested tree row's right-click overrides `selected_paths()` for
        // the menu's actions. Once the menu is gone — action taken, Esc,
        // click outside, compositor dismissal — the override is stale, and
        // Ctrl+C / Ctrl+X / a drag must go back to the real selection.
        if !context_menu.is_open() {
            app.context_override_paths.clear();
        }

        // ── Keyboard ────────────────────────────────────────────────────
        // Every press since the last frame, in the order typed and each with
        // the modifiers it was made with (keyboard.rs).
        let mut key_handled = false;
        while let Some(press) = state.kbd.pop() {
            key_handled = true;
            // Super+F11: "rice mode" — hide/show the title bar (window mode
            // only). The compositor deliberately lets Super+F11 fall through;
            // plain F11 still toggles compositor fullscreen.
            if press.logo && press.key == crate::keyboard::code::F11 && !state.desktop_mode {
                use std::sync::atomic::Ordering;
                let hidden = !crate::layout::CHROME_HIDDEN.load(Ordering::Relaxed);
                crate::layout::CHROME_HIDDEN.store(hidden, Ordering::Relaxed);
                if hidden && view_menu.is_open() {
                    if let Some(backend) = &mut state.popup_backend {
                        view_menu.close_popups(backend);
                    }
                }
            } else {
                handle_key(
                    app,
                    settings,
                    context_menu,
                    &mut state.popup_backend,
                    press,
                    &mut state.running,
                );
            }
        }

        // Key repeat (for text editing modes). Never in the frame that handled
        // the press itself: a handler that blocked would otherwise find the
        // deadline already passed and fire a second time.
        if key_handled {
            state.kbd.defer_repeat(Instant::now());
        } else if text_entry_active(app) {
            if let Some(press) = state.kbd.take_repeat(Instant::now()) {
                handle_key(
                    app,
                    settings,
                    context_menu,
                    &mut state.popup_backend,
                    press,
                    &mut state.running,
                );
            }
        }

        // ── Scroll ──────────────────────────────────────────────────────
        // Wheel detents move a boosted distance and ease toward the target
        // instead of the old rigid 1:1 jump per event.
        const SCROLL_STEP_MULT: f32 = 4.0;
        if app.quick_look.is_some() {
            // Quick Look covers the window; the view underneath stays put.
            state.scroll_delta = 0.0;
        }
        if state.scroll_delta.abs() > 0.01 {
            let scroll = state.scroll_delta * s * SCROLL_STEP_MULT;
            input.on_scroll(scroll);
            // Wheel over the unfocused split pane scrolls THAT pane (no
            // focus steal — hover-scroll like any modern split UI).
            let over_inactive = app
                .inactive_content_rect(wf, hf, s)
                .filter(|r| r.contains(cx, cy));
            if crate::scrollbar::wheel_aside(app, scroll, cx, cy, hf, s) {
                // The Properties dialog (open) or the sidebar (under the
                // pointer) scrolled; the file list stays where it is.
            } else if let Some(r) = over_inactive {
                if let Some(total_h) = inactive_view_content_height(app, r.w, s) {
                    if let Some(off) = app.inactive_scroll_mut() {
                        ScrollArea::apply_scroll(off, scroll, total_h, r.h);
                    }
                }
            } else {
                let content = active_content_rect(app, wf, hf, s);
                let total_h = view_content_height(app, content.w, s);
                let max = (total_h - content.h).max(0.0);
                let base = scroll_anim.unwrap_or(app.scroll_offset);
                scroll_anim = Some((base + scroll).clamp(0.0, max));
                scroll_anim_last = app.scroll_offset;
            }
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
            } else if app.pending_drop.is_some()
                && app.sudo_prompt.is_none()
                && app.conflict_dialog.is_none()
                && app.cloud_login.is_none()
                && app.drive_dialog.is_none()
                && !app.op_dialog_open()
            {
                // Drop confirmation modal — handle buttons (unless a modal
                // raised later is drawn on top of it; that one gets the click)
                if let Some(zone) = input.on_left_pressed() {
                    match zone {
                        ZONE_DROP_MOVE => {
                            if let Some(drop) = app.pending_drop.take() {
                                app.start_drag_drop(
                                    crate::conflict::PasteMode::Cut,
                                    drop.sources,
                                    drop.dest_dir,
                                    drop.reload_tab,
                                    drop.in_pane,
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
                                    drop.in_pane,
                                );
                            }
                        }
                        ZONE_DROP_CANCEL => {
                            app.pending_drop = None;
                        }
                        _ => {}
                    }
                }
            } else if app.properties.is_some()
                && app.sudo_prompt.is_none()
                && app.conflict_dialog.is_none()
                && app.cloud_login.is_none()
                && app.drive_dialog.is_none()
                && !app.op_dialog_open()
            {
                // Properties dialog is open (and nothing is stacked on top of
                // it — a sudo prompt raised by a finishing copy is drawn over
                // Properties and must get its own clicks, via handle_click)
                if let Some(zone) = input.on_left_pressed() {
                    if zone == 800 || zone == 801 {
                        // Close button or backdrop
                        app.properties = None;
                    } else if (810..=817).contains(&zone) {
                        // Section header toggle
                        if let Some(ref mut props) = app.properties {
                            let idx = (zone - 810) as usize;
                            if idx < props.section_open.len() {
                                props.section_open[idx] = !props.section_open[idx];
                            }
                        }
                    } else if (crate::ZONE_PROPS_AUDIO_FIELD_BASE..=crate::ZONE_PROPS_AUDIO_REVERT)
                        .contains(&zone)
                    {
                        // Audio tag editor: field focus, artwork, Save/Revert.
                        if let Some(audio) = app.properties.as_mut().and_then(|p| p.audio.as_mut())
                        {
                            audio.on_zone_pressed(zone);
                        }
                    } else if zone == crate::ZONE_PROPS_SCROLLBAR {
                        aside_drag = crate::scrollbar::press_properties(app, cy, s);
                    } else if zone == 802 {
                        // Panel body — keep the dialog open, drop field focus.
                        if let Some(audio) = app.properties.as_mut().and_then(|p| p.audio.as_mut())
                        {
                            audio.focused = None;
                        }
                    } else {
                        // The folder icon, the icon picker, the checksum
                        // row: acted on here, on the press (props_click.rs).
                        app.properties_pressed(zone);
                    }
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
                    // Same rect the bar was drawn from and the drag uses
                    // (split pane / pick bar / preview aware).
                    let content = active_content_rect(app, wf, hf, s);
                    let total_h = view_content_height(app, content.w, s);
                    let bar = crate::scrollbar::bar(&content, total_h, app.scroll_offset, s);
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
                } else if input.zone_at(cx, cy) == Some(crate::ZONE_SIDEBAR_SCROLLBAR) {
                    aside_drag = crate::scrollbar::press_sidebar(app, cy, hf, s);
                    input.on_left_pressed();
                    handled_scrollbar = true;
                }
                // Edge resize (window mode only)
                let mut handled_resize = false;
                if let Some(toplevel) = toplevel {
                    // Not when maximized: the edges are ordinary UI there
                    // (same rule as the cursor shape above).
                    if !handled_scrollbar && !state.maximized {
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
                    let prev_pins = app.pinned_tab_paths();
                    let action = handle_click(
                        input,
                        app,
                        view_menu,
                        context_menu,
                        &mut state.popup_backend,
                        &mut last_tab_click,
                        &mut tab_drag_press,
                        &mut fav_drag_press,
                        wf,
                        s,
                        bg_opacity,
                        "",
                        state.kbd.ctrl(),
                        state.kbd.shift(),
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
                    let favorites_changed = app.sidebar_favorites().len() != prev_favorites_len;
                    // A pin is written when it is made, not only at exit: a
                    // session that ends without a clean close keeps it.
                    // (A picker restores no pinned tabs: its list is not
                    // the one in the file.)
                    let pins = app.pinned_tab_paths();
                    if pins != prev_pins && app.pick.is_none() {
                        settings.pinned_tabs = pins;
                        settings_dirty = true;
                    }
                    // Don't persist the forced Tree view from pick mode — it's
                    // a transient launch decision, not a user preference.
                    if app.view_mode != prev_view && app.pick.is_none() {
                        settings.set_view_mode(app.view_mode);
                        settings_dirty = true;
                    }
                    if favorites_changed {
                        // Saves the other changed keys along with the list.
                        app.persist_favorites(settings);
                    } else if settings_dirty {
                        settings.save();
                    }
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
                                        // (No band in a picker for one item:
                                        // it would select several.)
                                        if !app.single_select_only() {
                                            app.rubber_band_start = Some((cx, cy));
                                            app.rubber_band_end = Some((cx, cy));
                                        }
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
                                    if !app.single_select_only() {
                                        app.rubber_band_start = Some((cx, cy));
                                        app.rubber_band_end = Some((cx, cy));
                                    }
                                }
                            }
                        }
                        // A modal took the press: no window move, no rubber
                        // band, no deselect underneath it.
                        ClickAction::Consumed => {}
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
                aside_drag = None;
                app.cloud_list_release();
                if app.rubber_band_start.is_some() {
                    app.rubber_band_start = None;
                    app.rubber_band_end = None;
                }
                if app.preview_drag.take().is_some() {
                    settings.preview_width = app.preview_width;
                    settings.save();
                }
                // Split divider drag release — persist the ratio.
                if app
                    .split
                    .as_mut()
                    .map_or(false, |sp| sp.divider_drag.take().is_some())
                {
                    settings.split_ratio = app.split_ratio;
                    settings.save();
                }
                // Favorite drag release — reorder
                if let Some(src_idx) = fav_drag.take() {
                    // The rows where they are now (the sidebar scrolls).
                    let layout = app.sidebar_layout(hf, s);
                    if let Some((_, cy)) = input.cursor() {
                        // Target slot is whichever favorite row the cursor is
                        // currently over. Off-row releases are a no-op, and
                        // so is one above or below the sidebar's strip: the
                        // rows scrolled out there are not under the pointer.
                        let v = layout.viewport;
                        let target = layout
                            .favorite_items
                            .iter()
                            .position(|r| cy >= r.y && cy < r.y + r.h)
                            .filter(|_| cy >= v.y && cy <= v.y + v.h);
                        if let Some(target_idx) = target {
                            if target_idx != src_idx && src_idx < app.sidebar_favorites().len() {
                                app.reorder_favorite(src_idx, target_idx);
                                app.persist_favorites(settings);
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
                        let target_idx = rects
                            .iter()
                            .position(|r| r.contains(cursor_x, r.y + r.h * 0.5))
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
                } else if app.drag_item.is_some()
                    || app.drag_tree_item.is_some()
                    || (!state.dnd_active && !state.dnd_paths.is_empty())
                {
                    // A live in-window drag always drops. The third clause
                    // is for a drag whose row index was cleared by a re-list:
                    // its snapshot is still pending — unless a Wayland drag
                    // owns it (then the paths are for the receiver).
                    // Internal drop. The sources are the paths captured when
                    // the drag started, not a fresh look-up by row index: the
                    // watcher may have re-listed the folder mid-drag, and the
                    // index would then name a different file (or none).
                    app.drag_item = None;
                    app.drag_tree_item = None;
                    let sources = std::mem::take(&mut state.dnd_paths);
                    if !sources.is_empty() {
                        let prev_fav_len = app.sidebar_favorites().len();
                        handle_drop(app, input, wf, hf, s, sources);
                        if app.sidebar_favorites().len() != prev_fav_len {
                            app.persist_favorites(settings);
                        }
                    }
                    app.pending_open = None;
                    app.pending_tree_open = None;
                    app.drag_pos = None;
                    state.dnd_paths.clear();
                } else if let Some(ti) = app.pending_tree_open.take() {
                    // Deferred tree-row action: the press armed a potential
                    // drag instead of acting; no drag started, so act now
                    // (expand or launch, or select first when the setting
                    // asks for a double click).
                    app.on_tree_row_click(ti);
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

        // A picker with a result is done. Double-click and Enter both go
        // through confirm_pick; before this the double-click only latched the
        // result and the dialog stayed open.
        if app.pick.is_some()
            && matches!(app.pick_result, Some(crate::PickResult::Selected(_)))
        {
            state.running = false;
        }

        // ── Right click ─────────────────────────────────────────────────
        let modal_open = app.quick_look.is_some()
            || app.op_dialog_open()
            || app.conflict_dialog.is_some()
            || app.sudo_prompt.is_some()
            || app.cloud_login.is_some()
            || app.drive_dialog.is_some()
            || app.properties.is_some()
            || app.pending_drop.is_some();
        if state.right_clicked && modal_open {
            // A context menu must not open over a modal and act on the view
            // hidden behind it.
            state.right_clicked = false;
        }
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
            handle_right_click(
                app,
                context_menu,
                &mut state.popup_backend,
                input,
                open_with_apps,
                wf,
                hf,
                s,
            );
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
            bg_opacity
        };
        // `palette` is kept current by the theme poll at the top of the loop.
        let render_palette = palette.with_bg_opacity(opacity);
        // The input above may have gone to another folder: its frame must
        // not show the branch chip of the one just left.
        hk.git_follow(app);
        let inline_evt = crate::render::render_frame(
            gpu,
            app,
            input,
            icon_cache,
            file_info,
            &hk.git,
            &render_palette,
            s,
            state.maximized,
            view_menu,
            context_menu,
            tab_drag,
            fav_drag,
            opacity,
            state.desktop_mode,
        );
        // Handle inline context menu events (desktop mode)
        if let Some(evt) = inline_evt {
            if matches!(evt, MenuEvent::Action(_)) {
                context_menu.close();
            }
            if let MenuEvent::SliderChanged {
                id: crate::CTX_ICON_SIZE,
                value,
            } = evt
            {
                apply_icon_zoom(app, value, wf, hf, s);
            } else {
                handle_ctx_event(
                    app,
                    settings,
                    context_menu,
                    &mut state.popup_backend,
                    open_with_apps,
                    file_info,
                    toplevel,
                    state.maximized,
                    &mut state.running,
                    evt,
                );
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
                        // Written now, not only at exit (which a session
                        // that is killed never reaches). Not from a file
                        // picker: what it shows is its own business.
                        if app.pick.is_none() {
                            settings.save();
                        }
                        app.reload();
                    } else if id == crate::VIEW_SHOW_TITLEBAR_ID {
                        // Live toggle + persisted as the open-time default.
                        // (Super+F11 stays a transient toggle, like the
                        // terminal's rice mode.)
                        crate::layout::CHROME_HIDDEN
                            .store(!checked, std::sync::atomic::Ordering::Relaxed);
                        settings.show_titlebar = checked;
                        settings.save();
                        // Hiding the bar removes the View label the menu is
                        // anchored to — close it along with the bar.
                        if !checked {
                            view_menu.close_popups(backend);
                        }
                    } else if id == crate::VIEW_SOLID_DIVIDERS_ID {
                        crate::sections::SOLID_DIVIDERS
                            .store(checked, std::sync::atomic::Ordering::Relaxed);
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
                if let MenuEvent::SliderChanged {
                    id: crate::CTX_ICON_SIZE,
                    value,
                } = evt
                {
                    apply_icon_zoom(app, value, wf, hf, s);
                } else {
                    handle_ctx_event(
                        app,
                        settings,
                        context_menu,
                        &mut state.popup_backend,
                        open_with_apps,
                        file_info,
                        toplevel,
                        state.maximized,
                        &mut state.running,
                        evt,
                    );
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
                                    &ctx.gpu,
                                    frame.encoder_mut(),
                                    &view,
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
                                        color_name,
                                        &ctx.gpu,
                                        &ctx.tex_pass,
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
                                        let (dx, dy, dw, dh) =
                                            crate::icons::fit_in_box(tex, ix, iy, isz, isz);
                                        tex_draws.push(lntrn_render::TextureDraw::new(
                                            tex, dx, dy, dw, dh,
                                        ));
                                    }
                                }
                                if !tex_draws.is_empty() {
                                    ctx.tex_pass.render_pass(
                                        &ctx.gpu,
                                        frame.encoder_mut(),
                                        &view,
                                        &tex_draws,
                                        None,
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

        // Drain deferred icon-cache invalidations queued by the Properties
        // icon picker. We can't mutate icon_cache during render_frame
        // (tex_draws still borrows it), so we apply changes between frames.
        if !app.pending_icon_apply.is_empty() {
            let pending = std::mem::take(&mut app.pending_icon_apply);
            for (folder, icon) in pending {
                icon_cache.invalidate(&folder);
                // Written and read back on a worker; the entries that show
                // the folder are updated when that lands.
                app.apply_folder_icon(folder, icon);
            }
        }
        // Pre-warm SVG thumbnails for the icon picker, if open. Picker
        // cell rects come from the previous frame's render — so the very
        // first frame after opening shows empty cells, then thumbnails
        // populate on the next frame (~16ms). That next frame is asked for
        // here: rows scrolled into view stayed blank until the pointer
        // happened to move.
        if let Some(ref props) = app.properties {
            for (path, _, _, _, _) in &props.picker_cell_rects {
                if icon_cache.ensure_svg_path(path, &gpu.ctx, &gpu.tex_pass) {
                    state.frame_done = true;
                }
            }
        }

        // What this frame's input set in motion (a navigation, a pane swap,
        // an operation) is picked up right away; if that changes what is
        // shown, the next iteration draws it without waiting.
        if hk.tick(app, file_info) {
            state.frame_done = true;
        }
        hk.frame_drawn(app);

        needs_anim = view_menu.is_open()
            || context_menu.is_open()
            || scroll_anim.is_some()
            || scrollbar_drag.is_some()
            || aside_drag.is_some()
            || app.cloud_list_dragging()
            || app.drag_item.is_some()
            || app.drag_tree_item.is_some()
            || app.rubber_band_start.is_some()
            || app.search_rx.is_some()
            || tab_drag.is_some()
            || fav_drag.is_some()
            || app.preview_drag.is_some()
            || app.ops.is_busy()
            // Polled for its result, and its dialog shows it is alive.
            || app.priv_busy()
            || icon_cache.has_pending()
            || app.dir_loading()
            || file_info.probing()
            || app.quick_look.as_ref().is_some_and(|ql| ql.loading())
            || app
                .properties
                .as_ref()
                .and_then(|p| p.audio.as_ref())
                .is_some_and(|a| a.busy())
            // The checksum lands on a worker thread and is only noticed by a
            // drawn frame; without this the row sat on "Computing…" until
            // the mouse moved.
            || app
                .properties
                .as_ref()
                .and_then(|p| p.checksum_job.as_ref())
                .is_some_and(|j| j.is_running());
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
        crate::app::ViewMode::List => {
            list_content_height(app.entries.len(), s, zoom) + list_header_h(s, zoom)
        }
        crate::app::ViewMode::Tree => tree_content_height(app.tree_entries.len(), s, zoom),
    }
}

/// Height of the List view's fixed "Name / Size / Modified" header. The rows
/// start below it, so it is part of the scrollable height — leaving it out
/// kept the last row out of reach.
fn list_header_h(s: f32, zoom: f32) -> f32 {
    32.0 * crate::layout::list_zoom_multiplier(zoom) * s
}

/// Apply a live icon-zoom change (View menu or right-click menu slider):
/// set the zoom and re-clamp the scroll offset against the new grid height.
fn apply_icon_zoom(app: &mut App, value: f32, wf: f32, hf: f32, s: f32) {
    app.icon_zoom = value;
    let content = active_content_rect(app, wf, hf, s);
    let total_h = view_content_height(app, content.w, s);
    ScrollArea::apply_scroll(&mut app.scroll_offset, 0.0, total_h, content.h);
}

/// The focused pane's content rect (split/pick aware) with the preview pane
/// subtracted (if it's open + this view supports it). Used for hit-testing
/// the rubber-band selection so the band doesn't start inside the info pane.
fn active_content_rect(app: &App, wf: f32, hf: f32, s: f32) -> lntrn_render::Rect {
    let full = app.active_content_rect(wf, hf, s);
    let view = if app.searching && !app.search_buf.is_empty() {
        crate::app::ViewMode::List
    } else {
        app.view_mode
    };
    let preview_supported = matches!(
        view,
        crate::app::ViewMode::List | crate::app::ViewMode::Tree
    ) && app.split.is_none();
    let preview_w = if preview_supported {
        crate::layout::preview_effective_w(full.w, app.preview_width, app.preview_open, s)
    } else {
        0.0
    };
    lntrn_render::Rect::new(full.x, full.y, full.w - preview_w, full.h)
}

/// Scrollable content height of the UNFOCUSED split pane (None when split
/// view is off). Mirrors `view_content_height` using the parked view state.
fn inactive_view_content_height(app: &App, content_w: f32, s: f32) -> Option<f32> {
    let (tab, view, _) = app.inactive_pane()?;
    let zoom = app.icon_zoom;
    Some(match view.view_mode {
        crate::app::ViewMode::Grid => {
            let cols = grid_columns(content_w, s, zoom);
            grid_content_height(tab.entries.len(), cols, s, zoom)
        }
        crate::app::ViewMode::List => {
            list_content_height(tab.entries.len(), s, zoom)
                + 32.0 * crate::layout::list_zoom_multiplier(zoom) * s
        }
        crate::app::ViewMode::Tree => tree_content_height(view.tree_entries.len(), s, zoom),
    })
}
