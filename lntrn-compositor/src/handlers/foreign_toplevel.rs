//! wlr-foreign-toplevel-management protocol implementation.
//!
//! Allows external taskbars/docks to list open windows and request
//! actions (activate, maximize, minimize, close, fullscreen).
//!
//! It is also the compositor's one list of open windows: each gets an id
//! of its own here, stable for its life, which the workspace IPC's window
//! feed (`window_management/dock_link.rs`) names it by.

use smithay::reexports::wayland_server::{
    backend::{ClientId, GlobalId},
    protocol::wl_surface::WlSurface,
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
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
    /// Never reused while the compositor runs.
    pub id: u64,
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
        let bytes: Vec<u8> = self.states.iter().flat_map(|s| s.to_ne_bytes()).collect();
        handle.state(bytes);
        handle.done();
    }

    fn broadcast_done(&self) {
        let bytes: Vec<u8> = self.states.iter().flat_map(|s| s.to_ne_bytes()).collect();
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
    #[allow(dead_code)] // must stay alive to keep the Wayland global registered
    global: GlobalId,
    /// Bound manager instances (one per client that binds the global).
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    /// Active toplevel entries keyed by their wl_surface id.
    toplevels: Vec<ForeignToplevelEntry>,
    /// The id the next toplevel gets.
    next_id: u64,
    /// Something in the list changed since `take_dirty`.
    dirty: bool,
    dh: DisplayHandle,
}

impl ForeignToplevelManagerState {
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<Lantern, ZwlrForeignToplevelManagerV1, _>(3, ());
        Self {
            global,
            managers: Vec::new(),
            toplevels: Vec::new(),
            next_id: 1,
            dirty: false,
            dh: dh.clone(),
        }
    }

    /// Whether the list changed since this was last asked, so the window
    /// feed has something to send.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Every open toplevel, oldest first: (id, app_id, title, states).
    pub fn windows(&self) -> impl Iterator<Item = (u64, &str, &str, &[u32])> {
        self.toplevels
            .iter()
            .filter(|e| !e.closed)
            .map(|e| (e.id, e.app_id.as_str(), e.title.as_str(), e.states.as_slice()))
    }

    /// The surface of the toplevel with this id.
    pub fn surface_of(&self, id: u64) -> Option<WlSurface> {
        self.toplevels
            .iter()
            .find(|e| e.id == id && !e.closed)
            .map(|e| e.surface.clone())
    }

    /// The app_id of the toplevel on this surface.
    pub fn app_id_of(&self, surface: &WlSurface) -> Option<&str> {
        self.toplevels
            .iter()
            .find(|e| e.surface == *surface && !e.closed)
            .map(|e| e.app_id.as_str())
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

        let id = self.next_id;
        self.next_id += 1;
        self.dirty = true;
        self.toplevels.push(ForeignToplevelEntry {
            surface: surface.clone(),
            id,
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
            self.dirty = true;
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
                self.dirty = true;
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
                self.dirty = true;
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

    /// Return (surface, app_id, title) for all active toplevels.
    pub fn surface_app_id_titles(&self) -> Vec<(WlSurface, String, String)> {
        self.toplevels
            .iter()
            .filter(|e| !e.closed)
            .map(|e| (e.surface.clone(), e.app_id.clone(), e.title.clone()))
            .collect()
    }

    /// Drop entries whose surface is dead. Safety net so a client crash can
    /// never leave a ghost dock entry, whatever cleanup path was missed —
    /// stale entries otherwise survive forever and get re-announced to every
    /// freshly-bound manager (ghost windows in the CC dock).
    pub fn reap_dead(&mut self) {
        for entry in &mut self.toplevels {
            if !entry.surface.is_alive() {
                entry.broadcast_closed();
                self.dirty = true;
            }
        }
        self.toplevels.retain(|e| e.surface.is_alive());
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
                self.dirty = true;
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

        // Silent reject for untrusted clients: init the resource (must,
        // or wayland-server panics), but do NOT announce any toplevels
        // and do NOT register the manager for future broadcasts. From
        // the client's perspective the compositor just has zero windows.
        if !crate::security::is_trusted_client(client) {
            return;
        }

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
                state.activate_surface(surface);
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
                state.close_surface(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetFullscreen { output: _ } => {
                // No fullscreen support yet, treat as maximize.
                tracing::info!("Foreign toplevel: set_fullscreen requested (treating as maximize)");
                state.maximize_request_surface(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetFullscreen => {
                tracing::info!(
                    "Foreign toplevel: unset_fullscreen requested (treating as unmaximize)"
                );
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
