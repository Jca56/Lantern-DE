//! Wired-port cards pinned between the header and the network list:
//! one per Ethernet port, showing link state, speed, address, and
//! whether it's the connection carrying the traffic. Status only, so
//! there is nothing here to hit-test.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::wifi::tile::draw_ethernet_icon;
use crate::controls::wifi::{EthLink, EthPort, Wifi};

use super::{
    ETH_BLOCK_BOTTOM_GAP, ETH_CARD_GAP, ETH_CARD_H, ETH_DETAIL_FONT, ETH_ICON_SIZE, ETH_TEXT_GAP,
    MAX_ETH_CARDS, ROW_FONT, ROW_RIGHT_GAP, ROW_SIGNAL_GAP,
};

/// Physical-px height the cards take up above the network list,
/// trailing gap included. Zero on a machine with no wired port.
pub(super) fn block_height(wifi: &Wifi, scale: f32) -> f32 {
    let n = wifi.ethernet().len().min(MAX_ETH_CARDS);
    if n == 0 {
        return 0.0;
    }
    (n as f32 * ETH_CARD_H + (n - 1) as f32 * ETH_CARD_GAP + ETH_BLOCK_BOTTOM_GAP) * scale
}

/// Right-edge status word, and whether it earns the gold accent.
fn status_label(port: &EthPort) -> (&'static str, bool) {
    match port.link {
        EthLink::Disabled => ("Off", false),
        EthLink::NoLink => ("No link", false),
        EthLink::Up if port.address.is_none() => ("Connecting…", true),
        EthLink::Up if port.default_route => ("In use", true),
        // Linked and configured, but WiFi holds the default route.
        EthLink::Up => ("Standby", false),
    }
}

/// Second line of a card: the interface name, then whatever is worth
/// knowing in its current state.
fn detail_line(port: &EthPort) -> String {
    let mut parts = vec![port.name.clone()];
    match port.link {
        EthLink::Disabled => parts.push("port is switched off".into()),
        EthLink::NoLink => parts.push("check the cable".into()),
        EthLink::Up => {
            if let Some(mbps) = port.speed_mbps {
                parts.push(format_speed(mbps));
            }
            parts.push(
                port.address
                    .clone()
                    .unwrap_or_else(|| "waiting for an address".into()),
            );
        }
    }
    parts.join(" · ")
}

/// "100 Mbit/s", "1 Gbit/s", "2.5 Gbit/s".
fn format_speed(mbps: u32) -> String {
    if mbps < 1000 {
        format!("{} Mbit/s", mbps)
    } else if mbps % 1000 == 0 {
        format!("{} Gbit/s", mbps / 1000)
    } else {
        format!("{:.1} Gbit/s", mbps as f32 / 1000.0)
    }
}

/// Draw the cards stacked downward from `top_y`.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_cards(
    painter: &mut Painter,
    text: &mut TextRenderer,
    wifi: &Wifi,
    inner_x: f32,
    inner_w: f32,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

    let title_font = ROW_FONT * scale;
    let detail_font = ETH_DETAIL_FONT * scale;
    let status_font = ROW_FONT * 0.8 * scale;
    let icon_size = ETH_ICON_SIZE * scale;
    let gap = ROW_SIGNAL_GAP * scale;
    let text_gap = ETH_TEXT_GAP * scale;
    let radius = 10.0 * scale;
    // A max_width of exactly the measured width can clip the last glyph.
    let slack = 6.0 * scale;

    let mut card_y = top_y;
    for port in wifi.ethernet().iter().take(MAX_ETH_CARDS) {
        let card = Rect::new(inner_x, card_y, inner_w, ETH_CARD_H * scale);
        let in_use = port.in_use();
        let linked = port.link == EthLink::Up;

        painter.rect_filled(card, radius, white.with_alpha(0.04 * alpha));
        if in_use {
            painter.rect_filled(card, radius, gold.with_alpha(0.18 * alpha));
        }

        let icon_color = if in_use {
            gold.with_alpha(alpha)
        } else if linked {
            white.with_alpha(alpha)
        } else {
            white.with_alpha(0.30 * alpha)
        };
        let icon_x = card.x + gap;
        draw_ethernet_icon(
            painter,
            icon_x,
            card.y + (card.h - icon_size) / 2.0,
            icon_size,
            icon_size,
            icon_color,
        );

        // Status word, right-aligned with the lock icons of the rows below.
        let (status, accent) = status_label(port);
        let status_w = text.measure_width(status, status_font);
        let status_x = card.x + card.w - ROW_RIGHT_GAP * scale - status_w;
        text.queue(
            status,
            status_font,
            status_x,
            card.y + (card.h - status_font) / 2.0,
            if accent {
                gold.with_alpha(alpha)
            } else {
                muted
            },
            status_w + slack,
            surface_w,
            surface_h,
        );

        // Title over the detail line, centred vertically as a pair.
        let text_x = icon_x + icon_size + gap;
        let text_w = (status_x - gap - text_x).max(0.0);
        let title_y = card.y + (card.h - (title_font + text_gap + detail_font)) / 2.0;
        let title_color = if in_use {
            gold.with_alpha(alpha)
        } else {
            white.with_alpha(0.86 * alpha)
        };
        text.queue(
            "Ethernet",
            title_font,
            text_x,
            title_y,
            title_color,
            text_w,
            surface_w,
            surface_h,
        );
        text.queue(
            &detail_line(port),
            detail_font,
            text_x,
            title_y + title_font + text_gap,
            muted,
            text_w,
            surface_w,
            surface_h,
        );

        card_y += (ETH_CARD_H + ETH_CARD_GAP) * scale;
    }
}
