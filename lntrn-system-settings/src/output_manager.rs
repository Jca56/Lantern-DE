//! Client-side wlr-output-management-unstable-v1 protocol handling.
//!
//! Receives output head/mode information from the compositor and can
//! apply configuration changes (resolution, refresh rate, scale, position).

use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use crate::wayland::State;

// ── Data types ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    pub refresh: i32, // mHz
    pub preferred: bool,
    pub mode_obj: ZwlrOutputModeV1,
}

#[derive(Clone, Debug)]
pub struct HeadInfo {
    pub name: String,
    pub enabled: bool,
    pub position: (i32, i32),
    pub scale: f64,
    pub phys_w: i32,
    pub phys_h: i32,
    pub modes: Vec<ModeInfo>,
    pub current_mode: Option<usize>,
    pub head_obj: ZwlrOutputHeadV1,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConfigResult {
    Succeeded,
    Failed,
    Cancelled,
}

pub struct OutputManagerClient {
    pub manager: Option<ZwlrOutputManagerV1>,
    pub serial: u32,
    pub heads: Vec<HeadInfo>,
    pub config_result: Option<ConfigResult>,
    /// Heads currently being built (before done event).
    building_heads: Vec<HeadInfo>,
}

impl OutputManagerClient {
    pub fn new() -> Self {
        Self {
            manager: None,
            serial: 0,
            heads: Vec::new(),
            config_result: None,
            building_heads: Vec::new(),
        }
    }

    /// Find available resolutions for a head (deduplicated).
    pub fn resolutions_for_head(&self, head_idx: usize) -> Vec<(i32, i32)> {
        let Some(head) = self.heads.get(head_idx) else { return Vec::new() };
        let mut resolutions: Vec<(i32, i32)> = head
            .modes
            .iter()
            .map(|m| (m.width, m.height))
            .collect();
        resolutions.sort_by(|a, b| (b.0 * b.1).cmp(&(a.0 * a.1))); // highest first
        resolutions.dedup();
        resolutions
    }

    /// Find available refresh rates for a head at a specific resolution.
    pub fn refresh_rates_for_resolution(
        &self,
        head_idx: usize,
        width: i32,
        height: i32,
    ) -> Vec<(i32, usize)> {
        let Some(head) = self.heads.get(head_idx) else { return Vec::new() };
        let mut rates: Vec<(i32, usize)> = head
            .modes
            .iter()
            .enumerate()
            .filter(|(_, m)| m.width == width && m.height == height)
            .map(|(i, m)| (m.refresh, i))
            .collect();
        rates.sort_by(|a, b| b.0.cmp(&a.0)); // highest first
        rates
    }
}

// ── Apply configuration ────────────────────────────────────────────

/// A change to apply to a single output head.
pub struct HeadChange {
    pub head_idx: usize,
    pub mode_idx: Option<usize>,
    pub position: Option<(i32, i32)>,
    pub scale: Option<f64>,
}

pub fn apply_config(
    state: &State,
    qh: &QueueHandle<State>,
    changes: &[HeadChange],
) {
    let Some(mgr) = &state.output_mgr.manager else { return };
    let config: ZwlrOutputConfigurationV1 = mgr.create_configuration(state.output_mgr.serial, qh, ());
    for change in changes {
        let Some(head) = state.output_mgr.heads.get(change.head_idx) else { continue };
        let head_config: ZwlrOutputConfigurationHeadV1 =
            config.enable_head(&head.head_obj, qh, ());
        if let Some(mode_idx) = change.mode_idx {
            if let Some(mode) = head.modes.get(mode_idx) {
                head_config.set_mode(&mode.mode_obj);
            }
        }
        if let Some((x, y)) = change.position {
            head_config.set_position(x, y);
        }
        if let Some(scale) = change.scale {
            head_config.set_scale(scale);
        }
    }
    config.apply();
}

// ── Dispatch: Manager ──────────────────────────────────────────────

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head: _ } => {
                // Head object created — we handle its events in the head dispatch.
                // A new HeadInfo will be pushed in the head's Name event.
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.output_mgr.serial = serial;
                // Swap building_heads into heads
                state.output_mgr.heads = std::mem::take(&mut state.output_mgr.building_heads);
            }
            zwlr_output_manager_v1::Event::Finished => {
                state.output_mgr.manager = None;
            }
            _ => {}
        }
    }
}

