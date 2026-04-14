//! wlr-output-management-unstable-v1 protocol implementation.
//!
//! Exposes output modes, scale, and position to clients (e.g. wlr-randr,
//! lntrn-system-settings) and allows live configuration changes.

use std::sync::Mutex;

use smithay::reexports::wayland_server::{
    backend::{ClientId, GlobalId},
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
};
use wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::ZwlrOutputHeadV1,
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};

use crate::Lantern;

// ── Data types ─────────────────────────────────────────────────────

pub struct OutputModeInfo {
    pub width: i32,
    pub height: i32,
    pub refresh: i32, // mHz
    pub preferred: bool,
    pub drm_mode_index: usize,
    instances: Vec<Weak<ZwlrOutputModeV1>>,
}

pub struct OutputHead {
    pub output_name: String,
    pub modes: Vec<OutputModeInfo>,
    pub current_mode_idx: usize,
    pub enabled: bool,
    pub position: (i32, i32),
    pub scale: f64,
    pub phys_size: (i32, i32),
    instances: Vec<Weak<ZwlrOutputHeadV1>>,
}

/// Change requested by a client configuration.
pub struct OutputChange {
    pub output_name: String,
    pub drm_mode_index: Option<usize>,
    pub position: Option<(i32, i32)>,
    pub scale: Option<f64>,
}

/// Pending configuration — stored as user data on ZwlrOutputConfigurationV1.
pub struct PendingConfig {
    serial: u32,
    heads: Vec<PendingHeadConfig>,
}

struct PendingHeadConfig {
    output_name: String,
    mode_idx: Option<usize>,
    position: Option<(i32, i32)>,
    scale: Option<f64>,
}

/// User data for ZwlrOutputConfigurationHeadV1 — index into PendingConfig.heads.
struct ConfigHeadData {
    /// Weak ref to parent config (so we can find the PendingConfig).
    config: ZwlrOutputConfigurationV1,
    head_index: usize,
}

// ── Global state ───────────────────────────────────────────────────

pub struct OutputManagementState {
    #[allow(dead_code)]
    global: GlobalId,
    managers: Vec<ZwlrOutputManagerV1>,
    pub heads: Vec<OutputHead>,
    serial: u32,
    dh: DisplayHandle,
}

