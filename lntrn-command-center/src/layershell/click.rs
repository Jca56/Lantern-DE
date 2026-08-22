//! Per-frame click + pin-drag dispatch. Drains
//! `wl.{left_clicked, right_clicked, left_released_this_frame}` and
//! applies the resulting mutations to [`AppState`]. The control-view
//! click cascade lives in the sibling [`super::view_click`] module so
//! this file can stay focused on the top-level cascade.

/// Resolve every per-frame mouse click and pin-drag action — context
/// menus, the main left-click cascade (control views, tiles, dock,
/// click-outside), pin-drag motion and release, and the right-click
/// context-menu opener. Drains `wl.left_clicked` / `wl.right_clicked` /
/// `wl.left_released_this_frame` and applies the resulting mutations
/// to `app`.
use super::view_click::handle_control_view_click;

pub(super) fn handle_clicks(
    wl: &mut super::WlState,
    app: &mut crate::app::AppState,
    text: &mut lntrn_render::TextRenderer,
    thumbs: &mut crate::thumbs::CcThumbsClient,
) {
    use crate::app::PanelRect;

    // Handle left-click: if outside the panel rect → close, otherwise
    // hit-test against the launcher and launch if a tile/row was hit.
    if wl.left_clicked {
        wl.left_clicked = false;
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let phys_h = wl.phys_height().max(1);
        let panel = PanelRect::compute_with_dims(
            phys_w,
            scale_f,
            app.desired_panel_w_logical(),
            app.desired_panel_h_logical(),
        );
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);

        // A click back inside the panel while a collapse-then-close
        // fold is pending means the user changed their mind — disarm
        // the deferred close and let the click land normally.
        if app.close_after_collapse && panel.contains(phys_cx, phys_cy) {
            app.close_after_collapse = false;
        }

        // Toolbar edit mode owns every press: Done button, ✕ remove
        // badges, tray chips (begin enable-drag), and widgets (begin
        // move-drag). All other click handling is suspended while editing.
        if app.toolbar_edit {
            use crate::controls::toolbar_edit as te;
            use crate::controls::widget_settings as ws;

            // An open settings popover gets first crack at the click.
            if let Some(id) = app.widget_settings_open {
                if let Some(h) = ws::hit(id, &app.controls, panel_rect, scale_f, phys_cx, phys_cy) {
                    match h {
                        ws::Hit::Slider(slider) => {
                            let v = ws::slider_value_at(
                                id,
                                slider,
                                &app.controls,
                                panel_rect,
                                scale_f,
                                phys_cx,
                            );
                            let o = app.controls.toolbar.opts_mut(id);
                            match slider {
                                ws::WidgetSlider::Size => o.size = v,
                                ws::WidgetSlider::Space => o.space = v,
                            }
                            app.widget_slider_drag = Some((id, slider));
                        }
                        ws::Hit::ToggleDate => {
                            app.controls.toolbar.opts_mut(id).clock.show_date ^= true
                        }
                        ws::Hit::SetDatePos(p) => {
                            app.controls.toolbar.opts_mut(id).clock.date_pos = p
                        }
                        ws::Hit::Toggle24h => {
                            app.controls.toolbar.opts_mut(id).clock.hour24 ^= true
                        }
                        ws::Hit::ToggleSeconds => {
                            app.controls.toolbar.opts_mut(id).clock.seconds ^= true
                        }
                    }
                    app.controls.toolbar.save();
                    return;
                }
                // Clicking another widget's ⚙ switches the popover; Done
                // still works; anything else just closes the popover.
                if let Some(gid) =
                    te::hit_gear_badge(&app.controls, panel_rect, scale_f, phys_cx, phys_cy)
                {
                    app.widget_settings_open = if gid == id { None } else { Some(gid) };
                } else if te::hit_done(panel_rect, scale_f, phys_cx, phys_cy) {
                    app.exit_toolbar_edit();
                } else {
                    app.widget_settings_open = None;
                }
                return;
            }

            if te::hit_done(panel_rect, scale_f, phys_cx, phys_cy) {
                app.exit_toolbar_edit();
            } else if let Some(id) =
                te::hit_gear_badge(&app.controls, panel_rect, scale_f, phys_cx, phys_cy)
            {
                app.widget_settings_open = Some(id);
            } else if let Some(id) =
                te::hit_close_badge(&app.controls, panel_rect, scale_f, phys_cx, phys_cy)
            {
                app.controls.toolbar.disable(id);
                app.controls.toolbar.save();
            } else if let Some(id) =
                te::hit_tray_chip(&app.controls, panel_rect, scale_f, phys_cx, phys_cy)
            {
                app.widget_drag = Some(te::WidgetDrag {
                    id,
                    from_tray: true,
                    cursor: (phys_cx, phys_cy),
                });
            } else if let Some(id) =
                te::hit_widget(&app.controls, panel_rect, scale_f, phys_cx, phys_cy)
            {
                app.widget_drag = Some(te::WidgetDrag {
                    id,
                    from_tray: false,
                    cursor: (phys_cx, phys_cy),
                });
            } else {
                // Outer-chrome widgets (strip buttons, dots, sliders,
                // media) — edit mode includes the media slot even when
                // nothing is playing so it stays grabbable.
                let outer_pos =
                    crate::outer_zones::positions(&app.outer, panel_rect, scale_f, true);
                if let Some(id) = crate::outer_zones::hit_widget(&outer_pos, phys_cx, phys_cy) {
                    app.outer_drag = Some(crate::outer_edit::OuterDrag {
                        id,
                        cursor: (phys_cx, phys_cy),
                    });
                }
            }
            return;
        }

        // Outer-chrome widget positions for this frame — every strip
        // button, the dots, sliders, and media card hit-test against
        // these so clicks land wherever the user dragged them.
        let outer_pos = crate::outer_zones::positions(
            &app.outer,
            panel_rect,
            scale_f,
            crate::media::render::is_active(&app.media),
        );

        // Bar-mode sliders: grab one if the press lands on its track,
        // seek the value to the cursor, and stash the dragging slider
        // so subsequent motion events keep updating it. Bar sliders are
        // only visible / hittable while CC is collapsed.
        if app.collapse_progress() > 0.5 {
            if let Some(slot) =
                crate::outer_zones::rect_in(&outer_pos, crate::outer_zones::OuterId::Sliders)
            {
                if let Some(slider) = crate::bar_sliders::hit_test(slot, scale_f, phys_cx, phys_cy)
                {
                    let v = crate::bar_sliders::value_at_cursor(slot, scale_f, slider, phys_cx);
                    crate::bar_sliders::set_value(slider, v);
                    app.bar_sliders.dragging = Some(slider);
                    return;
                }
            }
        }

        // Floating now-playing card: transport buttons live in the
        // card's outer-zone slot while collapsed, same as the sliders.
        // (The slot only exists while something is playing.)
        if app.collapse_progress() > 0.5 {
            if let Some(card) =
                crate::outer_zones::rect_in(&outer_pos, crate::outer_zones::OuterId::Media)
            {
                if let Some(btn) =
                    crate::media::render::floating_button_hit(card, scale_f, phys_cx, phys_cy)
                {
                    app.media.command(btn);
                    return;
                }
            }
        }

        // Context menu intercepts every left-click while open: a
        // click on an item runs that action; anywhere else dismisses.
        let menu_consumed = if let Some(menu) = app.context_menu.clone() {
            // Hit-test against the full surface — matches the bounds
            // the menu was drawn with, so dock menus that anchor
            // outside the panel are still clickable.
            let surface_bounds = lntrn_render::Rect::new(
                0.0,
                0.0,
                wl.phys_width().max(1) as f32,
                wl.phys_height().max(1) as f32,
            );
            if let Some(action) = crate::launcher::context_menu::hit_test(
                &menu,
                surface_bounds,
                scale_f,
                phys_cx,
                phys_cy,
            ) {
                app.run_menu_action(action);
            } else {
                app.context_menu = None;
            }
            true
        } else if let Some(event_menu) = app.controls.clock.event_menu.clone() {
            // Event-row menu intercepts: Delete row → remove event.
            if crate::controls::clock::event_menu_hit_delete(
                &event_menu,
                panel_rect,
                scale_f,
                phys_cx,
                phys_cy,
            ) {
                app.controls
                    .events
                    .remove_at(event_menu.date, event_menu.idx_in_date);
            }
            app.controls.clock.event_menu = None;
            true
        } else {
            false
        };

        // First: if a control's full-content view is up, see if the
        // click hit one of its interactive widgets (battery toggle,
        // audio slider, audio device list). Skip when an overlay
        // (context menu, power confirm) has the click.
        let consumed_by_view = !menu_consumed
            && app.power_confirm.is_none()
            && handle_control_view_click(app, text, panel_rect, scale_f, phys_cx, phys_cy);

        // Power confirm modal intercepts every click while open.
        let confirm_consumed = if let Some(pending) = app.power_confirm {
            let surface_w_px = wl.phys_width().max(1);
            let surface_h_px = wl.phys_height().max(1);
            match crate::power::hit_test_confirm(
                surface_w_px,
                surface_h_px,
                scale_f,
                phys_cx,
                phys_cy,
            ) {
                Some(crate::power::ConfirmHit::Confirm) => {
                    tracing::debug!(?pending, "power confirm → run");
                    crate::power::run(pending);
                    app.power_confirm = None;
                    app.close();
                }
                Some(crate::power::ConfirmHit::Cancel) => {
                    tracing::debug!(?pending, "power confirm → cancel");
                    app.power_confirm = None;
                }
                Some(crate::power::ConfirmHit::CardBody) => {
                    // Click inside the card but missed the buttons — eat it.
                }
                None => {
                    // Click outside the card → treat as cancel.
                    tracing::debug!(?pending, "power confirm → click-out cancel");
                    app.power_confirm = None;
                }
            }
            true
        } else {
            false
        };

        let power_btn = if app.collapsed || app.panel_view != crate::app::PanelView::Default {
            None
        } else {
            crate::power::hit_test(panel_rect, scale_f, phys_cx, phys_cy)
        };
        use crate::outer_zones::{hit_id, rect_in, OuterId};
        let arrow_hit = crate::view_arrows::hit_test(panel_rect, scale_f, phys_cx, phys_cy);
        let gear_hit = hit_id(&outer_pos, OuterId::Settings, phys_cx, phys_cy);
        let restart_hit = hit_id(&outer_pos, OuterId::Close, phys_cx, phys_cy);
        let emoji_btn_hit = hit_id(&outer_pos, OuterId::Emojis, phys_cx, phys_cy);
        let clipboard_btn_hit = hit_id(&outer_pos, OuterId::Clipboard, phys_cx, phys_cy);
        let notes_btn_hit = hit_id(&outer_pos, OuterId::Notes, phys_cx, phys_cy);
        let usage_btn_hit = hit_id(&outer_pos, OuterId::Usage, phys_cx, phys_cy);
        let dot_hit = rect_in(&outer_pos, OuterId::Dots)
            .and_then(|r| crate::view_indicator::dot_at(r, scale_f, phys_cx, phys_cy));
        // Mini-dock hit-tests — try preview-tile first (when hover
        // points to an app with open windows) then the icon body.
        // (dock_idx, window_idx_within_app, hit_kind)
        let mut dock_preview_hit: Option<(usize, usize, crate::mini_dock::PreviewHit)> = None;
        let mut dock_pin: Option<usize> = None;
        let mut dock_layout: Option<crate::mini_dock::DockLayout> = None;
        // Dock is visible (and clickable) in every view while collapsed.
        if app.collapse_progress() > 0.005 {
            let pinned = app.launcher.pinned_entries(&app.apps);
            let phys_h_f = wl.phys_height().max(1) as f32;
            if let Some(layout) = crate::mini_dock::compute_layout(
                panel_rect,
                phys_h_f,
                scale_f,
                &pinned,
                &app.toplevels,
                &app.apps,
                Some((phys_cx, phys_cy)),
            ) {
                if let Some(hover_idx) = app.mini_dock_hover {
                    if let Some(entry) = layout.entries.get(hover_idx) {
                        let windows =
                            crate::mini_dock::windows_for_app(&app.toplevels, &entry.app_id);
                        if !windows.is_empty() {
                            dock_preview_hit = crate::mini_dock::hit_test_preview(
                                &layout,
                                panel_rect,
                                hover_idx,
                                windows.len(),
                                phys_cx,
                                phys_cy,
                            )
                            .map(|(wi, h)| (hover_idx, wi, h));
                        }
                    }
                }
                if dock_preview_hit.is_none() {
                    dock_pin = crate::mini_dock::hit_test(&layout, phys_cx, phys_cy);
                }
                dock_layout = Some(layout);
            }
        }
        if menu_consumed || confirm_consumed {
            // Already handled by an overlay — fall through to render.
        } else if let Some(action) = power_btn {
            tracing::debug!(?action, "power button click → open confirm modal");
            app.power_confirm = Some(action);
        } else if app.desktop_settings_open && {
            // While the Desktop Widgets page is open, clicks inside the
            // panel route to its row hit-test first.
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            match crate::desktop_settings::hit_test_page(
                panel_rect, top_y, scale_f, phys_cx, phys_cy,
            ) {
                crate::desktop_settings::PageHit::ClockToggle => {
                    app.desktop_widgets = crate::desktop_settings::toggle_clock();
                    true
                }
                crate::desktop_settings::PageHit::VisualizerToggle => {
                    app.desktop_widgets = crate::desktop_settings::toggle_visualizer();
                    true
                }
                crate::desktop_settings::PageHit::RainbowToggle => {
                    app.desktop_widgets = crate::desktop_settings::toggle_rainbow();
                    true
                }
                crate::desktop_settings::PageHit::Background => false,
            }
        } {
            // Already handled.
        } else if app.settings_open && {
            // While the settings page is open all clicks inside the
            // panel route to the settings hit-test first. Offset by the
            // scroll so clicks line up with the on-screen rows.
            let top_y = crate::controls::content_top_y(panel_rect, scale_f) - app.settings_scroll;
            let rows = crate::settings::layout(panel_rect, top_y, scale_f, app.config.text_size);
            if let Some(hit) = crate::settings::hit_test(&rows, phys_cx, phys_cy) {
                match hit {
                    crate::settings::SettingHit::Toggle(key) => {
                        let cur = crate::settings::current_value(&app.config, key);
                        if let crate::settings::SettingValue::B(b) = cur {
                            crate::settings::apply_value(
                                &mut app.config,
                                key,
                                crate::settings::SettingValue::B(!b),
                            );
                            app.config.save();
                        }
                    }
                    crate::settings::SettingHit::SliderSeek(key, value) => {
                        if key.defers_during_drag() {
                            // Stash for release-time commit so the panel
                            // doesn't resize out from under the cursor.
                            app.settings_drag_pending = Some(value);
                        } else {
                            crate::settings::apply_value(
                                &mut app.config,
                                key,
                                crate::settings::SettingValue::F(value),
                            );
                            app.config.save();
                        }
                        app.settings_drag = Some(key);
                    }
                }
                true
            } else {
                false
            }
        } {
            // Already handled.
        } else if restart_hit {
            tracing::info!("dev restart X clicked — relaunching daemon");
            // Spawn a delayed re-launch so the new daemon starts after
            // we've released the IPC socket. setsid in spawn_detached
            // keeps the child alive across our exit.
            crate::app::spawn_detached(
                "sleep 0.3 && exec \"$HOME/.lantern/bin/lntrn-command-center\" --show",
            );
            std::process::exit(0);
        } else if gear_hit {
            // Settings page is only visible in the expanded panel —
            // un-collapse first so opening settings while collapsed
            // actually shows the page.
            if app.collapsed && !app.settings_open {
                app.toggle_collapsed();
            }
            app.toggle_settings();
        } else if emoji_btn_hit {
            if app.collapsed && !app.emojis.open {
                app.toggle_collapsed();
            }
            app.toggle_emojis();
        } else if clipboard_btn_hit {
            if app.collapsed && !app.clipboard.open {
                app.toggle_collapsed();
            }
            app.toggle_clipboard();
        } else if notes_btn_hit {
            if app.collapsed && !app.notes.open {
                app.toggle_collapsed();
            }
            app.toggle_notes();
        } else if usage_btn_hit {
            if app.collapsed && !app.usage.open {
                app.toggle_collapsed();
            }
            app.toggle_usage();
        } else if app.notes.open && {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let panel_bottom = panel_rect.y + panel_rect.h;
            if app.notes.confirm_delete {
                let (dialog, cancel, confirm) =
                    crate::notes::confirm_delete_rects(panel_rect, scale_f);
                if crate::notes::point_in(confirm, phys_cx, phys_cy) {
                    app.notes.delete_selected();
                    app.notes.confirm_delete = false;
                } else if crate::notes::point_in(cancel, phys_cx, phys_cy)
                    || !crate::notes::point_in(dialog, phys_cx, phys_cy)
                {
                    app.notes.confirm_delete = false;
                }
                true
            } else {
                let hit = crate::notes::hit_test(
                    &app.notes,
                    panel_rect,
                    top_y,
                    scale_f,
                    panel_bottom,
                    phys_cx,
                    phys_cy,
                );
                match hit {
                    crate::notes::Hit::None => false,
                    crate::notes::Hit::Filter => {
                        app.notes.flush_edits_to_selected();
                        app.notes.focus = crate::notes::NoteFocus::Filter;
                        // Begin a drag-selection in the filter input.
                        let filter_bar = crate::notes::filter_rect(panel_rect, top_y, scale_f);
                        let pad_left = crate::notes::filter_text_left_pad(filter_bar, scale_f);
                        let font = (app.config.text_size * scale_f).max(14.0);
                        let q = app.notes.filter.query().to_string();
                        let byte = crate::notes::input_byte_at(
                            filter_bar, pad_left, &q, font, phys_cx, text,
                        );
                        app.notes.filter.begin_drag_at(byte);
                        app.notes.drag_field = Some(crate::notes::DragField::Filter);
                        true
                    }
                    crate::notes::Hit::NewBtn => {
                        app.notes.new_note();
                        true
                    }
                    crate::notes::Hit::ListRow(vis_idx) => {
                        let visible = app.notes.visible_indices();
                        if let Some(&nidx) = visible.get(vis_idx) {
                            let id = app.notes.notes[nidx].id;
                            app.notes.select(id);
                        }
                        true
                    }
                    crate::notes::Hit::TitleInput => {
                        app.notes.focus = crate::notes::NoteFocus::Title;
                        // Begin a drag-selection in the title input.
                        let editor =
                            crate::notes::editor_rect(panel_rect, top_y, scale_f, panel_bottom);
                        let title_field = crate::notes::title_field_rect(editor, scale_f);
                        let title_pad = 12.0 * scale_f;
                        let title_font = (app.config.text_size * scale_f).max(15.0);
                        let q = app.notes.title.query().to_string();
                        let byte = crate::notes::input_byte_at(
                            title_field,
                            title_pad,
                            &q,
                            title_font,
                            phys_cx,
                            text,
                        );
                        app.notes.title.begin_drag_at(byte);
                        app.notes.drag_field = Some(crate::notes::DragField::Title);
                        true
                    }
                    crate::notes::Hit::BodyEditor => {
                        app.notes.focus = crate::notes::NoteFocus::Body;
                        // Begin a drag-selection in the body editor.
                        let editor =
                            crate::notes::editor_rect(panel_rect, top_y, scale_f, panel_bottom);
                        let body = crate::notes::body_rect(editor, scale_f);
                        let inner = crate::notes::body_inner_rect(body, scale_f);
                        let buf = app.notes.body.raw().to_string();
                        let byte = crate::notes::body_byte_at(
                            inner,
                            &buf,
                            app.notes.body_scroll,
                            app.config.text_size,
                            scale_f,
                            phys_cx,
                            phys_cy,
                            text,
                        );
                        app.notes.body.begin_drag_at(byte);
                        app.notes.drag_field = Some(crate::notes::DragField::Body);
                        true
                    }
                    crate::notes::Hit::PinAction => {
                        app.notes.toggle_pin_selected();
                        true
                    }
                    crate::notes::Hit::StickyAction => {
                        app.notes.flush_edits_to_selected();
                        let spawn = crate::notes::sticky::spawn_pos(
                            &app.notes.notes,
                            panel_rect,
                            scale_f,
                            phys_w as f32,
                            phys_h as f32,
                        );
                        crate::notes::sticky::toggle_selected(&mut app.notes, spawn);
                        true
                    }
                    crate::notes::Hit::ExportAction => {
                        app.notes.flush_edits_to_selected();
                        app.notes.export_selected();
                        true
                    }
                    crate::notes::Hit::DeleteAction => {
                        if app.notes.selected_id.is_some() {
                            app.notes.confirm_delete = true;
                        }
                        true
                    }
                }
            }
        } {
            // Handled by notes overlay.
        } else if app.clipboard.open && {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let panel_bottom = panel_rect.y + panel_rect.h;
            if app.clipboard.confirm_clear {
                // Modal eats clicks — let Esc/Enter drive it.
                true
            } else {
                let hit = crate::clipboard::hit_test(
                    &app.clipboard,
                    panel_rect,
                    top_y,
                    scale_f,
                    panel_bottom,
                    phys_cx,
                    phys_cy,
                );
                match hit {
                    crate::clipboard::Hit::None => false,
                    crate::clipboard::Hit::Filter => true,
                    crate::clipboard::Hit::ClearAll => {
                        if !app.clipboard.entries.is_empty() {
                            app.clipboard.confirm_clear = true;
                        }
                        true
                    }
                    crate::clipboard::Hit::Row(vis_idx) => {
                        let visible = app.clipboard.visible_indices();
                        if let Some(&eidx) = visible.get(vis_idx) {
                            if let Some(entry) = app.clipboard.entries.get(eidx) {
                                let id = entry.id;
                                if crate::clipboard::ipc::copy(id) {
                                    app.clipboard.recent_copy =
                                        Some((id, std::time::Instant::now()));
                                }
                            }
                        }
                        true
                    }
                    crate::clipboard::Hit::PinStar(vis_idx) => {
                        let visible = app.clipboard.visible_indices();
                        if let Some(&eidx) = visible.get(vis_idx) {
                            if let Some(entry) = app.clipboard.entries.get(eidx) {
                                let id = entry.id;
                                let new_val = !entry.pinned;
                                if crate::clipboard::ipc::pin(id, new_val) {
                                    app.refresh_clipboard();
                                }
                            }
                        }
                        true
                    }
                    crate::clipboard::Hit::Delete(vis_idx) => {
                        let visible = app.clipboard.visible_indices();
                        if let Some(&eidx) = visible.get(vis_idx) {
                            if let Some(entry) = app.clipboard.entries.get(eidx) {
                                let id = entry.id;
                                if crate::clipboard::ipc::delete(id) {
                                    app.refresh_clipboard();
                                }
                            }
                        }
                        true
                    }
                }
            }
        } {
            // Handled by clipboard overlay.
        } else if app.emojis.open && {
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let panel_bottom = panel_rect.y + panel_rect.h;
            let hit = crate::emojis::hit_test(
                &app.emojis,
                panel_rect,
                top_y,
                scale_f,
                panel_bottom,
                phys_cx,
                phys_cy,
            );
            match hit {
                crate::emojis::Hit::None => false,
                crate::emojis::Hit::Filter => {
                    // Click on filter: just focus (already focused). No-op.
                    true
                }
                crate::emojis::Hit::Category(c) => {
                    app.emojis.active_category = c;
                    app.emojis.reset_scroll();
                    true
                }
                crate::emojis::Hit::Cell(visible_idx) => {
                    let visible = app.emojis.visible_indices();
                    if let Some(&entry_idx) = visible.get(visible_idx) {
                        let entry = &crate::emojis::data::EMOJIS[entry_idx];
                        // Copy to the clipboard (so paste works too)…
                        if let Some(clip) = app.clipboard_handle.as_ref() {
                            clip.set_text(entry.glyph);
                        }
                        // …and ask the compositor to type it straight into
                        // the window we were last working in. CC keeps focus,
                        // so you can keep clicking to insert more.
                        thumbs.type_text(entry.glyph);
                        app.emojis.recent_copy = Some((visible_idx, std::time::Instant::now()));
                    }
                    true
                }
            }
        } {
            // Already handled by emojis overlay.
        } else if let Some(dot_idx) = dot_hit {
            if let Some(v) = crate::app::PanelView::ALL.get(dot_idx) {
                app.set_view(*v);
            }
        } else if let Some(side) = arrow_hit {
            match side {
                crate::view_arrows::Side::Left => app.cycle_view_prev(),
                crate::view_arrows::Side::Right => app.cycle_view_next(),
            }
        } else if hit_id(&outer_pos, OuterId::Desktop, phys_cx, phys_cy) {
            // Page is body-sized — uncollapse first when collapsed so
            // the user actually sees the page they just opened.
            if app.collapsed && !app.desktop_settings_open {
                app.toggle_collapsed();
            }
            app.toggle_desktop_settings();
        } else if let Some((idx, window_idx, hit)) = dock_preview_hit {
            let entry = dock_layout
                .as_ref()
                .and_then(|l| l.entries.get(idx))
                .cloned();
            if let Some(entry) = entry {
                let windows = crate::mini_dock::windows_for_app(&app.toplevels, &entry.app_id);
                if let Some(target) = windows.get(window_idx).copied() {
                    match hit {
                        crate::mini_dock::PreviewHit::Body => {
                            tracing::debug!(
                                app_id = %entry.app_id, window_idx,
                                "dock preview → activate"
                            );
                            app.window_actions.push(crate::app::WindowAction {
                                app_id: target.app_id.clone(),
                                title: target.title.clone(),
                                instance: Some(window_idx),
                                kind: crate::app::WindowActionKind::Activate,
                            });
                            app.close();
                        }
                        crate::mini_dock::PreviewHit::Close => {
                            tracing::debug!(
                                app_id = %entry.app_id, window_idx,
                                "dock preview → close"
                            );
                            app.window_actions.push(crate::app::WindowAction {
                                app_id: target.app_id.clone(),
                                title: target.title.clone(),
                                instance: Some(window_idx),
                                kind: crate::app::WindowActionKind::Close,
                            });
                        }
                    }
                }
            }
        } else if let Some(dock_idx) = dock_pin {
            // Click on a dock icon: if the app has open windows,
            // cycle focus to the next one (after whichever is
            // currently activated). With no windows, launch a new
            // instance — but only for pinned slots; running-only
            // slots vanish when their last window closes so this
            // branch only fires while at least one window exists.
            let entry = dock_layout
                .as_ref()
                .and_then(|l| l.entries.get(dock_idx))
                .cloned();
            if let Some(entry) = entry {
                let windows = crate::mini_dock::windows_for_app(&app.toplevels, &entry.app_id);
                if windows.is_empty() && entry.pinned {
                    tracing::debug!(pin = dock_idx, "mini-dock click → launch (no windows)");
                    app.activate_at(crate::app::HitTarget::Pin(dock_idx));
                } else if !windows.is_empty() {
                    let next_idx = if let Some(cur) = windows.iter().position(|w| w.activated) {
                        (cur + 1) % windows.len()
                    } else {
                        0
                    };
                    let target = windows[next_idx];
                    tracing::debug!(
                        app_id = %entry.app_id,
                        total = windows.len(),
                        next_idx,
                        "mini-dock click → cycle to next window",
                    );
                    app.window_actions.push(crate::app::WindowAction {
                        app_id: target.app_id.clone(),
                        title: target.title.clone(),
                        instance: Some(next_idx),
                        kind: crate::app::WindowActionKind::Activate,
                    });
                    app.close();
                }
            }
        } else if let Some((id, part)) = (!panel.contains(phys_cx, phys_cy))
            .then(|| {
                crate::notes::sticky::hit(
                    &app.notes.notes,
                    scale_f,
                    phys_w as f32,
                    phys_h as f32,
                    phys_cx,
                    phys_cy,
                )
            })
            .flatten()
        {
            // Click landed on a floating sticky note (outside the
            // panel — the panel always wins where they overlap).
            use crate::notes::sticky::{StickyDrag, StickyPart};
            match part {
                StickyPart::Unstick => {
                    if let Some(i) = app.notes.notes.iter().position(|n| n.id == id) {
                        app.notes.notes[i].sticky = false;
                        let _ = crate::notes::store::save_one(&app.notes.notes[i]);
                    }
                }
                StickyPart::Body | StickyPart::Resize => {
                    if let Some(n) = app.notes.notes.iter().find(|n| n.id == id) {
                        let r =
                            crate::notes::sticky::rect(n, scale_f, phys_w as f32, phys_h as f32);
                        let resize = part == StickyPart::Resize;
                        let grab = if resize {
                            (r.x + r.w - phys_cx, r.y + r.h - phys_cy)
                        } else {
                            (phys_cx - r.x, phys_cy - r.y)
                        };
                        app.sticky_drag = Some(StickyDrag { id, resize, grab });
                    }
                }
            }
        } else if !panel.contains(phys_cx, phys_cy) {
            tracing::debug!(
                cursor = ?(phys_cx, phys_cy),
                panel = ?(panel.x, panel.y, panel.w, panel.h),
                "click outside panel → close + focus underlying window"
            );
            // Ask the compositor to focus whatever toplevel is at
            // the click point so the user doesn't need a second
            // click to start typing in it. cursor_x/y from wayland
            // are already in surface-logical pixels (== compositor
            // logical for a single-output, fullscreen layer surface).
            let logical_x = wl.cursor_x.round() as i32;
            let logical_y = wl.cursor_y.round() as i32;
            thumbs.focus_at(logical_x, logical_y);
            app.dismiss();
        } else if consumed_by_view {
            // Already handled.
        } else if let Some(tile_id) =
            app.controls
                .hit_test(panel_rect, scale_f, phys_cx, phys_cy, app.panel_view)
        {
            match tile_id {
                crate::controls::TileId::Collapse => {
                    tracing::debug!("collapse chevron click → toggle");
                    app.toggle_collapsed();
                }
                crate::controls::TileId::TerminalClear => {
                    app.terminal.clear();
                }
                crate::controls::TileId::Workspace => {
                    // Workspace number is a passive indicator — no view.
                }
                crate::controls::TileId::Gaming => {
                    // Toggle Gaming Mode via IPC — no expanded view.
                    // Optimistically flip so the tile reacts on this frame;
                    // the compositor pushes the canonical state right back.
                    app.controls.gaming_ipc.send_toggle();
                    if let Some(cur) = app.controls.gaming_ipc.gaming_mode {
                        app.controls.gaming_ipc.gaming_mode = Some(!cur);
                    }
                }
                crate::controls::TileId::Audio => {
                    // Audio tile is split: speaker icon opens the
                    // expanded audio page (for output device picker
                    // etc.); the bar sets volume + starts a drag.
                    use crate::controls::audio::InlineHit;
                    if let Some(layout) = app.controls.tile_layout(
                        crate::controls::TileId::Audio,
                        panel_rect,
                        scale_f,
                        app.panel_view,
                    ) {
                        match crate::controls::audio::hit_test_inline(
                            &layout, scale_f, phys_cx, phys_cy,
                        ) {
                            Some(InlineHit::SpeakerIcon) => {
                                app.show_control(crate::controls::TileId::Audio);
                            }
                            Some(InlineHit::VolumeBar) => {
                                let bar = crate::controls::audio::inline_bar_rect(&layout, scale_f);
                                let frac = ((phys_cx - bar.x) / bar.w).clamp(0.0, 1.0);
                                app.controls.audio.set_volume(frac);
                                app.dragging = Some(crate::app::DragTarget::AudioInlineSlider);
                            }
                            None => {}
                        }
                    }
                }
                _ => {
                    tracing::debug!(?tile_id, "left-click on controls tile → switch view");
                    app.show_control(tile_id);
                }
            }
        } else if matches!(app.mode, crate::app::PanelMode::Launcher)
            && app.panel_view == crate::app::PanelView::Default
        {
            // Waffle "all apps" button on the search bar.
            let waffle = crate::search::input::waffle_rect(panel_rect, scale_f);
            if phys_cx >= waffle.x
                && phys_cx <= waffle.x + waffle.w
                && phys_cy >= waffle.y
                && phys_cy <= waffle.y + waffle.h
            {
                tracing::debug!("waffle click → show all apps");
                if app.search.all_apps_mode {
                    // Toggle off — back to pinned launcher.
                    app.search.reset();
                } else {
                    app.search.show_all_apps(&app.apps, app.launcher.hidden());
                }
            } else if let Some(target) = app.hit_test_launcher(panel, scale_f, phys_cx, phys_cy) {
                // Pin tiles defer activation until release — that way
                // a press-and-drag can reorder instead of launching.
                if let crate::app::HitTarget::Pin(i) = target {
                    tracing::debug!(pin = i, "pin press → start drag candidate");
                    app.pin_drag = Some(crate::app::PinDrag {
                        from_idx: i,
                        press_x: phys_cx,
                        press_y: phys_cy,
                        current_x: phys_cx,
                        current_y: phys_cy,
                        started: false,
                    });
                } else {
                    tracing::debug!(?target, "left-click on launcher entry → activate");
                    app.activate_at(target);
                }
            }
        }
        // Click inside the panel but not on a clickable entity is
        // a no-op.
    }

    // Pin drag — update current cursor each frame, promote to
    // "started" once movement exceeds the threshold.
    if let Some(drag) = app.pin_drag.as_mut() {
        let scale_f = wl.fractional_scale() as f32;
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        drag.current_x = phys_cx;
        drag.current_y = phys_cy;
        if !drag.started {
            let dx = phys_cx - drag.press_x;
            let dy = phys_cy - drag.press_y;
            let threshold = crate::app::PIN_DRAG_THRESHOLD * scale_f;
            if (dx * dx + dy * dy).sqrt() > threshold {
                drag.started = true;
            }
        }
    }

    // Pin drag — release commits a reorder when the drag actually
    // started, otherwise treats it as a plain click on the pin.
    if wl.left_released_this_frame {
        wl.left_released_this_frame = false;
        if let Some(drag) = app.pin_drag.take() {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w,
                scale_f,
                app.desired_panel_w_logical(),
                app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            if drag.started {
                let pin_top_y = panel_rect.y
                    + crate::controls::total_logical_height() * scale_f
                    + (crate::search::input::SEARCH_HORIZONTAL_PAD * 0.5
                        + crate::search::input::SEARCH_ROW_HEIGHT)
                        * scale_f;
                let row_top = crate::launcher::pins_row_top_y(pin_top_y, scale_f);
                let num_pins = app.launcher.pinned_items(&app.apps).len();
                let to = crate::launcher::pin_drop_slot(
                    panel_rect,
                    scale_f,
                    row_top,
                    num_pins,
                    drag.current_x,
                    drag.current_y,
                );
                tracing::info!(from = drag.from_idx, to, "pin drag commit");
                app.launcher.reorder_pins(drag.from_idx, to, &app.apps);
            } else {
                // Treat as a plain click on the pin.
                app.activate_at(crate::app::HitTarget::Pin(drag.from_idx));
            }
        }
    }
}
