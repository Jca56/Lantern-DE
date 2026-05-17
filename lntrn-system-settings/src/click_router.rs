//! Routes a single `zone_id` from the interaction layer to the right action.
//!
//! This is the dispatch table that used to live inline in `wayland.rs::run()`:
//!   * Sidebar zones → switch active panel.
//!   * `ZONE_SAVE`   → flush dirty monitor settings, save config, apply WiFi.
//!   * `ZONE_CANCEL` → revert to last saved snapshot.
//!   * Anything else → forward to the active panel's `handle_*_click`.
//!
//! Adding a panel? Wire its click handler into [`route_panel_click`] alongside
//! its `ZONE_*` ids. Sidebar/save/cancel routing here is panel-agnostic.
//!
//! The dropdown menu's own clicks short-circuit *before* this router is called
//! (see `wayland.rs`), so we don't need to think about menu hit-tests here.

use wayland_client::QueueHandle;

use crate::appearance_themes::ThemesPanelState;
use crate::config::LanternConfig;
use crate::display_panel::{self, DisplayPanelState};
use crate::icon_panel;
use crate::input_panel;
use crate::monitor_settings::persist_monitor_settings;
use crate::notifications_panel;
use crate::output_manager::{apply_config, HeadChange};
use crate::panels::{self, PanelState};
use crate::power_panel;
use crate::wayland::{Panel, State, PANELS, ZONE_SIDEBAR_BASE};

/// Handle one left-click on `zone_id`. Returns nothing — every effect happens
/// through the mutable references.
#[allow(clippy::too_many_arguments)]
pub(crate) fn route_zone_click(
    zone_id: u32,
    active_panel: &mut Panel,
    config: &mut LanternConfig,
    saved_config: &mut LanternConfig,
    panel_state: &mut PanelState,
    themes_state: &mut ThemesPanelState,
    display_state: &mut DisplayPanelState,
    icon_panel_state: &mut icon_panel::IconPanelState,
    input_state: &input_panel::InputPanelState,
    state: &State,
    qh: &QueueHandle<State>,
    cx: f32,
    cy: f32,
) {
    // ── Themes UI (always tested first when the Appearance panel is
    // active — the modal needs to eat clicks even over sidebar zones).
    if *active_panel == Panel::Appearance
        && crate::appearance_themes::handle_themes_click(
            themes_state, config, panel_state, zone_id, cx, cy,
        )
    {
        return;
    }

    // ── Sidebar (panel switch) ──────────────────────────────────────
    if zone_id >= ZONE_SIDEBAR_BASE && zone_id < ZONE_SIDEBAR_BASE + PANELS.len() as u32 {
        *active_panel = PANELS[(zone_id - ZONE_SIDEBAR_BASE) as usize].0;
        panel_state.close_dropdown();
        return;
    }

    // ── Save / Cancel ───────────────────────────────────────────────
    match zone_id {
        panels::ZONE_SAVE => {
            apply_save(config, saved_config, display_state, state, qh);
            return;
        }
        panels::ZONE_CANCEL => {
            *config = saved_config.clone();
            return;
        }
        _ => {}
    }

    // ── Per-panel click handlers ────────────────────────────────────
    route_panel_click(
        *active_panel, zone_id, config, panel_state,
        display_state, icon_panel_state, input_state, state, cx, cy,
    );
}

/// Apply pending monitor changes (output-manager + config.monitors),
/// persist the config, and kick off WiFi modprobe if those values changed.
fn apply_save(
    config: &mut LanternConfig,
    saved_config: &mut LanternConfig,
    display_state: &mut DisplayPanelState,
    state: &State,
    qh: &QueueHandle<State>,
) {
    let wifi_changed = config.power.wifi_power_save != saved_config.power.wifi_power_save
        || config.power.wifi_power_scheme != saved_config.power.wifi_power_scheme;

    if display_state.monitor_settings.dirty {
        if let Some(selected_name) = display_state.monitor_arrange.selected_output_name() {
            if let Some(hi) = state.output_mgr.heads.iter().position(|h| h.name == selected_name) {
                let changes = vec![HeadChange {
                    head_idx: hi,
                    mode_idx: display_state.monitor_settings.selected_mode_idx,
                    position: None,
                    scale: display_state.monitor_settings.selected_scale,
                }];
                apply_config(state, qh, &changes);
                persist_monitor_settings(
                    config,
                    &state.output_mgr,
                    hi,
                    &selected_name,
                    display_state.monitor_settings.selected_scale,
                    display_state.monitor_settings.selected_mode_idx,
                );
                display_state.monitor_settings.dirty = false;
            }
        }
    }

    config.save();
    if wifi_changed {
        power_panel::apply_wifi_power(&config.power);
    }
    *saved_config = config.clone();
}

#[allow(clippy::too_many_arguments)]
fn route_panel_click(
    active_panel: Panel,
    zone_id: u32,
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    display_state: &mut DisplayPanelState,
    icon_panel_state: &mut icon_panel::IconPanelState,
    input_state: &input_panel::InputPanelState,
    state: &State,
    cx: f32,
    cy: f32,
) {
    match active_panel {
        Panel::Appearance => {
            crate::appearance_panel::handle_appearance_click(
                config, panel_state, zone_id, cx, cy,
            );
        }
        Panel::Power => {
            power_panel::handle_power_click(config, panel_state, zone_id, cx, cy);
        }
        Panel::Display => {
            display_panel::handle_display_click(
                config, display_state, zone_id, cx, cy, &state.output_mgr,
            );
        }
        Panel::Input => {
            input_panel::handle_input_click(config, input_state, zone_id);
        }
        Panel::Notifications => {
            notifications_panel::handle_notifications_click(config, zone_id);
        }
        Panel::AppIcons => {
            icon_panel_state.on_click(zone_id);
        }
    }
}