// ── Dispatch: Head ─────────────────────────────────────────────────

impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Find or create the building head for this proxy
        let head = match state
            .output_mgr
            .building_heads
            .iter_mut()
            .find(|h| h.head_obj == *proxy)
        {
            Some(h) => h,
            None => {
                // First event for this head — create placeholder
                state.output_mgr.building_heads.push(HeadInfo {
                    name: String::new(),
                    enabled: false,
                    position: (0, 0),
                    scale: 1.0,
                    phys_w: 0,
                    phys_h: 0,
                    modes: Vec::new(),
                    current_mode: None,
                    head_obj: proxy.clone(),
                });
                state.output_mgr.building_heads.last_mut().unwrap()
            }
        };

        match event {
            zwlr_output_head_v1::Event::Name { name } => head.name = name,
            zwlr_output_head_v1::Event::Enabled { enabled } => head.enabled = enabled != 0,
            zwlr_output_head_v1::Event::Position { x, y } => head.position = (x, y),
            zwlr_output_head_v1::Event::Scale { scale } => head.scale = scale,
            zwlr_output_head_v1::Event::PhysicalSize { width, height } => {
                head.phys_w = width;
                head.phys_h = height;
            }
            zwlr_output_head_v1::Event::Mode { mode: _ } => {
                // Mode object created — we handle its events in the mode dispatch.
                // A new ModeInfo is pushed in the mode's Size event.
            }
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                // Find the mode index by matching the mode proxy
                let idx = head.modes.iter().position(|m| m.mode_obj == mode);
                head.current_mode = idx;
            }
            zwlr_output_head_v1::Event::Finished => {
                state
                    .output_mgr
                    .building_heads
                    .retain(|h| h.head_obj != *proxy);
            }
            _ => {}
        }
    }
}

// ── Dispatch: Mode ─────────────────────────────────────────────────

impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Find the mode in any building head
        let mode = state
            .output_mgr
            .building_heads
            .iter_mut()
            .flat_map(|h| h.modes.iter_mut())
            .find(|m| m.mode_obj == *proxy);

        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                if mode.is_none() {
                    // First event — find parent head and add mode
                    // The mode was created by the head's Mode event, but we need to find
                    // which head it belongs to. The compositor sends mode events in order
                    // after the head event, so the last building head is the parent.
                    if let Some(head) = state.output_mgr.building_heads.last_mut() {
                        head.modes.push(ModeInfo {
                            width,
                            height,
                            refresh: 0,
                            preferred: false,
                            mode_obj: proxy.clone(),
                        });
                    }
                } else if let Some(m) = mode {
                    m.width = width;
                    m.height = height;
                }
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => {
                if let Some(m) = find_mode_mut(&mut state.output_mgr.building_heads, proxy) {
                    m.refresh = refresh;
                }
            }
            zwlr_output_mode_v1::Event::Preferred => {
                if let Some(m) = find_mode_mut(&mut state.output_mgr.building_heads, proxy) {
                    m.preferred = true;
                }
            }
            zwlr_output_mode_v1::Event::Finished => {
                // Remove mode from any head
                for head in &mut state.output_mgr.building_heads {
                    head.modes.retain(|m| m.mode_obj != *proxy);
                }
            }
            _ => {}
        }
    }
}

fn find_mode_mut<'a>(
    heads: &'a mut [HeadInfo],
    proxy: &ZwlrOutputModeV1,
) -> Option<&'a mut ModeInfo> {
    heads
        .iter_mut()
        .flat_map(|h| h.modes.iter_mut())
        .find(|m| m.mode_obj == *proxy)
}

// ── Dispatch: Configuration ────────────────────────────────────────

impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_configuration_v1::Event::Succeeded => {
                state.output_mgr.config_result = Some(ConfigResult::Succeeded);
            }
            zwlr_output_configuration_v1::Event::Failed => {
                state.output_mgr.config_result = Some(ConfigResult::Failed);
            }
            zwlr_output_configuration_v1::Event::Cancelled => {
                state.output_mgr.config_result = Some(ConfigResult::Cancelled);
            }
            _ => {}
        }
    }
}

// ── Dispatch: Configuration Head (no events) ───────────────────────

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrOutputConfigurationHeadV1,
        _event: <ZwlrOutputConfigurationHeadV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // No events defined for configuration head
    }
}
