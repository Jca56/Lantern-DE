//! Click-expand panel drawing, hit-testing, and layout for the WiFi
//! control. Pure-render code: all backend state lives on [`Wifi`],
//! which this module only reads from. (The inline tile + shared
//! signal icon live in `super::tile`.)

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::modal::draw_modal;
use super::tile::{draw_signal_icon, signal_to_bars};
use super::{Band, Network, Wifi, WifiState};

// ── Click-expand view ───────────────────────────────────────────────────────

const VIEW_TOP_PAD: f32 = 24.0;
const VIEW_HEADER_FONT: f32 = 26.0;
const VIEW_HEADER_BOTTOM_GAP: f32 = 12.0;
const ROW_HEIGHT: f32 = 64.0;
const ROW_FONT: f32 = 26.0;
const ROW_SIGNAL_SIZE: f32 = 32.0;
const ROW_SIGNAL_GAP: f32 = 16.0;
const ROW_LOCK_SIZE: f32 = 22.0;
const ROW_RIGHT_GAP: f32 = 12.0;
/// Cap on rendered network rows. We scroll past this, so it's purely a
/// "don't try to lay out the entire 200-AP city scan" sanity bound.
const MAX_NETWORK_ROWS: usize = 64;
/// Bottom padding the list reserves so the last row doesn't hug the
/// panel edge.
const LIST_BOTTOM_PAD: f32 = 16.0;
/// Logical px reserved for the expanded detail+button area beneath
/// the row header. One detail line per displayed property.
const EXPAND_PAD_TOP: f32 = 12.0;
const EXPAND_PAD_BOTTOM: f32 = 14.0;
const EXPAND_LINE_GAP: f32 = 8.0;
const EXPAND_DETAIL_FONT: f32 = 19.0;
const EXPAND_LABEL_W_FRAC: f32 = 0.28;
const EXPAND_BUTTON_TOP_GAP: f32 = 14.0;
const EXPAND_BUTTON_H: f32 = 44.0;
const EXPAND_BUTTON_FONT: f32 = 22.0;
const EXPAND_BUTTON_W: f32 = 160.0;
/// Band-selector pills sit between the details list and the Connect
/// button. Shown only when an SSID is advertised on multiple bands.
const BAND_ROW_TOP_GAP: f32 = 14.0;
const BAND_PILL_H: f32 = 36.0;
const BAND_PILL_W: f32 = 72.0;
const BAND_PILL_GAP: f32 = 8.0;
const BAND_PILL_FONT: f32 = 18.0;
const BAND_LABEL_FONT: f32 = 18.0;

/// "VPN: ON/OFF" pill on the right edge of the header row.
const VPN_LABEL_FONT: f32 = 22.0;
/// Hit-zone padding around the VPN label so it's comfortable to click.
const VPN_HIT_PAD_X: f32 = 8.0;
const VPN_HIT_PAD_Y: f32 = 4.0;

/// Width fraction (of the expanded inner row) for the left column
/// (details + band pills + Connect button). The right column hosts
/// the top-BSSID cards.
const LEFT_COL_FRAC: f32 = 0.58;
/// Column-gutter padding between left and right columns.
const COL_GUTTER: f32 = 14.0;
/// BSSID card constants (right column).
const BSSID_HEADER_FONT: f32 = 18.0;
const BSSID_HEADER_BOTTOM_GAP: f32 = 6.0;
const BSSID_CARD_H: f32 = 56.0;
const BSSID_CARD_GAP: f32 = 6.0;
const BSSID_MAC_FONT: f32 = 17.0;
const BSSID_META_FONT: f32 = 14.0;
const BSSID_LOCK_SIZE: f32 = 22.0;
const BSSID_LOCK_PAD: f32 = 10.0;
const MAX_BSSID_CARDS: usize = 5;

/// Saved-profile card constants.
const PROFILE_SECTION_TOP_GAP: f32 = 12.0;
const PROFILE_HEADER_FONT: f32 = 18.0;
const PROFILE_HEADER_BOTTOM_GAP: f32 = 6.0;
const PROFILE_CARD_H: f32 = 56.0;
const PROFILE_CARD_GAP: f32 = 6.0;
const PROFILE_NAME_FONT: f32 = 17.0;
const PROFILE_META_FONT: f32 = 14.0;
const PROFILE_DELETE_SIZE: f32 = 22.0;
const PROFILE_DELETE_PAD: f32 = 10.0;
const PROFILE_ACTIVE_DOT: f32 = 8.0;
const MAX_PROFILE_CARDS: usize = 6;

/// Y-coordinate (physical px) of the first network row, BEFORE scroll
/// is applied. Header sits above the list and doesn't scroll.
pub fn row_list_top_y(panel_top_y: f32, scale: f32) -> f32 {
    panel_top_y + VIEW_TOP_PAD * scale + VIEW_HEADER_FONT * scale + VIEW_HEADER_BOTTOM_GAP * scale
}

/// Total content height of the visible-network list in physical px,
/// including any expanded rows. Used to compute scrollbar geometry +
/// the max scroll offset.
pub fn content_height(wifi: &Wifi, scale: f32) -> f32 {
    let row_h = ROW_HEIGHT * scale;
    let mut total = 0.0;
    for net in wifi.networks().iter().take(MAX_NETWORK_ROWS) {
        let is_expanded = wifi.expanded_ssid.as_deref() == Some(net.ssid.as_str());
        let extra = if is_expanded { expanded_extra_height(net, scale) } else { 0.0 };
        total += row_h + extra;
    }
    total
}

