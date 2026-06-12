use std::time::Instant;

use lntrn_ui::gpu::{ContextMenu, InteractionContext, MenuItem, WaylandPopupBackend};

use crate::app::{App, ContextTarget};
use crate::desktop;
use crate::wayland::State;
use crate::{
    ClickAction, VIEW_SHOW_HIDDEN_ID, VIEW_SLIDER_ID, ZONE_BREADCRUMB_BASE, ZONE_CLOSE,
    ZONE_DRIVE_DIALOG_CANCEL, ZONE_DRIVE_DIALOG_CONFIRM, ZONE_DRIVE_DIALOG_OK,
    ZONE_DRIVE_DIALOG_SCRIM, ZONE_DRIVE_ITEM_BASE, ZONE_FAVORITE_ITEM_BASE, ZONE_FILE_ITEM_BASE,
    ZONE_MAXIMIZE, ZONE_MENU_VIEW, ZONE_MINIMIZE, ZONE_NAV_BACK, ZONE_NAV_FORWARD, ZONE_NAV_SEARCH,
    ZONE_NAV_SORT, ZONE_NAV_UP, ZONE_NAV_VIEW_TOGGLE, ZONE_PATH_INPUT, ZONE_PHONE_ITEM_BASE,
    ZONE_SIDEBAR_DEVICES_HEADER, ZONE_SIDEBAR_FAVORITES_HEADER, ZONE_SIDEBAR_FAVORITES_PLUS,
    ZONE_SIDEBAR_ITEM_BASE, ZONE_SIDEBAR_PLACES_HEADER, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE,
    ZONE_TAB_NEW, ZONE_TREE_ITEM_BASE,
};

