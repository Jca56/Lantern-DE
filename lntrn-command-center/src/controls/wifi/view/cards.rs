//! Right column of the WiFi expanded panel: BSSID cards on top,
//! saved-profile cards below.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::wifi::Network;

use super::draw::draw_lock;
use super::layout::{
    bssid_card_rect, bssid_lock_rect, left_col_width, profile_card_rect, profile_delete_rect,
    visible_bssid_card_count, visible_profile_card_count,
};
use super::{
    BSSID_CARD_GAP, BSSID_CARD_H, BSSID_HEADER_BOTTOM_GAP, BSSID_HEADER_FONT, BSSID_LOCK_PAD,
    BSSID_LOCK_SIZE, BSSID_MAC_FONT, BSSID_META_FONT, COL_GUTTER, EXPAND_PAD_TOP,
    PROFILE_ACTIVE_DOT, PROFILE_DELETE_PAD, PROFILE_DELETE_SIZE, PROFILE_HEADER_FONT,
    PROFILE_META_FONT, PROFILE_NAME_FONT, PROFILE_SECTION_TOP_GAP,
};

/// Right column of the expanded panel: BSSID cards on top, saved-profile
/// cards below (each with delete X + click-to-activate).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_right_column(
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
