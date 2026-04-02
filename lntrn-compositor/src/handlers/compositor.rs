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
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self
                .space
                .elements()
                .find(|w| w.get_wl_surface().as_ref() == Some(&root))
            {
                window.on_commit();
            }
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);

        // Propagate title/app_id changes to foreign-toplevel clients
        if self.space.elements().any(|w| w.get_wl_surface().as_ref() == Some(surface)) {
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
                    .space
                    .outputs()
                    .next()
                    .and_then(|o| self.space.output_geometry(o));

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

                    tracing::info!(
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

        // Apply keyboard focus for layer surfaces (after borrow of layer_surfaces ends)
        if let Some((wl_surface, kb_interactivity)) = layer_kb_action {
            use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
            if kb_interactivity == KeyboardInteractivity::Exclusive {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.set_focus(self, Some(wl_surface), serial);
            } else if kb_interactivity == KeyboardInteractivity::None {
                let keyboard = self.seat.get_keyboard().unwrap();
                let has_focus = keyboard.current_focus()
                    .map_or(false, |f| f == wl_surface);
                if has_focus {
                    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                    keyboard.set_focus(self, Option::<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>::None, serial);
                }
            }
        }

        self.schedule_client_render();
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
