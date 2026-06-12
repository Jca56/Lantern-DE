//! Per-frame hover-state refresh — drives the gold ring / row
//! highlight on every cursor-aware widget in the panel chrome.

use super::util::files_strip_rect;

/// Per-frame hover-state refresh for every cursor-aware widget in the
/// panel: WiFi rows, the power column, view-switcher arrows, the
/// emoji grid, control tiles, Files toolbar, and the mini-dock. Each
/// section is gated on the relevant view so we do no work when the
/// hover state cannot possibly change.
pub(super) fn track_hovers(wl: &mut super::WlState, app: &mut crate::app::AppState) {
    use crate::app::PanelRect;
    // Update WiFi row hover so the highlight tracks the cursor.
    // Cheap (a few rect tests) and only does work when the WiFi
    // view is open.
    if matches!(app.mode, crate::app::PanelMode::Control(crate::controls::TileId::Wifi)) {
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let view_top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        let new_hover = match crate::controls::wifi::hit_test_network(
            &app.controls.wifi, panel_rect, view_top_y, scale_f, phys_cx, phys_cy,
        ) {
            Some(crate::controls::wifi::NetworkHit::Row(s))
            | Some(crate::controls::wifi::NetworkHit::ConnectButton(s))
            | Some(crate::controls::wifi::NetworkHit::BandPill(s, _))
            | Some(crate::controls::wifi::NetworkHit::LockBssid(s, _))
            | Some(crate::controls::wifi::NetworkHit::ProfileActivate(s, _))
            | Some(crate::controls::wifi::NetworkHit::ProfileDelete(s, _)) => Some(s),
            Some(crate::controls::wifi::NetworkHit::ToggleVpn) | None => None,
        };
        if app.controls.wifi.hovered_ssid != new_hover {
            app.controls.wifi.hovered_ssid = new_hover;
        }
    } else if app.controls.wifi.hovered_ssid.is_some() {
        app.controls.wifi.hovered_ssid = None;
    }

    // Power-column hover tracking (buttons live to the right of the
    // panel). Skipped when the panel is collapsed — the column is
    // hidden then and we don't want a phantom hover ring.
    if app.collapsed || app.panel_view != crate::app::PanelView::Default {
        app.power_hover = None;
    } else {
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        app.power_hover = crate::power::hit_test(panel_rect, scale_f, phys_cx, phys_cy);
    }

    // View-switcher arrow hover. Always tracked (arrows are now
    // anchored to the row so they work in collapsed mode too).
    {
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        use crate::outer_zones::{hit_id, rect_in, OuterId};
        app.view_arrow_hover = crate::view_arrows::hit_test(panel_rect, scale_f, phys_cx, phys_cy);
        // Outer-chrome widgets hit-test against their zone-layout slots.
        let outer_pos = crate::outer_zones::positions(
            &app.outer, panel_rect, scale_f,
            crate::media::render::is_active(&app.media),
        );
        app.desktop_button_hover = hit_id(&outer_pos, OuterId::Desktop, phys_cx, phys_cy);
        app.desktop_settings_hover = if app.desktop_settings_open {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            crate::desktop_settings::hovered_row(panel_rect, top_y, scale_f, phys_cx, phys_cy)
        } else {
            None
        };
        app.gear_hover = hit_id(&outer_pos, OuterId::Settings, phys_cx, phys_cy);
        app.restart_hover = hit_id(&outer_pos, OuterId::Close, phys_cx, phys_cy);
        app.emoji_hover = hit_id(&outer_pos, OuterId::Emojis, phys_cx, phys_cy);
        app.clipboard_hover = hit_id(&outer_pos, OuterId::Clipboard, phys_cx, phys_cy);
        app.notes_hover = hit_id(&outer_pos, OuterId::Notes, phys_cx, phys_cy);
        app.usage_hover = hit_id(&outer_pos, OuterId::Usage, phys_cx, phys_cy);
        // Bar-mode sliders only hover-react while CC is collapsed.
        app.bar_sliders.hover = if app.collapse_progress() > 0.5 {
            rect_in(&outer_pos, OuterId::Sliders)
                .and_then(|r| crate::bar_sliders::hit_test(r, scale_f, phys_cx, phys_cy))
        } else {
            None
        };
        // Floating media transport buttons — same collapsed-only gating
        // (the card's slot only exists while something is playing).
        app.media.hover = if app.collapse_progress() > 0.5 {
            rect_in(&outer_pos, OuterId::Media).and_then(|r| {
                crate::media::render::floating_button_hit(r, scale_f, phys_cx, phys_cy)
            })
        } else {
            None
        };
        // Emoji overlay hover routing.
        if app.emojis.open {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let panel_bottom = panel_rect.y + panel_rect.h;
            let hit = crate::emojis::hit_test(
                &app.emojis, panel_rect, top_y, scale_f, panel_bottom, phys_cx, phys_cy,
            );
            app.emojis.hover_entry = None;
            app.emojis.hover_category = None;
            match hit {
                crate::emojis::Hit::Cell(idx) => app.emojis.hover_entry = Some(idx),
                crate::emojis::Hit::Category(c) => app.emojis.hover_category = Some(c),
                _ => {}
            }
        } else {
            app.emojis.hover_entry = None;
            app.emojis.hover_category = None;
        }
        // Notes overlay hover routing.
        if app.notes.open && !app.notes.confirm_delete {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let panel_bottom = panel_rect.y + panel_rect.h;
            let hit = crate::notes::hit_test(
                &app.notes, panel_rect, top_y, scale_f, panel_bottom, phys_cx, phys_cy,
            );
            app.notes.hover_idx = match hit {
                crate::notes::Hit::ListRow(i) => Some(i),
                _ => None,
            };
        } else {
            app.notes.hover_idx = None;
        }
        // Clipboard overlay hover routing.
        if app.clipboard.open && !app.clipboard.confirm_clear {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let panel_bottom = panel_rect.y + panel_rect.h;
            let hit = crate::clipboard::hit_test(
                &app.clipboard, panel_rect, top_y, scale_f, panel_bottom, phys_cx, phys_cy,
            );
            app.clipboard.hover_idx = match hit {
                crate::clipboard::Hit::Row(i)
                | crate::clipboard::Hit::PinStar(i)
                | crate::clipboard::Hit::Delete(i) => Some(i),
                _ => None,
            };
        } else {
            app.clipboard.hover_idx = None;
        }
        app.hovered_control_tile = app.controls.hit_test(
            panel_rect, scale_f, phys_cx, phys_cy, app.panel_view,
        );
        // Waffle "all apps" button hover (search row).
        let waffle = crate::search::input::waffle_rect(panel_rect, scale_f);
        app.waffle_hover = phys_cx >= waffle.x
            && phys_cx <= waffle.x + waffle.w
            && phys_cy >= waffle.y
            && phys_cy <= waffle.y + waffle.h;

        // Files view hover tracking — toolbar lives in the controls row,
        // sidebar + list live in the body.
        app.files.hover_nav = None;
        app.files.hover_entry = None;
        app.files.hover_crumb = None;
        app.files.hover_location = None;
        if app.panel_view == crate::app::PanelView::Files {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let strip_hit = files_strip_rect(&app, panel_rect, scale_f).map(|strip| {
                crate::files::hit_strip(&app.files, strip, scale_f, phys_cx, phys_cy)
            });
            match strip_hit {
                Some(crate::files::FilesHit::Nav(b)) => app.files.hover_nav = Some(b),
                Some(crate::files::FilesHit::Crumb(i)) => app.files.hover_crumb = Some(i),
                _ => {
                    match crate::files::hit_body(
                        &app.files, panel_rect, top_y, scale_f,
                        app.config.text_size, phys_cx, phys_cy,
                    ) {
                        crate::files::FilesHit::Sidebar(l) => app.files.hover_location = Some(l),
                        crate::files::FilesHit::Entry(i) => app.files.hover_entry = Some(i),
                        _ => {}
                    }
                }
            }
        }
    }

    // Mini-dock hover tracking — only meaningful while the dock is
    // actually rendered (collapse-progress > 0; the dock shows in
    // every view while collapsed). The hover index sticks while the
    // cursor is over the preview tile so users can move down onto it
    // without it disappearing.
    if app.collapse_progress() <= 0.005 {
        app.mini_dock_hover = None;
    } else {
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let phys_h_f = wl.phys_height().max(1) as f32;
        let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        let pinned = app.launcher.pinned_entries(&app.apps);
        let layout = crate::mini_dock::compute_layout(
            panel_rect, phys_h_f, scale_f, &pinned, &app.toplevels, &app.apps,
            Some((phys_cx, phys_cy)),
        );
        if let Some(layout) = layout {
            let icon_hit = crate::mini_dock::hit_test(&layout, phys_cx, phys_cy);
            if icon_hit.is_some() {
                app.mini_dock_hover = icon_hit;
            } else if let Some(prev) = app.mini_dock_hover {
                // Generous zone covering icon + every preview tile
                // so the user can drag the cursor up into any of
                // the per-window thumbnails without losing it.
                let nwin = layout
                    .entries
                    .get(prev)
                    .map(|e| {
                        crate::mini_dock::windows_for_app(&app.toplevels, &e.app_id).len()
                    })
                    .unwrap_or(0)
                    .max(1);
                let still_in_zone = crate::mini_dock::hit_test_preview_zone(
                    &layout, panel_rect, prev, nwin, phys_cx, phys_cy,
                );
                if !still_in_zone {
                    app.mini_dock_hover = None;
                }
            }
        } else {
            app.mini_dock_hover = None;
        }
    }
}
