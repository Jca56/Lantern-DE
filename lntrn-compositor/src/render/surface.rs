//! The per-frame DRM-output pipeline. `render_surface` is intentionally
//! long — see the [`super`] module doc.

use std::time::{Duration, Instant};

use smithay::{
    backend::{
        drm::compositor::FrameFlags,
        renderer::{
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                texture::TextureRenderElement,
                utils::RescaleRenderElement,
                AsRenderElements, Id, Kind,
            },
            gles::{element::PixelShaderElement, GlesRenderer, Uniform},
            Renderer,
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use tracing::{trace, warn};

use crate::layer_position::layer_surface_position;
use crate::shaders::{
    HOT_CORNER_GLOW_COLOR, HOT_CORNER_GLOW_SIGMA, HOT_CORNER_GLOW_SIZE,
    TOP_CENTER_GLOW_COLOR, TOP_CENTER_GLOW_HEIGHT, TOP_CENTER_GLOW_LINE_HALF,
    TOP_CENTER_GLOW_SIGMA, TOP_CENTER_GLOW_WIDTH,
};
use crate::udev::{frame_callback_interval, UdevOutputId, BG_COLOR};
use crate::Lantern;

use super::helpers::{capture_window_snapshot, send_presentation_feedback};
use super::CustomRenderElements;

pub fn render_surface(
    state: &mut Lantern,
    node: smithay::backend::drm::DrmNode,
    crtc: smithay::reexports::drm::control::crtc::Handle,
) {
    let render_start = Instant::now();

    // Drain deferred relayout flag set by config-poll (e.g. WM gap change).
    // Must run before any state borrows.
    if state.pending_layout {
        state.pending_layout = false;
        state.apply_tiling_layout();
    }

    // Live-reload monitor positions from config (must run before any udev borrows)
    // Uses wallpaper_frame_counter which is incremented later in this function.
    if state.wallpaper_frame_counter == 0 {
        crate::udev_device::reload_monitor_positions(state);
    }

    // Clear pending state FIRST so early returns don't leave flags stuck.
    // Without this, a failure in render_elements_for_output would leave
    // pending_render=true and cause a busy loop.
    {
        let udev = match state.udev.as_mut() {
            Some(u) => u,
            None => return,
        };
        let backend = match udev.backends.get_mut(&node) {
            Some(b) => b,
            None => return,
        };
        let surface = match backend.surfaces.get_mut(&crtc) {
            Some(s) => s,
            None => return,
        };
        surface.pending_render = false;
    }

    let output = match state.workspaces.outputs_iter().find(|o| {
        o.user_data()
            .get::<UdevOutputId>()
            .map(|id| id.device_id == node && id.crtc == crtc)
            .unwrap_or(false)
    }) {
        Some(o) => o.clone(),
        None => return,
    };

    // Tick animations and handle finished close animations (before borrowing udev)
    let finished_closes = state.animations.tick();
    for surface in &finished_closes {
        if state.closing_windows.iter().any(|cw| cw.surface == *surface) {
            // Zombie window (client-initiated close) — clean up
            state.finish_zombie_close(surface);
        } else {
            // Live window (Super+Q) — send close request
            state.finish_close_animation(surface);
        }
    }
    state.tiling_anim.tick();
    state.workspace_anim.tick();
    state.window_state_anim.tick();
    state.process_pending_workspace_moves();
    let finished_minimizes = state.minimize_anim.tick();
    for surface in &finished_minimizes {
        state.finish_minimize_animation(surface);
    }
    state.poll_workspace_ipc();
    crate::clipboard_ipc::poll(state);

    // Get cursor position relative to this output (logical -> physical)
    let pointer_location = state
        .seat
        .get_pointer()
        .map(|ptr| ptr.current_location())
        .unwrap_or_default();
    let output_pos = state
        .space
        .output_geometry(&output)
        .unwrap_or_default();
    let scale = output
        .current_scale()
        .fractional_scale();
    let cursor_pos: Point<f64, Physical> = (
        (pointer_location.x - output_pos.loc.x as f64) * scale,
        (pointer_location.y - output_pos.loc.y as f64) * scale,
    )
        .into();

    // Promote silent switcher to visible if hold threshold reached
    if state.alt_tab_switcher.should_promote() {
        state.alt_tab_switcher.promote_to_visible();
    }
    let switcher_visible = state.alt_tab_switcher.is_visible();
    // Pre-compute switcher layout before borrowing udev (avoids borrow conflict)
    let thumbnail_slots = if switcher_visible {
        state.alt_tab_switcher.update_sizes(output_pos.size);
        state.alt_tab_switcher.thumbnail_slots(output_pos.size)
    } else {
        Vec::new()
    };
    // Process Command-Center IPC early so any focus-at requests
    // (click-outside-CC → focus underlying window) commit before we
    // take the long-lived immutable borrows below.
    state.cc_thumbs.poll();
    let focus_at_points = state.cc_thumbs.take_focus_at();
    for (px, py) in focus_at_points {
        let pos = smithay::utils::Point::<f64, smithay::utils::Logical>::from(
            (px as f64, py as f64),
        );
        let target = state.visible_element_under(pos).map(|(w, _)| w.clone());
        if let Some(window) = target {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            state.focus_window(&window, serial);
        }
    }

    // Pre-compute fullscreen and maximized surfaces before udev borrows state.
    // Use slices for O(n) linear scan instead of HashSet allocation — these
    // lists are typically 0–2 entries so linear beats hashing overhead.
    let fullscreen_surfaces: &[_] = &state.fullscreen_windows;
    let focused_surface = state.focused_surface.clone();
    let hot_corner = state.hot_corner.corner;
    // SSD state is accessed directly via state.ssd in the render loop

    // Pre-lookup windows for thumbnail slots before udev borrows state
    // Check both mapped windows AND minimized windows (which are unmapped from space)
    use smithay::desktop::Window;
    let thumb_windows: Vec<(usize, Window)> = thumbnail_slots
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| {
            state.find_mapped_window(&slot.surface)
                .or_else(|| {
                    state.minimized_windows.iter()
                        .find(|m| m.surface == slot.surface)
                        .map(|m| m.window.clone())
                })
                .map(|w| (i, w))
        })
        .collect();

    // ── Hover preview pre-computation ────────────────────────────────
    state.hover_preview.poll();
    let pointer_pos = state.seat.get_pointer()
        .map(|p| p.current_location())
        .unwrap_or_default();
    state.hover_preview.tick(pointer_pos.x, pointer_pos.y, output_pos.size);
    let hover_active = state.hover_preview.is_active() && !switcher_visible;
    let hover_slots_and_windows: Vec<(crate::hover_preview::PreviewSlot, Window)> = if hover_active {
        let toplevel_ids = state.foreign_toplevel_state.surface_app_ids();
        let surfaces = state.hover_preview.find_surfaces(&toplevel_ids);
        let windows: Vec<(WlSurface, Window)> = surfaces.iter().filter_map(|surf| {
            let win = state.find_mapped_window(surf)
                .or_else(|| state.minimized_windows.iter()
                    .find(|m| m.surface == *surf)
                    .map(|m| m.window.clone()));
            win.map(|w| (surf.clone(), w))
        }).collect();
        state.hover_preview.set_window_count(windows.len());
        let surfs: Vec<WlSurface> = windows.iter().map(|(s, _)| s.clone()).collect();
        let slots = state.hover_preview.thumbnail_slots(&surfs, output_pos.size);
        slots.into_iter().zip(windows.into_iter().map(|(_, w)| w))
            .map(|(slot, win)| (slot, win))
            .collect()
    } else {
        Vec::new()
    };
    let hover_card = if !hover_slots_and_windows.is_empty() {
        state.hover_preview.render_card(output_pos.size, scale)
    } else {
        Vec::new()
    };

    // ── Command Center thumbnail pre-computation ─────────────────────
    // (cc_thumbs.poll + focus_at draining already ran above so we can
    //  re-borrow state immutably here.)
    let cc_slots_and_windows: Vec<(crate::cc_thumbs::ThumbSlot, Window)> = {
        let slots = state.cc_thumbs.slots();
        if slots.is_empty() {
            Vec::new()
        } else {
            let toplevels = state.foreign_toplevel_state.surface_app_id_titles();
            slots
                .iter()
                .filter_map(|slot| {
                    // Prefer (app_id, title) match; fall back to app_id-only if
                    // title doesn't match (e.g. title raced an update).
                    let surf = toplevels
                        .iter()
                        .find(|(_, a, t)| a == &slot.app_id && t == &slot.title)
                        .or_else(|| toplevels.iter().find(|(_, a, _)| a == &slot.app_id))
                        .map(|(s, _, _)| s.clone())?;
                    let win = state
                        .find_mapped_window(&surf)
                        .or_else(|| {
                            state
                                .minimized_windows
                                .iter()
                                .find(|m| m.surface == surf)
                                .map(|m| m.window.clone())
                        })?;
                    Some((slot.clone(), win))
                })
                .collect()
        }
    };

    // Decay cursor spin-to-grow scale each frame (must be before udev borrow)
    let spin_needs_redraw = state.cursor.tick_spin_decay();

    // Build the list of windows to render BEFORE we mutably borrow
    // `state.udev` — that borrow is held for the rest of the function and
    // would prevent further immutable accesses to `state.workspaces`.
    //
    // During a workspace transition both the outgoing and incoming
    // workspaces contribute windows so the slide animation can show them
    // moving in/out at the same time. Otherwise we only iterate the
    // active workspace's windows for this output. The scratchpad (when
    // present) is tacked on last so it stays visible across workspace
    // switches.
    let output_name_for_lookup = output.name();
    let mut windows: Vec<smithay::desktop::Window> = Vec::new();
    {
        let active_id = state.workspaces.active_id(&output_name_for_lookup);
        let transition = state
            .workspace_anim
            .get(&output_name_for_lookup)
            .map(|t| (t.from_ws, t.to_ws));
        let ids_to_render: Vec<u32> = match transition {
            Some((from, to)) if from != to => vec![from, to],
            _ => vec![active_id],
        };
        for ws_id in ids_to_render {
            if let Some(space) = state.workspace_space(&output_name_for_lookup, ws_id) {
                for w in space.elements() {
                    if !windows.iter().any(|existing| existing == w) {
                        windows.push(w.clone());
                    }
                }
            }
        }
        if let Some(ref scratch_surface) = state.scratchpad_surface {
            if let Some(scratch_win) = state.space
                .elements()
                .find(|w| {
                    crate::window_ext::WindowExt::get_wl_surface(*w).as_ref()
                        == Some(scratch_surface)
                })
                .cloned()
            {
                if !windows.iter().any(|w| w == &scratch_win) {
                    windows.push(scratch_win);
                }
            }
        }
    }

    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let shadow_shader = &udev.shadow_shader;
    let border_shader = &udev.border_shader;
    let border_width = state.border_width as i32;
    let hot_corner_glow_shader = &udev.hot_corner_glow_shader;
    let top_center_glow_shader = &udev.top_center_glow_shader;
    let ssd_icon_shader = &udev.ssd_icon_shader;
    let ssd_header_shader = &udev.ssd_header_shader;
    let corner_shader = &udev.corner_shader;
    let renderer = match udev.renderer.as_mut() {
        Some(r) => r,
        None => return,
    };

    let t_elements = Instant::now();
    trace!("render: gathering elements");

    // Render windows manually with per-window alpha instead of using
    // render_elements_for_output, which applies a single alpha to all windows.
    let output_scale = output.current_scale().fractional_scale();
    let output_geo = match state.workspaces.output_geometry(&output) {
        Some(geo) => geo,
        None => return,
    };

    let mut window_elements: Vec<CustomRenderElements> = Vec::new();
    let mut fullscreen_elements: Vec<CustomRenderElements> = Vec::new();
    // Blur backdrop tracking: (insert index, screen-logical rect)
    let mut blur_backdrops: Vec<(usize, Rectangle<i32, Logical>, f32, f32)> = Vec::new();

    let output_name_str = output.name();
    let ws_transition_now = std::time::Instant::now();

    for window in windows.iter().rev() {
        let win_bbox = {
            let loc = state.workspaces.element_location(window).unwrap_or_default();
            let mut bbox = window.bbox();
            bbox.loc += loc - window.geometry().loc;
            bbox
        };
        if !output_geo.overlaps(win_bbox) {
            continue;
        }

        let location = state.workspaces.element_location(window).unwrap_or_default();
        let _ = location;
        let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(window) else { continue };

        // Space only contains active-workspace windows (unmap/remap on switch).
        // Apply slide offset if a transition is running on this output.
        let mut ws_slide_offset = 0.0f64;
        if let Some((win_output, win_ws)) = state.workspaces.window_workspace(&surface) {
            if win_output == output_name_str {
                if let Some(t) = state.workspace_anim.get(&output_name_str) {
                    if let Some(off) = t.offset_for(win_ws, output_geo.size.w as f64, ws_transition_now) {
                        ws_slide_offset = off;
                    }
                }
            }
        }

        // ── Active-animation rect resolution ─────────────────────────────
        // Priority: minimize > window_state anim > tiling anim > state target > live.
        // window.geometry() is the surface's live geometry — but it can lag
        // when the client is slow to ack a maximize/fullscreen/snap configure.
        // After the state animation finishes, we fall back to the *target* rect
        // stored on the MaximizedWindow/FullscreenWindow/SnappedWindow entry so
        // the window doesn't briefly snap to its stale pre-configure size.
        let win_geo = window.geometry();
        let win_size = win_geo.size;
        let tiling_anim_rect = state.tiling_anim.current_rect(&surface);
        let state_anim_rect = state.window_state_anim.current_rect(&surface);
        let minimize_params = state.minimize_anim.get(&surface).map(|m| m.render_params());
        // Configured-rect fallback used AFTER the state animation finishes,
        // so we render against the rect we asked the client for rather than
        // its (possibly stale) live buffer geometry. Without solo_tiled in
        // this chain, a slow client like Firefox would post-anim collapse
        // back to its previously-acked size for a beat — looking like a
        // "resize after the slide" pop.
        let state_target_rect = state.maximized_windows.iter()
            .find(|e| e.surface == surface).map(|e| e.target)
            .or_else(|| state.fullscreen_windows.iter()
                .find(|e| e.surface == surface).map(|e| e.target))
            .or_else(|| state.snapped_windows.iter()
                .find(|e| e.surface == surface).map(|e| e.target))
            .or_else(|| state.solo_tiled_windows.iter()
                .find(|e| e.surface == surface).map(|e| e.target));

        // Effective rect: where the (un-zoomed, pre-open/close) visible
        // geometry should appear on screen. (x, y, w, h) in logical coords.
        let (eff_x, eff_y, eff_w, eff_h, minimize_alpha) =
            if let Some(p) = &minimize_params {
                // MinimizeParams was historically authored assuming a center
                // pivot on win_size; translate that back to an explicit visual
                // rect here so the new math doesn't need to know.
                let vw = win_size.w as f64 * p.scale.0;
                let vh = win_size.h as f64 * p.scale.1;
                let vx = p.render_loc.x + (win_size.w as f64 - vw) / 2.0;
                let vy = p.render_loc.y + (win_size.h as f64 - vh) / 2.0;
                (vx, vy, vw, vh, p.alpha)
            } else if let Some(rect) = state_anim_rect {
                (rect.loc.x as f64, rect.loc.y as f64,
                 rect.size.w as f64, rect.size.h as f64, 1.0)
            } else if let Some(ref rect) = tiling_anim_rect {
                (rect.loc.x as f64, rect.loc.y as f64,
                 rect.size.w as f64, rect.size.h as f64, 1.0)
            } else if let Some(rect) = state_target_rect {
                (rect.loc.x as f64, rect.loc.y as f64,
                 rect.size.w as f64, rect.size.h as f64, 1.0)
            } else {
                let loc = state.workspaces.element_location(window).unwrap_or_default();
                (loc.x as f64, loc.y as f64,
                 win_size.w as f64, win_size.h as f64, 1.0)
            };

        let is_fullscreen = fullscreen_surfaces.iter().any(|e| e.surface == surface);
        let win_app_id = crate::window_ext::WindowExt::get_app_id(window);
        let blur_excluded = state.blur_exclude.iter().any(|id| id == &win_app_id);
        // Buffers are always rendered at full alpha — translucency now comes
        // exclusively from each client honoring [windows].background_opacity
        // when it draws its own background.
        let mut base_alpha = 1.0f32;
        if state.show_desktop_active {
            base_alpha *= 0.05;
        }
        let zoom = state.window_zoom.get(&surface).copied().unwrap_or(1.0);

        // Open/close anim: pure scale + fade pivoted on the EFF RECT center.
        let anim_params = state.animations.get(&surface).map(|a| a.render_params());
        let anim_alpha = anim_params.as_ref().map(|p| p.alpha).unwrap_or(1.0);
        let anim_scale = anim_params.as_ref().map(|p| p.scale).unwrap_or(1.0);
        let alpha = base_alpha * anim_alpha * minimize_alpha;

        // Apply zoom + open/close scale centered on the eff rect's geometry
        // center → final visible rect on screen.
        let extra_scale = anim_scale * zoom;
        let final_w = eff_w * extra_scale;
        let final_h = eff_h * extra_scale;
        let final_x = eff_x + (eff_w - final_w) / 2.0;
        let final_y = eff_y + (eff_h - final_h) / 2.0;

        // Output-relative top-left of the visible geometry (+ workspace slide).
        let rel_x = final_x - output_geo.loc.x as f64 + ws_slide_offset;
        let rel_y = final_y - output_geo.loc.y as f64;

        // Render scale: surface tree (size = win_size) must end up at final
        // size. Independent of the buffer-acked size jumping mid-animation:
        // when the buffer swaps, render_scale recomputes to keep final size
        // pinned (no teleport).
        let combined_scale_x = if win_size.w > 0 { final_w / win_size.w as f64 } else { 1.0 };
        let combined_scale_y = if win_size.h > 0 { final_h / win_size.h as f64 } else { 1.0 };

        // Surface tree origin in physical coords. The geometry sits at
        // geo.loc within the surface tree, so phys_loc = final_top_left -
        // geo.loc * render_scale (the offset shrinks with the scale).
        let phys_loc_log_x = rel_x - win_geo.loc.x as f64 * combined_scale_x;
        let phys_loc_log_y = rel_y - win_geo.loc.y as f64 * combined_scale_y;
        let phys_loc: Point<i32, Physical> = (
            (phys_loc_log_x * output_scale).round() as i32,
            (phys_loc_log_y * output_scale).round() as i32,
        ).into();

        // Surface tree is always rendered at a UNIFORM `output_scale` —
        // Smithay doesn't anisotropically stretch a Wayland surface via
        // its render_scale parameter, so we explicitly wrap the resulting
        // elements in a `RescaleRenderElement` below with the per-axis
        // `combined_scale`. That gives us a true smooth-resize animation
        // for both the pre-resize snapshot AND the live surface.
        let render_scale = smithay::utils::Scale::from(output_scale);
        let combined_scale = smithay::utils::Scale::from((
            combined_scale_x,
            combined_scale_y,
        ));

        // Final on-screen visible size = the animation's interpolated
        // rect. Both the snapshot AND the live surface get scaled into
        // this rect during a resize anim, with their alphas tied to
        // anim progress so they CROSSFADE — that hides content reflow
        // (e.g. terminal font auto-resize) instead of popping it in.
        let effective_size = smithay::utils::Size::<i32, Logical>::from((
            final_w.round() as i32,
            final_h.round() as i32,
        ));
        let has_ssd = state.ssd.has_ssd(&surface);

        // Determine corner rounding based on window state
        let is_maximized = state.maximized_windows.iter().any(|m| m.surface == surface);
        let snap_zone = state.snapped_windows.iter()
            .find(|s| s.surface == surface)
            .map(|s| s.zone);
        let corners = if is_maximized {
            crate::ssd::RoundedCorners::none()
        } else if let Some(zone) = snap_zone {
            crate::ssd::RoundedCorners::for_snap(zone)
        } else {
            crate::ssd::RoundedCorners::all()
        };

        // Tiled windows get a much smaller corner radius so they read as
        // grid cells rather than floating cards.
        let is_tiled_now = state.workspaces.contains(&surface);
        let win_corner_r_logical = if is_tiled_now {
            crate::ssd::tiled_corner_radius()
        } else {
            crate::ssd::corner_radius()
        };

        let win_log_loc: Point<i32, Logical> = Point::from((
            rel_x as i32,
            rel_y as i32,
        ));

        // Z-order (front-to-back): corner masks → SSD overlay → window → shadow
        // Elements pushed first = higher z (drawn on top).

        // SSD: render header overlay on top of the window.
        // Use effective_size + final top-left so the bar matches the visible
        // (animated) window rather than the live buffer size — otherwise the
        // bar lags during maximize/minimize.
        if has_ssd && !is_fullscreen {
            if let Some(ssd_state) = state.ssd.get_mut(&surface) {
                let visible_phys_loc: Point<i32, Physical> = (
                    (rel_x * output_scale).round() as i32,
                    (rel_y * output_scale).round() as i32,
                ).into();
                let (solid_elems, shader_elems) = crate::ssd::render_decoration(
                    ssd_state, visible_phys_loc, win_log_loc,
                    effective_size, output_scale, ssd_icon_shader.as_ref(),
                    ssd_header_shader.as_ref(), corners,
                    win_corner_r_logical,
                );
                for elem in shader_elems {
                    window_elements.push(CustomRenderElements::Shader(elem));
                }
                for elem in solid_elems {
                    window_elements.push(CustomRenderElements::Overlay(elem));
                }
            }
        }

        // Capture window snapshot for close animation (clean render at native scale).
        // Skip during open animation to avoid interference and wasted work.
        let is_opening = state.animations.get(&surface)
            .map(|a| a.kind == crate::animation::AnimationKind::Open)
            .unwrap_or(false);
        // Throttle snapshot to ~10Hz: used as the close-animation fallback
        // when a client dies AND as the source for the smooth resize
        // transition below. Capturing every frame burns CPU on synchronous
        // GPU sync via frame.finish().
        //
        // Suppress capture while a state animation is in flight so the
        // snapshot stays frozen at its pre-animation content — that's the
        // texture the resize anim will stretch. Without this, the snapshot
        // would refresh mid-anim and capture the post-configure buffer,
        // re-introducing the content pop.
        let resize_animating = state_anim_rect.is_some();
        if !is_fullscreen && !is_opening && !resize_animating
            && state.wallpaper_frame_counter % 6 == 0
        {
            if let Some(snap) = capture_window_snapshot(renderer, window, win_geo.size, output_scale) {
                state.window_snapshots.insert(surface.clone(), snap);
            }
        }

        // Window surface render + crossfade with snapshot during anim.
        //
        // Crossfade strategy (macOS-style):
        //   - `progress` is the eased animation progress [0..1].
        //   - Snapshot (pre-anim content): alpha = (1 - progress).
        //   - Live surface (post-resize content, may include reflowed
        //     font sizes, etc.): alpha = progress.
        //   - BOTH are rendered into the same `effective_size` rect — the
        //     snapshot via TextureRenderElement's dst_size, the live
        //     surface via RescaleRenderElement wrapping each element with
        //     `combined_scale` around `phys_loc`.
        //
        // Out of animation, `progress` defaults to 1 (no crossfade),
        // `combined_scale` is 1.0 (rescale is a no-op), and the snapshot
        // branch is skipped — identical fast path to before.
        let target = if is_fullscreen { &mut fullscreen_elements } else { &mut window_elements };
        let progress = if resize_animating {
            state.window_state_anim.eased_progress(&surface).unwrap_or(1.0)
        } else {
            1.0
        };
        let snap_alpha = (alpha * (1.0 - progress) as f32).clamp(0.0, 1.0);
        let live_alpha = (alpha * progress as f32).clamp(0.0, 1.0);

        // 1) Snapshot (top of crossfade), fading OUT.
        if resize_animating && snap_alpha > 0.01 {
            if let Some((snap_tex, snap_phys)) = state.window_snapshots.get(&surface).cloned() {
                let ctx_id = renderer.context_id();
                let snap_loc = Point::<f64, Physical>::from((
                    (rel_x * output_scale).round(),
                    (rel_y * output_scale).round(),
                ));
                let dst_size = Size::<i32, Logical>::from((
                    effective_size.w.max(1),
                    effective_size.h.max(1),
                ));
                // Explicit src spans the full texture in *logical*
                // coords — without this, sampling outside [0,1] UVs
                // wraps and tiles the snapshot mid grow-animation.
                let src_rect = smithay::utils::Rectangle::<f64, Logical>::from_size(
                    smithay::utils::Size::from((
                        snap_phys.w as f64 / output_scale,
                        snap_phys.h as f64 / output_scale,
                    )),
                );
                let tex_elem = smithay::backend::renderer::element::texture::TextureRenderElement::from_static_texture(
                    smithay::backend::renderer::element::Id::new(),
                    ctx_id,
                    snap_loc,
                    snap_tex,
                    1,
                    smithay::utils::Transform::Normal,
                    Some(snap_alpha),
                    Some(src_rect),
                    Some(dst_size),
                    None,
                    smithay::backend::renderer::element::Kind::Unspecified,
                );
                target.push(CustomRenderElements::Backdrop(tex_elem));
            }
        }

        // 2) Live surface, fading IN (and rescaled to the anim rect).
        let win_render_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            window.render_elements(renderer, phys_loc, render_scale, live_alpha);
        let win_phys_w_raw = (win_geo.size.w as f64 * output_scale) as f32;
        let win_phys_h_raw = (win_geo.size.h as f64 * output_scale) as f32;
        let corner_r = win_corner_r_logical * output_scale as f32;
        // Skip rounding when the surface is smaller than the corner diameter —
        // the SDF mask would be degenerate and tiny transient surfaces (Proton
        // bootstrap splashes etc.) can otherwise trigger GL_INVALID_VALUE when
        // their buffer resizes underneath the element.
        let too_small_for_rounding = win_phys_w_raw < corner_r * 2.0 + 1.0
            || win_phys_h_raw < corner_r * 2.0 + 1.0;
        // Skip surface rounding when an SSD bar is present (bar+surface rounding
        // mismatch produces a notch) or when the window is tiled (client CSD
        // headers don't expect compositor clipping; clients receive set_tiled
        // and can flatten their own corners).
        let needs_rounding = !is_fullscreen && !is_maximized
            && snap_zone.is_none()
            && !too_small_for_rounding
            && !has_ssd
            && !is_tiled_now
            && udev.rounded_tex_shader.is_some();
        // Wrap each live surface element in a RescaleRenderElement so its
        // visible footprint matches `effective_size`. `combined_scale` is
        // 1.0 at rest so this is a no-op outside animations.
        let rescale = |elem: WaylandSurfaceRenderElement<GlesRenderer>| {
            smithay::backend::renderer::element::utils::RescaleRenderElement::from_element(
                elem, phys_loc, combined_scale,
            )
        };
        if needs_rounding {
            let shader = udev.rounded_tex_shader.as_ref().unwrap();
            // Corner mask shader uses the post-rescale texture size so
            // the SDF mask lands at the actual visible edges.
            let win_phys_w = (win_phys_w_raw as f64 * combined_scale_x) as f32;
            let win_phys_h = (win_phys_h_raw as f64 * combined_scale_y) as f32;
            target.extend(win_render_elements.into_iter().map(|e| {
                CustomRenderElements::RoundedSurface(
                    crate::rounded_element::RoundedSurfaceElement::new(
                        rescale(e), shader.clone(), [win_phys_w, win_phys_h], corner_r,
                    ),
                )
            }));
        } else {
            target.extend(
                win_render_elements
                    .into_iter()
                    .map(|e| CustomRenderElements::Rescaled(rescale(e))),
            );
        }

        // Push a blur backdrop behind every non-fullscreen, non-excluded
        // window whenever the system-wide background opacity is < 1. Lantern
        // apps draw their backgrounds at that alpha, so the blur shows
        // through their transparent regions. Opaque clients (Firefox, etc.)
        // simply cover the blur — no visual diff, only a small render cost,
        // and `blur_exclude` is the escape hatch.
        if !is_fullscreen && !blur_excluded && state.system_bg_opacity < 0.99 {
            let ssd_bar = if has_ssd { crate::ssd::SsdManager::bar_height() } else { 0 };
            let log_rect = Rectangle::<i32, Logical>::new(
                Point::from((
                    rel_x.round() as i32,
                    (rel_y - ssd_bar as f64).round() as i32,
                )),
                Size::from((
                    effective_size.w,
                    effective_size.h + ssd_bar,
                )),
            );
            // Backdrop fades with the window. base_alpha is the user's
            // resting transparency preference — that's already what makes
            // the window count as transparent, so the blur underlay should
            // sit at full opacity at rest. Multiply only the animation
            // contributions (anim_alpha + minimize_alpha) so open/close and
            // minimize fade the blur in lockstep with the window itself.
            let blur_alpha = (anim_alpha * minimize_alpha).clamp(0.0, 1.0);
            blur_backdrops.push((window_elements.len(), log_rect, blur_alpha, win_corner_r_logical));
        }

        // Window drop shadow / focus glow (behind window, so pushed after = lower z)
        if !is_fullscreen {
            if let Some(ref shader) = shadow_shader {
                let is_focused = focused_surface.as_ref() == Some(&surface);
                let shadow_expand = if is_focused { 48i32 } else { 40i32 };
                let corner_r = win_corner_r_logical;
                let ssd_bar = if has_ssd { crate::ssd::SsdManager::bar_height() } else { 0 };
                let win_x = rel_x.round() as i32;
                let win_y = rel_y.round() as i32 - ssd_bar;
                let win_w = effective_size.w;
                let win_h = effective_size.h + ssd_bar;
                let shadow_area = Rectangle::<i32, Logical>::new(
                    (win_x - shadow_expand, win_y - shadow_expand).into(),
                    (win_w + shadow_expand * 2, win_h + shadow_expand * 2).into(),
                );
                // Focused windows get a subtle colored glow, others get a dark shadow
                let (sigma, shadow_color) = if is_focused && state.focus_glow {
                    let mut c = state.focus_glow_color;
                    c[3] = state.focus_glow_intensity;
                    (14.0f32, c)
                } else {
                    (12.0f32, [0.0f32, 0.0, 0.0, 0.4])
                };
                let shadow_elem = PixelShaderElement::new(
                    shader.clone(),
                    shadow_area,
                    None,
                    alpha,
                    vec![
                        Uniform::new("window_size", [win_w as f32, win_h as f32]),
                        Uniform::new("sigma", sigma),
                        Uniform::new("corner_radius", corner_r),
                        Uniform::new("shadow_color", shadow_color),
                    ],
                    Kind::Unspecified,
                );
                window_elements.push(CustomRenderElements::Shader(shadow_elem));
            }

            // Window border — sits between shadow and window content. Skipped when
            // border_width is 0. Color matches focus_glow_color (full alpha) so it
            // visually ties into the same accent palette.
            if border_width > 0 {
                if let Some(ref shader) = border_shader {
                    let corner_r = win_corner_r_logical;
                    let ssd_bar = if has_ssd { crate::ssd::SsdManager::bar_height() } else { 0 };
                    let win_x = rel_x.round() as i32;
                    let win_y = rel_y.round() as i32 - ssd_bar;
                    let win_w = effective_size.w;
                    let win_h = effective_size.h + ssd_bar;
                    // Expand by border_width + 1 so the ring isn't clipped at the edge.
                    let pad = border_width + 1;
                    let border_area = Rectangle::<i32, Logical>::new(
                        (win_x - pad, win_y - pad).into(),
                        (win_w + pad * 2, win_h + pad * 2).into(),
                    );
                    let mut bc = state.border_color;
                    bc[3] = 1.0;
                    let border_elem = PixelShaderElement::new(
                        shader.clone(),
                        border_area,
                        None,
                        alpha,
                        vec![
                            Uniform::new("window_size", [win_w as f32, win_h as f32]),
                            Uniform::new("corner_radius", corner_r),
                            Uniform::new("border_width", border_width as f32),
                            Uniform::new("border_color", bc),
                        ],
                        Kind::Unspecified,
                    );
                    window_elements.push(CustomRenderElements::Shader(border_elem));
                }
            }
        }
    }

    // Render zombie closing windows (client-initiated closes) using captured snapshots
    {
        let ctx_id = renderer.context_id();
        for cw in &state.closing_windows {
            let anim = match state.animations.get(&cw.surface) {
                Some(a) => a,
                None => continue,
            };
            let params = anim.render_params();
            let anim_alpha = params.alpha;
            let anim_scale = params.scale;
            let (snap_tex, _snap_phys_size) = match state.window_snapshots.get(&cw.surface) {
                Some(s) => s,
                None => continue,
            };

            let render_location = cw.location - output_geo.loc;
            let rel_x = render_location.x as f64 - output_geo.loc.x as f64;
            let rel_y = render_location.y as f64 - output_geo.loc.y as f64;

            // SSD bar offset
            let ssd_bar = if cw.had_ssd { crate::ssd::SsdManager::bar_height() } else { 0 };

            // Centered scale transform (same logic as live windows)
            let win_w = cw.size.w as f64;
            let win_h = cw.size.h as f64;
            let center_x = rel_x + win_w / 2.0;
            let center_y = rel_y + win_h / 2.0;
            let scaled_x = center_x - (win_w / 2.0) * anim_scale;
            let scaled_y = center_y - (win_h / 2.0) * anim_scale;
            let phys_x = (scaled_x * output_scale).round() as i32;
            let phys_y = (scaled_y * output_scale).round() as i32;

            let dst_w_log = (win_w * anim_scale).round() as i32;
            let dst_h_log = (win_h * anim_scale).round() as i32;
            if dst_w_log <= 0 || dst_h_log <= 0 { continue; }

            let loc = Point::<f64, Physical>::from((phys_x as f64, phys_y as f64));
            let dst_size = Size::from((dst_w_log, dst_h_log));

            let tex_elem = TextureRenderElement::from_static_texture(
                Id::new(),
                ctx_id.clone(),
                loc,
                snap_tex.clone(),
                1,
                Transform::Normal,
                Some(anim_alpha),
                None, // full src
                Some(dst_size),
                None,
                Kind::Unspecified,
            );
            window_elements.push(CustomRenderElements::Backdrop(tex_elem));

            // Shadow behind the zombie
            if let Some(ref shader) = shadow_shader {
                let shadow_expand = 40i32;
                let corner_r = crate::ssd::corner_radius();
                let win_x = (scaled_x).round() as i32;
                let win_y = (scaled_y).round() as i32 - ssd_bar;
                let shadow_w = (win_w * anim_scale).round() as i32;
                let shadow_h = ((win_h + ssd_bar as f64) * anim_scale).round() as i32;
                let shadow_area = Rectangle::<i32, Logical>::new(
                    (win_x - shadow_expand, win_y - shadow_expand).into(),
                    (shadow_w + shadow_expand * 2, shadow_h + shadow_expand * 2).into(),
                );
                let shadow_elem = PixelShaderElement::new(
                    shader.clone(),
                    shadow_area,
                    None,
                    anim_alpha,
                    vec![
                        Uniform::new("window_size", [shadow_w as f32, shadow_h as f32]),
                        Uniform::new("sigma", 12.0f32),
                        Uniform::new("corner_radius", corner_r),
                        Uniform::new("shadow_color", [0.0f32, 0.0, 0.0, 0.4]),
                    ],
                    Kind::Unspecified,
                );
                window_elements.push(CustomRenderElements::Shader(shadow_elem));
            }
        }
    }

    let elements_elapsed = t_elements.elapsed();
    let t_post_loop = Instant::now();

    // Build combined elements front-to-back: cursor, switcher overlay, top layers, windows, bottom layers, wallpaper.
    let mut elements: Vec<CustomRenderElements> =
        Vec::with_capacity(window_elements.len() + 16);

    // Cursor: either compositor-drawn xcursor or client surface cursor
    if let Some(cursor_elem) = state.cursor.render_element(renderer, cursor_pos) {
        elements.push(CustomRenderElements::Memory(cursor_elem));
    } else if let smithay::input::pointer::CursorImageStatus::Surface(ref surface) = state.cursor.status {
        use smithay::wayland::compositor::with_states;
        use smithay::input::pointer::CursorImageSurfaceData;
        // Smithay stores hotspot under `Mutex<CursorImageAttributes>`
        // (aliased as `CursorImageSurfaceData`) — querying the bare
        // `CursorImageAttributes` type returns None and silently falls
        // through to a (0, 0) hotspot, making every client-side custom
        // cursor render with its top-left at the pointer position
        // instead of its declared hotspot.
        let hotspot = with_states(surface, |states| {
            states
                .data_map
                .get::<CursorImageSurfaceData>()
                .map(|m| m.lock().unwrap().hotspot)
                .unwrap_or_default()
        });
        let surface_pos: Point<i32, Physical> = (
            (cursor_pos.x - hotspot.x as f64 * scale) as i32,
            (cursor_pos.y - hotspot.y as f64 * scale) as i32,
        ).into();
        let cursor_surface_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                surface,
                surface_pos,
                scale,
                1.0,
                Kind::Cursor,
            );
        elements.extend(cursor_surface_elements.into_iter().map(CustomRenderElements::Surface));
    }

    // Hot corner glow feedback (above windows, below cursor)
    if let (Some(corner), Some(ref glow_shader)) = (hot_corner, &hot_corner_glow_shader) {
        use crate::hot_corners::ScreenCorner;
        // Top-center: dedicated shader handles the wide horizontal
        // bloom in a single draw call. Distance is measured to a
        // horizontal line segment so the band is uniformly bright in
        // the middle and fades toward the ends.
        if corner == ScreenCorner::TopCenter {
            if let Some(ref shader) = top_center_glow_shader {
                let pos_x = output_pos.loc.x + output_pos.size.w / 2 - TOP_CENTER_GLOW_WIDTH / 2;
                let pos_y = output_pos.loc.y;
                let area = Rectangle::new(
                    (pos_x, pos_y).into(),
                    (TOP_CENTER_GLOW_WIDTH, TOP_CENTER_GLOW_HEIGHT).into(),
                );
                let elem = PixelShaderElement::new(
                    shader.clone(),
                    area,
                    None,
                    1.0,
                    vec![
                        Uniform::new("glow_color", TOP_CENTER_GLOW_COLOR),
                        Uniform::new("sigma", TOP_CENTER_GLOW_SIGMA),
                        Uniform::new("line_half_len", TOP_CENTER_GLOW_LINE_HALF),
                    ],
                    Kind::Unspecified,
                );
                elements.push(CustomRenderElements::Shader(elem));
            }
            // Suppress unused-var warning when only this branch is taken.
            let _ = glow_shader;
        } else {
        let (glow_w, glow_h, corner_uniform, pos_x, pos_y, color, sigma) = match corner {
            ScreenCorner::TopLeft => (
                HOT_CORNER_GLOW_SIZE, HOT_CORNER_GLOW_SIZE,
                [0.0f32, 0.0],
                output_pos.loc.x,
                output_pos.loc.y,
                HOT_CORNER_GLOW_COLOR, HOT_CORNER_GLOW_SIGMA,
            ),
            ScreenCorner::TopRight => (
                HOT_CORNER_GLOW_SIZE, HOT_CORNER_GLOW_SIZE,
                [1.0, 0.0],
                output_pos.loc.x + output_pos.size.w - HOT_CORNER_GLOW_SIZE,
                output_pos.loc.y,
                HOT_CORNER_GLOW_COLOR, HOT_CORNER_GLOW_SIGMA,
            ),
            ScreenCorner::BottomLeft => (
                HOT_CORNER_GLOW_SIZE, HOT_CORNER_GLOW_SIZE,
                [0.0, 1.0],
                output_pos.loc.x,
                output_pos.loc.y + output_pos.size.h - HOT_CORNER_GLOW_SIZE,
                HOT_CORNER_GLOW_COLOR, HOT_CORNER_GLOW_SIGMA,
            ),
            ScreenCorner::BottomRight => (
                HOT_CORNER_GLOW_SIZE, HOT_CORNER_GLOW_SIZE,
                [1.0, 1.0],
                output_pos.loc.x + output_pos.size.w - HOT_CORNER_GLOW_SIZE,
                output_pos.loc.y + output_pos.size.h - HOT_CORNER_GLOW_SIZE,
                HOT_CORNER_GLOW_COLOR, HOT_CORNER_GLOW_SIGMA,
            ),
            ScreenCorner::TopCenter => unreachable!("handled above"),
        };
        let glow_area = Rectangle::new(
            (pos_x, pos_y).into(),
            (glow_w, glow_h).into(),
        );
        let glow_elem = PixelShaderElement::new(
            glow_shader.clone(),
            glow_area,
            None, // opaque_regions
            1.0,  // alpha
            vec![
                Uniform::new("corner", corner_uniform),
                Uniform::new("glow_color", color),
                Uniform::new("sigma", sigma),
            ],
            Kind::Unspecified,
        );
        elements.push(CustomRenderElements::Shader(glow_elem));
        }
    }

    // Alt+Tab switcher: elements are ordered front-to-back (first = highest Z).
    // Layer order: close btn / minimized dim → thumbnails → cards / highlights → panel → dim.
    if switcher_visible {
        // Chrome elements from render_overlay. The returned order is:
        //   [dim, panel, (highlight?, card, min_dim?, close_btn?) × N]
        // We split into base chrome (behind thumbnails) and top chrome (above thumbnails).
        let (base_chrome, top_chrome) = state
            .alt_tab_switcher
            .render_overlay_split(output_pos.size, scale);

        // 1) Top overlays (close button, minimized dim) — highest Z, above thumbnails
        let mut top: Vec<_> = top_chrome
            .into_iter()
            .map(CustomRenderElements::Overlay)
            .collect();
        top.reverse();
        elements.extend(top);

        // 2) Thumbnail surfaces
        for &(slot_idx, ref window) in &thumb_windows {
            let slot = &thumbnail_slots[slot_idx];
            let win_geo = window.geometry();
            if win_geo.size.w <= 0 || win_geo.size.h <= 0 {
                continue;
            }

            let scale_x = slot.size.w as f64 / win_geo.size.w as f64;
            let scale_y = slot.size.h as f64 / win_geo.size.h as f64;
            let thumb_scale = scale_x.min(scale_y);

            let rendered_w = (win_geo.size.w as f64 * thumb_scale).round() as i32;
            let rendered_h = (win_geo.size.h as f64 * thumb_scale).round() as i32;
            let offset_x = (slot.size.w - rendered_w) / 2;
            let offset_y = (slot.size.h - rendered_h) / 2;

            let content_phys: Point<i32, Physical> = (
                ((slot.position.x + offset_x) as f64 * output_scale).round() as i32,
                ((slot.position.y + offset_y) as f64 * output_scale).round() as i32,
            ).into();

            let geo_loc = win_geo.loc;
            let base_phys: Point<i32, Physical> = (
                content_phys.x - (geo_loc.x as f64 * output_scale).round() as i32,
                content_phys.y - (geo_loc.y as f64 * output_scale).round() as i32,
            ).into();

            let full_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                window.render_elements(
                    renderer,
                    base_phys,
                    smithay::utils::Scale::from(output_scale),
                    1.0,
                );

            for elem in full_elements {
                let rescaled = RescaleRenderElement::from_element(
                    elem,
                    content_phys,
                    smithay::utils::Scale::from(thumb_scale),
                );
                elements.push(CustomRenderElements::Rescaled(rescaled));
            }
        }

        // 3) Base chrome (cards, highlights, panel, dim) — behind thumbnails
        let mut base: Vec<_> = base_chrome
            .into_iter()
            .map(CustomRenderElements::Overlay)
            .collect();
        base.reverse();
        elements.extend(base);
    }

    // ── Hover preview (above bar, below alt-tab) ──────────────────
    if !hover_slots_and_windows.is_empty() {
        for (ref slot, ref hover_window) in &hover_slots_and_windows {
            let win_geo = hover_window.geometry();
            if win_geo.size.w > 0 && win_geo.size.h > 0 {
                let scale_x = slot.size.w as f64 / win_geo.size.w as f64;
                let scale_y = slot.size.h as f64 / win_geo.size.h as f64;
                let thumb_scale = scale_x.min(scale_y);
                let rendered_w = (win_geo.size.w as f64 * thumb_scale).round() as i32;
                let rendered_h = (win_geo.size.h as f64 * thumb_scale).round() as i32;
                let offset_x = (slot.size.w - rendered_w) / 2;
                let offset_y = (slot.size.h - rendered_h) / 2;

                let content_phys: Point<i32, Physical> = (
                    ((slot.position.x + offset_x) as f64 * output_scale).round() as i32,
                    ((slot.position.y + offset_y) as f64 * output_scale).round() as i32,
                ).into();

                let geo_loc = win_geo.loc;
                let base_phys: Point<i32, Physical> = (
                    content_phys.x - (geo_loc.x as f64 * output_scale).round() as i32,
                    content_phys.y - (geo_loc.y as f64 * output_scale).round() as i32,
                ).into();

                let full_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    hover_window.render_elements(
                        renderer,
                        base_phys,
                        Scale::from(output_scale),
                        1.0,
                    );

                for elem in full_elements {
                    let rescaled = RescaleRenderElement::from_element(
                        elem,
                        content_phys,
                        Scale::from(thumb_scale),
                    );
                    elements.push(CustomRenderElements::Rescaled(rescaled));
                }
            }
        }
        // Card background (behind thumbnails)
        for card_elem in hover_card {
            elements.push(CustomRenderElements::Overlay(card_elem));
        }
    }

    // ── Command Center in-tile thumbnails ─────────────────────────────
    // Pushed BEFORE layer-surface insertion (later in this fn) so they
    // sit IN FRONT of the CC panel layer surface in z-order (Smithay
    // renders elements front-to-back from index 0).
    if !cc_slots_and_windows.is_empty() {
        // Close button overlays — pushed FIRST so they end up at lower
        // indices in `elements`, which means they sit in FRONT of the
        // thumbnails (Smithay's front-most-first order).
        let close_buttons: Vec<(crate::cc_thumbs::CloseBtn, bool)> = cc_slots_and_windows
            .iter()
            .filter_map(|(s, _)| s.close.map(|c| (c, c.hovered)))
            .collect();
        if !close_buttons.is_empty() {
            let first_size = close_buttons[0].0.rect.size;
            state.cc_thumbs.close_bg_idle.resize((first_size.w, first_size.h));
            state.cc_thumbs.close_bg_hover.resize((first_size.w, first_size.h));
            // Glyph displays at ~58% of the bg size so it has a little
            // breathing room around it.
            let glyph_inset = (first_size.w as f32 * 0.21).round() as i32;

            for (close, hovered) in &close_buttons {
                let phys = |x: i32, y: i32| -> smithay::utils::Point<i32, smithay::utils::Physical> {
                    (
                        (x as f64 * output_scale).round() as i32,
                        (y as f64 * output_scale).round() as i32,
                    )
                        .into()
                };
                let scale = smithay::utils::Scale::from(output_scale);
                let kind = smithay::backend::renderer::element::Kind::Unspecified;
                let bg_buf = if *hovered {
                    &state.cc_thumbs.close_bg_hover
                } else {
                    &state.cc_thumbs.close_bg_idle
                };

                // X glyph (front-most) — bitmap so the strokes can be
                // proper diagonals, not an axis-aligned "+".
                let glyph_phys_pos: smithay::utils::Point<f64, smithay::utils::Physical> = (
                    (close.rect.loc.x + glyph_inset) as f64 * output_scale,
                    (close.rect.loc.y + glyph_inset) as f64 * output_scale,
                )
                    .into();
                let glyph_dst = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
                    close.rect.size.w - 2 * glyph_inset,
                    close.rect.size.h - 2 * glyph_inset,
                ));
                if let Ok(x_elem) =
                    smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        glyph_phys_pos,
                        &state.cc_thumbs.x_glyph,
                        None,
                        None,
                        Some(glyph_dst),
                        kind,
                    )
                {
                    elements.push(CustomRenderElements::Memory(x_elem));
                }

                let bg = smithay::backend::renderer::element::solid::SolidColorRenderElement::from_buffer(
                    bg_buf,
                    phys(close.rect.loc.x, close.rect.loc.y),
                    scale,
                    1.0,
                    kind,
                );
                elements.push(CustomRenderElements::Overlay(bg));
            }
        }

        for (ref slot, ref win) in &cc_slots_and_windows {
            let win_geo = win.geometry();
            if win_geo.size.w <= 0 || win_geo.size.h <= 0 {
                continue;
            }
            let scale_x = slot.rect.size.w as f64 / win_geo.size.w as f64;
            let scale_y = slot.rect.size.h as f64 / win_geo.size.h as f64;
            let thumb_scale = scale_x.min(scale_y);
            let rendered_w = (win_geo.size.w as f64 * thumb_scale).round() as i32;
            let rendered_h = (win_geo.size.h as f64 * thumb_scale).round() as i32;
            let offset_x = (slot.rect.size.w - rendered_w) / 2;
            let offset_y = (slot.rect.size.h - rendered_h) / 2;
            let content_phys: Point<i32, Physical> = (
                ((slot.rect.loc.x + offset_x) as f64 * output_scale).round() as i32,
                ((slot.rect.loc.y + offset_y) as f64 * output_scale).round() as i32,
            )
                .into();
            let geo_loc = win_geo.loc;
            let base_phys: Point<i32, Physical> = (
                content_phys.x - (geo_loc.x as f64 * output_scale).round() as i32,
                content_phys.y - (geo_loc.y as f64 * output_scale).round() as i32,
            )
                .into();
            let full_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = win
                .render_elements(
                    renderer,
                    base_phys,
                    smithay::utils::Scale::from(output_scale),
                    1.0,
                );
            for elem in full_elements {
                let rescaled = RescaleRenderElement::from_element(
                    elem,
                    content_phys,
                    smithay::utils::Scale::from(thumb_scale),
                );
                elements.push(CustomRenderElements::Rescaled(rescaled));
            }
        }

    }

    // Fullscreen windows render above layer surfaces (e.g. above the bar).
    elements.extend(fullscreen_elements);

    // Layer surfaces: single pass, bucket into top (above windows) and bottom (behind windows).
    let mut bottom_layer_elements: Vec<CustomRenderElements> = Vec::new();
    {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::wlr_layer::{LayerSurfaceCachedState, Layer};
        for ls in &state.layer_surfaces {
            if !ls.alive() {
                continue;
            }
            let cached = with_states(ls.wl_surface(), |states| {
                *states.cached_state.get::<LayerSurfaceCachedState>().current()
            });
            let is_top = cached.layer == Layer::Top || cached.layer == Layer::Overlay;
            let is_bottom = cached.layer == Layer::Background || cached.layer == Layer::Bottom;
            if !is_top && !is_bottom {
                continue;
            }
            let ls_pos = layer_surface_position(&cached, output_pos, scale);
            let surface_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    renderer,
                    ls.wl_surface(),
                    ls_pos,
                    scale,
                    1.0,
                    Kind::Unspecified,
                );
            let target = if is_top { &mut elements } else { &mut bottom_layer_elements };
            target.extend(surface_elements.into_iter().map(CustomRenderElements::Surface));
        }
    }

    // (window_elements and bottom_layer_elements extended after blur pipeline below)

    let t_after_chrome = Instant::now();

    // Periodically check if wallpaper config changed
    state.wallpaper_frame_counter += 1;
    if state.wallpaper_frame_counter >= 300 {
        state.wallpaper_frame_counter = 0;
        state.wallpaper.reload_if_changed();
    }

    // Periodically reload input config (mouse speed, cursor theme).
    // ~0.5s at 60Hz; reads are mtime-cached so this is just a stat() most of
    // the time — keep snappy so System Settings changes feel instant.
    state.input_config_counter += 1;
    if state.input_config_counter >= 30 {
        state.input_config_counter = 0;
        state.mouse_speed = crate::input::read_input_setting_f64("mouse_speed", 0.0);
        state.scroll_speed = crate::input::read_input_setting_f64("scroll_speed", 1.0);
        let new_accel = crate::input::read_input_setting("pointer_acceleration", "true") == "true";
        if new_accel != state.pointer_acceleration {
            state.pointer_acceleration = new_accel;
            for device in &state.libinput_devices {
                crate::udev::apply_pointer_accel(device, new_accel);
            }
        }
        let new_theme = crate::input::read_input_setting("cursor_theme", "default");
        if new_theme != state.cursor_theme_name {
            state.cursor_theme_name = new_theme.clone();
            state.cursor.set_custom_theme(&new_theme);
        }
        let new_size = crate::input::read_input_setting_f64("cursor_size", 24.0).round() as u32;
        if new_size != state.cursor.cursor_size() {
            state.cursor.set_cursor_size(new_size);
        }
        state.power.reload_from_config();
        state.power.tick();
        // Gap sync runs here, but the actual relayout has to happen outside
        // the udev borrow — set a flag the next render() pass picks up.
        if state.workspaces.sync_gaps_from_config() {
            state.pending_layout = true;
        }
        state.system_bg_opacity = crate::read_config_f32("background_opacity", 1.0);
        state.blur_exclude = crate::read_config_list("windows", "blur_exclude");
        state.focus_glow = crate::read_config("window_manager", "focus_glow", "true") == "true";
        state.focus_glow_color = crate::parse_glow_color(&crate::read_config("window_manager", "focus_glow_color", "#4A9EFF"));
        state.border_color = crate::parse_glow_color(&crate::read_config("window_manager", "border_color", "#4A9EFF"));
        state.blur_tint_color = crate::parse_glow_color(&crate::read_config("windows", "blur_tint_color", "#4A9EFF"));
        state.focus_glow_intensity = crate::read_config("window_manager", "focus_glow_intensity", "0.2")
            .parse::<f32>().unwrap_or(0.2).clamp(0.0, 0.6);
        state.border_width = crate::read_config("window_manager", "border_width", "0")
            .parse::<u32>().unwrap_or(0).clamp(0, 10);
    }

    let t_after_config = Instant::now();

    // ── Blur pipeline: render background, dual-kawase blur, insert backdrops ──
    let output_phys = Size::<i32, Physical>::from((
        (output_geo.size.w as f64 * output_scale).round() as i32,
        (output_geo.size.h as f64 * output_scale).round() as i32,
    ));
    let blur_intensity = crate::read_config_f32("blur_intensity", 0.8);
    let blur_enabled = blur_intensity >= 0.05;
    if !blur_backdrops.is_empty() && blur_enabled {
        if let (Some(ref down_shader), Some(ref up_shader)) =
            (&udev.blur_down_shader, &udev.blur_up_shader)
        {
            let blur_tint = crate::read_config_f32("blur_tint", 0.15);
            let blur_darken = crate::read_config_f32("blur_darken", 0.0);
            let passes = if blur_intensity < 0.3 { 2usize }
                else if blur_intensity < 0.6 { 3 }
                else if blur_intensity < 0.8 { 4 }
                else { 5 };

            if crate::blur::ensure_textures(renderer, output_phys, passes, &mut udev.blur_state) {
                // Blur source: everything behind transparent windows.
                // List is front-to-back, so take elements after the topmost
                // transparent window's insert point (behind = higher indices).
                let top_idx = blur_backdrops.iter().map(|(i, _, _, _)| *i).min().unwrap_or(0);
                let below_windows = &window_elements[top_idx..];

                let mut wp_elements: Vec<CustomRenderElements> = Vec::new();
                if let Some(wp_elem) = state.wallpaper.render_element_for_output(renderer, &output.name(), output_pos.size, scale) {
                    wp_elements.push(CustomRenderElements::Memory(wp_elem));
                }

                // Back-to-front render order: wallpaper → bottom layers → windows
                let element_groups: Vec<&[CustomRenderElements]> = vec![
                    &wp_elements,
                    &bottom_layer_elements,
                    below_windows,
                ];

                // Premultiplied tint for the shader. Color comes from the
                // configured `blur_tint_color` (same palette the focus glow
                // uses), scaled by `blur_tint` strength.
                let tint_rgba = if blur_tint > 0.001 {
                    let t = blur_tint.clamp(0.0, 1.0);
                    let c = state.blur_tint_color;
                    [c[0] * t, c[1] * t, c[2] * t, t]
                } else {
                    [0.0f32, 0.0, 0.0, 0.0]
                };

                let blur_state = udev.blur_state.as_mut().unwrap();

                // Throttle blur: re-render at most every 100ms unless an
                // animation is actively changing the background. The previous
                // frame's blur result is reused when we skip — visually
                // imperceptible since the wallpaper + non-transparent windows
                // don't move during cursor motion or transparent-window
                // re-renders. Saves ~30ms of GPU sync per skipped frame.
                let any_anim_active = state.animations.has_active()
                    || state.tiling_anim.has_active()
                    || state.workspace_anim.is_active()
                    || state.window_state_anim.has_active()
                    || state.minimize_anim.has_active();
                let needs_reblur = blur_state.last_blur.map_or(true, |t| {
                    any_anim_active || t.elapsed() >= std::time::Duration::from_millis(100)
                });

                let blur_result = if needs_reblur {
                    let r = crate::blur::render_and_blur(
                        renderer, blur_state, &element_groups, BG_COLOR.into(),
                        output_phys, output_scale, down_shader, up_shader,
                        tint_rgba, blur_darken,
                    );
                    if r.is_ok() {
                        blur_state.last_blur = Some(std::time::Instant::now());
                    }
                    r
                } else {
                    Ok(())
                };

                match blur_result {
                    Ok(()) => {
                        let ctx_id = {
                            use smithay::backend::renderer::Renderer as _;
                            renderer.context_id()
                        };
                        let output_logical = Size::<i32, Logical>::from((
                            output_geo.size.w, output_geo.size.h,
                        ));
                        let blur_tex_w = (output_phys.w / 2).max(1) as f32;
                        let blur_tex_h = (output_phys.h / 2).max(1) as f32;

                        for (idx, log_rect, alpha, radius_logical) in blur_backdrops.iter().rev() {
                            let corner_r = radius_logical * output_scale as f32;
                            let backdrop = crate::blur::create_backdrop(
                                blur_state, ctx_id.clone(), *log_rect,
                                output_logical, output_scale, *alpha,
                            );
                            // Wrap in rounded backdrop with SDF corner masking
                            if let Some(ref shader) = udev.backdrop_shader {
                                let phys_w = (log_rect.size.w as f64 * output_scale).round() as f32;
                                let phys_h = (log_rect.size.h as f64 * output_scale).round() as f32;
                                let rounded = crate::rounded_element::RoundedBackdropElement::new(
                                    backdrop, shader.clone(),
                                    [phys_w, phys_h], corner_r,
                                    [blur_tex_w, blur_tex_h],
                                );
                                window_elements.insert(
                                    *idx, CustomRenderElements::RoundedBackdrop(rounded),
                                );
                            } else {
                                window_elements.insert(
                                    *idx, CustomRenderElements::Backdrop(backdrop),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("blur: render_and_blur failed: {:?}", e);
                    }
                }
            }
        }
    }

    elements.extend(window_elements);
    elements.extend(bottom_layer_elements);

    if let Some(wallpaper_elem) = state.wallpaper.render_element_for_output(renderer, &output.name(), output_pos.size, scale) {
        elements.push(CustomRenderElements::Memory(wallpaper_elem));
    }

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    let surface = match backend.surfaces.get_mut(&crtc) {
        Some(s) => s,
        None => return,
    };

    // Always composite — never allow DRM primary plane scanout.
    // Scanout bypasses our compositor, which means software-rendered cursor
    // and overlays (SSD, hotcorner glow, etc.) won't be drawn.
    // CSD-only windows like Firefox would get scanned out and freeze the cursor.
    let frame_flags = FrameFlags::empty();

    let t_render = Instant::now();
    let result = surface.drm_output.render_frame(
        renderer,
        &elements,
        BG_COLOR,
        frame_flags,
    );
    let render_elapsed = t_render.elapsed();

    let (rendered, frame_is_empty) = match result {
        Ok(result) => (!result.is_empty, result.is_empty),
        Err(err) => {
            warn!("Render error: {:?}", err);
            return;
        }
    };


    // Fulfill any pending screencopy requests after a successful render
    if rendered && !state.pending_screencopy.is_empty() {
        let pending: Vec<_> = state.pending_screencopy.drain(..).collect();
        crate::screencopy_render::fulfill_screencopy(renderer, &output, pending);
    }

    // Send frame callbacks even if frame is empty (clients need them to
    // know when to submit new content).
    let mut frame_callback_count = 0;
    if state.pending_client_frame_callbacks {
        frame_callback_count = state.space.elements().count();
        state.space.elements().for_each(|window| {
            window.send_frame(
                &output,
                state.start_time.elapsed(),
                Some(frame_callback_interval(&output)),
                |_, _| Some(output.clone()),
            );
        });
        for ls in &state.layer_surfaces {
            if ls.alive() {
                smithay::desktop::utils::send_frames_surface_tree(
                    ls.wl_surface(),
                    &output,
                    state.start_time.elapsed(),
                    Some(frame_callback_interval(&output)),
                    |_, _| Some(output.clone()),
                );
            }
        }
        state.pending_client_frame_callbacks = false;
    }

    // presentation-time: fire feedback for every surface that has a pending
    // callback. Games (Unity, Proton) use this for frame pacing — if we
    // advertise the protocol but never fire feedback, callbacks pile up.
    // Borrow individual fields so it coexists with the udev &mut borrow above.
    send_presentation_feedback(
        &state.space,
        &state.layer_surfaces,
        state.start_time,
        &output,
        rendered,
    );

    // Only submit to DRM when there's actual damage — skip the atomic
    // commit when nothing changed (saves GPU and bus bandwidth).
    let mut queued = false;
    if rendered {
        surface.frame_pending = true;
        surface.frame_pending_since = Some(Instant::now());
        trace!("render: queue_frame starting");
        if let Err(e) = surface.drm_output.queue_frame(()) {
            surface.frame_pending = false;
            surface.frame_pending_since = None;
            warn!("Failed to queue frame: {:?}", e);
        } else {
            queued = true;
        }
        trace!("render: queue_frame done");
    } else if frame_is_empty {
        trace!("render: frame is empty, skipping queue_frame");
    }

    // Vblank watchdog: arm a recovery timer in case the page-flip we just
    // queued never produces a vblank (e.g. DRM master timing during early
    // session activation). Without this, frame_pending would stay true
    // forever and the compositor would visually freeze.
    if queued {
        crate::udev::arm_vblank_watchdog(state);
    }

    state.record_render(frame_callback_count);

    // Keep rendering while animations are active
    // Also keep rendering while switcher is silently waiting for hold threshold
    let switcher_pending = state.alt_tab_switcher.is_active() && !state.alt_tab_switcher.is_visible();
    let needs_anim_redraw = state.animations.has_active()
        || state.tiling_anim.has_active()
        || state.workspace_anim.is_active()
        || state.window_state_anim.has_active()
        || state.minimize_anim.has_active()
        || state.alt_tab_switcher.needs_redraw()
        || state.hover_preview.needs_redraw()
        || switcher_pending;

    // If we sent frame callbacks this frame, keep the vblank stream going at
    // 60Hz for the next ~500ms. Without this, the DRM driver only emits
    // vblank events for queued page flips — so vblanks were firing at the
    // CLIENT'S commit rate, which was ~10Hz when terminals waited on FIFO
    // present (chicken-and-egg between client commits and our vblanks).
    // Keeping pending_render true means: vblank → render → callback →
    // client renders → commits → vblank → ... at a stable 60Hz.
    if frame_callback_count > 0 {
        state.last_callback_render = Instant::now();
    }
    let recently_active_clients = state.last_callback_render.elapsed()
        < Duration::from_millis(500);

    if needs_anim_redraw || recently_active_clients {
        state.schedule_render();
    }

    let total_elapsed = render_start.elapsed();
    if total_elapsed > Duration::from_millis(4) {
        let prelude_ms = (t_elements - render_start).as_secs_f64() * 1000.0;
        let chrome_ms = (t_after_chrome - t_post_loop).as_secs_f64() * 1000.0;
        let config_ms = (t_after_config - t_after_chrome).as_secs_f64() * 1000.0;
        let blur_ms = (t_render - t_after_config).as_secs_f64() * 1000.0;
        warn!(
            total_ms = total_elapsed.as_secs_f64() * 1000.0,
            prelude_ms,
            elements_ms = elements_elapsed.as_secs_f64() * 1000.0,
            chrome_ms,    // cursor + switcher + layer surfaces
            config_ms,    // wallpaper reload + config reread
            blur_ms,      // blur pipeline + element list assembly
            render_ms = render_elapsed.as_secs_f64() * 1000.0,
            "Slow render detected"
        );
    }
    if state.debug_counters.enabled {
        state.debug_counters.render_micros += total_elapsed.as_micros() as u64;
    }
}