/// Maximum logical-px scroll offset given a viewport height in physical
/// px. Returns logical px so the wheel handler can clamp directly to it.
pub fn max_scroll(wifi: &Wifi, viewport_h: f32, scale: f32) -> f32 {
    let total = content_height(wifi, scale);
    let extra = (total - viewport_h).max(0.0);
    extra / scale
}

/// All non-empty detail rows for an expanded network, as
/// (label, value) pairs. Pulls per-band fields from the currently
/// selected band so the panel updates when the user flips pills.
fn detail_rows(net: &Network) -> Vec<(&'static str, String)> {
    // When the user has pinned a BSSID, prefer it for headline details;
    // otherwise show the strongest BSSID of the selected band.
    let pinned_entry = net
        .pinned_bssid
        .as_ref()
        .and_then(|mac| net.aps.iter().find(|a| &a.bssid == mac));
    let band_entry = net
        .bands
        .iter()
        .find(|b| b.band == net.selected_band)
        .or_else(|| net.bands.first());
    let entry = pinned_entry.or(band_entry);

    let mut rows: Vec<(&'static str, String)> = Vec::new();
    rows.push((
        "Status",
        if net.in_use {
            "Connected".into()
        } else if net.saved {
            "Saved".into()
        } else {
            "Not saved".into()
        },
    ));
    let signal = entry.map(|e| e.signal).unwrap_or(net.signal);
    rows.push(("Signal", format!("{}%", signal)));
    let security = if net.flags_summary.is_empty() {
        if net.security.is_empty() || net.security == "--" {
            "Open".to_string()
        } else {
            net.security.clone()
        }
    } else {
        net.flags_summary.clone()
    };
    rows.push(("Security", security));
    if let Some(e) = entry {
        if !e.bssid.is_empty() {
            rows.push(("BSSID", e.bssid.clone()));
        }
    }
    if !net.mode.is_empty() {
        rows.push(("Mode", net.mode.clone()));
    }
    if let Some(e) = entry {
        if !e.channel.is_empty() {
            rows.push(("Channel", e.channel.clone()));
        }
        if !e.frequency.is_empty() {
            rows.push((
                "Frequency",
                format!("{} ({})", e.frequency, e.band.long_label()),
            ));
        }
        if !e.rate.is_empty() {
            rows.push(("Rate", e.rate.clone()));
        }
    }
    // Access-point summary — only meaningful when we heard more than one.
    if net.aps.len() > 1 {
        rows.push(("Access pts", format!("{} BSSIDs", net.aps.len())));
    }
    // Show the pin status explicitly so it doesn't surprise the user.
    if let Some(mac) = &net.pinned_bssid {
        rows.push(("Pinned to", mac.clone()));
    }
    rows
}

/// True when the network has more than one band and the user should
/// see a band-selector pill row in the expanded view.
fn has_band_selector(net: &Network) -> bool {
    net.bands.len() > 1
}

/// Physical-px height contribution of the band-selector pill row.
/// Zero when there's no choice to make.
fn band_row_height(net: &Network, scale: f32) -> f32 {
    if has_band_selector(net) {
        BAND_ROW_TOP_GAP * scale + BAND_PILL_H * scale
    } else {
        0.0
    }
}

/// Logical height of the left-column body inside the expand panel.
fn left_column_height(net: &Network, scale: f32) -> f32 {
    let n = detail_rows(net).len() as f32;
    let line_h = EXPAND_DETAIL_FONT * scale + EXPAND_LINE_GAP * scale;
    EXPAND_PAD_TOP * scale
        + n * line_h
        + band_row_height(net, scale)
        + EXPAND_BUTTON_TOP_GAP * scale
        + EXPAND_BUTTON_H * scale
}

/// Number of BSSID cards that'll render in the right column.
fn visible_bssid_card_count(net: &Network) -> usize {
    net.aps.len().min(MAX_BSSID_CARDS)
}

fn visible_profile_card_count(net: &Network) -> usize {
    net.profiles.len().min(MAX_PROFILE_CARDS)
}

/// Logical height of just the BSSID block in the right column.
fn bssid_block_height(net: &Network, scale: f32) -> f32 {
    let n = visible_bssid_card_count(net);
    if n == 0 {
        return 0.0;
    }
    BSSID_HEADER_FONT * scale
        + BSSID_HEADER_BOTTOM_GAP * scale
        + (n as f32) * BSSID_CARD_H * scale
        + ((n.saturating_sub(1)) as f32) * BSSID_CARD_GAP * scale
}

/// Logical height of just the profile block in the right column.
fn profile_block_height(net: &Network, scale: f32) -> f32 {
    let n = visible_profile_card_count(net);
    if n == 0 {
        return 0.0;
    }
    PROFILE_HEADER_FONT * scale
        + PROFILE_HEADER_BOTTOM_GAP * scale
        + (n as f32) * PROFILE_CARD_H * scale
        + ((n.saturating_sub(1)) as f32) * PROFILE_CARD_GAP * scale
}

/// Logical height of the right-column body (BSSIDs + Profiles).
fn right_column_height(net: &Network, scale: f32) -> f32 {
    let bssid = bssid_block_height(net, scale);
    let profile = profile_block_height(net, scale);
    if bssid <= 0.0 && profile <= 0.0 {
        return 0.0;
    }
    let gap = if bssid > 0.0 && profile > 0.0 {
        PROFILE_SECTION_TOP_GAP * scale
    } else {
        0.0
    };
    EXPAND_PAD_TOP * scale + bssid + gap + profile
}

/// Total expanded-section height in physical px (header excluded —
/// this is purely the part that's added when the row opens). Uses the
/// taller of the two columns so neither gets clipped.
fn expanded_extra_height(net: &Network, scale: f32) -> f32 {
    let body = left_column_height(net, scale).max(right_column_height(net, scale));
    body + EXPAND_PAD_BOTTOM * scale
}

/// Rect of the i-th BSSID card inside the expanded body's right column.
fn bssid_card_rect(
    inner_x: f32,
    inner_w: f32,
    body_top: f32,
    scale: f32,
    i: usize,
) -> Rect {
    let gutter = COL_GUTTER * scale;
    let left_w = inner_w * LEFT_COL_FRAC;
    let right_x = inner_x + left_w + gutter;
    let right_w = (inner_x + inner_w) - right_x - 6.0 * scale;
    let top = body_top
        + EXPAND_PAD_TOP * scale
        + BSSID_HEADER_FONT * scale
        + BSSID_HEADER_BOTTOM_GAP * scale;
    let stride = BSSID_CARD_H * scale + BSSID_CARD_GAP * scale;
    Rect::new(right_x, top + i as f32 * stride, right_w, BSSID_CARD_H * scale)
}

/// Rect of the lock toggle inside a BSSID card.
fn bssid_lock_rect(card: Rect, scale: f32) -> Rect {
    let size = BSSID_LOCK_SIZE * scale;
    let pad = BSSID_LOCK_PAD * scale;
    let x = card.x + card.w - size - pad;
    let y = card.y + (card.h - size) / 2.0;
    Rect::new(x, y, size, size)
}

/// Rect of the i-th saved-profile card. Sits below the BSSID cards in
/// the right column with a section gap between them.
fn profile_card_rect(
    inner_x: f32,
    inner_w: f32,
    body_top: f32,
    scale: f32,
    bssid_count: usize,
    i: usize,
) -> Rect {
    let gutter = COL_GUTTER * scale;
    let left_w = inner_w * LEFT_COL_FRAC;
    let right_x = inner_x + left_w + gutter;
    let right_w = (inner_x + inner_w) - right_x - 6.0 * scale;

    let mut y = body_top + EXPAND_PAD_TOP * scale;
    if bssid_count > 0 {
        y += BSSID_HEADER_FONT * scale + BSSID_HEADER_BOTTOM_GAP * scale;
        y += bssid_count as f32 * BSSID_CARD_H * scale
            + (bssid_count.saturating_sub(1)) as f32 * BSSID_CARD_GAP * scale;
        y += PROFILE_SECTION_TOP_GAP * scale;
    }
    // Account for the profile-section header before the cards.
    y += PROFILE_HEADER_FONT * scale + PROFILE_HEADER_BOTTOM_GAP * scale;
    let stride = PROFILE_CARD_H * scale + PROFILE_CARD_GAP * scale;
    Rect::new(right_x, y + i as f32 * stride, right_w, PROFILE_CARD_H * scale)
}

/// Rect of the delete-X button on a profile card.
fn profile_delete_rect(card: Rect, scale: f32) -> Rect {
    let size = PROFILE_DELETE_SIZE * scale;
    let pad = PROFILE_DELETE_PAD * scale;
    let x = card.x + card.w - size - pad;
    let y = card.y + (card.h - size) / 2.0;
    Rect::new(x, y, size, size)
}

/// What was clicked inside the WiFi network list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkHit {
    /// The header part of a row (any area outside the Connect button
    /// in the expanded section). Caller toggles the expanded ssid.
    Row(String),
    /// The Connect / Disconnect button inside the expanded section.
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
    /// VPN ON/OFF indicator on the header row.
    ToggleVpn,
}

