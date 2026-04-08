use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::solid::SolidColorRenderElement,
            element::{memory::MemoryRenderBufferRenderElement, render_elements, AsRenderElements, Kind},
            element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
    utils::{IsAlive, Physical, Point, Rectangle, Transform},
};

use crate::Lantern;

render_elements! {
    WinitRenderElements<=GlesRenderer>;
    Wallpaper=MemoryRenderBufferRenderElement<GlesRenderer>,
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Overlay=SolidColorRenderElement,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
}

// Fox Dark background: #181818 → normalized RGBA
const BG_COLOR: [f32; 4] = [0.094, 0.094, 0.094, 1.0];
const LANTERN_OUTPUT_SCALE: f64 = 1.25;

fn frame_callback_interval(output: &Output) -> Duration {
    let refresh = output.current_mode().map(|mode| mode.refresh).unwrap_or(60_000);
    let refresh = u64::try_from(refresh.max(1)).unwrap_or(60_000);
    Duration::from_nanos(1_000_000_000u64 / refresh)
}

pub fn init_winit(
    event_loop: &mut EventLoop<'static, Lantern>,
    state: &mut Lantern,
) -> Result<(), Box<dyn std::error::Error>> {
    let (backend, winit) = winit::init()?;
    let backend = std::rc::Rc::new(std::cell::RefCell::new(backend));

    let mode = Mode {
        size: backend.borrow().window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "lantern-0".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Lantern".into(),
            model: "Virtual".into(),
        },
    );

    let _global = output.create_global::<Lantern>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(Scale::Fractional(LANTERN_OUTPUT_SCALE)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    state.canvas.set_screen_size(
        mode.size.w as f64 / LANTERN_OUTPUT_SCALE,
        mode.size.h as f64 / LANTERN_OUTPUT_SCALE,
    );

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let redraw_interval = frame_callback_interval(&output);

    {
        let backend = backend.clone();
        event_loop.handle().insert_source(
            Timer::from_duration(redraw_interval),
            move |_, _, state| {
                if state.take_winit_redraw_request() {
                    state.record_winit_redraw_request();
                    backend.borrow_mut().window().request_redraw();
                }
                TimeoutAction::ToDuration(redraw_interval)
            },
        )?;
    }

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    let backend = backend.borrow_mut();
                    output.change_current_state(
                        Some(Mode {
                            size,
                            refresh: 60_000,
                        }),
                        None,
                        Some(Scale::Fractional(LANTERN_OUTPUT_SCALE)),
                        None,
                    );
                    backend.window().request_redraw();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    let mut backend = backend.borrow_mut();
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);

                    {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        let output_geo = state
                            .space
                            .output_geometry(&output)
                            .unwrap_or_else(|| Rectangle::from_size((size.w, size.h).into()));
                        let scale = output.current_scale().fractional_scale();
                        let mut elements: Vec<WinitRenderElements> = Vec::new();

                        let cursor_phys_pos: Point<f64, Physical> = state
                            .seat
                            .get_pointer()
                            .map(|pointer| {
                                let position = pointer.current_location();
                                ((position.x * scale), (position.y * scale)).into()
                            })
                            .unwrap_or_default();

                        if let Some(cursor) = state.cursor.render_element(
                            renderer,
                            cursor_phys_pos,
                        ) {
                            elements.push(WinitRenderElements::Wallpaper(cursor));
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
                                (cursor_phys_pos.x - hotspot.x as f64 * scale) as i32,
                                (cursor_phys_pos.y - hotspot.y as f64 * scale) as i32,
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
                            elements.extend(cursor_surface_elements.into_iter().map(WinitRenderElements::Surface));
                        }

                        let switcher_visible = state.alt_tab_switcher.is_visible();

                        // Alt+Tab overlay chrome + thumbnails
                        if switcher_visible {
                            state.alt_tab_switcher.update_sizes(output_geo.size);
                            let slots = state.alt_tab_switcher.thumbnail_slots(output_geo.size);

                            let (base, top) = state
                                .alt_tab_switcher
                                .render_overlay_split(output_geo.size, scale);
                            elements.extend(top.into_iter().map(WinitRenderElements::Overlay));
                            // base chrome added after thumbnails below
                            let base_chrome: Vec<_> = base.into_iter().map(WinitRenderElements::Overlay).collect();

                            for slot in &slots {
                                if let Some(window) = state.find_mapped_window(&slot.surface) {
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

                                    let thumb_phys_loc: Point<i32, Physical> = (
                                        ((slot.position.x + offset_x) as f64 * scale).round() as i32,
                                        ((slot.position.y + offset_y) as f64 * scale).round() as i32,
                                    ).into();
                                    let thumb_render_scale = smithay::utils::Scale::from(scale * thumb_scale);

                                    let thumb_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                                        window.render_elements(renderer, thumb_phys_loc, thumb_render_scale, 1.0);
                                    elements.extend(
                                        thumb_elements.into_iter().map(WinitRenderElements::Surface),
                                    );
                                }
                            }
                            elements.extend(base_chrome);
                        }

                        // Render windows manually with per-window alpha
                        let windows: Vec<_> = state.space.elements().cloned().collect();
                        for window in windows.iter().rev() {
                            let loc = state.space.element_location(&window).unwrap_or_default();
                            let mut win_bbox = window.bbox();
                            win_bbox.loc += loc - window.geometry().loc;
                            if !output_geo.overlaps(win_bbox) {
                                continue;
                            }
                            let render_location = loc - window.geometry().loc;
                            let phys_loc: Point<i32, Physical> = (
                                ((render_location.x - output_geo.loc.x) as f64 * scale).round() as i32,
                                ((render_location.y - output_geo.loc.y) as f64 * scale).round() as i32,
                            ).into();
                            let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(window) else { continue };
                            let alpha = {
                                let a = state.get_window_opacity(&surface);
                                if switcher_visible { a * 0.3 } else { a }
                            };
                            let win_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                                window.render_elements(
                                    renderer,
                                    phys_loc,
                                    smithay::utils::Scale::from(scale),
                                    alpha,
                                );
                            elements.extend(
                                win_elements
                                    .into_iter()
                                    .map(WinitRenderElements::Surface),
                            );
                        }

                        state.wallpaper_frame_counter += 1;
                        if state.wallpaper_frame_counter >= 300 {
                            state.wallpaper_frame_counter = 0;
                            state.wallpaper.reload_if_changed();
                        }
                        if let Some(wallpaper) = state.wallpaper.render_element(renderer, output_geo.size, scale) {
                            elements.push(WinitRenderElements::Wallpaper(wallpaper));
                        }

                        damage_tracker
                            .render_output(
                            renderer,
                            &mut framebuffer,
                            0,
                            &elements,
                            BG_COLOR,
                        )
                        .unwrap();
                    }
                    backend.submit(Some(&[damage])).unwrap();

                    let mut frame_callback_count = 0;
                    if state.pending_client_frame_callbacks {
                        frame_callback_count = state.space.elements().count();
                        state.space.elements().for_each(|window| {
                            window.send_frame(
                                &output,
                                state.start_time.elapsed(),
                                Some(frame_callback_interval(&output)),
                                |_, _| Some(output.clone()),
                            )
                        });
                        state.pending_client_frame_callbacks = false;
                    }

                    // Handle dead windows: animate client-initiated closes
                    let dead_windows: Vec<_> = state.space.elements()
                        .filter(|w| !w.alive())
                        .filter_map(|w| {
                            let surface = crate::window_ext::WindowExt::get_wl_surface(w)?;
                            let location = state.space.element_location(w)?;
                            let size = w.geometry().size;
                            let had_ssd = state.ssd.has_ssd(&surface);
                            Some(crate::animation::ClosingWindow { surface, location, size, had_ssd })
                        })
                        .collect();
                    for cw in dead_windows {
                        if state.animations.take_close_done(&cw.surface) {
                            state.forget_window(&cw.surface);
                        } else {
                            let surface = cw.surface.clone();
                            state.animations.start_close(&surface);
                            state.closing_windows.push(cw);
                            state.schedule_render();
                        }
                    }
                    state.space.refresh();
                    state.popups.cleanup();
                    state.check_exclusive_zone_change();

                    let _ = state.display_handle.flush_clients();
                    state.record_render(frame_callback_count);
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            };
        })?;

    state.request_winit_redraw();

    Ok(())
}
