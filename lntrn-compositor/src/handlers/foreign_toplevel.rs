//! wlr-foreign-toplevel-management protocol implementation.
//!
//! Allows external taskbars/docks to list open windows and request
//! actions (activate, maximize, minimize, close, fullscreen).

use std::collections::HashMap;

use smithay::{
    reexports::wayland_server::{
        backend::{ClientId, GlobalId},
        protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
    },
    utils::Serial,
};
use wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

use crate::Lantern;

// ── Per-toplevel state ──────────────────────────────────────────────

/// Compositor-side state for a single foreign-toplevel handle.
pub struct ForeignToplevelEntry {
    pub surface: WlSurface,
    /// One protocol object per bound manager instance.
    instances: Vec<Weak<ZwlrForeignToplevelHandleV1>>,
    title: String,
    app_id: String,
    states: Vec<u32>,
    closed: bool,
}

impl ForeignToplevelEntry {
    fn send_state_to(&self, handle: &ZwlrForeignToplevelHandleV1) {
        handle.title(self.title.clone());
        handle.app_id(self.app_id.clone());
        let bytes: Vec<u8> = self
            .states
            .iter()
            .flat_map(|s| s.to_ne_bytes())
            .collect();
        handle.state(bytes);
        handle.done();
    }

    fn broadcast_done(&self) {
        let bytes: Vec<u8> = self
            .states
            .iter()
            .flat_map(|s| s.to_ne_bytes())
            .collect();
        for weak in &self.instances {
            if let Ok(h) = weak.upgrade() {
                h.title(self.title.clone());
                h.app_id(self.app_id.clone());
                h.state(bytes.clone());
                h.done();
            }
        }
    }

    fn broadcast_closed(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        for weak in self.instances.drain(..) {
            if let Ok(h) = weak.upgrade() {
                h.closed();
            }
        }
    }
}

// ── Global state ────────────────────────────────────────────────────

pub struct ForeignToplevelManagerState {
    global: GlobalId,
    /// Bound manager instances (one per client that binds the global).
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    /// Active toplevel entries keyed by their wl_surface id.
    toplevels: Vec<ForeignToplevelEntry>,
    dh: DisplayHandle,
}

impl ForeignToplevelManagerState {
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<Lantern, ZwlrForeignToplevelManagerV1, _>(3, ());
        Self {
            global,
            managers: Vec::new(),
            toplevels: Vec::new(),
            dh: dh.clone(),
        }
    }

    /// Announce a new toplevel to all bound clients.
    pub fn new_toplevel(&mut self, surface: &WlSurface, title: &str, app_id: &str) {
        // Avoid duplicates
        if self
            .toplevels
            .iter()
            .any(|e| e.surface == *surface && !e.closed)
        {
            return;
        }

        let mut instances = Vec::new();
        for mgr in &self.managers {
            let Ok(client) = self.dh.get_client(mgr.id()) else {
                continue;
            };
            let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, Lantern>(
                &self.dh,
                mgr.version(),
                surface.clone(),
            ) else {
                continue;
            };
            mgr.toplevel(&handle);
            handle.title(title.to_string());
            handle.app_id(app_id.to_string());
            handle.state(Vec::new());
            handle.done();
            instances.push(handle.downgrade());
        }

        self.toplevels.push(ForeignToplevelEntry {
            surface: surface.clone(),
            instances,
            title: title.to_string(),
            app_id: app_id.to_string(),
            states: Vec::new(),
            closed: false,
        });
    }

    /// Remove a toplevel and send closed events.
    pub fn toplevel_closed(&mut self, surface: &WlSurface) {
        if let Some(entry) = self.toplevels.iter_mut().find(|e| e.surface == *surface) {
            entry.broadcast_closed();
        }
        self.toplevels.retain(|e| e.surface != *surface);
    }

    /// Update title for a toplevel.
    pub fn set_title(&mut self, surface: &WlSurface, title: &str) {
        if let Some(entry) = self
            .toplevels
            .iter_mut()
            .find(|e| e.surface == *surface && !e.closed)
        {
            if entry.title != title {
                entry.title = title.to_string();
                entry.broadcast_done();
            }
        }
    }

    /// Update app_id for a toplevel.
    pub fn set_app_id(&mut self, surface: &WlSurface, app_id: &str) {
        if let Some(entry) = self
            .toplevels
            .iter_mut()
            .find(|e| e.surface == *surface && !e.closed)
        {
            if entry.app_id != app_id {
                entry.app_id = app_id.to_string();
                entry.broadcast_done();
            }
        }
    }

    /// Return a list of (surface, app_id) pairs for all active toplevels.
    pub fn surface_app_ids(&self) -> Vec<(WlSurface, String)> {
        self.toplevels
            .iter()
            .filter(|e| !e.closed)
            .map(|e| (e.surface.clone(), e.app_id.clone()))
            .collect()
    }

    /// Update the state flags for a toplevel and broadcast.
    pub fn set_states(&mut self, surface: &WlSurface, new_states: Vec<u32>) {
        if let Some(entry) = self
            .toplevels
            .iter_mut()
            .find(|e| e.surface == *surface && !e.closed)
        {
            if entry.states != new_states {
                entry.states = new_states;
                entry.broadcast_done();
            }
        }
    }
}