/// Rect for the VPN ON/OFF indicator on the header row. Returns `None`
/// when the indicator isn't being drawn (Mullvad CLI absent). Anchored
/// to the right edge of the panel; width is generous so "VPN: OFF" fits
/// comfortably without measuring text from the hit-test path.
pub fn vpn_hit_rect(wifi: &Wifi, panel: Rect, panel_top_y: f32, scale: f32) -> Option<Rect> {
    wifi.vpn_connected?;
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let header_font = VIEW_HEADER_FONT * scale;
    let label_font = VPN_LABEL_FONT * scale;
    let header_y = panel_top_y + VIEW_TOP_PAD * scale;
    // Approximate width — wide enough for "VPN: OFF" at any plausible
    // scale plus a comfy hit pad.
    let w = label_font * 5.5 + VPN_HIT_PAD_X * 2.0 * scale;
    let h = header_font + VPN_HIT_PAD_Y * 2.0 * scale;
    let x = panel.x + panel.w - pad - w + VPN_HIT_PAD_X * scale;
    let y = header_y - VPN_HIT_PAD_Y * scale;
    Some(Rect::new(x, y, w, h))
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

    // VPN indicator sits above the network list, on the header row, so
    // check it BEFORE the `y < list_top` early-return below.
    if let Some(r) = vpn_hit_rect(wifi, panel, panel_top_y, scale) {
        if x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h {
            return Some(NetworkHit::ToggleVpn);
        }
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
        let extra = if is_expanded { expanded_extra_height(net, scale) } else { 0.0 };

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
                    if x >= lock.x
                        && x <= lock.x + lock.w
                        && y >= lock.y
                        && y <= lock.y + lock.h
                    {
                        let mac = net.aps[i].bssid.clone();
                        return Some(NetworkHit::LockBssid(net.ssid.clone(), mac));
                    }
                }
                // Profile cards (delete X first; otherwise activate).
                let profile_n = visible_profile_card_count(net);
                for i in 0..profile_n {
                    let card = profile_card_rect(
                        inner_x, inner_w, body_top, scale, card_n, i,
                    );
                    let del = profile_delete_rect(card, scale);
                    if x >= del.x
                        && x <= del.x + del.w
                        && y >= del.y
                        && y <= del.y + del.h
                    {
                        let uuid = net.profiles[i].uuid.clone();
                        return Some(NetworkHit::ProfileDelete(
                            net.ssid.clone(),
                            uuid,
                        ));
                    }
                    if x >= card.x
                        && x <= card.x + card.w
                        && y >= card.y
                        && y <= card.y + card.h
                    {
                        let name = net.profiles[i].name.clone();
                        return Some(NetworkHit::ProfileActivate(
                            net.ssid.clone(),
                            name,
                        ));
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
                                return Some(NetworkHit::BandPill(
                                    net.ssid.clone(),
                                    entry.band,
                                ));
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

/// Y-coordinate of the top of the band-selector pill row (the row
/// itself starts after `BAND_ROW_TOP_GAP`). Used by both draw + hit
/// test so they share one truth.
fn band_row_top(net: &Network, body_top: f32, scale: f32) -> f32 {
    let n = detail_rows(net).len() as f32;
    let line_h = EXPAND_DETAIL_FONT * scale + EXPAND_LINE_GAP * scale;
    body_top + EXPAND_PAD_TOP * scale + n * line_h
}

/// Logical width of the left column inside the expanded body.
fn left_col_width(inner_w: f32) -> f32 {
    inner_w * LEFT_COL_FRAC
}

/// Rect of a specific band pill within an expanded row body. Pills are
/// laid out left-to-right in the order they appear in `net.bands`
/// (strongest first), starting after a small "Band" label.
fn band_pill_rect(
    net: &Network,
    band: Band,
    inner_x: f32,
    inner_w: f32,
    body_top: f32,
    scale: f32,
) -> Option<Rect> {
    if !has_band_selector(net) {
        return None;
    }
    let idx = net.bands.iter().position(|b| b.band == band)?;
    let row_top = band_row_top(net, body_top, scale) + BAND_ROW_TOP_GAP * scale;
    let pill_w = BAND_PILL_W * scale;
    let pill_h = BAND_PILL_H * scale;
    let gap = BAND_PILL_GAP * scale;
    // Label column eats the same indent as the detail rows so the
    // pills line up under the values.
    let label_w = left_col_width(inner_w) * EXPAND_LABEL_W_FRAC;
    let pills_x = inner_x + 16.0 * scale + label_w;
    let x = pills_x + idx as f32 * (pill_w + gap);
    Some(Rect::new(x, row_top, pill_w, pill_h))
}

/// Compute the Connect button rect for an expanded row body that
/// starts at `body_top` (just under the row header). The button sits
/// at the bottom-right of the LEFT column so it doesn't overlap the
/// BSSID list on the right.
fn connect_button_rect(net: &Network, inner_x: f32, inner_w: f32, body_top: f32, scale: f32) -> Rect {
    let after_details = band_row_top(net, body_top, scale);
    let after_bands = after_details + band_row_height(net, scale);
    let btn_y = after_bands + EXPAND_BUTTON_TOP_GAP * scale;
    let btn_w = EXPAND_BUTTON_W * scale;
    let btn_h = EXPAND_BUTTON_H * scale;
    let left_w = left_col_width(inner_w);
    let btn_x = inner_x + left_w - btn_w - EXPAND_PAD_TOP * scale;
    Rect::new(btn_x, btn_y, btn_w, btn_h)
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    wifi: &Wifi,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let header_font = VIEW_HEADER_FONT * scale;
    let header_gap = VIEW_HEADER_BOTTOM_GAP * scale;
    let row_h = ROW_HEIGHT * scale;
    let row_font = ROW_FONT * scale;
    let signal_size = ROW_SIGNAL_SIZE * scale;
    let signal_gap = ROW_SIGNAL_GAP * scale;
    let lock_size = ROW_LOCK_SIZE * scale;
    let right_gap = ROW_RIGHT_GAP * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

    // ── Header: "Wi-Fi" + connected SSID ──
    let header = match wifi.state() {
        WifiState::Connected { ssid, .. } => format!("Wi-Fi · {}", ssid),
        WifiState::Disconnected => "Wi-Fi · Disconnected".to_string(),
        WifiState::Off => "Wi-Fi · Off".to_string(),
    };
    let header_y = panel_top_y + VIEW_TOP_PAD * scale;
    text.queue(
        &header,
        header_font,
        inner_x,
        header_y,
        white.with_alpha(alpha),
        inner_w,
        surface_w,
        surface_h,
    );

    // VPN ON/OFF indicator on the far right of the header row. Only
    // rendered when the `mullvad` CLI is present; otherwise the slot is
    // simply empty.
    if let Some(connected) = wifi.vpn_connected {
        let vpn_label = if connected { "VPN: ON" } else { "VPN: OFF" };
        let vpn_color = if connected {
            Color::from_rgb8(0x4c, 0xd9, 0x64).with_alpha(alpha)
        } else {
            Color::from_rgb8(0xff, 0x4d, 0x4d).with_alpha(alpha)
        };
        let vpn_font = VPN_LABEL_FONT * scale;
        let lbl_w = text.measure_width(vpn_label, vpn_font);
        let lbl_x = panel.x + panel.w - pad - lbl_w;
        // Vertically center the VPN label against the header text's
        // visual mid-line (header_y is the text top edge).
        let lbl_y = header_y + (header_font - vpn_font) / 2.0;
        text.queue(
            vpn_label, vpn_font, lbl_x, lbl_y, vpn_color, lbl_w,
            surface_w, surface_h,
        );
    }

    // ── Network rows ──
    let list_top = header_y + header_font + header_gap;
    let _ = list_top; // keep symmetry with the layout helper above

    if wifi.networks().is_empty() {
        let msg_y = row_list_top_y(panel_top_y, scale);
        text.queue(
            "Scanning…",
            row_font,
            inner_x,
            msg_y,
            muted,
            inner_w,
            surface_w,
            surface_h,
        );
        return msg_y + row_font;
    }

    // List clip: from row_list_top_y down to the bottom of the panel
    // (minus a small bottom pad). Rows that scroll out of this rect
    // get clipped rather than bleeding into chrome below.
    let list_top = row_list_top_y(panel_top_y, scale);
    let list_bottom = panel.y + panel.h - LIST_BOTTOM_PAD * scale;
    let viewport_h = (list_bottom - list_top).max(0.0);
    let max = max_scroll(wifi, viewport_h, scale);
    let scroll_clamped = wifi.scroll.clamp(0.0, max);
    let scroll_px = scroll_clamped * scale;
    let list_clip = Rect::new(panel.x, list_top, panel.w, viewport_h);
    painter.push_clip(list_clip);
    text.push_clip([list_clip.x, list_clip.y, list_clip.w, list_clip.h]);

    let mut row_y = list_top - scroll_px;
    for (i, net) in wifi.networks().iter().take(MAX_NETWORK_ROWS).enumerate() {
        let is_expanded = wifi.expanded_ssid.as_deref() == Some(net.ssid.as_str());
        let extra_h = if is_expanded { expanded_extra_height(net, scale) } else { 0.0 };
        let total_h = row_h + extra_h;
        // The full container (header + expanded body) — used for
        // bg fill so the expanded body and header share a plate.
        let container_rect = Rect::new(inner_x, row_y, inner_w, total_h);
        // Header-only rect — used for the gold tint on the in-use
        // row (we don't tint the whole expanded panel).
        let row_rect = Rect::new(inner_x, row_y, inner_w, row_h);

        // Subtle row-stripe for readability + brighter highlight on the
        // currently-connected row.
        let is_hovered = wifi.hovered_ssid.as_deref() == Some(net.ssid.as_str());
        if is_expanded {
            // Darker grey plate behind the expanded card so the BSSID
            // grid + Connect button read clearly against the panel bg.
            painter.rect_filled(
                container_rect,
                10.0 * scale,
                Color::rgba(0.0, 0.0, 0.0, 0.35 * alpha),
            );
        } else if i % 2 == 0 {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                white.with_alpha(0.04 * alpha),
            );
        }
        if net.in_use {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                gold.with_alpha(0.18 * alpha),
            );
        }
        // Hover highlight sits on top of the stripe so it reads
        // consistently across odd/even rows. Skipped for the in-use
        // row since the gold tint already signals selection clearly,
        // and skipped while expanded since the container plate
        // already differentiates the row.
        if is_hovered && !net.in_use && !is_expanded {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                white.with_alpha(0.10 * alpha),
            );
        }

        // Signal % to the left of the bars icon. Sized smaller than
        // the SSID so it reads as metadata, not a focal element.
        let pct_str = format!("{}%", net.signal);
        let pct_font = row_font * 0.75;
        let pct_w = text.measure_width(&pct_str, pct_font);
        // Pass a slightly padded max_width so the trailing "%" never
        // wraps to a second line when the measured width sits right on
        // the glyph boundary.
        let pct_box = pct_w + 6.0 * scale;

        let pct_x = inner_x + signal_gap;
        text.queue(
            &pct_str,
            pct_font,
            pct_x,
            row_y + (row_h - pct_font) / 2.0,
            muted,
            pct_box,
            surface_w,
            surface_h,
        );

        // Signal icon, just to the right of the percent (use the
        // padded box width so spacing stays consistent).
        let icon_x = pct_x + pct_box + signal_gap * 0.6;
        let icon_y = row_y + (row_h - signal_size) / 2.0;
        let bars = signal_to_bars(net.signal);
        draw_signal_icon(painter, icon_x, icon_y, signal_size, signal_size, bars, alpha);

        // SSID label. The connected network gets a larger, gold label
        // so it stands out at a glance even before reading the badge.
        let label_x = icon_x + signal_size + signal_gap;
        let (ssid_font, ssid_color) = if net.in_use {
            (row_font * 1.20, gold.with_alpha(alpha))
        } else {
            (row_font, white.with_alpha(0.86 * alpha))
        };
        let label_y = row_y + (row_h - ssid_font) / 2.0;
        text.queue(
            &net.ssid,
            ssid_font,
            label_x,
            label_y,
            ssid_color,
            inner_w * 0.7,
            surface_w,
            surface_h,
        );

        // Right side, walking R→L: lock first (right-most), then
        // Saved/Connected/Connecting status text to its left.
        let mut right_x = inner_x + inner_w - right_gap;
        if !net.security.is_empty() && net.security != "--" {
            let lock_x = right_x - lock_size;
            let lock_y = row_y + (row_h - lock_size) / 2.0;
            draw_lock(painter, lock_x, lock_y, lock_size, lock_size, alpha);
            right_x = lock_x - right_gap;
        }
        // Status text: "Connecting…" wins over Connected/Saved while
        // a connect attempt is in flight to that ssid.
        let connecting_now = wifi.is_connecting_to(&net.ssid);
        if connecting_now {
            let s = "Connecting…";
            let f = row_font * 0.8;
            let w = text.measure_width(s, f);
            right_x -= w;
            text.queue(
                s,
                f,
                right_x,
                row_y + (row_h - f) / 2.0,
                gold.with_alpha(alpha),
                w,
                surface_w,
                surface_h,
            );
        } else if net.in_use {
            let s = "Connected";
            let f = row_font * 0.8;
            let w = text.measure_width(s, f);
            right_x -= w;
            text.queue(
                s,
                f,
                right_x,
                row_y + (row_h - f) / 2.0,
                gold.with_alpha(alpha),
                w,
                surface_w,
                surface_h,
            );
        } else if net.saved {
            let s = "Saved";
            let f = row_font * 0.7;
            let w = text.measure_width(s, f);
            right_x -= w;
            text.queue(
                s,
                f,
                right_x,
                row_y + (row_h - f) / 2.0,
                muted,
                w,
                surface_w,
                surface_h,
            );
        }
        let _ = right_x;

        // Expanded section: details list + Connect button (left column)
        // and Top-BSSID list (right column).
        if is_expanded {
            let body_top = row_y + row_h;
            let detail_font = EXPAND_DETAIL_FONT * scale;
            let line_h = detail_font + EXPAND_LINE_GAP * scale;
            let left_w = left_col_width(inner_w);
            let label_w = left_w * EXPAND_LABEL_W_FRAC;
            let mut dy = body_top + EXPAND_PAD_TOP * scale;
            let label_x = inner_x + 16.0 * scale;
            let value_x = label_x + label_w;

            for (label, value) in detail_rows(net) {
                text.queue(
                    label,
                    detail_font,
                    label_x,
                    dy,
                    muted,
                    label_w,
                    surface_w,
                    surface_h,
                );
                text.queue(
                    &value,
                    detail_font,
                    value_x,
                    dy,
                    white.with_alpha(0.92 * alpha),
                    left_w - (value_x - inner_x) - 16.0 * scale,
                    surface_w,
                    surface_h,
                );
                dy += line_h;
            }

            // ── Right column: top BSSIDs ──
            draw_right_column(
                painter, text, net, inner_x, inner_w, body_top, scale, alpha,
                surface_w, surface_h,
            );

            // Band-selector pills (only when 2+ bands available).
            if has_band_selector(net) {
                let pill_label_font = BAND_LABEL_FONT * scale;
                let pill_font = BAND_PILL_FONT * scale;
                let pill_radius = (BAND_PILL_H * scale) * 0.5;
                let row_top = band_row_top(net, body_top, scale) + BAND_ROW_TOP_GAP * scale;
                let pill_h = BAND_PILL_H * scale;
                // "Band" label left of the pills.
                text.queue(
                    "Band",
                    pill_label_font,
                    label_x,
                    row_top + (pill_h - pill_label_font) / 2.0,
                    muted,
                    label_w,
                    surface_w,
                    surface_h,
                );
                for entry in &net.bands {
                    let Some(pill) = band_pill_rect(net, entry.band, inner_x, inner_w, body_top, scale)
                    else {
                        continue;
                    };
                    let selected = entry.band == net.selected_band;
                    let bg = if selected {
                        gold.with_alpha(0.85 * alpha)
                    } else {
                        white.with_alpha(0.08 * alpha)
                    };
                    painter.rect_filled(pill, pill_radius, bg);
                    if !selected {
                        painter.rect_stroke_sdf(
                            pill,
                            pill_radius,
                            1.0 * scale,
                            white.with_alpha(0.18 * alpha),
                        );
                    }
                    let lbl = entry.band.short_label();
                    let lw = text.measure_width(lbl, pill_font);
                    let tx = pill.x + (pill.w - lw) / 2.0;
                    let ty = pill.y + (pill.h - pill_font) / 2.0;
                    let tcol = if selected {
                        Color::rgba(0.0, 0.0, 0.0, alpha)
                    } else {
                        white.with_alpha(0.90 * alpha)
                    };
                    text.queue(lbl, pill_font, tx, ty, tcol, pill.w, surface_w, surface_h);
                }
            }

            // Connect button.
            let btn = connect_button_rect(net, inner_x, inner_w, body_top, scale);
            let connecting_now = wifi.is_connecting_to(&net.ssid);
            let label = if connecting_now {
                "Connecting…"
            } else if net.in_use {
                "Connected"
            } else {
                "Connect"
            };
            let btn_font = EXPAND_BUTTON_FONT * scale;
            let btn_radius = 10.0 * scale;
            let bg = if connecting_now {
                gold.with_alpha(0.35 * alpha)
            } else if net.in_use {
                gold.with_alpha(0.30 * alpha)
            } else {
                gold.with_alpha(0.95 * alpha)
            };
            painter.rect_filled(btn, btn_radius, bg);
            let lw = text.measure_width(label, btn_font);
            text.queue(
                label,
                btn_font,
                btn.x + (btn.w - lw) / 2.0,
                btn.y + (btn.h - btn_font) / 2.0,
                Color::rgba(0.0, 0.0, 0.0, alpha),
                btn.w,
                surface_w,
                surface_h,
            );
        }

        row_y += row_h + extra_h;
    }

    if let Some(err) = wifi.last_error() {
        let err_y = row_y + row_font * 0.5;
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        text.queue(
            err,
            row_font * 0.85,
            inner_x,
            err_y,
            red,
            inner_w,
            surface_w,
            surface_h,
        );
    }

    painter.pop_clip();
    text.pop_clip();

    // Scrollbar — thin track on the right side of the panel.
    if max > 0.0 {
        let track_w = 4.0 * scale;
        let track_x = panel.x + panel.w - track_w - 6.0 * scale;
        painter.rect_filled(
            Rect::new(track_x, list_top, track_w, viewport_h),
            track_w / 2.0,
            white.with_alpha(0.06 * alpha),
        );
        let thumb_h = (viewport_h * viewport_h / (viewport_h + max * scale))
            .max(24.0 * scale);
        let thumb_y = list_top
            + (viewport_h - thumb_h)
                * (scroll_px / (max * scale)).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::new(track_x, thumb_y, track_w, thumb_h),
            track_w / 2.0,
            white.with_alpha(0.30 * alpha),
        );
    }

    let bottom = list_bottom;

    // Draw the password prompt on top of the network list when active.
    // Layer 1 so the modal's painter rects can occlude already-queued
    // text from the network list — see lntrn-render/TEXT_OCCLUSION_FIX.
    if let Some(prompt) = &wifi.prompt {
        painter.set_layer(1);
        text.set_layer(1);
        draw_modal(
            painter,
            text,
            prompt,
            panel,
            panel_top_y,
            scale,
            alpha,
            wifi.last_error(),
            surface_w,
            surface_h,
        );
    }

    bottom
}

