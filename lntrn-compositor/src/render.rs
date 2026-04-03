/// Render pipeline: builds render elements and submits frames to DRM outputs.

use std::time::{Duration, Instant};

use smithay::{
    backend::{
        drm::compositor::FrameFlags,
        renderer::{
            element::{
                memory::MemoryRenderBufferRenderElement,
                render_elements,
                solid::SolidColorRenderElement,
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                texture::TextureRenderElement,
                utils::RescaleRenderElement,
                AsRenderElements, Kind,
            },
            gles::{
                element::{PixelShaderElement, TextureShaderElement},
                GlesRenderer, GlesTexture, Uniform,
            },
        },
    },
    desktop::space::SpaceRenderElements,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use tracing::{trace, warn};

use crate::shaders::{HOT_CORNER_GLOW_COLOR, HOT_CORNER_GLOW_SIGMA, HOT_CORNER_GLOW_SIZE};
use crate::udev::{frame_callback_interval, UdevOutputId, BG_COLOR, RENDER_INTERVAL};
use crate::Lantern;

// Combined render element enum: space windows + cursor overlay
render_elements! {
    pub CustomRenderElements<=GlesRenderer>;
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Overlay=SolidColorRenderElement,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Shader=PixelShaderElement,
    Rescaled=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    TextureShader=TextureShaderElement,
    Backdrop=TextureRenderElement<GlesTexture>,
    RoundedSurface=crate::rounded_element::RoundedSurfaceElement,
}

pub fn render_surface(
    state: &mut Lantern,
    node: smithay::backend::drm::DrmNode,
    crtc: smithay::reexports::drm::control::crtc::Handle,
) {
    let render_start = Instant::now();

    // Clear pending state FIRST so early returns don't leave flags stuck.
    // Without this, a failure in render_elements_for_output would leave
    // pending_render=true with no cooldown, causing a CPU-burning busy loop.
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
        surface.cooldown_until = Instant::now() + surface.pending_interval;
        surface.pending_interval = RENDER_INTERVAL;
    }

    let output = match state.space.outputs().find(|o| {
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
        state.finish_close_animation(surface);
    }

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
    // Pre-compute fullscreen and maximized surfaces before udev borrows state.
    // Use slices for O(n) linear scan instead of HashSet allocation — these
    // lists are typically 0–2 entries so linear beats hashing overhead.
    let fullscreen_surfaces: &[_] = &state.fullscreen_windows;
    let _focused_surface = state.focused_surface.clone();
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

    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let shadow_shader = &udev.shadow_shader;
    let hot_corner_glow_shader = &udev.hot_corner_glow_shader;
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
    let output_geo = match state.space.output_geometry(&output) {
        Some(geo) => geo,
        None => return,
    };

    // Tick canvas animation
    let canvas_animating = state.canvas.tick(1.0 / 60.0);

    // Iterate windows back-to-front (space stores front-to-back, so reverse).
    // We must collect because the loop body calls state.space.element_location().
    let windows: Vec<_> = state.space.elements().cloned().collect();
    let mut window_elements: Vec<CustomRenderElements> = Vec::new();
    let mut fullscreen_elements: Vec<CustomRenderElements> = Vec::new();
    // Blur backdrop tracking: (insert index, behind-content-end index, screen-logical rect)
    let mut blur_backdrops: Vec<(usize, usize, Rectangle<i32, Logical>)> = Vec::new();

    // Canvas transform: compute viewport in canvas-space for culling
    let canvas_offset = state.canvas.offset;
    let canvas_zoom = state.canvas.zoom;

    for window in windows.iter().rev() {
        // Window bounding box is in canvas-space
        let win_bbox = {
            let loc = state.space.element_location(window).unwrap_or_default();
            let mut bbox = window.bbox();
            bbox.loc += loc - window.geometry().loc;
            bbox
        };

        // Transform bbox to screen-space for viewport culling
        let screen_bbox = Rectangle::new(
            Point::from((
                ((win_bbox.loc.x as f64 - canvas_offset.0) * canvas_zoom) as i32,
                ((win_bbox.loc.y as f64 - canvas_offset.1) * canvas_zoom) as i32,
            )),
            Size::from((
                (win_bbox.size.w as f64 * canvas_zoom).ceil() as i32,
                (win_bbox.size.h as f64 * canvas_zoom).ceil() as i32,
            )),
        );
        if !output_geo.overlaps(screen_bbox) {
            continue;
        }

        let location = state.space.element_location(window).unwrap_or_default();
        let render_location = location - window.geometry().loc;

        let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(window) else { continue };
        let is_fullscreen = fullscreen_surfaces.iter().any(|e| e.surface == surface);
        let mut base_alpha = if is_fullscreen {
            1.0
        } else {
            state.window_opacity.get(&surface).copied()
                .unwrap_or(state.default_window_opacity)
        };
        if state.show_desktop_active {
            base_alpha *= 0.05;
        }
        let zoom = state.window_zoom.get(&surface).copied().unwrap_or(1.0);

        // Compute combined scale: animation scale * zoom
        let anim_params = state.animations.get(&surface).map(|a| a.render_params());
        let anim_alpha = anim_params.map(|(a, _)| a).unwrap_or(1.0);
        let anim_scale = anim_params.map(|(_, s)| s).unwrap_or(1.0);
        let alpha = base_alpha * anim_alpha;

        // Center the scale transform around the window's center
        let win_geo = window.geometry();

        // Apply canvas transform: canvas-space → screen-space
        let (screen_x, screen_y) = state.canvas.canvas_to_screen(
            render_location.x as f64,
            render_location.y as f64,
        );
        let rel_x = screen_x - output_geo.loc.x as f64;
        let rel_y = screen_y - output_geo.loc.y as f64;

        // Include canvas zoom in the combined scale
        let combined_scale = anim_scale * zoom * canvas_zoom;

        let phys_loc: Point<i32, Physical> = if (combined_scale - 1.0).abs() > f64::EPSILON {
            let center_x = rel_x + win_geo.size.w as f64 * canvas_zoom / 2.0;
            let center_y = rel_y + win_geo.size.h as f64 * canvas_zoom / 2.0;
            let scaled_x = center_x - (win_geo.size.w as f64 / 2.0) * combined_scale;
            let scaled_y = center_y - (win_geo.size.h as f64 / 2.0) * combined_scale;
            (
                (scaled_x * output_scale).round() as i32,
                (scaled_y * output_scale).round() as i32,
            ).into()
        } else {
            (
                (rel_x * output_scale).round() as i32,
                (rel_y * output_scale).round() as i32,
            ).into()
        };

        let render_scale = smithay::utils::Scale::from(output_scale * combined_scale);

        let win_geo = window.geometry();
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

        let win_log_loc: Point<i32, Logical> = Point::from((
            (rel_x * canvas_zoom) as i32,
            (rel_y * canvas_zoom) as i32,
        ));

        // Z-order (front-to-back): corner masks → SSD overlay → window → shadow
        // Elements pushed first = higher z (drawn on top).

        // Record index before this window's elements for blur source slicing
        let win_elem_start = window_elements.len();

        // SSD: render header overlay on top of the window
        if has_ssd && !is_fullscreen {
            if let Some(ssd_state) = state.ssd.get_mut(&surface) {
                let (solid_elems, shader_elems) = crate::ssd::render_decoration(
                    ssd_state, phys_loc, win_log_loc,
                    win_geo.size, output_scale, ssd_icon_shader.as_ref(),
                    ssd_header_shader.as_ref(), corners,
                );
                for elem in shader_elems {
                    window_elements.push(CustomRenderElements::Shader(elem));
                }
                for elem in solid_elems {
                    window_elements.push(CustomRenderElements::Overlay(elem));
                }
            }
        }

        // Window surface (behind SSD overlay, in front of shadow)
        let win_render_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            window.render_elements(renderer, phys_loc, render_scale, alpha);
        let target = if is_fullscreen { &mut fullscreen_elements } else { &mut window_elements };
        let needs_rounding = !is_fullscreen && !is_maximized
            && snap_zone.is_none()
            && udev.rounded_tex_shader.is_some();
        if needs_rounding {
            let shader = udev.rounded_tex_shader.as_ref().unwrap();
            let win_phys_w = (win_geo.size.w as f64 * output_scale * canvas_zoom) as f32;
            let win_phys_h = (win_geo.size.h as f64 * output_scale * canvas_zoom) as f32;
            let corner_r = crate::ssd::CORNER_RADIUS * output_scale as f32;
            target.extend(win_render_elements.into_iter().map(|e| {
                CustomRenderElements::RoundedSurface(
                    crate::rounded_element::RoundedSurfaceElement::new(
                        e, shader.clone(), [win_phys_w, win_phys_h], corner_r,
                    ),
                )
            }));
        } else {
            target.extend(
                win_render_elements.into_iter().map(CustomRenderElements::Surface),
            );
        }

        // Track transparent windows for blur backdrop
        if !is_fullscreen && alpha < 0.99 {
            let ssd_bar = if has_ssd { crate::ssd::SsdManager::bar_height() } else { 0 };
            // Screen-logical rect (rel_x/rel_y are already in screen-logical space)
            let log_rect = Rectangle::<i32, Logical>::new(
                Point::from((
                    rel_x.round() as i32,
                    (rel_y - ssd_bar as f64).round() as i32,
                )),
                Size::from((
                    (win_geo.size.w as f64 * canvas_zoom).round() as i32,
                    ((win_geo.size.h + ssd_bar) as f64 * canvas_zoom).round() as i32,
                )),
            );
            blur_backdrops.push((window_elements.len(), win_elem_start, log_rect));
        }

        // Window drop shadow (behind window, so pushed after = lower z)
        if !is_fullscreen {
            if let Some(ref shader) = shadow_shader {
                let shadow_expand = 40i32;
                let corner_r = crate::ssd::CORNER_RADIUS;
                let ssd_bar = if has_ssd { crate::ssd::SsdManager::bar_height() } else { 0 };
                let win_x = rel_x.round() as i32;
                let win_y = rel_y.round() as i32 - ssd_bar;
                let win_w = win_geo.size.w;
                let win_h = win_geo.size.h + ssd_bar;
                let shadow_area = Rectangle::<i32, Logical>::new(
                    (win_x - shadow_expand, win_y - shadow_expand).into(),
                    (win_w + shadow_expand * 2, win_h + shadow_expand * 2).into(),
                );
                // Smithay sets the shader `size` uniform in logical pixels (scale 1).
                // All other uniforms must match that coordinate space.
                let shadow_elem = PixelShaderElement::new(
                    shader.clone(),
                    shadow_area,
                    None,
                    alpha,
                    vec![
                        Uniform::new("window_size", [win_w as f32, win_h as f32]),
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

    // Build combined elements front-to-back: cursor, switcher overlay, top layers, windows, bottom layers, wallpaper.
    let mut elements: Vec<CustomRenderElements> =
        Vec::with_capacity(window_elements.len() + 16);

    // Cursor: either compositor-drawn xcursor or client surface cursor
    if let Some(cursor_elem) = state.cursor.render_element(renderer, cursor_pos) {
        elements.push(CustomRenderElements::Memory(cursor_elem));
    } else if let smithay::input::pointer::CursorImageStatus::Surface(ref surface) = state.cursor.status {
        use smithay::wayland::compositor::with_states;
        use smithay::input::pointer::CursorImageAttributes;
        let hotspot = with_states(surface, |states| {
            states
                .data_map
                .get::<CursorImageAttributes>()
                .map(|attrs| attrs.hotspot)
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
        let glow_size = HOT_CORNER_GLOW_SIZE;
        let (corner_uniform, pos_x, pos_y) = match corner {
            ScreenCorner::TopLeft => (
                [0.0f32, 0.0],
                output_pos.loc.x,
                output_pos.loc.y,
            ),
            ScreenCorner::TopRight => (
                [1.0, 0.0],
                output_pos.loc.x + output_pos.size.w - glow_size,
                output_pos.loc.y,
            ),
            ScreenCorner::BottomLeft => (
                [0.0, 1.0],
                output_pos.loc.x,
                output_pos.loc.y + output_pos.size.h - glow_size,
            ),
            ScreenCorner::BottomRight => (
                [1.0, 1.0],
                output_pos.loc.x + output_pos.size.w - glow_size,
                output_pos.loc.y + output_pos.size.h - glow_size,
            ),
        };
        let glow_area = Rectangle::new(
            (pos_x, pos_y).into(),
            (glow_size, glow_size).into(),
        );
        let glow_elem = PixelShaderElement::new(
            glow_shader.clone(),
            glow_area,
            None, // opaque_regions
            1.0,  // alpha
            vec![
                Uniform::new("corner", corner_uniform),
                Uniform::new("glow_color", HOT_CORNER_GLOW_COLOR),
                Uniform::new("sigma", HOT_CORNER_GLOW_SIGMA),
            ],
            Kind::Unspecified,
        );
        elements.push(CustomRenderElements::Shader(glow_elem));
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

    // Periodically check if wallpaper config changed
    state.wallpaper_frame_counter += 1;
    if state.wallpaper_frame_counter >= 300 {
        state.wallpaper_frame_counter = 0;
        state.wallpaper.reload_if_changed();
    }

    // Periodically reload input config (mouse speed, cursor theme)
    state.input_config_counter += 1;
    if state.input_config_counter >= 300 {
        state.input_config_counter = 0;
        state.mouse_speed = crate::input::read_input_setting_f64("mouse_speed", 0.0);
        state.mouse_acceleration =
            crate::input::read_input_setting("mouse_acceleration", "true") == "true";
        let new_theme = crate::input::read_input_setting("cursor_theme", "default");
        if new_theme != state.cursor_theme_name {
            state.cursor_theme_name = new_theme.clone();
            state.cursor.set_custom_theme(&new_theme);
        }
        state.default_window_opacity = crate::read_config_f32("window_opacity", 1.0);
    }

    // ── Blur pipeline: render background, dual-kawase blur, insert backdrops ──
    let output_phys = Size::<i32, Physical>::from((
        (output_geo.size.w as f64 * output_scale).round() as i32,
        (output_geo.size.h as f64 * output_scale).round() as i32,
    ));
    if !blur_backdrops.is_empty() {
        if let (Some(ref down_shader), Some(ref up_shader)) =
            (&udev.blur_down_shader, &udev.blur_up_shader)
        {
            let blur_intensity = crate::read_config_f32("blur_intensity", 0.8);
            let passes = if blur_intensity < 0.3 { 2usize }
                else if blur_intensity < 0.6 { 3 }
                else if blur_intensity < 0.8 { 4 }
                else { 5 };

            if crate::blur::ensure_textures(renderer, output_phys, passes, &mut udev.blur_state) {
                // Blur source: everything behind transparent windows.
                // List is front-to-back, so take elements after the topmost
                // transparent window's insert point (behind = higher indices).
                let top_idx = blur_backdrops.iter().map(|(i, _, _)| *i).min().unwrap_or(0);
                let below_windows = &window_elements[top_idx..];

                let mut wp_elements: Vec<CustomRenderElements> = Vec::new();
                if let Some(wp_elem) = state.wallpaper.render_element(renderer, output_pos, scale) {
                    wp_elements.push(CustomRenderElements::Memory(wp_elem));
                }

                // Back-to-front render order: wallpaper → bottom layers → windows
                let element_groups: Vec<&[CustomRenderElements]> = vec![
                    &wp_elements,
                    &bottom_layer_elements,
                    below_windows,
                ];

                let blur_state = udev.blur_state.as_mut().unwrap();
                match crate::blur::render_and_blur(
                    renderer, blur_state, &element_groups, BG_COLOR.into(),
                    output_phys, output_scale, down_shader, up_shader,
                ) {
                    Ok(()) => {
                        let ctx_id = {
                            use smithay::backend::renderer::Renderer as _;
                            renderer.context_id()
                        };
                        let output_logical = Size::<i32, Logical>::from((
                            output_geo.size.w, output_geo.size.h,
                        ));
                        for (idx, _, log_rect) in blur_backdrops.iter().rev() {
                            let backdrop = crate::blur::create_backdrop(
                                blur_state, ctx_id.clone(), *log_rect,
                                output_logical, output_scale,
                            );
                            window_elements.insert(
                                *idx,
                                CustomRenderElements::Backdrop(backdrop),
                            );
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

    if let Some(wallpaper_elem) = state.wallpaper.render_element(renderer, output_pos, scale) {
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

    // Only submit to DRM when there's actual damage — skip the atomic
    // commit when nothing changed (saves GPU and bus bandwidth).
    if rendered {
        surface.frame_pending = true;
        trace!("render: queue_frame starting");
        if let Err(e) = surface.drm_output.queue_frame(()) {
            surface.frame_pending = false;
            warn!("Failed to queue frame: {:?}", e);
        }
        trace!("render: queue_frame done");
    } else if frame_is_empty {
        trace!("render: frame is empty, skipping queue_frame");
    }

    state.record_render(frame_callback_count);

    // Keep rendering while animations are active
    // Also keep rendering while switcher is silently waiting for hold threshold
    let switcher_pending = state.alt_tab_switcher.is_active() && !state.alt_tab_switcher.is_visible();
    if state.animations.has_active()
        || state.alt_tab_switcher.needs_redraw()
        || state.hover_preview.needs_redraw()
        || switcher_pending
        || canvas_animating
    {
        state.schedule_render();
    }

    let total_elapsed = render_start.elapsed();
    if total_elapsed > Duration::from_millis(8) {
        warn!(
            total_ms = total_elapsed.as_secs_f64() * 1000.0,
            elements_ms = elements_elapsed.as_secs_f64() * 1000.0,
            render_ms = render_elapsed.as_secs_f64() * 1000.0,
            "Slow render detected"
        );
    }
}

// Re-export for external callers (e.g. input.rs hit testing)
pub use crate::layer_position::layer_surface_position_logical;
use crate::layer_position::layer_surface_position;
