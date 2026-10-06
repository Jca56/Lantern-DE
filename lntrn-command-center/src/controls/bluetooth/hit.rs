//! Hit-testing for the Bluetooth view: the header toggles, then the
//! device rows as they sit in the scrolled list.

use lntrn_render::Rect;

use super::detail;
use super::layout::{list_viewport, toggle_rect, toggles_row_layout, walk_devices};
use super::prompt::{prompt_button_rects, row_prompt};
use super::Bluetooth;

/// Hit-test result for the BT view.
pub enum BtClick {
    PowerToggle,
    DiscoverableToggle,
    ScanToggle,
    /// Click on the **header** row of a device — toggles the expanded
    /// detail panel.
    DeviceRow(String),
    /// Click on the Connect / Disconnect / Pair button inside the
    /// expanded panel.
    ConnectButton(String),
    /// Click on the Send-file button inside the expanded panel. Only
    /// fires when the device exposes an OBEX push profile.
    SendButton(String),
    /// Accept button on an inline request strip (incoming file, incoming
    /// pair, or outgoing pair-confirm) attached to this device's row.
    PromptAccept(String),
    /// Reject button on an inline request strip.
    PromptReject(String),
}

/// Hit-test all interactive regions in the BT view.
pub fn hit_test(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    text_size: f32,
    x: f32,
    y: f32,
) -> Option<BtClick> {
    let inside = |r: Rect| x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h;

    let power = toggle_rect(panel, panel_top_y, scale);
    if inside(power) {
        return Some(BtClick::PowerToggle);
    }
    if bt.is_powered() {
        let togs = toggles_row_layout(panel, panel_top_y, scale);
        if inside(togs.discoverable_toggle) {
            return Some(BtClick::DiscoverableToggle);
        }
        if inside(togs.scan_toggle) {
            return Some(BtClick::ScanToggle);
        }
    }

    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    if x < inner_x || x > inner_x + inner_w {
        return None;
    }

    // Rows scrolled out of the list's viewport are clipped away, so they
    // can't be clicked either.
    let vp = list_viewport(bt, panel, panel_top_y, text_size, scale);
    if y < vp.top || y > vp.top + vp.height {
        return None;
    }

    let mut hit: Option<BtClick> = None;
    let top = vp.top - vp.scroll;
    walk_devices(bt, panel, top, text_size, scale, |dev, geom| {
        // Inline request strip buttons take priority over the header
        // toggle so a click on Accept/Reject doesn't also collapse/expand.
        if row_prompt(bt, dev).is_some() {
            let (accept, reject, _field) =
                prompt_button_rects(bt, dev, inner_x, inner_w, geom.strip_top, text_size, scale);
            if inside(accept) {
                hit = Some(BtClick::PromptAccept(dev.mac.clone()));
                return true;
            }
            if inside(reject) {
                hit = Some(BtClick::PromptReject(dev.mac.clone()));
                return true;
            }
        }
        if inside(geom.header) {
            hit = Some(BtClick::DeviceRow(dev.mac.clone()));
            return true;
        }
        if bt.expanded_mac.as_deref() == Some(dev.mac.as_str()) {
            let connect = detail::connect_button_rect(
                inner_x,
                inner_w,
                geom.expanded_top,
                dev,
                text_size,
                scale,
            );
            if inside(connect) {
                hit = Some(BtClick::ConnectButton(dev.mac.clone()));
                return true;
            }
            if let Some(send) = detail::send_button_rect_expanded(
                inner_x,
                inner_w,
                geom.expanded_top,
                dev,
                text_size,
                scale,
            ) {
                if inside(send) {
                    hit = Some(BtClick::SendButton(dev.mac.clone()));
                    return true;
                }
            }
        }
        false
    });
    hit
}
