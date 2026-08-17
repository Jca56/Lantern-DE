mod compositor;
pub mod foreign_toplevel;
mod layer_shell;
pub mod output_management;
pub mod screencopy;
pub mod session_lock;
pub mod xdg_foreign;
mod xdg_shell;
pub mod xwayland;

use crate::Lantern;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::compositor::with_states;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::fractional_scale::{self, FractionalScaleHandler};
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::pointer_constraints::{PointerConstraintsHandler, with_pointer_constraint};
use smithay::input::pointer::PointerHandle;
use smithay::utils::{Logical, Point};
use smithay::wayland::selection::data_device::{
    set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
    ServerDndGrabHandler,
};
use smithay::wayland::selection::ext_data_control::{
    DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
};
use smithay::wayland::selection::wlr_data_control::{
    DataControlHandler as WlrDataControlHandler, DataControlState as WlrDataControlState,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use std::os::unix::io::OwnedFd;
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::{
    delegate_cursor_shape, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_ext_data_control, delegate_fractional_scale, delegate_idle_inhibit,
    delegate_layer_shell, delegate_output, delegate_pointer_constraints,
    delegate_pointer_gestures, delegate_presentation, delegate_relative_pointer, delegate_seat,
    delegate_text_input_manager, delegate_viewporter, delegate_xdg_activation,
    delegate_xdg_decoration,
};

fn lantern_output_scale() -> f64 { crate::output_scale() }

impl SeatHandler for Lantern {
    type KeyboardFocus = crate::keyboard_focus::KeyboardFocusTarget;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Lantern> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: smithay::input::pointer::CursorImageStatus) {
        self.cursor.set_status(image);
        self.schedule_render();
    }

    fn focus_changed(
        &mut self,
        seat: &Seat<Self>,
        focused: Option<&crate::keyboard_focus::KeyboardFocusTarget>,
    ) {
        use smithay::wayland::seat::WaylandFocus;
        let dh = &self.display_handle;
        let wl_surface = focused.and_then(|f| f.wl_surface().map(|c| c.into_owned()));
        let client = wl_surface
            .as_ref()
            .and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
        // Setting focus runs smithay's internal liveness purge; if the
        // selection just got cleared because its source died, take over.
        crate::clipboard_manager::recheck_clipboard(self);
    }
}

delegate_seat!(Lantern);

impl SelectionHandler for Lantern {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if !matches!(ty, SelectionTarget::Clipboard) {
            return;
        }
        match source {
            Some(src) => crate::clipboard_manager::capture_selection(self, &src),
            None => crate::clipboard_manager::on_selection_cleared(self),
        }
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        crate::clipboard_manager::on_send_request(self, ty, mime_type, fd);
    }
}

impl DataDeviceHandler for Lantern {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Lantern {}
impl ServerDndGrabHandler for Lantern {}

delegate_data_device!(Lantern);

impl WlrDataControlHandler for Lantern {
    fn data_control_state(&self) -> &WlrDataControlState {
        &self.data_control_state
    }
}

delegate_data_control!(Lantern);

impl ExtDataControlHandler for Lantern {
    fn data_control_state(&self) -> &ExtDataControlState {
        &self.ext_data_control_state
    }
}

delegate_ext_data_control!(Lantern);

impl OutputHandler for Lantern {}
delegate_output!(Lantern);

impl FractionalScaleHandler for Lantern {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self.workspaces.outputs_iter().next()
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(lantern_output_scale());
        with_states(&surface, |states| {
            fractional_scale::with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(scale);
            });
        });
    }
}

delegate_fractional_scale!(Lantern);

/// Push the current per-output fractional scale to every mapped surface.
/// Smithay only sends preferred_scale when the client creates its
/// wp_fractional_scale object, so live scale changes (Settings slider,
/// Gaming Mode) need this sweep. `set_preferred_scale` dedupes and no-ops
/// for surfaces whose client never bound the protocol, so it's safe to
/// call on everything.
pub fn refresh_fractional_scales(state: &Lantern) {
    use crate::window_ext::WindowExt;
    let fallback = state.workspaces.outputs_iter().next()
        .map(|o| o.current_scale().fractional_scale())
        .unwrap_or(lantern_output_scale());
    // The global Space holds every mapped window (all workspaces, active or
    // not, plus scratchpad and override-redirect); minimized windows live
    // nowhere, so sweep them separately.
    let minimized = state.minimized_windows.iter().map(|e| &e.window);
    for window in state.space.elements().chain(minimized) {
        let Some(surface) = window.get_wl_surface() else { continue };
        let scale = state.output_for_window(window)
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(fallback);
        with_states(&surface, |states| {
            fractional_scale::with_fractional_scale(states, |f| f.set_preferred_scale(scale));
        });
    }
    for output in state.workspaces.outputs_iter() {
        let scale = output.current_scale().fractional_scale();
        let map = smithay::desktop::layer_map_for_output(output);
        for layer in map.layers() {
            with_states(layer.wl_surface(), |states| {
                fractional_scale::with_fractional_scale(states, |f| f.set_preferred_scale(scale));
            });
        }
    }
}
delegate_viewporter!(Lantern);

