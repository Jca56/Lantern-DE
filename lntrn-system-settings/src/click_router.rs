//! Routes a single `zone_id` from the interaction layer to the right action.
//!
//! Per frame the sidebar appends a `SidebarAction` per visible row; sidebar
//! clicks resolve through that map so we get one consistent contiguous zone
//! range regardless of which categories are expanded.

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
use crate::sidebar::{self, SidebarAction, SidebarState};
use crate::wayland::{Panel, State, ZONE_SIDEBAR_BASE};

#[allow(clippy::too_many_arguments)]
pub(crate) fn route_zone_click(
    zone_id: u32,
    active_panel: &mut Panel,
    sidebar_state: &mut SidebarState,
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
    // ── Themes UI (always tested first when the Themes subpanel is active —
    // the modal needs to eat clicks even over sidebar zones). ─────────────
    if *active_panel == Panel::Themes
        && crate::appearance_themes::handle_themes_click(
            themes_state, config, panel_state, zone_id, cx, cy,
        )
    {
        return;
    }

    // ── Sidebar (panel switch or category toggle) ──────────────────────
    if zone_id >= ZONE_SIDEBAR_BASE
        && (zone_id - ZONE_SIDEBAR_BASE) < sidebar_state.row_actions.len() as u32
    {
        let row = (zone_id - ZONE_SIDEBAR_BASE) as usize;
        match sidebar_state.action_for_row(row) {
            Some(SidebarAction::ToggleCategory(idx)) => sidebar_state.toggle(idx),
            Some(SidebarAction::SelectPanel(p)) => {
                *active_panel = p;
                sidebar_state.ensure_expanded(sidebar::category_of(p));
                panel_state.close_dropdown();
            }
            None => {}
        }
        return;
    }

    // ── Save / Cancel ───────────────────────────────────────────────────
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

    // ── Per-subpanel click handlers ─────────────────────────────────────
    route_panel_click(
        *active_panel, zone_id, config, panel_state,
        display_state, icon_panel_state, input_state, state, cx, cy,
    );
}

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
        Panel::Home => {}
        Panel::Themes | Panel::Animations => {
            crate::appearance_panel::handle_appearance_click(
                config, panel_state, zone_id, cx, cy,
            );
        }
        Panel::LidIdle | Panel::Battery | Panel::WifiPower => {
            power_panel::handle_power_click(config, panel_state, zone_id, cx, cy);
        }
        Panel::Monitors | Panel::Wallpaper => {
            display_panel::handle_display_click(
                config, display_state, zone_id, cx, cy, &state.output_mgr,
            );
        }
        Panel::Mouse => {
            input_panel::handle_input_click(config, input_state, zone_id);
        }
        Panel::NotifBehavior | Panel::NotifSound | Panel::NotifTesting => {
            notifications_panel::handle_notifications_click(config, zone_id);
        }
        Panel::AppIcons => {
            icon_panel_state.on_click(zone_id);
        }
    }
}
