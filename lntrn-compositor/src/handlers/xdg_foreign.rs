//! xdg-foreign-unstable-v2 protocol implementation.
//!
//! Allows apps to establish parent-child window relationships across
//! different Wayland clients. Client A exports a surface to get a handle
//! token, shares it with client B, and client B imports it to set up a
//! parent-child stacking relationship.

use std::collections::HashMap;

use rand::Rng;
use smithay::reexports::wayland_server::{
    backend::ClientId,
    protocol::wl_surface::WlSurface,
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};
use wayland_protocols::xdg::foreign::zv2::server::{
    zxdg_exported_v2::{self, ZxdgExportedV2},
    zxdg_exporter_v2::{self, ZxdgExporterV2},
    zxdg_imported_v2::{self, ZxdgImportedV2},
    zxdg_importer_v2::{self, ZxdgImporterV2},
};

use crate::Lantern;

// ── State ───────────────────────────────────────────────────────────

/// Compositor-side state for xdg-foreign.
pub struct XdgForeignState {
    _exporter_global: smithay::reexports::wayland_server::backend::GlobalId,
    _importer_global: smithay::reexports::wayland_server::backend::GlobalId,
    /// token -> exported surface
    pub exports: HashMap<String, WlSurface>,
    /// child surface -> parent surface
    pub imports: HashMap<WlSurface, WlSurface>,
}

impl XdgForeignState {
    pub fn new(dh: &DisplayHandle) -> Self {
        let exporter_global = dh.create_global::<Lantern, ZxdgExporterV2, _>(1, ());
        let importer_global = dh.create_global::<Lantern, ZxdgImporterV2, _>(1, ());
        Self {
            _exporter_global: exporter_global,
            _importer_global: importer_global,
            exports: HashMap::new(),
            imports: HashMap::new(),
        }
    }

    /// Generate a random handle token (32 hex chars).
    fn generate_token() -> String {
        let mut rng = rand::thread_rng();
        let bytes: [u8; 16] = rng.gen();
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Look up the parent surface for a child, if any import relationship exists.
    pub fn parent_of(&self, child: &WlSurface) -> Option<&WlSurface> {
        self.imports.get(child)
    }

    /// Remove all exports and imports associated with a surface (when it's destroyed).
    pub fn surface_destroyed(&mut self, surface: &WlSurface) {
        // Remove any exports of this surface
        self.exports.retain(|_, s| s != surface);
        // Remove any imports where this surface is the child
        self.imports.remove(surface);
        // Remove any imports where this surface is the parent
        self.imports.retain(|_, parent| parent != surface);
    }
}

/// Per-exported-handle data: the token assigned to it.
pub struct ExportedData {
    pub token: String,
}

/// Per-imported-handle data: the token that was imported.
pub struct ImportedData {
    pub token: String,
}

// ── Exporter global ─────────────────────────────────────────────────

impl GlobalDispatch<ZxdgExporterV2, (), Lantern> for Lantern {
    fn bind(
        _state: &mut Lantern,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgExporterV2>,
        _data: &(),
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZxdgExporterV2, (), Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        _resource: &ZxdgExporterV2,
        request: zxdg_exporter_v2::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zxdg_exporter_v2::Request::ExportToplevel { id, surface } => {
                let token = XdgForeignState::generate_token();

                // Store the export
                state
                    .xdg_foreign_state
                    .exports
                    .insert(token.clone(), surface);

                // Create the exported object and send the handle
                let exported = data_init.init(id, ExportedData { token: token.clone() });
                exported.handle(token);

                tracing::info!("xdg-foreign: surface exported");
            }
            zxdg_exporter_v2::Request::Destroy => {}
            _ => {}
        }
    }
}

// ── Exported handle ─────────────────────────────────────────────────

impl Dispatch<ZxdgExportedV2, ExportedData, Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        _resource: &ZxdgExportedV2,
        request: zxdg_exported_v2::Request,
        data: &ExportedData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zxdg_exported_v2::Request::Destroy => {
                // Revoke the export
                state.xdg_foreign_state.exports.remove(&data.token);
                // Invalidate any imports that used this token
                let parent = state
                    .xdg_foreign_state
                    .exports
                    .get(&data.token)
                    .cloned();
                if let Some(parent) = parent {
                    state
                        .xdg_foreign_state
                        .imports
                        .retain(|_, p| p != &parent);
                }
                tracing::info!("xdg-foreign: export revoked");
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Lantern,
        _client: ClientId,
        _resource: &ZxdgExportedV2,
        data: &ExportedData,
    ) {
        state.xdg_foreign_state.exports.remove(&data.token);
    }
}

// ── Importer global ─────────────────────────────────────────────────

impl GlobalDispatch<ZxdgImporterV2, (), Lantern> for Lantern {
    fn bind(
        _state: &mut Lantern,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgImporterV2>,
        _data: &(),
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZxdgImporterV2, (), Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        _resource: &ZxdgImporterV2,
        request: zxdg_importer_v2::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zxdg_importer_v2::Request::ImportToplevel { id, handle } => {
                let imported =
                    data_init.init(id, ImportedData { token: handle.clone() });

                if state.xdg_foreign_state.exports.contains_key(&handle) {
                    tracing::info!("xdg-foreign: surface imported via handle");
                } else {
                    // Handle doesn't exist -- send destroyed event
                    tracing::warn!("xdg-foreign: import failed, unknown handle");
                    imported.destroyed();
                }
            }
            zxdg_importer_v2::Request::Destroy => {}
            _ => {}
        }
    }
}

// ── Imported handle ─────────────────────────────────────────────────

impl Dispatch<ZxdgImportedV2, ImportedData, Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        _resource: &ZxdgImportedV2,
        request: zxdg_imported_v2::Request,
        data: &ImportedData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zxdg_imported_v2::Request::SetParentOf { surface } => {
                if let Some(parent) = state.xdg_foreign_state.exports.get(&data.token) {
                    let parent = parent.clone();
                    state
                        .xdg_foreign_state
                        .imports
                        .insert(surface, parent);
                    tracing::info!("xdg-foreign: parent-child relationship established");
                } else {
                    tracing::warn!("xdg-foreign: set_parent_of failed, export no longer valid");
                }
            }
            zxdg_imported_v2::Request::Destroy => {
                // The spec says destroying the imported object invalidates the relationship.
                // Remove any imports that used this token.
                if let Some(parent) = state.xdg_foreign_state.exports.get(&data.token) {
                    let parent = parent.clone();
                    state
                        .xdg_foreign_state
                        .imports
                        .retain(|_, p| p != &parent);
                }
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Lantern,
        _client: ClientId,
        _resource: &ZxdgImportedV2,
        data: &ImportedData,
    ) {
        // Clean up import relationships tied to this token
        if let Some(parent) = state.xdg_foreign_state.exports.get(&data.token) {
            let parent = parent.clone();
            state
                .xdg_foreign_state
                .imports
                .retain(|_, p| p != &parent);
        }
    }
}