/// Right column of the expanded panel: BSSID cards on top, saved-profile
/// cards below (each with delete X + click-to-activate).
#[allow(clippy::too_many_arguments)]
fn draw_right_column(
    painter: &mut Painter,
    text: &mut TextRenderer,
    net: &Network,
    inner_x: f32,
    inner_w: f32,
    body_top: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let n = visible_bssid_card_count(net);
    let profile_n = visible_profile_card_count(net);
    if n == 0 && profile_n == 0 {
        return;
    }
    let gutter = COL_GUTTER * scale;
    let left_w = left_col_width(inner_w);
    let right_x = inner_x + left_w + gutter;
    let right_w = (inner_x + inner_w) - right_x - 6.0 * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

    // BSSID column header.
    if n > 0 {
        let header_font = BSSID_HEADER_FONT * scale;
        let header_y = body_top + EXPAND_PAD_TOP * scale;
        text.queue(
            if n == 1 { "Access point" } else { "Top access points" },
            header_font,
            right_x,
            header_y,
            muted,
            right_w,
            surface_w,
            surface_h,
        );
    }

    let mac_font = BSSID_MAC_FONT * scale;
    let meta_font = BSSID_META_FONT * scale;

    for i in 0..n {
        let ap = &net.aps[i];
        let card = bssid_card_rect(inner_x, inner_w, body_top, scale, i);
        let pinned = net.pinned_bssid.as_deref() == Some(ap.bssid.as_str());
        let radius = 10.0 * scale;

        // Card background — gold tint when pinned, otherwise neutral.
        let bg = if pinned {
            gold.with_alpha(0.20 * alpha)
        } else {
            white.with_alpha(0.06 * alpha)
        };
        painter.rect_filled(card, radius, bg);
        if pinned {
            painter.rect_stroke_sdf(card, radius, 1.4 * scale, gold.with_alpha(0.80 * alpha));
        } else {
            painter.rect_stroke_sdf(card, radius, 1.0 * scale, white.with_alpha(0.14 * alpha));
        }

        // BSSID MAC.
        let mac_x = card.x + 10.0 * scale;
        let mac_y = card.y + 8.0 * scale;
        text.queue(
            &ap.bssid,
            mac_font,
            mac_x,
            mac_y,
            white.with_alpha(0.95 * alpha),
            card.w - 16.0 * scale - BSSID_LOCK_SIZE * scale - BSSID_LOCK_PAD * scale,
            surface_w,
            surface_h,
        );

        // Meta line: signal + channel + band.
        let meta = format!(
            "{}%  ·  ch {}  ·  {}",
            ap.signal,
            ap.channel,
            ap.band.short_label()
        );
        let meta_y = mac_y + mac_font + 4.0 * scale;
        text.queue(
            &meta,
            meta_font,
            mac_x,
            meta_y,
            muted,
            card.w - 16.0 * scale - BSSID_LOCK_SIZE * scale - BSSID_LOCK_PAD * scale,
            surface_w,
            surface_h,
        );

        // Lock-pin button: gold-filled when pinned, neutral outline otherwise.
        let lock = bssid_lock_rect(card, scale);
        if pinned {
            painter.rect_filled(lock, lock.w / 2.0, gold.with_alpha(0.85 * alpha));
        }
        draw_lock(
            painter,
            lock.x,
            lock.y,
            lock.w,
            lock.h,
            if pinned { 1.6 * alpha } else { alpha },
        );
    }

    // ── Saved profiles section ──
    if profile_n > 0 {
        // Section header: align under the BSSID block plus a gap.
        let mut header_y = body_top + EXPAND_PAD_TOP * scale;
        if n > 0 {
            header_y += BSSID_HEADER_FONT * scale + BSSID_HEADER_BOTTOM_GAP * scale;
            header_y += n as f32 * BSSID_CARD_H * scale
                + (n.saturating_sub(1)) as f32 * BSSID_CARD_GAP * scale;
            header_y += PROFILE_SECTION_TOP_GAP * scale;
        }
        let p_header_font = PROFILE_HEADER_FONT * scale;
        let label = if profile_n == 1 {
            "Saved profile".to_string()
        } else {
            format!("Saved profiles ({})", net.profiles.len())
        };
        text.queue(
            &label,
            p_header_font,
            right_x,
            header_y,
            muted,
            right_w,
            surface_w,
            surface_h,
        );

        let p_name_font = PROFILE_NAME_FONT * scale;
        let p_meta_font = PROFILE_META_FONT * scale;
        for i in 0..profile_n {
            let p = &net.profiles[i];
            let card = profile_card_rect(inner_x, inner_w, body_top, scale, n, i);
            let radius = 10.0 * scale;

            let bg = if p.active {
                gold.with_alpha(0.18 * alpha)
            } else {
                white.with_alpha(0.06 * alpha)
            };
            painter.rect_filled(card, radius, bg);
            if p.active {
                painter.rect_stroke_sdf(
                    card,
                    radius,
                    1.4 * scale,
                    gold.with_alpha(0.80 * alpha),
                );
            } else {
                painter.rect_stroke_sdf(
                    card,
                    radius,
                    1.0 * scale,
                    white.with_alpha(0.14 * alpha),
                );
            }

            // Active dot on the left.
            let dot_d = PROFILE_ACTIVE_DOT * scale;
            let dot_cx = card.x + 12.0 * scale + dot_d / 2.0;
            let dot_cy = card.y + card.h / 2.0;
            let dot_color = if p.active {
                gold.with_alpha(0.95 * alpha)
            } else {
                white.with_alpha(0.20 * alpha)
            };
            painter.rect_filled(
                Rect::new(
                    dot_cx - dot_d / 2.0,
                    dot_cy - dot_d / 2.0,
                    dot_d,
                    dot_d,
                ),
                dot_d / 2.0,
                dot_color,
            );

            // Name + meta.
            let text_x = dot_cx + dot_d / 2.0 + 10.0 * scale;
            let text_max_w =
                card.w - (text_x - card.x) - PROFILE_DELETE_SIZE * scale - PROFILE_DELETE_PAD * scale - 6.0 * scale;
            let name_y = card.y + 8.0 * scale;
            text.queue(
                &p.name,
                p_name_font,
                text_x,
                name_y,
                white.with_alpha(0.95 * alpha),
                text_max_w.max(0.0),
                surface_w,
                surface_h,
            );

            // Meta line — band/bssid pins, or "tap to switch" hint when no pins.
            let mut meta = String::new();
            if let Some(b) = &p.pinned_band {
                meta.push_str(match b.as_str() {
                    "bg" => "Band: 2.4 GHz",
                    "a" => "Band: 5/6 GHz",
                    _ => "",
                });
            }
            if let Some(mac) = &p.pinned_bssid {
                if !meta.is_empty() {
                    meta.push_str(" · ");
                }
                meta.push_str("BSSID ");
                meta.push_str(mac);
            }
            if meta.is_empty() {
                meta = if p.active {
                    "Active".to_string()
                } else {
                    "Tap to activate".to_string()
                };
            }
            let meta_y = name_y + p_name_font + 4.0 * scale;
            text.queue(
                &meta,
                p_meta_font,
                text_x,
                meta_y,
                muted,
                text_max_w.max(0.0),
                surface_w,
                surface_h,
            );

            // Delete X chip.
            let del = profile_delete_rect(card, scale);
            painter.rect_filled(
                del,
                del.w / 2.0,
                Color::from_rgb8(0xd0, 0x4a, 0x4a).with_alpha(0.18 * alpha),
            );
            painter.rect_stroke_sdf(
                del,
                del.w / 2.0,
                1.0 * scale,
                Color::from_rgb8(0xd0, 0x4a, 0x4a).with_alpha(0.65 * alpha),
            );
            let arm = del.w * 0.28;
            let cx = del.x + del.w / 2.0;
            let cy = del.y + del.h / 2.0;
            let stroke = 1.8 * scale;
            let red = Color::from_rgb8(0xd0, 0x4a, 0x4a).with_alpha(0.95 * alpha);
            painter.line_round(cx - arm, cy - arm, cx + arm, cy + arm, stroke, red);
            painter.line_round(cx + arm, cy - arm, cx - arm, cy + arm, stroke, red);
        }
    }
}

/// Tiny lock icon — body rect + shackle arch. Pure shapes.
fn draw_lock(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.65 * alpha);
    // Body: bottom 60 % of the box, full width.
    let body_y = y + h * 0.40;
    let body_h = h * 0.60;
    painter.rect_filled(
        Rect::new(x, body_y, w, body_h),
        w * 0.16,
        color,
    );
    // Shackle: U-shape on top, drawn as a stroked rounded rect with the
    // bottom hidden behind the body. We approximate by drawing a rect
    // outline above the body.
    let shackle_x = x + w * 0.18;
    let shackle_w = w * 0.64;
    let shackle_h = h * 0.50;
    painter.rect_stroke_sdf(
        Rect::new(shackle_x, y, shackle_w, shackle_h),
        shackle_w * 0.45,
        w * 0.13,
        color,
    );
}