use super::sort_menu_items;

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_click(
    input: &mut InteractionContext,
    app: &mut App,
    view_menu: &mut ContextMenu,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    last_tab_click: &mut Option<(usize, Instant)>,
    tab_drag_press: &mut Option<(usize, f32)>,
    fav_drag_press: &mut Option<(usize, f32)>,
    wf: f32,
    s: f32,
    _bg_opacity: f32,
    current_theme: &str,
    ctrl: bool,
    shift: bool,
) -> ClickAction {
    let _ = current_theme;
    if let Some(zone_id) = input.on_left_pressed() {
        // ── Conflict dialog: captures all clicks while open ─────────
        if app.conflict_dialog.is_some() {
            use crate::conflict::ConflictAction;
            let is_rename = app.pending_rename.is_some();
            match zone_id {
                crate::ZONE_CONFLICT_APPLY_TO_ALL => {
                    if let Some(d) = app.conflict_dialog.as_mut() {
                        d.apply_to_all = !d.apply_to_all;
                    }
                }
                crate::ZONE_CONFLICT_REPLACE => {
                    if is_rename { app.resolve_rename_conflict(ConflictAction::Replace); }
                    else { app.resolve_conflict(ConflictAction::Replace); }
                }
                crate::ZONE_CONFLICT_KEEP_BOTH => {
                    if is_rename { app.resolve_rename_conflict(ConflictAction::KeepBoth); }
                    else { app.resolve_conflict(ConflictAction::KeepBoth); }
                }
                crate::ZONE_CONFLICT_SKIP => {
                    if is_rename { app.resolve_rename_conflict(ConflictAction::Skip); }
                    else { app.resolve_conflict(ConflictAction::Skip); }
                }
                crate::ZONE_CONFLICT_CANCEL | crate::ZONE_CONFLICT_SCRIM => {
                    if is_rename { app.cancel_rename_conflict(); }
                    else { app.cancel_paste(); }
                }
                _ => {}
            }
            return ClickAction::None;
        }
        // ── Sudo password modal: capture all clicks while open ──────
        if app.sudo_prompt.is_some() {
            match zone_id {
                crate::ZONE_SUDO_PASSWORD => {
                    if let Some(p) = app.sudo_prompt.as_mut() {
                        p.cursor = p.password.chars().count();
                    }
                }
                crate::ZONE_SUDO_SUBMIT => app.submit_sudo_prompt(),
                crate::ZONE_SUDO_CANCEL | crate::ZONE_SUDO_SCRIM => {
                    app.cancel_sudo_prompt();
                }
                _ => {}
            }
            return ClickAction::None;
        }
        // ── Cloud login dialog: capture all clicks while open ───────
        if app.cloud_login.is_some() {
            match zone_id {
                crate::ZONE_CLOUD_LOGIN_EMAIL => {
                    if let Some(d) = app.cloud_login.as_mut() {
                        d.focused = crate::dialogs::LoginField::Email;
                        d.email_cursor = d.email_buf.chars().count();
                    }
                }
                crate::ZONE_CLOUD_LOGIN_PASSWORD => {
                    if let Some(d) = app.cloud_login.as_mut() {
                        d.focused = crate::dialogs::LoginField::Password;
                        d.password_cursor = d.password_buf.chars().count();
                    }
                }
                crate::ZONE_CLOUD_LOGIN_SUBMIT => app.submit_cloud_login(),
                crate::ZONE_CLOUD_LOGIN_CANCEL | crate::ZONE_CLOUD_LOGIN_SCRIM => {
                    app.cloud_login = None;
                }
                _ => {}
            }
            return ClickAction::None;
        }
        // ── Drive dialog: capture all clicks while open ─────────────
        if app.drive_dialog.is_some() {
            match zone_id {
                ZONE_DRIVE_DIALOG_CONFIRM => app.confirm_drive_format(),
                ZONE_DRIVE_DIALOG_OK | ZONE_DRIVE_DIALOG_CANCEL | ZONE_DRIVE_DIALOG_SCRIM => {
                    app.dismiss_drive_dialog();
                }
                _ => {}
            }
            return ClickAction::None;
        }
        // If path editing, commit on any click outside the path input
        // (breadcrumb clicks cancel edit and fall through to navigate)
        if app.path_editing && zone_id != ZONE_PATH_INPUT {
            if zone_id >= ZONE_BREADCRUMB_BASE && zone_id < ZONE_BREADCRUMB_BASE + 100 {
                app.cancel_path_edit();
                // Fall through to handle the breadcrumb click below
            } else {
                app.commit_path_edit();
                return ClickAction::None;
            }
        }
        // If renaming, commit on any click outside the rename input
        if app.renaming.is_some() && zone_id != crate::ZONE_RENAME_INPUT {
            app.commit_rename();
            return ClickAction::None;
        }
        match zone_id {
            ZONE_CLOSE => return ClickAction::Close,
            ZONE_MINIMIZE => return ClickAction::Minimize,
            ZONE_MAXIMIZE => return ClickAction::ToggleMaximize,
            ZONE_MENU_VIEW => {
                if !view_menu.is_open() {
                    // Open as popup surface (like right-click menu)
                    let label_x = 10.0; // logical coords for popup positioner
                    let label_y = (crate::layout::title_bar_rect(0.0, s).h / s) as f32;
                    view_menu.set_scale(s);
                    if let Some(backend) = popup_backend {
                        view_menu.open_popup(
                            label_x as f32, label_y,
                            vec![
                                MenuItem::slider(VIEW_SLIDER_ID, "Scale", app.icon_zoom),
                                MenuItem::checkbox(VIEW_SHOW_HIDDEN_ID, "Show Hidden Files", app.show_hidden),
                                MenuItem::checkbox(
                                    crate::VIEW_SHOW_TITLEBAR_ID,
                                    "Show Title Bar",
                                    !crate::layout::chrome_hidden(),
                                ),
                                MenuItem::checkbox(
                                    crate::VIEW_SOLID_DIVIDERS_ID,
                                    "Solid Accent Dividers",
                                    crate::sections::SOLID_DIVIDERS
                                        .load(std::sync::atomic::Ordering::Relaxed),
                                ),
                            ],
                            backend,
                        );
                    }
                } else {
                    if let Some(backend) = popup_backend {
                        view_menu.close_popups(backend);
                    }
                }
            }
            ZONE_NAV_VIEW_TOGGLE => {
                app.cycle_view_mode();
            }
            crate::ZONE_NAV_PREVIEW_TOGGLE => {
                app.preview_open = !app.preview_open;
            }
            crate::ZONE_PREVIEW_RESIZE => {
                if let Some((cx, _)) = input.cursor() {
                    app.preview_drag = Some((cx, app.preview_width));
                }
            }
            ZONE_PATH_INPUT => {
                if !app.path_editing {
                    app.start_path_edit();
                }
            }
            id if id >= ZONE_BREADCRUMB_BASE && id < ZONE_BREADCRUMB_BASE + 100 => {
                let segments = crate::sections::breadcrumb_segments(&app.current_dir, s);
                let seg_idx = (id - ZONE_BREADCRUMB_BASE) as usize + app.breadcrumb_skip;
                if let Some((_, path)) = segments.get(seg_idx) {
                    app.navigate_to(path.clone());
                }
            }
            ZONE_NAV_BACK => app.go_back(),
            ZONE_NAV_FORWARD => app.go_forward(),
            ZONE_NAV_UP => app.go_up(),
            crate::ZONE_NAV_CLOUD => app.open_cloud_or_login(),
            ZONE_NAV_SEARCH => {
                if app.searching {
                    app.close_search();
                } else {
                    app.start_search();
                }
            }
            ZONE_NAV_SORT => {
                if context_menu.is_open() {
                    if let Some(backend) = popup_backend {
                        context_menu.close_popups(backend);
                    }
                } else {
                    // Anchor below the sort button. Use logical coordinates for
                    // the popup positioner — the title bar offset is in the
                    // sort_button_rect's y already, so convert from physical.
                    app.context_target = Some(ContextTarget::Empty);
                    let btn = crate::layout::sort_button_rect(wf, s);
                    let lx = (btn.x / s) as f32;
                    let ly = ((btn.y + btn.h) / s) as f32;
                    context_menu.set_scale(s);
                    let items = sort_menu_items(app);
                    if let Some(backend) = popup_backend {
                        context_menu.open_popup(lx, ly, items, backend);
                    } else {
                        context_menu.open(btn.x, btn.y + btn.h, items);
                    }
                }
            }
            ZONE_TAB_NEW => {
                app.new_tab();
            }
            id if id >= ZONE_TAB_CLOSE_BASE && id < ZONE_TAB_NEW => {
                let idx = (id - ZONE_TAB_CLOSE_BASE) as usize;
                app.close_tab(idx);
            }
            id if id >= ZONE_TAB_BASE && id < ZONE_TAB_CLOSE_BASE => {
                let idx = (id - ZONE_TAB_BASE) as usize;
                let now = Instant::now();
                let is_double = if let Some((prev_idx, prev_time)) = *last_tab_click {
                    prev_idx == idx && now.duration_since(prev_time).as_millis() < 400
                } else {
                    false
                };
                if is_double {
                    app.toggle_pin(idx);
                    *last_tab_click = None;
                } else {
                    app.switch_tab(idx);
                    *last_tab_click = Some((idx, now));
                    // Record press for pinned tab drag detection
                    if idx < app.tabs.len() && app.tabs[idx].pinned {
                        if let Some((cx, _)) = input.cursor() {
                            *tab_drag_press = Some((idx, cx));
                        }
                    }
                }
            }
            id if id >= ZONE_TREE_ITEM_BASE => {
                let idx = (id - ZONE_TREE_ITEM_BASE) as usize;
                if idx < app.tree_entries.len() {
                    let te = &app.tree_entries[idx];
                    if !ctrl && !shift && app.pick.is_none() {
                        // Plain press: defer the action (expand/collapse for
                        // folders, launch for files) to release so a >5px move
                        // becomes a drag instead — same pending pattern as
                        // grid/list. The release handler in wayland_loop runs
                        // the deferred action when no drag started.
                        app.pending_tree_open = Some(idx);
                        if let Some((cx, cy)) = input.cursor() {
                            app.press_pos = Some((cx, cy));
                        }
                        app.press_shift = false;
                        app.press_ctrl = false;
                    } else if te.entry.is_dir && !ctrl && !shift {
                        // Pick-mode folder click: toggle expansion and point
                        // current_dir at it so the path bar follows the click,
                        // the listing refreshes, and Save targets it. Use the
                        // lightweight tree navigate — NOT navigate_to, whose
                        // scroll reset would jump the tree to the top on every
                        // click and break drilling in. The tree stays anchored
                        // at `tree_root`. (Pickers act at press — no drag.)
                        let path = te.entry.path.clone();
                        app.toggle_tree_expand(path.clone());
                        app.pick_tree_navigate(path);
                    } else {
                        let path = te.entry.path.clone();
                        let entry_idx = app.entries.iter().position(|e| e.path == path);
                        let is_pick = app.pick.is_some();
                        let multi = app.pick.as_ref().map_or(false, |p| p.multiple);

                        if ctrl || shift {
                            // Mark the press so the rubber-band branch in the
                            // loop doesn't clear our selection.
                            if let Some((cx, cy)) = input.cursor() {
                                app.press_pos = Some((cx, cy));
                            }
                            if let Some(i) = entry_idx {
                                app.pending_open = Some(i);
                            } else {
                                // Nested rows have no entries index — guard the
                                // rubber-band via a dedicated flag instead.
                                app.suppress_rubber_band = true;
                            }
                            app.press_shift = shift;
                            app.press_ctrl = ctrl;
                        }
                        if ctrl {
                            if let Some(i) = entry_idx {
                                app.entries[i].selected = !app.entries[i].selected;
                                app.selection_anchor = Some(i);
                            }
                            if is_pick {
                                if app.pick_tree_selection.contains(&path) {
                                    app.pick_tree_selection.remove(&path);
                                } else {
                                    app.pick_tree_selection.insert(path.clone());
                                }
                            }
                        } else if shift {
                            if let Some(i) = entry_idx {
                                let anchor = app.selection_anchor.unwrap_or(i);
                                app.select_range(anchor, i);
                                app.selection_anchor = Some(i);
                            }
                        } else if is_pick {
                            // Pick mode single-click: select this row.
                            // Track via pick_tree_selection so nested rows work.
                            let now = std::time::Instant::now();
                            let is_double = app.last_click_path.as_ref().map_or(false, |p| p == &path)
                                && app.last_click_time.map_or(false, |t| now.duration_since(t).as_millis() < 400);
                            app.last_click_time = Some(now);
                            app.last_click_path = Some(path.clone());

                            if !multi {
                                app.clear_selection();
                                app.pick_tree_selection.clear();
                            }
                            app.pick_tree_selection.insert(path.clone());
                            if let Some(i) = entry_idx {
                                app.entries[i].selected = true;
                                app.selection_anchor = Some(i);
                            }
                            app.suppress_rubber_band = true;
                            if let Some((cx, cy)) = input.cursor() {
                                app.press_pos = Some((cx, cy));
                            }
                            if is_double {
                                app.confirm_pick();
                            }
                        }
                        // (Non-pick plain clicks never reach here — the
                        // pending_tree_open arm above defers them to release.)
                    }
                }
            }
            id if id >= ZONE_FILE_ITEM_BASE => {
                let idx = (id - ZONE_FILE_ITEM_BASE) as usize;
                if app.searching && !app.search_buf.is_empty() {
                    // Search result clicked — navigate to parent and highlight,
                    // or open file directly
                    if idx < app.search_results.len() {
                        let entry = app.search_results[idx].clone();
                        if entry.is_dir {
                            app.close_search();
                            app.navigate_to(entry.path);
                        } else {
                            let path = entry.path.clone();
                            let ext = path.extension()
                                .and_then(|e| e.to_str())
                                .map(|s| s.to_lowercase())
                                .unwrap_or_default();
                            if let Some(app) = desktop::default_app_for_extension(&ext) {
                                desktop::launch_app(&app.exec, &path);
                            } else {
                                std::thread::spawn(move || {
                                    let _ = std::process::Command::new("xdg-open")
                                        .arg(&path).spawn();
                                });
                            }
                        }
                    }
                } else if idx < app.entries.len() {
                    // Always record `pending_open` so the rubber-band branch
                    // below (which fires for ClickAction::None when no
                    // pending_open is set) doesn't kick in and clear the
                    // selection we just made. The release handler uses the
                    // press_* modifiers to decide what to actually do.
                    app.pending_open = Some(idx);
                    if let Some((cx, cy)) = input.cursor() {
                        app.press_pos = Some((cx, cy));
                    }
                    app.press_shift = shift;
                    app.press_ctrl = ctrl;
                    if ctrl {
                        // Toggle this entry's selection immediately so the user
                        // sees feedback. Release will skip on_item_click.
                        app.entries[idx].selected = !app.entries[idx].selected;
                        app.selection_anchor = Some(idx);
                    } else if !shift {
                        app.select_item(idx);
                        app.selection_anchor = Some(idx);
                    }
                }
            }
            id if id >= ZONE_SIDEBAR_ITEM_BASE && id < ZONE_DRIVE_ITEM_BASE => {
                let idx = (id - ZONE_SIDEBAR_ITEM_BASE) as usize;
                app.on_sidebar_click(idx);
            }
            id if id >= ZONE_DRIVE_ITEM_BASE && id < ZONE_PHONE_ITEM_BASE => {
                let idx = (id - ZONE_DRIVE_ITEM_BASE) as usize;
                app.on_drive_click(idx);
            }
            id if id >= ZONE_PHONE_ITEM_BASE && id < ZONE_TAB_BASE => {
                let idx = (id - ZONE_PHONE_ITEM_BASE) as usize;
                app.on_phone_click(idx);
            }
            id if id >= ZONE_FAVORITE_ITEM_BASE && id < ZONE_FAVORITE_ITEM_BASE + 100 => {
                let idx = (id - ZONE_FAVORITE_ITEM_BASE) as usize;
                app.on_favorite_click(idx);
                // Record press position so the loop can promote it to a
                // reorder drag if the cursor moves > threshold vertically.
                if let Some((_, cy)) = input.cursor() {
                    *fav_drag_press = Some((idx, cy));
                }
            }
            ZONE_SIDEBAR_PLACES_HEADER => {
                app.places_collapsed = !app.places_collapsed;
            }
            ZONE_SIDEBAR_FAVORITES_HEADER => {
                app.favorites_collapsed = !app.favorites_collapsed;
            }
            ZONE_SIDEBAR_DEVICES_HEADER => {
                app.devices_collapsed = !app.devices_collapsed;
            }
            ZONE_SIDEBAR_FAVORITES_PLUS => {
                // Pin the current directory.
                let cur = app.current_dir.clone();
                app.add_favorite(cur);
            }
            crate::ZONE_PROGRESS_CANCEL => {
                app.cancel_op();
            }
            crate::ZONE_PICK_CONFIRM => {
                app.confirm_pick();
                return ClickAction::Close;
            }
            crate::ZONE_PICK_CANCEL => {
                app.cancel_pick();
                return ClickAction::Close;
            }
            crate::ZONE_PICK_FILTER => {
                app.cycle_filter();
            }
            crate::ZONE_PICK_FILENAME => {
                if !app.save_name_editing {
                    app.save_name_editing = true;
                    app.save_name_cursor = app.save_name_buf.len();
                }
            }
            _ => {}
        }
    }
    ClickAction::None
}