impl OutputManagementState {
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<Lantern, ZwlrOutputManagerV1, _>(4, ());
        Self {
            global,
            managers: Vec::new(),
            heads: Vec::new(),
            serial: 1,
            dh: dh.clone(),
        }
    }

    /// Register a new output head (call from connector_connected).
    pub fn add_head(
        &mut self,
        name: &str,
        modes: &[smithay::reexports::drm::control::Mode],
        current_mode_idx: usize,
        scale: f64,
        position: (i32, i32),
        phys_size: (i32, i32),
    ) {
        use smithay::reexports::drm::control::ModeTypeFlags;

        let mode_infos: Vec<OutputModeInfo> = modes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let (w, h) = m.size();
                OutputModeInfo {
                    width: w as i32,
                    height: h as i32,
                    refresh: (m.vrefresh() as i32) * 1000, // Hz → mHz
                    preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
                    drm_mode_index: i,
                    instances: Vec::new(),
                }
            })
            .collect();

        let head = OutputHead {
            output_name: name.to_string(),
            modes: mode_infos,
            current_mode_idx,
            enabled: true,
            position,
            scale,
            phys_size,
            instances: Vec::new(),
        };
        self.heads.push(head);

        // Announce to all bound managers
        let head_idx = self.heads.len() - 1;
        let mgrs: Vec<_> = self.managers.clone();
        for mgr in &mgrs {
            let Ok(client) = self.dh.get_client(mgr.id()) else { continue };
            self.send_head_to_client(&client, mgr, head_idx);
        }
    }

    /// Remove a head (call from connector_disconnected).
    pub fn remove_head(&mut self, name: &str) {
        if let Some(idx) = self.heads.iter().position(|h| h.output_name == name) {
            let head = &mut self.heads[idx];
            // Send finished to all mode instances
            for mode in &head.modes {
                for weak in &mode.instances {
                    if let Ok(m) = weak.upgrade() {
                        m.finished();
                    }
                }
            }
            // Send finished to all head instances
            for weak in &head.instances {
                if let Ok(h) = weak.upgrade() {
                    h.finished();
                }
            }
            self.heads.remove(idx);
        }
    }

    /// Update a head's properties after a config change, notifying all clients.
    pub fn update_head(
        &mut self,
        output_name: &str,
        scale: Option<f64>,
        position: Option<(i32, i32)>,
        current_mode_idx: Option<usize>,
    ) {
        let Some(head) = self.heads.iter_mut().find(|h| h.output_name == output_name) else {
            return;
        };
        if let Some(s) = scale {
            head.scale = s;
        }
        if let Some(pos) = position {
            head.position = pos;
        }
        if let Some(mi) = current_mode_idx {
            head.current_mode_idx = mi;
        }

        // Send updated properties to all existing head protocol instances
        for weak in &head.instances {
            let Ok(head_obj) = weak.upgrade() else { continue };
            if let Some(s) = scale {
                head_obj.scale(s);
            }
            if let Some((x, y)) = position {
                head_obj.position(x, y);
            }
            if let Some(mi) = current_mode_idx {
                if let Some(mode) = head.modes.get(mi) {
                    for mode_weak in &mode.instances {
                        if let Ok(mode_obj) = mode_weak.upgrade() {
                            head_obj.current_mode(&mode_obj);
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Increment serial and send done to all managers.
    pub fn broadcast_done(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        for mgr in &self.managers {
            mgr.done(self.serial);
        }
    }

    /// Send a single head (with its modes) to a specific client's manager.
    fn send_head_to_client(
        &mut self,
        client: &Client,
        mgr: &ZwlrOutputManagerV1,
        head_idx: usize,
    ) {
        let dh = self.dh.clone();
        let head = &self.heads[head_idx];
        let Ok(head_obj) = client.create_resource::<ZwlrOutputHeadV1, _, Lantern>(
            &dh,
            mgr.version(),
            head_idx as u32,
        ) else {
            return;
        };
        mgr.head(&head_obj);
        head_obj.name(head.output_name.clone());
        head_obj.enabled(head.enabled as i32);
        head_obj.physical_size(head.phys_size.0, head.phys_size.1);

        let current_mode_idx = head.current_mode_idx;
        let enabled = head.enabled;
        let position = head.position;
        let scale = head.scale;
        let mode_count = head.modes.len();

        // Send modes — collect mode objects first, then store weak refs
        let mut mode_objs: Vec<(usize, ZwlrOutputModeV1)> = Vec::new();
        for (mi, mode) in self.heads[head_idx].modes.iter().enumerate() {
            let mode_data = (head_idx as u32, mi as u32);
            let Ok(mode_obj) = client.create_resource::<ZwlrOutputModeV1, _, Lantern>(
                &dh,
                mgr.version(),
                mode_data,
            ) else {
                continue;
            };
            head_obj.mode(&mode_obj);
            mode_obj.size(mode.width, mode.height);
            mode_obj.refresh(mode.refresh);
            if mode.preferred {
                mode_obj.preferred();
            }
            if mi == current_mode_idx {
                head_obj.current_mode(&mode_obj);
            }
            mode_objs.push((mi, mode_obj));
        }

        // Store weak refs
        for (mi, mode_obj) in mode_objs {
            self.heads[head_idx].modes[mi]
                .instances
                .push(mode_obj.downgrade());
        }

        if enabled {
            head_obj.position(position.0, position.1);
            head_obj.scale(scale);
        }

        self.heads[head_idx]
            .instances
            .push(head_obj.downgrade());
    }

    /// Resolve a pending config into OutputChanges. Returns None if serial mismatch.
    pub fn resolve_config(&self, config: &PendingConfig) -> Option<Vec<OutputChange>> {
        if config.serial != self.serial {
            return None;
        }
        let mut changes = Vec::new();
        for hc in &config.heads {
            let head = self.heads.iter().find(|h| h.output_name == hc.output_name)?;
            let drm_idx = hc.mode_idx.map(|mi| head.modes[mi].drm_mode_index);
            changes.push(OutputChange {
                output_name: hc.output_name.clone(),
                drm_mode_index: drm_idx,
                position: hc.position,
                scale: hc.scale,
            });
        }
        Some(changes)
    }
}

// ── GlobalDispatch: Manager ────────────────────────────────────────

impl GlobalDispatch<ZwlrOutputManagerV1, (), Lantern> for Lantern {
    fn bind(
        state: &mut Lantern,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        let mgr = data_init.init(resource, ());

        let oms = &mut state.output_management_state;
        for hi in 0..oms.heads.len() {
            oms.send_head_to_client(client, &mgr, hi);
        }
        mgr.done(oms.serial);
        oms.managers.push(mgr);
    }
}

// ── Dispatch: Manager ──────────────────────────────────────────────

impl Dispatch<ZwlrOutputManagerV1, (), Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                let config = PendingConfig {
                    serial,
                    heads: Vec::new(),
                };
                data_init.init(id, Mutex::new(config));
            }
            zwlr_output_manager_v1::Request::Stop => {
                state
                    .output_management_state
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
        resource: &ZwlrOutputManagerV1,
        _data: &(),
    ) {
        state
            .output_management_state
            .managers
            .retain(|m| m != resource);
    }
}

// ── Dispatch: Head (read-only, only release) ───────────────────────

impl Dispatch<ZwlrOutputHeadV1, u32, Lantern> for Lantern {
    fn request(
        _state: &mut Lantern,
        _client: &Client,
        _head: &ZwlrOutputHeadV1,
        _request: <ZwlrOutputHeadV1 as Resource>::Request,
        _data: &u32,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        // Only request is release (v3+), handled by destruction
    }

    fn destroyed(
        state: &mut Lantern,
        _client: ClientId,
        resource: &ZwlrOutputHeadV1,
        data: &u32,
    ) {
        let idx = *data as usize;
        if let Some(head) = state.output_management_state.heads.get_mut(idx) {
            head.instances
                .retain(|w| w.upgrade().map(|h| h != *resource).unwrap_or(false));
        }
    }
}

// ── Dispatch: Mode (read-only, only release) ───────────────────────

impl Dispatch<ZwlrOutputModeV1, (u32, u32), Lantern> for Lantern {
    fn request(
        _state: &mut Lantern,
        _client: &Client,
        _mode: &ZwlrOutputModeV1,
        _request: <ZwlrOutputModeV1 as Resource>::Request,
        _data: &(u32, u32),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        // Only request is release (v3+)
    }

    fn destroyed(
        state: &mut Lantern,
        _client: ClientId,
        resource: &ZwlrOutputModeV1,
        data: &(u32, u32),
    ) {
        let (hi, mi) = (data.0 as usize, data.1 as usize);
        if let Some(head) = state.output_management_state.heads.get_mut(hi) {
            if let Some(mode) = head.modes.get_mut(mi) {
                mode.instances
                    .retain(|w| w.upgrade().map(|m| m != *resource).unwrap_or(false));
            }
        }
    }
}

// ── Dispatch: Configuration ────────────────────────────────────────

impl Dispatch<ZwlrOutputConfigurationV1, Mutex<PendingConfig>, Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        config_obj: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        data: &Mutex<PendingConfig>,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let head_idx = *head.data::<u32>().unwrap_or(&0) as usize;
                let output_name = state
                    .output_management_state
                    .heads
                    .get(head_idx)
                    .map(|h| h.output_name.clone())
                    .unwrap_or_default();

                let mut pending = data.lock().unwrap();
                let hc_idx = pending.heads.len();
                pending.heads.push(PendingHeadConfig {
                    output_name,
                    mode_idx: None,
                    position: None,
                    scale: None,
                });
                drop(pending);

                data_init.init(
                    id,
                    ConfigHeadData {
                        config: config_obj.clone(),
                        head_index: hc_idx,
                    },
                );
            }
            zwlr_output_configuration_v1::Request::DisableHead { head: _ } => {
                // We don't support disabling outputs yet
            }
            zwlr_output_configuration_v1::Request::Apply => {
                let pending = data.lock().unwrap();
                let changes = state.output_management_state.resolve_config(&pending);
                drop(pending);

                if let Some(changes) = changes {
                    let ok = crate::udev_device::apply_output_config(state, changes);
                    if ok {
                        state.output_management_state.broadcast_done();
                        config_obj.succeeded();
                    } else {
                        config_obj.failed();
                    }
                } else {
                    config_obj.cancelled();
                }
            }
            zwlr_output_configuration_v1::Request::Test => {
                let pending = data.lock().unwrap();
                if pending.serial == state.output_management_state.serial {
                    config_obj.succeeded();
                } else {
                    config_obj.cancelled();
                }
            }
            zwlr_output_configuration_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

// ── Dispatch: Configuration Head ───────────────────────────────────

impl Dispatch<ZwlrOutputConfigurationHeadV1, ConfigHeadData, Lantern> for Lantern {
    fn request(
        _state: &mut Lantern,
        _client: &Client,
        _head_config: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &ConfigHeadData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        let config_data = data.config.data::<Mutex<PendingConfig>>();
        let Some(config_data) = config_data else { return };
        let mut pending = config_data.lock().unwrap();
        let Some(hc) = pending.heads.get_mut(data.head_index) else { return };

        match request {
            zwlr_output_configuration_head_v1::Request::SetMode { mode } => {
                let (_, mi) = *mode.data::<(u32, u32)>().unwrap_or(&(0, 0));
                hc.mode_idx = Some(mi as usize);
            }
            zwlr_output_configuration_head_v1::Request::SetCustomMode { .. } => {
                // Custom modes not supported
            }
            zwlr_output_configuration_head_v1::Request::SetPosition { x, y } => {
                hc.position = Some((x, y));
            }
            zwlr_output_configuration_head_v1::Request::SetScale { scale } => {
                hc.scale = Some(scale);
            }
            zwlr_output_configuration_head_v1::Request::SetTransform { .. } => {
                // Transform not supported yet
            }
            zwlr_output_configuration_head_v1::Request::SetAdaptiveSync { .. } => {
                // VRR not supported yet
            }
            _ => {}
        }
    }
}
