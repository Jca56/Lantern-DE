use crate::{grabs::resize_grab, state::ClientState, window_ext::WindowExt, Lantern};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_surface::WlSurface},
        Client,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, with_states, CompositorClientState,
            CompositorHandler, CompositorState,
        },
        shell::xdg::XdgToplevelSurfaceData,
        shm::{ShmHandler, ShmState},
    },
    xwayland::XWaylandClientData,
};

use super::xdg_shell;

impl CompositorHandler for Lantern {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<ClientState>() {
            return &state.compositor_state;
        }
        if let Some(state) = client.get_data::<XWaylandClientData>() {
            return &state.compositor_state;
        }
        panic!("Unknown client data type");
    }

    fn commit(&mut self, surface: &WlSurface) {
        let commit_start = if self.debug_counters.enabled {
            self.debug_counters.commits += 1;
            Some(std::time::Instant::now())
        } else { None };
        on_commit_buffer_handler::<Self>(surface);
        // One mapped-window lookup for the whole handler. This used to be
        // five separate linear space scans per commit — a 240fps client
        // commits 240 times a second, so every scan here is hot.
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let surface_is_root = root == *surface;
        let mut mapped_window: Option<smithay::desktop::Window> = None;
        if !is_sync_subsurface(surface) {
            let root_window = self
                .space
                .elements()
                .find(|w| w.get_wl_surface().as_ref() == Some(&root))
                .cloned();
            if let Some(window) = &root_window {
                window.on_commit();
            }
            if surface_is_root {
                mapped_window = root_window;
            }
        };

        if let Some(window) = mapped_window.clone() {
            self.apply_initial_window_size(&window, surface);
        }
        xdg_shell::handle_commit(&mut self.popups, mapped_window.as_ref(), surface);
        resize_grab::handle_commit(self, surface);

        // Smooth-resize handoff: if a held visual is still in effect and
        // this commit lands at the matching size, drop the hold so the
        // renderer stops stretching the buffer. Looking up the window's
        // geometry here is post-commit, so `geometry().size` already
        // reflects whatever the client just acked.
        if let Some(window) = &mapped_window {
            let committed_size = window.geometry().size;
            self.window_state_anim
                .clear_held_scale_if_matched(surface, committed_size);
            // Floating clients own their size — adopt client-chosen sizes
            // so later configures don't snap the window back to a stale
            // suggestion (details on `adopt_client_size`).
            self.adopt_client_size(window, surface);
        }

        // Center windows that are waiting for their first real geometry
        self.center_pending_window(surface);

        // Propagate title/app_id changes to foreign-toplevel clients
        if mapped_window.is_some() {
            with_states(surface, |states| {
                if let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() {
                    let attrs = data.lock().unwrap();
                    if let Some(ref title) = attrs.title {
                        self.foreign_toplevel_state.set_title(surface, title);
                    }
                    if let Some(ref app_id) = attrs.app_id {
                        self.foreign_toplevel_state.set_app_id(surface, app_id);
                    }
                }
            });
        }

        // Handle layer surface commits: compute size from anchor + output geometry
        let mut layer_kb_action = None;
        for ls in &self.layer_surfaces {
            if ls.wl_surface() == surface {
                use smithay::wayland::compositor::with_states;
                use smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState;

                let output_geo = self
                    .layer_surface_outputs
                    .get(surface)
                    .or_else(|| self.workspaces.outputs_iter().next())
                    .and_then(|o| self.workspaces.output_geometry(o));

                if let Some(geo) = output_geo {
                    let cached = with_states(surface, |states| {
                        *states
                            .cached_state
                            .get::<LayerSurfaceCachedState>()
                            .current()
                    });

                    let mut width = cached.size.w;
                    let mut height = cached.size.h;

                    // Compute exclusive zone reductions from other layer surfaces
                    use smithay::wayland::shell::wlr_layer::{Anchor as A, ExclusiveZone};
                    let mut excl_top = 0i32;
                    let mut excl_bottom = 0i32;
                    let mut excl_left = 0i32;
                    let mut excl_right = 0i32;
                    let is_neutral = matches!(cached.exclusive_zone, ExclusiveZone::Neutral);
                    if is_neutral {
                        for other in &self.layer_surfaces {
                            if other.wl_surface() == surface { continue; }
                            let oc = with_states(other.wl_surface(), |s| {
                                *s.cached_state.get::<LayerSurfaceCachedState>().current()
                            });
                            let ex = match oc.exclusive_zone {
                                ExclusiveZone::Exclusive(v) => v as i32,
                                _ => continue,
                            };
                            if oc.anchor.contains(A::BOTTOM) && !oc.anchor.contains(A::TOP) {
                                excl_bottom += ex;
                            } else if oc.anchor.contains(A::TOP) && !oc.anchor.contains(A::BOTTOM) {
                                excl_top += ex;
                            } else if oc.anchor.contains(A::LEFT) && !oc.anchor.contains(A::RIGHT) {
                                excl_left += ex;
                            } else if oc.anchor.contains(A::RIGHT) && !oc.anchor.contains(A::LEFT) {
                                excl_right += ex;
                            }
                        }
                    }

                    if cached.anchor.anchored_horizontally() && width == 0 {
                        width = geo.size.w - cached.margin.left - cached.margin.right - excl_left - excl_right;
                    }
                    if cached.anchor.anchored_vertically() && height == 0 {
                        height = geo.size.h - cached.margin.top - cached.margin.bottom - excl_top - excl_bottom;
                    }

                    tracing::trace!(
                        width, height,
                        anchor = ?cached.anchor,
                        output_w = geo.size.w,
                        "Layer surface configure"
                    );

                    ls.with_pending_state(|state| {
                        state.size = Some(smithay::utils::Size::from((width, height)));
                    });
                }

                // Check keyboard interactivity (acted on after the borrow ends)
                let kb_state = with_states(surface, |states| {
                    states.cached_state.get::<LayerSurfaceCachedState>().current().keyboard_interactivity
                });
                layer_kb_action = Some((ls.wl_surface().clone(), kb_state));

                ls.send_pending_configure();
                break;
            }
        }

        // Apply keyboard focus for layer surfaces (after borrow of layer_surfaces ends).
        // Skip entirely while locked — the lock surface must keep keyboard focus,
        // or a layer surface (e.g. Command Center) re-grabs it and the password
        // field never receives keystrokes.
        if let Some((wl_surface, kb_interactivity)) = layer_kb_action.filter(|_| !self.is_locked()) {
            use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
            if kb_interactivity == KeyboardInteractivity::Exclusive {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.set_focus(
                    self,
                    Some(crate::keyboard_focus::KeyboardFocusTarget::Wayland(wl_surface)),
                    serial,
                );
            } else if kb_interactivity == KeyboardInteractivity::None {
                let keyboard = self.seat.get_keyboard().unwrap();
                let has_focus = keyboard.current_focus().map_or(false, |f| {
                    use smithay::wayland::seat::WaylandFocus;
                    f.wl_surface().map(|c| c.into_owned()).as_ref() == Some(&wl_surface)
                });
                if has_focus {
                    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                    // A keyboard-grabbing layer surface (the Command Center) just
                    // released its grab. Instead of leaving the keyboard focused
                    // on nothing, hand focus back to the top-most window so the
                    // user can keep typing/working immediately on close.
                    if let Some(window) = self.focused_window() {
                        self.focus_window(&window, serial);
                    } else {
                        keyboard.set_focus(
                            self,
                            Option::<crate::keyboard_focus::KeyboardFocusTarget>::None,
                            serial,
                        );
                    }
                }
            }
        }

        self.schedule_client_render_for_surface(&root);
        if let Some(t) = commit_start {
            self.debug_counters.commit_micros += t.elapsed().as_micros() as u64;
        }
    }
}

impl BufferHandler for Lantern {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Lantern {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(Lantern);
delegate_shm!(Lantern);
