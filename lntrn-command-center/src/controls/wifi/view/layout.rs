//! Layout math for the WiFi panel — produces the rects + heights the
//! draw and hit-test paths share. No painter / text calls here.

use lntrn_render::Rect;

use crate::controls::wifi::{Band, Network, Wifi};

use super::{
    BAND_PILL_GAP, BAND_PILL_H, BAND_PILL_W, BAND_ROW_TOP_GAP, BSSID_CARD_GAP, BSSID_CARD_H,
    BSSID_HEADER_BOTTOM_GAP, BSSID_HEADER_FONT, BSSID_LOCK_PAD, BSSID_LOCK_SIZE, COL_GUTTER,
    EXPAND_BUTTON_H, EXPAND_BUTTON_TOP_GAP, EXPAND_BUTTON_W, EXPAND_DETAIL_FONT,
    EXPAND_LABEL_W_FRAC, EXPAND_LINE_GAP, EXPAND_PAD_BOTTOM, EXPAND_PAD_TOP, LEFT_COL_FRAC,
    MAX_BSSID_CARDS, MAX_NETWORK_ROWS, MAX_PROFILE_CARDS, PROFILE_CARD_GAP, PROFILE_CARD_H,
    PROFILE_DELETE_PAD, PROFILE_DELETE_SIZE, PROFILE_HEADER_BOTTOM_GAP, PROFILE_HEADER_FONT,
    PROFILE_SECTION_TOP_GAP, ROW_HEIGHT, VIEW_HEADER_BOTTOM_GAP, VIEW_HEADER_FONT, VIEW_TOP_PAD,
};

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
        let extra = if is_expanded {
            expanded_extra_height(net, scale)
        } else {
            0.0
        };
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
pub(super) fn detail_rows(net: &Network) -> Vec<(&'static str, String)> {
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
pub(super) fn has_band_selector(net: &Network) -> bool {
    net.bands.len() > 1
}

/// Physical-px height contribution of the band-selector pill row.
/// Zero when there's no choice to make.
pub(super) fn band_row_height(net: &Network, scale: f32) -> f32 {
    if has_band_selector(net) {
        BAND_ROW_TOP_GAP * scale + BAND_PILL_H * scale
    } else {
        0.0
    }
}

/// Logical height of the left-column body inside the expand panel.
pub(super) fn left_column_height(net: &Network, scale: f32) -> f32 {
    let n = detail_rows(net).len() as f32;
    let line_h = EXPAND_DETAIL_FONT * scale + EXPAND_LINE_GAP * scale;
    EXPAND_PAD_TOP * scale
        + n * line_h
        + band_row_height(net, scale)
        + EXPAND_BUTTON_TOP_GAP * scale
        + EXPAND_BUTTON_H * scale
}

/// Number of BSSID cards that'll render in the right column.
pub(super) fn visible_bssid_card_count(net: &Network) -> usize {
    net.aps.len().min(MAX_BSSID_CARDS)
}

pub(super) fn visible_profile_card_count(net: &Network) -> usize {
    net.profiles.len().min(MAX_PROFILE_CARDS)
}

/// Logical height of just the BSSID block in the right column.
pub(super) fn bssid_block_height(net: &Network, scale: f32) -> f32 {
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
pub(super) fn profile_block_height(net: &Network, scale: f32) -> f32 {
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
pub(super) fn right_column_height(net: &Network, scale: f32) -> f32 {
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
pub(super) fn expanded_extra_height(net: &Network, scale: f32) -> f32 {
    let body = left_column_height(net, scale).max(right_column_height(net, scale));
    body + EXPAND_PAD_BOTTOM * scale
}

/// Rect of the i-th BSSID card inside the expanded body's right column.
pub(super) fn bssid_card_rect(
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
    Rect::new(
        right_x,
        top + i as f32 * stride,
        right_w,
        BSSID_CARD_H * scale,
    )
}

/// Rect of the lock toggle inside a BSSID card.
pub(super) fn bssid_lock_rect(card: Rect, scale: f32) -> Rect {
    let size = BSSID_LOCK_SIZE * scale;
    let pad = BSSID_LOCK_PAD * scale;
    let x = card.x + card.w - size - pad;
    let y = card.y + (card.h - size) / 2.0;
    Rect::new(x, y, size, size)
}

/// Rect of the i-th saved-profile card. Sits below the BSSID cards in
/// the right column with a section gap between them.
pub(super) fn profile_card_rect(
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
    Rect::new(
        right_x,
        y + i as f32 * stride,
        right_w,
        PROFILE_CARD_H * scale,
    )
}

/// Rect of the delete-X button on a profile card.
pub(super) fn profile_delete_rect(card: Rect, scale: f32) -> Rect {
    let size = PROFILE_DELETE_SIZE * scale;
    let pad = PROFILE_DELETE_PAD * scale;
    let x = card.x + card.w - size - pad;
    let y = card.y + (card.h - size) / 2.0;
    Rect::new(x, y, size, size)
}

/// Y-coordinate of the top of the band-selector pill row (the row
/// itself starts after `BAND_ROW_TOP_GAP`). Used by both draw + hit
/// test so they share one truth.
pub(super) fn band_row_top(net: &Network, body_top: f32, scale: f32) -> f32 {
    let n = detail_rows(net).len() as f32;
    let line_h = EXPAND_DETAIL_FONT * scale + EXPAND_LINE_GAP * scale;
    body_top + EXPAND_PAD_TOP * scale + n * line_h
}

/// Logical width of the left column inside the expanded body.
pub(super) fn left_col_width(inner_w: f32) -> f32 {
    inner_w * LEFT_COL_FRAC
}

/// Rect of a specific band pill within an expanded row body. Pills are
/// laid out left-to-right in the order they appear in `net.bands`
/// (strongest first), starting after a small "Band" label.
pub(super) fn band_pill_rect(
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
pub(super) fn connect_button_rect(
    net: &Network,
    inner_x: f32,
    inner_w: f32,
    body_top: f32,
    scale: f32,
) -> Rect {
    let after_details = band_row_top(net, body_top, scale);
    let after_bands = after_details + band_row_height(net, scale);
    let btn_y = after_bands + EXPAND_BUTTON_TOP_GAP * scale;
    let btn_w = EXPAND_BUTTON_W * scale;
    let btn_h = EXPAND_BUTTON_H * scale;
    let left_w = left_col_width(inner_w);
    let btn_x = inner_x + left_w - btn_w - EXPAND_PAD_TOP * scale;
    Rect::new(btn_x, btn_y, btn_w, btn_h)
}
