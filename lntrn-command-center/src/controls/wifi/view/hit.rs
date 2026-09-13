//! Hit-testing for the WiFi panel. Walks rows top-to-bottom honoring
//! per-row variable height so the expanded section doesn't shadow rows
//! below it.

use lntrn_render::Rect;

use crate::controls::wifi::{Band, Wifi};

use super::header::refresh_button_rect;
use super::layout::{
    band_pill_rect, bssid_card_rect, bssid_lock_rect, connect_button_rect, expanded_extra_height,
    has_band_selector, profile_card_rect, profile_delete_rect, row_list_top_y,
    visible_bssid_card_count, visible_profile_card_count,
};
use super::{MAX_NETWORK_ROWS, ROW_HEIGHT};

/// What was clicked inside the WiFi network list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkHit {
    /// The header part of a row (any area outside the Connect button
    /// in the expanded section). Caller toggles the expanded ssid.
    Row(String),
    /// The Connect / Disconnect button inside the expanded section
    /// (Disconnect when the network is the one in use).
    ConnectButton(String),
    /// A band-selector pill inside the expanded section.
    BandPill(String, Band),
    /// The lock toggle on a specific BSSID card. Caller toggles the
    /// pinned BSSID for this SSID.
    LockBssid(String, String),
    /// Click on a saved-profile card (anywhere except the delete X).
    /// Caller activates that profile via `nmcli connection up`.
    ProfileActivate(String, String), // (ssid, name)
    /// Click on the delete X overlaid on a profile card.
    ProfileDelete(String, String), // (ssid, uuid)
    /// Refresh button on the header row.
    Refresh,
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

/// Hit-test a click against the network list. Walks rows top-to-
/// bottom honoring per-row variable height so the expanded section
/// doesn't shadow rows below it.
pub fn hit_test_network(
    wifi: &Wifi,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<NetworkHit> {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    if x < inner_x || x > inner_x + inner_w {
        return None;
    }

    // The refresh button sits above the network list, so check it
    // BEFORE the `y < list_top` early-return below.
    if contains(refresh_button_rect(panel, panel_top_y, scale), x, y) {
        return Some(NetworkHit::Refresh);
    }

    let header_h = ROW_HEIGHT * scale;
    let list_top = row_list_top_y(panel_top_y, scale);
    // Reject clicks above the first row (header area) so the SSID
    // labels there don't grab clicks meant for scrollbar drag etc.
    if y < list_top {
        return None;
    }
    let scroll_px = wifi.scroll * scale;
    let mut row_y = list_top - scroll_px;
    for net in wifi.networks().iter().take(MAX_NETWORK_ROWS) {
        let is_expanded = wifi.expanded_ssid.as_deref() == Some(net.ssid.as_str());
        let extra = if is_expanded {
            expanded_extra_height(net, scale)
        } else {
            0.0
        };

        // Header area first.
        if y >= row_y && y <= row_y + header_h {
            return Some(NetworkHit::Row(net.ssid.clone()));
        }
        // Expanded area: check pill / button hits, otherwise treat as
        // a Row click so clicking inside the panel away from any
        // control doesn't leak through to the next row.
        if is_expanded {
            let body_top = row_y + header_h;
            let body_bottom = row_y + header_h + extra;
            if y >= body_top && y <= body_bottom {
                // BSSID lock toggles (right column).
                let card_n = visible_bssid_card_count(net);
                for i in 0..card_n {
                    let card = bssid_card_rect(inner_x, inner_w, body_top, scale, i);
                    let lock = bssid_lock_rect(card, scale);
                    if x >= lock.x && x <= lock.x + lock.w && y >= lock.y && y <= lock.y + lock.h {
                        let mac = net.aps[i].bssid.clone();
                        return Some(NetworkHit::LockBssid(net.ssid.clone(), mac));
                    }
                }
                // Profile cards (delete X first; otherwise activate).
                let profile_n = visible_profile_card_count(net);
                for i in 0..profile_n {
                    let card = profile_card_rect(inner_x, inner_w, body_top, scale, card_n, i);
                    let del = profile_delete_rect(card, scale);
                    if x >= del.x && x <= del.x + del.w && y >= del.y && y <= del.y + del.h {
                        let uuid = net.profiles[i].uuid.clone();
                        return Some(NetworkHit::ProfileDelete(net.ssid.clone(), uuid));
                    }
                    if x >= card.x && x <= card.x + card.w && y >= card.y && y <= card.y + card.h {
                        let name = net.profiles[i].name.clone();
                        return Some(NetworkHit::ProfileActivate(net.ssid.clone(), name));
                    }
                }
                if has_band_selector(net) {
                    for entry in &net.bands {
                        if let Some(pill) =
                            band_pill_rect(net, entry.band, inner_x, inner_w, body_top, scale)
                        {
                            if x >= pill.x
                                && x <= pill.x + pill.w
                                && y >= pill.y
                                && y <= pill.y + pill.h
                            {
                                return Some(NetworkHit::BandPill(net.ssid.clone(), entry.band));
                            }
                        }
                    }
                }
                let btn = connect_button_rect(net, inner_x, inner_w, body_top, scale);
                if x >= btn.x && x <= btn.x + btn.w && y >= btn.y && y <= btn.y + btn.h {
                    return Some(NetworkHit::ConnectButton(net.ssid.clone()));
                }
                return Some(NetworkHit::Row(net.ssid.clone()));
            }
        }

        row_y += header_h + extra;
    }
    None
}
