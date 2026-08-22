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
    let crate::app::PanelMode::Control(tile_id) = app.mode else {
        return false;
    };
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
                let track = crate::controls::audio::slider_rect_for(panel, view_top_y, dir, scale);
                let row_top = track.y - track.h * 2.0;
                let row_bot = track.y + track.h * 3.0;
                if phys_x >= track.x
                    && phys_x <= track.x + track.w
                    && phys_y >= row_top
                    && phys_y <= row_bot
                {
                    // Audio sliders map 0..track_width → 0..120 % so the
                    // user can boost quiet sinks (BT headphones especially).
                    // Brightness keeps its 0..1 mapping below.
                    let frac = ((phys_x - track.x) / track.w).clamp(0.0, 1.0) * 1.2;
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
            if let Some(dir) =
                crate::controls::audio::hit_test_icon(panel, view_top_y, scale, phys_x, phys_y)
            {
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
            let track = crate::controls::brightness::slider_rect(panel, view_top_y, scale);
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
            use crate::controls::bluetooth::BtClick;

            if let Some(hit) = crate::controls::bluetooth::hit_test(
                &app.controls.bluetooth,
                panel,
                view_top_y,
                scale,
                app.config.text_size,
                phys_x,
                phys_y,
            ) {
                let bt = &mut app.controls.bluetooth;
                match hit {
                    BtClick::PowerToggle => bt.toggle_power(),
                    BtClick::DiscoverableToggle => bt.toggle_discoverable(),
                    BtClick::ScanToggle => bt.toggle_scan(),
                    BtClick::DeviceRow(mac) => bt.toggle_expanded(&mac),
                    BtClick::ConnectButton(mac) => {
                        let is_paired = bt.devices().iter().any(|d| d.mac == mac && d.paired);
                        if is_paired {
                            bt.toggle_connection(&mac);
                        } else {
                            bt.pair(&mac);
                        }
                    }
                    BtClick::SendButton(mac) => bt.send_file(&mac),
                    // Inline request-strip buttons. Which reply fires
                    // depends on which request is live for this MAC: an
                    // outgoing pair prompt we own, an incoming pair, or
                    // an incoming file.
                    BtClick::PromptAccept(mac) => dispatch_prompt(bt, &mac, true),
                    BtClick::PromptReject(mac) => dispatch_prompt(bt, &mac, false),
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
                let hit =
                    crate::controls::wifi::hit_test_modal(panel, view_top_y, scale, phys_x, phys_y);
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
                    crate::controls::wifi::NetworkHit::ToggleVpn => {
                        app.controls.wifi.toggle_vpn();
                    }
                    crate::controls::wifi::NetworkHit::ConnectButton(ssid) => {
                        let net = app
                            .controls
                            .wifi
                            .networks()
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
        // Temp / Network / GPU / Disk share their expanded view (and
        // click behavior) with SysMon since they all funnel into it.
        crate::controls::TileId::SysMon
        | crate::controls::TileId::Temp
        | crate::controls::TileId::Network
        | crate::controls::TileId::Gpu
        | crate::controls::TileId::Disk => {
            if let Some(hit) = crate::controls::sysmon::view::hit_test_view(
                &app.controls.sysmon,
                panel,
                view_top_y,
                scale,
                app.config.text_size,
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
        crate::controls::TileId::Workspace
        | crate::controls::TileId::Gaming
        | crate::controls::TileId::Collapse
        | crate::controls::TileId::TerminalClear => false,
    }
}

/// Route an inline request-strip Accept/Reject for `mac` to the matching
/// backend reply. Priority mirrors `bluetooth::prompt::row_prompt`:
/// outgoing pair we own → incoming pair → incoming file.
fn dispatch_prompt(bt: &mut crate::controls::bluetooth::Bluetooth, mac: &str, accept: bool) {
    use crate::controls::bluetooth::PairPromptKind;

    // Outgoing pair flow we initiated (Confirm passkey / authorize / PIN).
    if let Some(kind) = bt
        .pair_prompt
        .as_ref()
        .filter(|p| p.mac == mac)
        .map(|p| p.kind.clone())
    {
        match (accept, kind) {
            (true, PairPromptKind::Enter) => bt.pair_submit_passkey(),
            (true, _) => bt.pair_confirm_yes(),
            (false, PairPromptKind::Enter) => bt.pair_cancel(),
            (false, _) => bt.pair_confirm_no(),
        }
        return;
    }

    // Incoming pair (another device pairing with us).
    if bt.pair_request.as_ref().is_some_and(|p| p.mac == mac) {
        if accept {
            bt.pair_request_accept();
        } else {
            bt.pair_request_reject();
        }
        return;
    }

    // Otherwise it's an incoming-file request on this row.
    if accept {
        bt.incoming_accept();
    } else {
        bt.incoming_reject();
    }
}
