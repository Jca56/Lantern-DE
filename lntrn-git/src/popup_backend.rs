use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface};

use super::wayland::State;

// ── Dispatch impls for popup protocol objects ─────────────────────────────

impl Dispatch<xdg_positioner::XdgPositioner, ()> for State {
    fn event(
        _: &mut Self, _: &xdg_positioner::XdgPositioner,
        _: xdg_positioner::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<xdg_surface::XdgSurface, u32> for State {
    fn event(
        state: &mut Self, xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event, popup_id: &u32, _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if let Some(backend) = &mut state.popup_backend {
                backend.mark_configured(*popup_id);
            }
            state.frame_done = true;
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, u32> for State {
    fn event(
        state: &mut Self, _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event, popup_id: &u32, _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { width, height, .. } => {
                if let Some(backend) = &mut state.popup_backend {
                    backend.configure_size(*popup_id, width as u32, height as u32);
                }
            }
            xdg_popup::Event::PopupDone => {
                state.popup_closed = true;
            }
            _ => {}
        }
    }
}
