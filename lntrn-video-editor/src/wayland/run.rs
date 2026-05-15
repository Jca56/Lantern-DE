//! Main event loop: surface creation, GPU init, menu wiring, per-frame
//! interaction dispatch, popup management, and frame submission.

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, MenuBar, PopupSurface, WaylandPopupBackend,
};
use wayland_client::{Connection, EventQueue, Proxy};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1;

use crate::chrome::TITLE_BAR_H;
use crate::playback::Playback;
use crate::preview::PreviewMonitor;
use crate::project::Project;

use super::menu::{
    apply_inspector_drag, delete_selected_clip, dispatch_menu_action, mute_selected_clip_track,
    nudge_selected_speed, split_at_playhead, trigger_open_project, trigger_save, unlink_selected,
    add_track,
};
use super::state::{
    edge_resize, resize_edge_to_cursor, PendingPick, PickKind, State, WaylandHandle, KEY_BACKSLASH,
    KEY_DELETE, KEY_E, KEY_ENTER, KEY_ESC, KEY_LEFTBRACE, KEY_M, KEY_O, KEY_RIGHTBRACE, KEY_S,
    KEY_SPACE, KEY_T,
};

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state
        .compositor
        .clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state
        .wm_base
        .clone()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    if state.width == 0 {
        state.width = 1280;
    }
    if state.height == 0 {
        state.height = 800;
    }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Edit".into());
    toplevel.set_app_id("lntrn-video-editor".into());
    toplevel.set_min_size(960, 600);

    if let Some(mgr) = &state.decoration_mgr {
        let deco = mgr.get_toplevel_decoration(&toplevel, &qh, ());
        deco.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
    }

    surface.commit();
    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    surface.set_buffer_scale(1);
    let viewport = state.viewporter.as_ref().map(|vp| {
        let vp = vp.get_viewport(&surface, &qh, ());
        vp.set_destination(state.width as i32, state.height as i32);
        vp
    });

    // GPU init
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let wl_handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };

    let phys_w = state.phys_width().max(1);
    let phys_h = state.phys_height().max(1);
    let mut gpu = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);
    let mut ix = InteractionContext::new();
    // Custom palette so menu labels and dropdowns match the lantern brown theme.
    let fox = FoxPalette {
        text: crate::chrome::text(),
        text_secondary: crate::chrome::text_dim(),
        accent: crate::chrome::accent(),
        bg: crate::chrome::BG,
        surface: crate::chrome::PANEL,
        surface_2: crate::chrome::PANEL_DARK,
        ..FoxPalette::night_sky()
    };
    let mut menu_bar = MenuBar::new(&fox);

    // Playback + preview
    let mut playback = Playback::new();
    let mut preview = PreviewMonitor::new(&gpu);
    let mut project = Project::new();

    // In-flight file picker (set when File>Open / Import Media spawned a chooser).
    let mut pending_pick: Option<PendingPick> = None;

    // True while the user is dragging the playhead in the timeline.
    let mut scrubbing = false;
    // Was playback running when the scrub started? If so, resume on release.
    let mut scrub_was_playing = false;
    // Last seek position sent during scrub — avoids hammering the decoder when
    // the cursor hasn't moved.
    let mut last_scrub_secs: f64 = -1.0;

    // Active trim drag, if any. `(clip_id, edge)` — set on mouse-down over an
    // edge handle, cleared on release.
    let mut trimming: Option<(crate::project::ClipId, crate::render::TrimEdge)> = None;
    // Active inspector slider drag.
    let mut inspector_drag: Option<crate::inspector::FieldHit> = None;

    // CLI arg: `.lproj` → load project; anything else → open as video and
    // drop on the timeline (the old default behavior).
    if let Some(arg) = std::env::args().nth(1) {
        let path = std::path::PathBuf::from(arg);
        if path.extension().map(|e| e == "lproj").unwrap_or(false) {
            match crate::projectio::load(&path) {
                Ok(mut loaded) => {
                    loaded.save_path = Some(path);
                    project = loaded;
                }
                Err(e) => eprintln!("[video-editor] failed to load project: {e}"),
            }
        } else if let Err(e) = playback.open_file(&path) {
            eprintln!("[video-editor] failed to open {}: {e}", path.display());
        } else {
            project.import_from_playback(&path, &playback);
            if let Some(new_id) = project.insert_selected_at_end() {
                let start = project
                    .timeline_clips
                    .iter()
                    .find(|c| c.id == new_id)
                    .map(|c| c.start)
                    .unwrap_or(0.0);
                playback.activate_clip_at(start, &project);
            }
        }
    }

    // Popup backend
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        let scale = state.fractional_scale() as f32;
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn,
            &compositor,
            &wm_base,
            &xdg_surf,
            vp,
            &gpu,
            scale,
            &qh,
        ));
    }

    let menus = super::menu::build_menus();

    // ── Main loop ──────────────────────────────────────────────────────────
    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[video-editor] dispatch error: {e}");
            break;
        }
        if !state.frame_done {
            continue;
        }
        state.frame_done = false;

        let s = state.fractional_scale() as f32;

        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.width() as f32;
        let hf = gpu.height() as f32;

        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state
                .popup_backend
                .as_ref()?
                .find_popup_id_by_wl_surface(ps)
        });

        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if pointer_on_popup.is_some() {
            ix.on_cursor_left();
        } else if state.pointer_in_surface {
            ix.on_cursor_moved(cx, cy);
        } else {
            ix.on_cursor_left();
        }
        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface {
                pointer_on_popup
            } else {
                None
            };
            backend.route_cursor(active, cx, cy);
        }

        // Keyboard
        if let Some(key) = state.key_pressed.take() {
            let ctrl = state.ctrl_held;
            match key {
                KEY_ESC => state.running = false,
                KEY_ENTER => {
                    if project
                        .insert_selected_at_playhead(playback.timeline_position)
                        .is_none()
                    {
                        eprintln!("[video-editor] no selected media to insert");
                    }
                }
                KEY_SPACE => playback.timeline_toggle(&project),
                KEY_S => {
                    if ctrl {
                        trigger_save(&mut project, &mut pending_pick);
                    } else {
                        split_at_playhead(&mut project, &playback);
                    }
                }
                KEY_DELETE => delete_selected_clip(&mut project),
                KEY_LEFTBRACE => nudge_selected_speed(&mut project, 1.0 / 1.25),
                KEY_RIGHTBRACE => nudge_selected_speed(&mut project, 1.25),
                KEY_BACKSLASH => unlink_selected(&mut project),
                KEY_M => mute_selected_clip_track(&mut project),
                KEY_T => add_track(&mut project, crate::project::TrackKind::Video),
                KEY_O if ctrl => trigger_open_project(&mut pending_pick),
                KEY_E if ctrl => {
                    let req = crate::export::ExportRequest::defaults_for(
                        crate::export::ExportFormat::Mp4,
                    );
                    if let Err(e) = crate::export::start(req, &project) {
                        eprintln!("[video-editor] export: {e}");
                    }
                }
                _ => {}
            }
        }

        // Drain decoder, update timeline position, handle clip transitions.
        playback.tick(&project);
        if playback.frame_changed {
            if let Some(frame) = &playback.current_frame {
                preview.upload_frame(&gpu, frame);
            }
        }

        // Update menu bar hit zones (must run before on_click consults label_rects).
        let title_h = TITLE_BAR_H * s;
        let menubar_rect = lntrn_render::Rect::new(0.0, 0.0, wf * 0.5, title_h);
        menu_bar.update(&mut ix, &menus, menubar_rect, s);

        // Compute layout up front so click handling can hit-test the timeline.
        let layout = crate::layout::Layout::compute(wf, hf, title_h, s);
        let scrub_rect = layout.timeline_scrub_rect(s);

        // Continuous scrub: while held, keep seeking to wherever the cursor is.
        // Only re-issue a seek if the target moved by at least one frame —
        // otherwise we'd thrash the decoder when the user holds without moving.
        if scrubbing && state.pointer_in_surface {
            let prog = ((cx - scrub_rect.x) / scrub_rect.w).clamp(0.0, 1.0);
            let timeline_dur = crate::render::timeline_visible_duration(&project, &playback);
            let target = prog as f64 * timeline_dur;
            let frame_step = 1.0 / playback.fps.max(1.0) as f64;
            if (target - last_scrub_secs).abs() >= frame_step * 0.5 {
                playback.timeline_seek(target, &project);
                last_scrub_secs = target;
            }
        }

        // Continuous inspector slider drag.
        if let Some(hit) = &inspector_drag {
            apply_inspector_drag(&mut project, hit, cx, s);
        }

        // Continuous trim: while a clip edge is being dragged, map cursor x to
        // a timeline-seconds value and update the clip in the project model.
        if let Some((clip_id, edge)) = trimming {
            if let Some(secs) = crate::render::cursor_to_timeline_secs(
                &project,
                &playback,
                &layout.timeline,
                cx,
                s,
            ) {
                match edge {
                    crate::render::TrimEdge::Left => project.trim_left(clip_id, secs),
                    crate::render::TrimEdge::Right => {
                        let source_dur = project
                            .timeline_clips
                            .iter()
                            .find(|c| c.id == clip_id)
                            .and_then(|c| project.media_by_id(c.media_id))
                            .map(|m| m.duration)
                            .unwrap_or(0.0);
                        project.trim_right(clip_id, secs, source_dur);
                    }
                }
            }
        }

        // Left press
        if state.left_pressed {
            state.left_pressed = false;
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else {
                let border = 10.0 * s;
                let controls_x = wf - 120.0 * s;
                if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                    if let Some(seat) = &state.seat {
                        toplevel.resize(seat, state.pointer_serial, edge);
                    }
                } else if cy < title_h {
                    let hit_r = 20.0 * s;
                    let btn_y = title_h * 0.5;
                    let close_cx = wf - 28.0 * s;
                    let max_cx = wf - 66.0 * s;
                    let min_cx = wf - 104.0 * s;
                    let dist = |bx: f32| ((cx - bx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    if dist(close_cx) < hit_r {
                        state.running = false;
                    } else if dist(max_cx) < hit_r {
                        if state.maximized {
                            toplevel.unset_maximized();
                        } else {
                            toplevel.set_maximized();
                        }
                    } else if dist(min_cx) < hit_r {
                        toplevel.set_minimized();
                    } else if !menu_bar.on_click(&mut ix, &menus, s) {
                        if let Some(seat) = &state.seat {
                            toplevel._move(seat, state.pointer_serial);
                        }
                    }
                } else if !menu_bar.on_click(&mut ix, &menus, s) {
                    if let Some(media_id) =
                        crate::render::media_item_at(&project, &layout.media_browser, cx, cy, s)
                    {
                        let path = project.select_media(media_id).map(|item| item.path.clone());
                        if let Some(path) = path {
                            if let Err(e) = playback.open_file(&path) {
                                eprintln!("[video-editor] failed to open {}: {e}", path.display());
                            }
                        }
                    } else if let Some(track_id) = crate::render::track_mute_at(
                        &project,
                        &layout.timeline,
                        cx,
                        cy,
                        s,
                    ) {
                        project.toggle_mute(track_id);
                    } else if let Some(hit) =
                        crate::inspector::field_at(&project, &layout.properties, cx, cy, s)
                    {
                        apply_inspector_drag(&mut project, &hit, cx, s);
                        inspector_drag = Some(hit);
                    } else if let Some((clip_id, edge)) = crate::render::timeline_clip_edge_at(
                        &project,
                        &playback,
                        &layout.timeline,
                        cx,
                        cy,
                        s,
                    ) {
                        project.select_clip(clip_id);
                        trimming = Some((clip_id, edge));
                    } else if let Some(clip_id) = crate::render::timeline_clip_at(
                        &project,
                        &playback,
                        &layout.timeline,
                        cx,
                        cy,
                        s,
                    ) {
                        project.select_clip(clip_id);
                    } else if scrub_rect.contains(cx, cy)
                        && !project.timeline_clips.is_empty()
                    {
                        project.clear_clip_selection();
                        scrub_was_playing = playback.is_playing();
                        if scrub_was_playing {
                            playback.pause();
                        }
                        scrubbing = true;
                        let prog = ((cx - scrub_rect.x) / scrub_rect.w).clamp(0.0, 1.0);
                        let timeline_dur =
                            crate::render::timeline_visible_duration(&project, &playback);
                        let target = prog as f64 * timeline_dur;
                        playback.timeline_seek(target, &project);
                        last_scrub_secs = target;
                    } else {
                        ix.on_left_pressed();
                    }
                }
            }
        }

        // Left release
        if state.left_released {
            state.left_released = false;
            if scrubbing {
                scrubbing = false;
                last_scrub_secs = -1.0;
                if scrub_was_playing {
                    playback.play();
                }
                scrub_was_playing = false;
            }
            if trimming.is_some() {
                trimming = None;
            }
            if inspector_drag.is_some() {
                inspector_drag = None;
            }
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_released();
                    }
                }
            } else {
                ix.on_left_released();
            }
        }

        // Right press (close menus)
        if state.right_pressed {
            state.right_pressed = false;
            menu_bar.close();
        }

        if state.popup_closed {
            state.popup_closed = false;
        }

        state.scroll_delta = 0.0;

        // Cursor shape
        if state.pointer_in_surface {
            let border = 10.0 * s;
            let controls_x = wf - 120.0 * s;
            let desired = if trimming.is_some() {
                wp_cursor_shape_device_v1::Shape::EwResize
            } else if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                resize_edge_to_cursor(edge)
            } else if crate::render::timeline_clip_edge_at(
                &project,
                &playback,
                &layout.timeline,
                cx,
                cy,
                s,
            )
            .is_some()
            {
                wp_cursor_shape_device_v1::Shape::EwResize
            } else {
                wp_cursor_shape_device_v1::Shape::Default
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // ── Render ─────────────────────────────────────────────────────────
        ix.begin_frame();
        painter.clear();

        let sw = gpu.width();
        let sh = gpu.height();
        // Background + chrome
        crate::chrome::draw_background(&mut painter, wf, hf);
        crate::chrome::draw_title_bar(&mut painter, &mut text, s, wf, sw, sh);
        let menu_labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
        menu_bar.draw_with_labels(&mut painter, &mut text, &fox, &menu_labels, sw, sh, s);
        crate::chrome::draw_controls(&mut painter, cx, cy, s, wf, title_h);

        // NLE panels (layout computed earlier this frame for hit-testing)
        crate::render::draw_panels(
            &mut painter,
            &mut text,
            &layout,
            &project,
            &playback,
            s,
            sw,
            sh,
        );

        // Preview monitor (draws over the preview panel area)
        preview.draw(
            &mut painter,
            &mut text,
            &layout.preview,
            &playback,
            s,
            sw,
            sh,
        );

        if !state.maximized {
            crate::chrome::draw_border(&mut painter, wf, hf);
        }

        // Menu bar overlay
        menu_bar.context_menu.update(0.016);
        if let Some(evt) = menu_bar
            .context_menu
            .draw(&mut painter, &mut text, &mut ix, sw, sh)
        {
            use lntrn_ui::gpu::MenuEvent;
            if let MenuEvent::Action(id) = evt {
                dispatch_menu_action(
                    id,
                    &mut state.running,
                    &mut pending_pick,
                    &mut project,
                    &mut playback,
                );
                menu_bar.close();
            }
        }

        // Drain a finished file picker (non-blocking).
        let mut picked_path = None;
        if let Some(pending) = &pending_pick {
            match pending.rx.try_recv() {
                Ok(path) => {
                    picked_path = Some((pending.kind, path));
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    pending_pick = None; // user cancelled or picker errored
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if let Some((kind, path)) = picked_path {
            match kind {
                PickKind::ImportMedia => {
                    if let Err(e) = playback.open_file(&path) {
                        eprintln!(
                            "[video-editor] failed to open {}: {e}",
                            path.display()
                        );
                    } else {
                        project.import_from_playback(&path, &playback);
                    }
                }
                PickKind::OpenProject => match crate::projectio::load(&path) {
                    Ok(mut loaded) => {
                        loaded.save_path = Some(path);
                        project = loaded;
                        // Drop whatever clip is in flight — the loaded project
                        // may not even reference the currently-playing media.
                        playback.pause();
                    }
                    Err(e) => eprintln!("[video-editor] open project: {e}"),
                },
                PickKind::SaveProject => match crate::projectio::save(&project, &path) {
                    Ok(()) => project.save_path = Some(path),
                    Err(e) => eprintln!("[video-editor] save project: {e}"),
                },
            }
            pending_pick = None;
        }

        // Popup surfaces
        if let Some(backend) = &mut state.popup_backend {
            backend.begin_frame_all();
        }
        if let Some(backend) = &mut state.popup_backend {
            backend.render_all();
        }

        // Submit frame: painter → video texture → text
        if let Ok(mut frame) = gpu.begin_frame("video-editor") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
            preview.render_pass(
                &gpu,
                frame.encoder_mut(),
                &view,
                &layout.preview,
                &playback,
                &project,
                s,
            );
            text.render_queued(&gpu, frame.encoder_mut(), &view);
            frame.submit(&gpu.queue);
        }

        ix.clear_scroll();
        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}