impl TabletSeatHandler for Lantern {}
delegate_cursor_shape!(Lantern);
delegate_layer_shell!(Lantern);

/// Send a configure for a decoration-mode change — but never before the
/// client's first commit. smithay marks any pre-commit send as THE initial
/// configure, which fires before `apply_initial_window_size` can inject the
/// `[windows] default_size_pct` default. Every winit app binds
/// xdg-decoration during window setup (pre-commit), so an unguarded send
/// here made all of them miss the configured default size. Pre-commit, the
/// pending decoration mode simply rides along with the real initial
/// configure sent on first commit.
fn send_decoration_configure(toplevel: &ToplevelSurface) {
    if toplevel.is_initial_configure_sent() {
        toplevel.send_configure();
    }
}

impl XdgDecorationHandler for Lantern {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        // Client supports xdg-decoration — tell it we prefer SSD and add our decoration.
        let surface = toplevel.wl_surface().clone();
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        self.ssd.add(surface);
        send_decoration_configure(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        let surface = toplevel.wl_surface().clone();
        if mode == DecorationMode::ClientSide {
            // Client wants CSD — respect that, remove our SSD
            toplevel.with_pending_state(|state| {
                state.decoration_mode = Some(DecorationMode::ClientSide);
            });
            self.ssd.remove(&surface);
        } else {
            toplevel.with_pending_state(|state| {
                state.decoration_mode = Some(DecorationMode::ServerSide);
            });
            // Already added in new_toplevel, but ensure it's there
            self.ssd.add(surface);
        }
        send_decoration_configure(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        // Default to SSD
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        send_decoration_configure(&toplevel);
    }
}

delegate_xdg_decoration!(Lantern);

// --- xdg-activation: lets apps request focus (e.g. terminal -> browser) ---

impl XdgActivationHandler for Lantern {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        // Only honor tokens less than 10 seconds old
        if token_data.timestamp.elapsed().as_secs() >= 10 {
            return;
        }

        let target = self
            .space
            .elements()
            .find(|w| {
                crate::window_ext::WindowExt::get_wl_surface(*w).as_ref() == Some(&surface)
            })
            .cloned();
        if let Some(window) = target {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            self.focus_window(&window, serial);
        }
    }
}

delegate_xdg_activation!(Lantern);

// --- wp_idle_inhibit: prevents screen lock during video playback ---

impl IdleInhibitHandler for Lantern {
    fn inhibit(&mut self, surface: WlSurface) {
        tracing::info!("Idle inhibit requested for surface {:?}", surface.id());
        // No idle/screensaver system yet -- when one is added, track inhibiting surfaces here.
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        tracing::info!("Idle inhibit released for surface {:?}", surface.id());
    }
}

delegate_idle_inhibit!(Lantern);

// --- linux-dmabuf: zero-copy GPU buffer sharing ---

impl DmabufHandler for Lantern {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if let Some(udev) = self.udev.as_mut() {
            if let Some(renderer) = udev.renderer.as_mut() {
                use smithay::backend::renderer::ImportDma;
                if renderer.import_dmabuf(&dmabuf, None).is_ok() {
                    let _ = notifier.successful::<Lantern>();
                    return;
                }
            }
        }
        notifier.failed();
    }
}

delegate_dmabuf!(Lantern);

// --- pointer gestures: touchpad swipe/pinch/hold forwarding to clients ---

delegate_pointer_gestures!(Lantern);

// --- pointer-constraints-v1: games lock/confine pointer for FPS-style input ---

impl PointerConstraintsHandler for Lantern {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // Activate the constraint immediately if the surface has pointer focus.
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: Point<f64, Logical>,
    ) {
        // Hint is informational — our cursor stays hidden while locked, so nothing to do.
    }
}

delegate_pointer_constraints!(Lantern);

// --- relative-pointer-v1: raw mouse deltas for FPS/strategy games ---

delegate_relative_pointer!(Lantern);

// --- text-input-v3: required for Unity/Proton games that open text fields ---

delegate_text_input_manager!(Lantern);

// --- presentation-time: frame timing feedback for Unity/VRR/FreeSync ---

delegate_presentation!(Lantern);
