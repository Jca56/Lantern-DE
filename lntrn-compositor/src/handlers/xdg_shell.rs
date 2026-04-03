use smithay::{
    delegate_xdg_shell,
    desktop::{find_popup_root_surface, get_popup_toplevel_coords, PopupKind, PopupManager, Space, Window},
    input::{
        pointer::{Focus, GrabStartData as PointerGrabStartData},
        Seat,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    window_ext::WindowExt,
    Lantern,
};

impl XdgShellHandler for Lantern {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Don't add SSD here — only add it when the client engages
        // xdg-decoration and we negotiate SSD mode. Clients that don't
        // support the protocol (like Firefox) draw their own CSD.
        let window = Window::new_wayland_window(surface);
        self.map_new_window(window);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();

            let window = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();

            let was_snapped = self.is_snapped(wl_surface);
            let was_maximized = self.is_maximized(wl_surface);
            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
                was_snapped,
                was_maximized,
                restored_this_drag: false,
                has_moved: false,
            };

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        tracing::info!("Client requested resize, edges: {:?}", edges);
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            tracing::info!("Resize grab started successfully");
            let pointer = seat.get_pointer().unwrap();

            let window = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });
            surface.send_pending_configure();

            let our_edges: crate::grabs::resize_grab::ResizeEdge = edges.into();
            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                our_edges,
                Rectangle::new(initial_window_location, initial_window_size),
            );

            pointer.set_grab(self, grab, serial, Focus::Clear);
            let icon = ResizeSurfaceGrab::cursor_icon_for_edges(our_edges);
            self.cursor.set_status(smithay::input::pointer::CursorImageStatus::Named(icon));
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: popup grabs
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("Client requested maximize");
        let result = self.maximize_request_surface(surface.wl_surface());
        tracing::info!("Maximize result: {}", result);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("Client requested unmaximize");
        let result = self.unmaximize_request_surface(surface.wl_surface());
        tracing::info!("Unmaximize result: {}", result);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("Client requested minimize");
        let result = self.minimize_request_surface(surface.wl_surface());
        tracing::info!("Minimize result: {}", result);
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>) {
        tracing::info!("Client requested fullscreen");
        self.fullscreen_request_surface(surface.wl_surface());
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("Client requested unfullscreen");
        self.unfullscreen_request_surface(surface.wl_surface());
    }
}

delegate_xdg_shell!(Lantern);

fn check_grab(
    seat: &Seat<Lantern>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<Lantern>> {
    let pointer = seat.get_pointer()?;

    if !pointer.has_grab(serial) {
        tracing::warn!("check_grab: no grab for serial {:?}", serial);
        return None;
    }

    let start_data = pointer.grab_start_data()?;
    let (focus, _) = match start_data.focus.as_ref() {
        Some(f) => f,
        None => {
            tracing::warn!("check_grab: grab has no focus");
            return None;
        }
    };
    if !focus.id().same_client_as(&surface.id()) {
        tracing::warn!("check_grab: focus client mismatch");
        return None;
    }

    Some(start_data)
}

pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    if let Some(window) = space
        .elements()
        .find(|w| w.get_wl_surface().as_ref() == Some(surface))
        .cloned()
    {
        // Only send initial configure for Wayland (XDG) windows
        if let Some(toplevel) = window.toplevel() {
            let initial_configure_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .initial_configure_sent
            });

            if !initial_configure_sent {
                toplevel.send_configure();
            }
        }
    }

    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl Lantern {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.get_wl_surface().as_ref() == Some(&root))
        else {
            return;
        };

        let output = self.space.outputs().next().unwrap();
        let output_geo = self.space.output_geometry(output).unwrap();
        let window_geo = self.space.element_geometry(window).unwrap();

        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