// ── Dispatch: Manager global ────────────────────────────────────────

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, (), Lantern> for Lantern {
    fn bind(
        state: &mut Lantern,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        let mgr = data_init.init(resource, ());

        // Announce all existing toplevels to the new client.
        let ftm = &mut state.foreign_toplevel_state;
        for entry in &mut ftm.toplevels {
            if entry.closed {
                continue;
            }
            let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, Lantern>(
                dh,
                mgr.version(),
                entry.surface.clone(),
            ) else {
                continue;
            };
            mgr.toplevel(&handle);
            entry.send_state_to(&handle);
            entry.instances.push(handle.downgrade());
        }

        ftm.managers.push(mgr);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, (), Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zwlr_foreign_toplevel_manager_v1::Request::Stop => {
                // Client no longer wants events. Remove the manager instance.
                state
                    .foreign_toplevel_state
                    .managers
                    .retain(|m| m != manager);
                manager.finished();
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Lantern,
        _client: ClientId,
        resource: &ZwlrForeignToplevelManagerV1,
        _data: &(),
    ) {
        state
            .foreign_toplevel_state
            .managers
            .retain(|m| m != resource);
    }
}

// ── Dispatch: Toplevel handle ───────────────────────────────────────

impl Dispatch<ZwlrForeignToplevelHandleV1, WlSurface, Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        surface: &WlSurface,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();

        match request {
            zwlr_foreign_toplevel_handle_v1::Request::Activate { seat: _ } => {
                tracing::info!("Foreign toplevel: activate requested");
                if let Some(window) = state.find_mapped_window(surface) {
                    state.focus_window(&window, serial);
                } else if let Some(window) = state.restore_minimized_by_surface(surface) {
                    state.focus_window(&window, serial);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetMaximized => {
                tracing::info!("Foreign toplevel: set_maximized requested");
                state.maximize_request_surface(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetMaximized => {
                tracing::info!("Foreign toplevel: unset_maximized requested");
                state.unmaximize_request_surface(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetMinimized => {
                tracing::info!("Foreign toplevel: set_minimized requested");
                state.minimize_request_surface(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetMinimized => {
                tracing::info!("Foreign toplevel: unset_minimized requested");
                if let Some(window) = state.restore_minimized_by_surface(surface) {
                    state.focus_window(&window, serial);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => {
                tracing::info!("Foreign toplevel: close requested");
                if let Some(window) = state.find_mapped_window(surface) {
                    crate::window_ext::WindowExt::request_close(&window);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetFullscreen { output: _ } => {
                // No fullscreen support yet, treat as maximize.
                tracing::info!("Foreign toplevel: set_fullscreen requested (treating as maximize)");
                state.maximize_request_surface(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetFullscreen => {
                tracing::info!("Foreign toplevel: unset_fullscreen requested (treating as unmaximize)");
                state.unmaximize_request_surface(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetRectangle { .. } => {
                // Hint for animation target -- we don't use it yet.
            }
            zwlr_foreign_toplevel_handle_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Lantern,
        _client: ClientId,
        resource: &ZwlrForeignToplevelHandleV1,
        surface: &WlSurface,
    ) {
        // Remove this handle instance from the entry.
        if let Some(entry) = state
            .foreign_toplevel_state
            .toplevels
            .iter_mut()
            .find(|e| e.surface == *surface)
        {
            entry
                .instances
                .retain(|w| w.upgrade().map(|h| h != *resource).unwrap_or(false));
        }
    }
}
