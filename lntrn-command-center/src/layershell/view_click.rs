//! Click handling for the control-view content area (Audio, Brightness,
//! Bluetooth, WiFi, Clock, SysMon, …). Each tile owns its own hit-test
//! and routes the click into the appropriate backend mutation.

use lntrn_render::TextRenderer;

use crate::app::AppState;

pub(super) fn handle_control_view_click(
    app: &mut AppState,
    text: &mut TextRenderer,
    panel: lntrn_render::Rect,
    scale: f32,
    phys_x: f32,
    phys_y: f32,
) -> bool {
    let crate::app::PanelMode::Control(tile_id) = app.mode else { return false };
    // The control view starts immediately beneath the controls-row underline.
    let view_top_y = crate::controls::content_top_y(panel, scale);

    match tile_id {
        crate::controls::TileId::Clock => {
            // Detail panel takes priority when open.
            if app.controls.clock.selected_day.is_some() {
                if let Some(hit) = crate::controls::clock::hit_test_detail(
                    panel,
                    view_top_y,
                    scale,
                    &app.controls.clock,
                    &app.controls.events,
                    text,
                    phys_x,
                    phys_y,
                ) {
                    match hit {
                        crate::controls::clock::DetailHit::Close => {
                            app.controls.clock.selected_day = None;
                            app.controls.clock.add_event_input = None;
                            app.controls.clock.event_menu = None;
                        }
                        crate::controls::clock::DetailHit::OpenAddInput => {
                            app.controls.clock.add_event_input =
                                Some(crate::search::input::Input::new());
                        }
                        crate::controls::clock::DetailHit::EventRow(_) => {
                            // Left-click on an event row currently does
                            // nothing — delete is via right-click menu.
                        }
                    }
                    return true;
                }
            }

            if let Some(hit) = crate::controls::clock::hit_test_view(
                panel,
                view_top_y,
                scale,
                &app.controls.clock,
                text,
                phys_x,
                phys_y,
            ) {
                match hit {
                    crate::controls::clock::CalendarHit::PrevMonth => {
                        app.controls.clock.prev_month();
                    }
                    crate::controls::clock::CalendarHit::NextMonth => {
                        app.controls.clock.next_month();
                    }
                    crate::controls::clock::CalendarHit::Day(date) => {
                        // Toggle: clicking the same day again closes.
                        if app.controls.clock.selected_day == Some(date) {
                            app.controls.clock.selected_day = None;
                            app.controls.clock.add_event_input = None;
                        } else {
                            app.controls.clock.selected_day = Some(date);
                            app.controls.clock.add_event_input = None;
                        }
                    }
                }
                return true;
            }
            false
        }
        crate::controls::TileId::Battery => {
            let toggle = crate::controls::battery::toggle_rect(panel, view_top_y, scale);
            if phys_x >= toggle.x
                && phys_x <= toggle.x + toggle.w
                && phys_y >= toggle.y
                && phys_y <= toggle.y + toggle.h
            {
                app.controls.battery.toggle_charge_limit();
                return true;
            }
            false
        }
        crate::controls::TileId::Audio => {
            use crate::controls::audio::Direction;

            // Sliders — try each direction's track. A slider click both
            // sets the volume immediately and starts a drag so motion
            // events keep updating until the button is released.
            for dir in [Direction::Output, Direction::Input] {
                let track =
                    crate::controls::audio::slider_rect_for(panel, view_top_y, dir, scale);
                let row_top = track.y - track.h * 2.0;
                let row_bot = track.y + track.h * 3.0;
                if phys_x >= track.x
                    && phys_x <= track.x + track.w
                    && phys_y >= row_top
                    && phys_y <= row_bot
                {
                    let frac = ((phys_x - track.x) / track.w).clamp(0.0, 1.0);
                    match dir {
                        Direction::Output => {
                            app.controls.audio.set_volume(frac);
                            app.dragging = Some(crate::app::DragTarget::AudioOutputSlider);
                        }
                        Direction::Input => {
                            app.controls.audio.set_input_volume(frac);
                            app.dragging = Some(crate::app::DragTarget::AudioInputSlider);
                        }
                    }
                    return true;
                }
            }

            // Speaker / mic icon click → toggle that direction's mute.
            if let Some(dir) = crate::controls::audio::hit_test_icon(
                panel, view_top_y, scale, phys_x, phys_y,
            ) {
                match dir {
                    Direction::Output => app.controls.audio.toggle_mute(),
                    Direction::Input => app.controls.audio.toggle_input_mute(),
                }
                return true;
            }

            // Device lists — click a row to set that device as default.
            if let Some((dir, dev_id)) = crate::controls::audio::hit_test_device_dir(
                &app.controls.audio,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match dir {
                    Direction::Output => app.controls.audio.set_default_sink(dev_id),
                    Direction::Input => app.controls.audio.set_default_source(dev_id),
                }
                return true;
            }
            false
        }
        crate::controls::TileId::Brightness => {
            let track =
                crate::controls::brightness::slider_rect(panel, view_top_y, scale);
            let row_top = track.y - track.h * 2.0;
            let row_bot = track.y + track.h * 3.0;
            if phys_x >= track.x
                && phys_x <= track.x + track.w
                && phys_y >= row_top
                && phys_y <= row_bot
            {
                let frac = ((phys_x - track.x) / track.w).clamp(0.0, 1.0);
                app.controls.brightness.set_fraction(frac);
                app.dragging = Some(crate::app::DragTarget::BrightnessSlider);
                return true;
            }
            false
        }
        crate::controls::TileId::Bluetooth => {
            use crate::controls::bluetooth::{
                BtClick, IncomingModalHit, PairModalHit, PairPromptKind,
            };

            // Incoming-file modal sits highest in priority.
            if app.controls.bluetooth.incoming_request.is_some() {
                let hit = crate::controls::bluetooth::hit_test_incoming_modal(
                    panel, view_top_y, scale, phys_x, phys_y,
                );
                match hit {
                    IncomingModalHit::Accept => app.controls.bluetooth.incoming_accept(),
                    IncomingModalHit::Reject | IncomingModalHit::Backdrop => {
                        app.controls.bluetooth.incoming_reject();
                    }
                    IncomingModalHit::Box => {}
                }
                return true;
            }

            // If the pair-prompt modal is up, every click in the BT
            // view goes to the modal first.
            if let Some(prompt) = app.controls.bluetooth.pair_prompt.as_ref() {
                let kind = prompt.kind.clone();
                let hit = crate::controls::bluetooth::hit_test_pair_modal(
                    prompt, panel, view_top_y, scale, phys_x, phys_y,
                );
                match hit {
                    PairModalHit::Primary => match kind {
                        PairPromptKind::Confirm(_) | PairPromptKind::Authorize(_) => {
                            app.controls.bluetooth.pair_confirm_yes();
                        }
                        PairPromptKind::Enter => {
                            app.controls.bluetooth.pair_submit_passkey();
                        }
                    },
                    PairModalHit::Secondary | PairModalHit::Backdrop => {
                        match kind {
                            PairPromptKind::Confirm(_) | PairPromptKind::Authorize(_) => {
                                app.controls.bluetooth.pair_confirm_no();
                            }
                            PairPromptKind::Enter => {
                                app.controls.bluetooth.pair_cancel();
                            }
                        }
                    }
                    PairModalHit::Field | PairModalHit::Box => {
                        // Inside the modal but not on a button — no-op.
                    }
                }
                return true;
            }

            if let Some(hit) = crate::controls::bluetooth::hit_test(
                &app.controls.bluetooth,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match hit {
                    BtClick::PowerToggle => app.controls.bluetooth.toggle_power(),
                    BtClick::DiscoverableToggle => {
                        app.controls.bluetooth.toggle_discoverable();
                    }
                    BtClick::ScanToggle => app.controls.bluetooth.toggle_scan(),
                    BtClick::DeviceRow(mac) => {
                        let is_paired = app
                            .controls
                            .bluetooth
                            .devices()
                            .iter()
                            .any(|d| d.mac == mac && d.paired);
                        if is_paired {
                            app.controls.bluetooth.toggle_connection(&mac);
                        } else {
                            app.controls.bluetooth.pair(&mac);
                        }
                    }
                    BtClick::SendButton(mac) => {
                        app.controls.bluetooth.send_file(&mac);
                    }
                }
                return true;
            }
            false
        }
        crate::controls::TileId::Wifi => {
            // If the password modal is up, every click in the WiFi
            // view goes to the modal first.
            if app.controls.wifi.prompt.is_some() {
                use crate::controls::wifi::ModalHit;
                let hit = crate::controls::wifi::hit_test_modal(
                    panel, view_top_y, scale, phys_x, phys_y,
                );
                match hit {
                    ModalHit::Connect => {
                        app.controls.wifi.submit_prompt();
                    }
                    ModalHit::Cancel | ModalHit::Backdrop => {
                        app.controls.wifi.close_prompt();
                    }
                    ModalHit::Field | ModalHit::Box => {
                        // No-op — clicks inside the box just dismiss
                        // pending hover state in a future iteration.
                    }
                }
                return true;
            }

            // Normal network-row click.
            if let Some(hit) = crate::controls::wifi::hit_test_network(
                &app.controls.wifi,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match hit {
                    crate::controls::wifi::NetworkHit::Row(ssid) => {
                        // Toggle: clicking the same row again collapses it.
                        if app.controls.wifi.expanded_ssid.as_deref() == Some(ssid.as_str()) {
                            app.controls.wifi.expanded_ssid = None;
                        } else {
                            app.controls.wifi.expanded_ssid = Some(ssid);
                        }
                    }
                    crate::controls::wifi::NetworkHit::BandPill(ssid, band) => {
                        app.controls.wifi.select_band(&ssid, band);
                    }
                    crate::controls::wifi::NetworkHit::LockBssid(ssid, bssid) => {
                        app.controls.wifi.toggle_pinned_bssid(&ssid, &bssid);
                    }
                    crate::controls::wifi::NetworkHit::ProfileActivate(_, name) => {
                        app.controls.wifi.activate_profile(&name);
                    }
                    crate::controls::wifi::NetworkHit::ProfileDelete(_, uuid) => {
                        app.controls.wifi.delete_profile(&uuid);
                    }
                    crate::controls::wifi::NetworkHit::ConnectButton(ssid) => {
                        let net = app.controls.wifi.networks()
                            .iter()
                            .find(|n| n.ssid == ssid)
                            .cloned();
                        let already_in_use = net.as_ref().is_some_and(|n| n.in_use);
                        let needs_password = match &net {
                            Some(n) => {
                                let secured = !n.security.is_empty() && n.security != "--";
                                secured && !n.saved && !n.in_use
                            }
                            None => false,
                        };
                        if already_in_use {
                            // Already connected → button is purely a label.
                        } else if needs_password {
                            app.controls.wifi.open_prompt(&ssid);
                        } else {
                            app.controls.wifi.connect(&ssid, None);
                        }
                        tracing::debug!(%ssid, needs_password, "wifi: connect button");
                    }
                }
                return true;
            }
            false
        }
        // Temp shares its expanded view (and click behavior) with
        // SysMon since they read from the same backend.
        crate::controls::TileId::SysMon | crate::controls::TileId::Temp => {
            if let Some(hit) = crate::controls::sysmon::view::hit_test_view(
                &app.controls.sysmon,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match hit {
                    crate::controls::sysmon::view::SysMonHit::SelectProcess(pid) => {
                        app.controls.sysmon.selected_pid = Some(pid);
                    }
                    crate::controls::sysmon::view::SysMonHit::KillProcess(pid) => {
                        crate::controls::sysmon::view::kill_process(pid);
                        // Clear selection so the user has to re-arm
                        // before another kill can fire.
                        app.controls.sysmon.selected_pid = None;
                    }
                    crate::controls::sysmon::view::SysMonHit::SortByCpu => {
                        let next = app.controls.sysmon.sort.toggle_cpu();
                        app.controls.sysmon.set_sort(next);
                    }
                    crate::controls::sysmon::view::SysMonHit::SortByMem => {
                        let next = app.controls.sysmon.sort.toggle_mem();
                        app.controls.sysmon.set_sort(next);
                    }
                    crate::controls::sysmon::view::SysMonHit::ClearFilter => {
                        app.controls.sysmon.filter.clear();
                    }
                }
                return true;
            }
            false
        }
        // No expanded view — click handling is shortcut in the press
        // path, so we never reach here for these.
        crate::controls::TileId::Collapse | crate::controls::TileId::TerminalClear => false,
    }
}
