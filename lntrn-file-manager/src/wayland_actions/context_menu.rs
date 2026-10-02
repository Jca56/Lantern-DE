use std::path::PathBuf;

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{ContextMenu, InteractionContext, MenuEvent, MenuItem, WaylandPopupBackend};

use crate::app::{App, ContextTarget};
use crate::desktop::{self, DesktopApp};
use crate::fs::SortBy;
use crate::settings::Settings;
use crate::wayland::State;
use crate::{
    CTX_ADD_FAVORITE, CTX_CHANGE_ICON, CTX_CLOSE_WINDOW, CTX_COMPRESS, CTX_COPY, CTX_COPY_NAME,
    CTX_COPY_PATH, CTX_CUT, CTX_DRIVE_EJECT, CTX_DRIVE_FORMAT, CTX_DRIVE_PROPERTIES, CTX_DUPLICATE,
    CTX_EMPTY_TRASH, CTX_EXTRACT, CTX_LNTRN, CTX_MAXIMIZE, CTX_MINIMIZE, CTX_NEW_FILE,
    CTX_NEW_FOLDER, CTX_NEW_FOLDER_BLUE, CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE,
    CTX_NEW_FOLDER_PLAIN, CTX_NEW_FOLDER_PURPLE, CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW,
    CTX_NEXT_TAB, CTX_OPEN, CTX_OPEN_AS_ROOT, CTX_OPEN_LOCATION, CTX_OPEN_TERMINAL, CTX_OPEN_WITH,
    CTX_OPEN_WITH_BASE, CTX_PASTE, CTX_PREV_TAB, CTX_PROPERTIES, CTX_REMOVE_FAVORITE, CTX_RENAME,
    CTX_ROOT_MODE, CTX_SELECT_ALL, CTX_SHOW_HIDDEN, CTX_SORT_BY, CTX_SORT_DATE, CTX_SORT_NAME,
    CTX_SORT_SIZE, CTX_SORT_TYPE, CTX_TRASH,
};

use super::{apply_sort_selection, sort_menu_items};

/// Build the standard right-click menu for a single file/folder. Shared
/// between the Item (entries-index) and Path (nested tree row) branches.
#[allow(clippy::too_many_arguments)]
fn build_item_menu(
    is_dir: bool,
    is_archive: bool,
    is_image: bool,
    allow_rename: bool,
    in_trash: bool,
    root_offered: bool,
    has_clipboard: bool,
    open_with_apps: &[DesktopApp],
    fav_state: FavoriteState,
) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::action(CTX_OPEN, "Open")];
    if !is_dir && !open_with_apps.is_empty() {
        let children: Vec<MenuItem> = open_with_apps
            .iter()
            .enumerate()
            .map(|(i, a)| MenuItem::action(CTX_OPEN_WITH_BASE + i as u32, &a.name))
            .collect();
        v.push(MenuItem::submenu(CTX_OPEN_WITH, "Open With", children));
    }
    // A folder only: it is opened with root mode on for it. Fox cannot
    // open a file as root, so a file's menu does not pretend to.
    if is_dir && root_offered {
        v.push(MenuItem::action(CTX_OPEN_AS_ROOT, "Open as Root"));
    }
    v.push(MenuItem::separator());
    v.push(MenuItem::action_with(CTX_CUT, "Cut", "Ctrl+X"));
    v.push(MenuItem::action_with(CTX_COPY, "Copy", "Ctrl+C"));
    if has_clipboard {
        v.push(MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+V"));
    }
    // Nothing is made inside the Trash: a duplicate, an archive or an
    // unpacked folder there would have no restore record.
    if !in_trash {
        v.push(MenuItem::action(CTX_DUPLICATE, "Duplicate"));
    }
    v.push(MenuItem::separator());
    v.push(MenuItem::action(CTX_COPY_PATH, "Copy Path"));
    v.push(MenuItem::action(CTX_COPY_NAME, "Copy Name"));
    v.push(MenuItem::separator());
    let group_start = v.len();
    if is_archive && !in_trash {
        v.push(MenuItem::action(CTX_EXTRACT, "Extract Here"));
    }
    if !in_trash {
        v.push(MenuItem::action(CTX_COMPRESS, "Compress"));
    }
    if is_image {
        v.push(MenuItem::action(
            crate::CTX_SET_WALLPAPER,
            "Set as Wallpaper",
        ));
    }
    // In the Trash this group can be empty: no second separator then.
    if v.len() > group_start {
        v.push(MenuItem::separator());
    }
    if allow_rename {
        v.push(MenuItem::action(CTX_RENAME, "Rename"));
    }
    if in_trash {
        v.push(MenuItem::action(crate::CTX_RESTORE, "Restore"));
        v.push(MenuItem::action_danger(CTX_TRASH, "Delete Permanently"));
    } else {
        v.push(MenuItem::action_danger(CTX_TRASH, "Move to Trash"));
    }
    v.push(MenuItem::separator());
    if is_dir {
        match fav_state {
            FavoriteState::NotFavorite => {
                v.push(MenuItem::action(CTX_ADD_FAVORITE, "Add to Favorites"))
            }
            FavoriteState::Favorite => v.push(MenuItem::action(
                CTX_REMOVE_FAVORITE,
                "Remove from Favorites",
            )),
            FavoriteState::NotApplicable => {}
        }
        // Change-icon now lives inside Properties — click the icon at the top
        // of the dialog to open the picker.
    }
    v.push(MenuItem::action(CTX_PROPERTIES, "Properties"));
    v
}

/// Whether the menus offer root mode in the folder shown: not in a file
/// chooser, not in the Trash, and not on a phone or a network folder, which
/// root cannot see. (`App::enter_root_mode` refuses the same places.)
fn root_offered(app: &App) -> bool {
    app.pick.is_none() && !app.in_trash() && !crate::fs::is_slow_path(&app.current_dir)
}

/// Mini title bar at the top of the content-area context menu (terminal
/// style): window controls, the "lntrn" brand label (navigates home), and
/// tab-cycle chevrons when more than one tab is open. Handy in rice mode
/// where the real title bar is hidden.
fn controls_row(app: &App) -> MenuItem {
    let mut controls = MenuItem::window_controls(CTX_MINIMIZE, CTX_MAXIMIZE, CTX_CLOSE_WINDOW)
        .controls_title(CTX_LNTRN, "lntrn");
    if app.tabs.len() > 1 {
        controls = controls.controls_nav(CTX_PREV_TAB, CTX_NEXT_TAB);
    }
    controls
}

/// Whether a right-clicked target is currently pinned. Controls which of
/// "Add to Favorites" / "Remove from Favorites" appears in the menu.
#[derive(Copy, Clone)]
enum FavoriteState {
    NotFavorite,
    Favorite,
    NotApplicable,
}

pub(crate) fn handle_right_click(
    app: &mut App,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    input: &InteractionContext,
    open_with_apps: &mut Vec<DesktopApp>,
    wf: f32,
    hf: f32,
    s: f32,
) {
    let Some((cx, cy)) = input.cursor() else {
        return;
    };
    // Quick Look covers the window; the view underneath must not react.
    if app.quick_look.is_some() {
        return;
    }

    // Rebuild the sidebar layout so hit-tests match what's currently on
    // screen (collapsed sections, favorites count, how far it is scrolled).
    // A row scrolled out of the sidebar's strip is not under the pointer,
    // whatever its rect says: the pointer is then on the nav or status bar.
    let sb = app.sidebar_layout(hf, s);
    let in_sidebar = sb.viewport.contains(cx, cy);

    // ── Sidebar places: right-click → place-specific menu ───────────────
    // Currently only the Trash place gets a menu (Empty Trash). Other places
    // could grow their own actions later.
    for (i, r) in sb.place_items.iter().enumerate() {
        if in_sidebar && r.contains(cx, cy) {
            let Some(place) = app.sidebar_places().get(i) else {
                return;
            };
            if place.name != "Trash" {
                return;
            }
            let items = vec![MenuItem::action_danger(CTX_EMPTY_TRASH, "Empty Trash")];
            context_menu.set_scale(s);
            if let Some(backend) = popup_backend {
                let lx = (cx / s) as f32;
                let ly = (cy / s) as f32;
                context_menu.open_popup(lx, ly, items, backend);
            } else {
                context_menu.open(cx, cy, items);
            }
            return;
        }
    }

    // ── Sidebar favorites: right-click → remove ─────────────────────────
    for (i, r) in sb.favorite_items.iter().enumerate() {
        if in_sidebar && r.contains(cx, cy) {
            app.context_target = Some(ContextTarget::Favorite(i));
            let items = vec![
                MenuItem::action(CTX_OPEN, "Open"),
                MenuItem::separator(),
                MenuItem::action_danger(CTX_REMOVE_FAVORITE, "Remove from Favorites"),
            ];
            context_menu.set_scale(s);
            if let Some(backend) = popup_backend {
                let lx = (cx / s) as f32;
                let ly = (cy / s) as f32;
                context_menu.open_popup(lx, ly, items, backend);
            } else {
                context_menu.open(cx, cy, items);
            }
            return;
        }
    }

    // ── Sidebar drives: right-click → eject / format / properties ───────
    for (i, r) in sb.drive_items.iter().enumerate() {
        if in_sidebar && r.contains(cx, cy) {
            let drive = app.drives[i].clone();
            let mut items = vec![
                MenuItem::action(CTX_DRIVE_FORMAT, "Format to ext4…"),
                MenuItem::separator(),
                MenuItem::action(CTX_DRIVE_PROPERTIES, "Properties"),
            ];
            if drive.mounted && drive.removable {
                items.insert(0, MenuItem::action(CTX_DRIVE_EJECT, "Eject"));
                items.insert(1, MenuItem::separator());
            }
            // Internal "System"/"Boot" drives: properties only
            if !drive.removable {
                items = vec![MenuItem::action(CTX_DRIVE_PROPERTIES, "Properties")];
            }
            app.context_target = Some(ContextTarget::Drive(drive.device.clone()));
            context_menu.set_scale(s);
            if let Some(backend) = popup_backend {
                let lx = (cx / s) as f32;
                let ly = (cy / s) as f32;
                context_menu.open_popup(lx, ly, items, backend);
            } else {
                context_menu.open(cx, cy, items);
            }
            return;
        }
    }

    // Split view: right-clicking the unfocused pane focuses it first, so the
    // flow below (which reads the flat = focused-pane state) targets the pane
    // that was actually clicked. Zone ids stay P2 — the row match below
    // handles both families.
    if app.split.is_some() {
        let is_p2 = matches!(
            input.zone_at(cx, cy),
            Some(zone) if zone == crate::ZONE_P2_CONTENT
                || zone == crate::ZONE_P2_SCROLLBAR
                || (crate::ZONE_P2_VIEW_TOGGLE..=crate::ZONE_P2_PATH).contains(&zone)
                || zone >= crate::ZONE_P2_FILE_BASE
        );
        if is_p2 {
            let other = match app.split_focused() {
                Some(crate::app::PaneSide::Left) => crate::app::PaneSide::Right,
                _ => crate::app::PaneSide::Left,
            };
            app.focus_pane(other);
        }
    }

    let cr = app.active_content_rect(wf, hf, s);
    if !cr.contains(cx, cy) {
        return;
    }

    // Search mode: use list-based hit detection against search_results
    if app.searching && !app.search_buf.is_empty() {
        let row_h = crate::layout::search_list_row_h(s, app.icon_zoom);
        let hdr_h = crate::layout::list_header_h(s, app.icon_zoom);
        let base_y = cr.y - app.scroll_offset;
        // Below the column header only: a row scrolled under it is not
        // what the pointer is on.
        let on_rows = crate::layout::list_rows_rect(cr, s, app.icon_zoom).contains(cx, cy);
        let clicked_idx = (0..app.search_results.len()).filter(|_| on_rows).find(|&i| {
            let y = base_y + hdr_h + i as f32 * row_h;
            Rect::new(cr.x, y, cr.w, row_h).contains(cx, cy)
        });
        if let Some(idx) = clicked_idx {
            app.context_target = Some(ContextTarget::SearchItem(idx));
            let entry = &app.search_results[idx];
            let mut items = vec![
                MenuItem::action(CTX_OPEN, "Open"),
                MenuItem::action(CTX_OPEN_LOCATION, "Open Location"),
                MenuItem::separator(),
                MenuItem::action(CTX_COPY_PATH, "Copy Path"),
                MenuItem::action(CTX_COPY_NAME, "Copy Name"),
            ];
            if !entry.is_dir {
                let ext = entry
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                *open_with_apps = desktop::apps_for_extension(&ext);
                if !open_with_apps.is_empty() {
                    let children: Vec<MenuItem> = open_with_apps
                        .iter()
                        .enumerate()
                        .map(|(i, a)| MenuItem::action(CTX_OPEN_WITH_BASE + i as u32, &a.name))
                        .collect();
                    items.insert(1, MenuItem::submenu(CTX_OPEN_WITH, "Open With", children));
                }
            }
            context_menu.set_scale(s);
            if let Some(backend) = popup_backend {
                // Mini title bar up top — window mode only; the desktop
                // surface has no window to control.
                items.insert(0, controls_row(app));
                items.insert(1, MenuItem::separator());
                let lx = (cx / s) as f32;
                let ly = (cy / s) as f32;
                context_menu.open_popup(lx, ly, items, backend);
            } else {
                context_menu.open(cx, cy, items);
            }
        }
        return;
    }

    // Clear any leftover override from a previous right-click — every press
    // starts from a clean slate (selection-based by default).
    app.context_override_paths.clear();

    // Use the zones registered by render.rs so the hit-test matches the
    // current view's actual row geometry. The previous grid-only math
    // (file_item_rect) picked the wrong row in List/Tree views because
    // their rows are taller than a grid cell.
    enum ClickedRow {
        Item(usize),
        NestedPath(PathBuf, bool), // (path, is_dir) for tree rows not in entries
        None,
    }
    let clicked_row = match input.zone_at(cx, cy) {
        // Unfocused-pane rows (the pane was focused above, so the flat
        // state these indices point into is the right pane's).
        Some(zone) if zone >= crate::ZONE_P2_TREE_BASE => {
            let ti = (zone - crate::ZONE_P2_TREE_BASE) as usize;
            if let Some(te) = app.tree_entries.get(ti) {
                let path = te.entry.path.clone();
                let is_dir = te.entry.is_dir;
                if let Some(idx) = app.entries.iter().position(|e| e.path == path) {
                    ClickedRow::Item(idx)
                } else {
                    ClickedRow::NestedPath(path, is_dir)
                }
            } else {
                ClickedRow::None
            }
        }
        Some(zone) if zone >= crate::ZONE_P2_FILE_BASE => {
            let fi = (zone - crate::ZONE_P2_FILE_BASE) as usize;
            if fi < app.entries.len() {
                ClickedRow::Item(fi)
            } else {
                ClickedRow::None
            }
        }
        Some(zone) if zone >= crate::ZONE_TREE_ITEM_BASE => {
            let ti = (zone - crate::ZONE_TREE_ITEM_BASE) as usize;
            if let Some(te) = app.tree_entries.get(ti) {
                let path = te.entry.path.clone();
                let is_dir = te.entry.is_dir;
                if let Some(idx) = app.entries.iter().position(|e| e.path == path) {
                    ClickedRow::Item(idx)
                } else {
                    ClickedRow::NestedPath(path, is_dir)
                }
            } else {
                ClickedRow::None
            }
        }
        Some(zone) if zone >= crate::ZONE_FILE_ITEM_BASE && zone < crate::ZONE_TREE_ITEM_BASE => {
            let fi = (zone - crate::ZONE_FILE_ITEM_BASE) as usize;
            if fi < app.entries.len() {
                ClickedRow::Item(fi)
            } else {
                ClickedRow::None
            }
        }
        _ => ClickedRow::None,
    };

    let has_clipboard = app.clipboard.is_some();
    let mut items = match clicked_row {
        ClickedRow::Item(idx) => {
            app.select_item(idx);
            app.context_target = Some(ContextTarget::Item(idx));
            let is_dir = app.entries[idx].is_dir;
            let is_archive = !is_dir && crate::file_ops::is_archive(&app.entries[idx].path);
            let ext = if !is_dir {
                app.entries[idx].extension()
            } else {
                String::new()
            };
            if !is_dir {
                *open_with_apps = desktop::apps_for_extension(&ext);
            }
            let fav_state = if is_dir {
                if app.is_favorite(&app.entries[idx].path) {
                    FavoriteState::Favorite
                } else {
                    FavoriteState::NotFavorite
                }
            } else {
                FavoriteState::NotApplicable
            };
            let is_image = !is_dir && crate::icons::is_raster_image_file(&app.entries[idx].name);
            build_item_menu(
                is_dir,
                is_archive,
                is_image,
                true,
                app.in_trash(),
                root_offered(app),
                has_clipboard,
                open_with_apps,
                fav_state,
            )
        }
        ClickedRow::NestedPath(path, is_dir) => {
            // Nested tree row — clear any entries-based selection so the
            // path override is the sole source of truth for this action.
            app.clear_selection();
            app.context_override_paths = vec![path.clone()];
            app.context_target = Some(ContextTarget::Path(path.clone()));
            let is_archive = !is_dir && crate::file_ops::is_archive(&path);
            let ext = if !is_dir {
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            if !is_dir {
                *open_with_apps = desktop::apps_for_extension(&ext);
            }
            let fav_state = if is_dir {
                if app.is_favorite(&path) {
                    FavoriteState::Favorite
                } else {
                    FavoriteState::NotFavorite
                }
            } else {
                FavoriteState::NotApplicable
            };
            // `allow_rename = false` — rename UI keys off an entries index and
            // doesn't have a path-based variant yet, so we hide it for nested rows.
            let is_image = !is_dir
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(crate::icons::is_raster_image_file);
            build_item_menu(
                is_dir,
                is_archive,
                is_image,
                false,
                app.in_trash(),
                root_offered(app),
                has_clipboard,
                open_with_apps,
                fav_state,
            )
        }
        ClickedRow::None => {
            app.clear_selection();
            app.context_target = Some(ContextTarget::Empty);
            let mut v = Vec::new();
            v.push(MenuItem::checkbox(
                CTX_SHOW_HIDDEN,
                "Show Hidden Files",
                app.show_hidden,
            ));
            v.push(MenuItem::slider(
                crate::CTX_ICON_SIZE,
                "Icon Size",
                app.icon_zoom,
            ));
            v.push(MenuItem::separator());
            if has_clipboard {
                v.push(MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+V"));
                v.push(MenuItem::separator());
            }
            v.push(MenuItem::action(CTX_NEW_FILE, "New File"));
            v.push(MenuItem::color_swatches(
                "New Folder",
                vec![
                    (CTX_NEW_FOLDER_PLAIN, Color::from_rgb8(140, 140, 140)),
                    (CTX_NEW_FOLDER_RED, Color::from_rgb8(220, 60, 60)),
                    (CTX_NEW_FOLDER_ORANGE, Color::from_rgb8(230, 150, 40)),
                    (CTX_NEW_FOLDER_YELLOW, Color::from_rgb8(220, 200, 50)),
                    (CTX_NEW_FOLDER_GREEN, Color::from_rgb8(70, 180, 80)),
                    (CTX_NEW_FOLDER_BLUE, Color::from_rgb8(60, 130, 220)),
                    (CTX_NEW_FOLDER_PURPLE, Color::from_rgb8(160, 80, 210)),
                ],
            ));
            v.push(MenuItem::separator());
            v.push(MenuItem::submenu(
                CTX_SORT_BY,
                "Sort By",
                sort_menu_items(app),
            ));
            v.push(MenuItem::separator());
            v.push(MenuItem::action(CTX_SELECT_ALL, "Select All"));
            v.push(MenuItem::action(CTX_OPEN_TERMINAL, "Open Terminal Here"));
            // Root mode for the folder shown, and the way back out of it
            // (the ROOT badge in the nav bar is the other).
            if app.root_mode() {
                v.push(MenuItem::separator());
                v.push(MenuItem::action(CTX_ROOT_MODE, "Leave Root Mode"));
            } else if root_offered(app) {
                v.push(MenuItem::separator());
                v.push(MenuItem::action_danger(CTX_ROOT_MODE, "Root Mode Here"));
            }
            v
        }
    };

    context_menu.set_scale(s);
    if let Some(backend) = popup_backend {
        // Window mode: open as xdg_popup surface, with the terminal-style
        // mini title bar up top (desktop surface has no window to control).
        items.insert(0, controls_row(app));
        items.insert(1, MenuItem::separator());
        let lx = (cx / s) as f32;
        let ly = (cy / s) as f32;
        context_menu.open_popup(lx, ly, items, backend);
    } else {
        // Desktop mode: open inline (rendered on same surface)
        context_menu.open(cx, cy, items);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_ctx_event(
    app: &mut App,
    settings: &mut Settings,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    open_with_apps: &[DesktopApp],
    file_info: &mut crate::file_info::FileInfoCache,
    toplevel: &Option<wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel>,
    maximized: bool,
    running: &mut bool,
    event: MenuEvent,
) {
    match event {
        MenuEvent::Action(id) => {
            match id {
                // Mini title bar (window controls row)
                CTX_MINIMIZE => {
                    if let Some(t) = toplevel {
                        t.set_minimized();
                    }
                }
                CTX_MAXIMIZE => {
                    if let Some(t) = toplevel {
                        if maximized {
                            t.unset_maximized();
                        } else {
                            t.set_maximized();
                        }
                    }
                }
                CTX_CLOSE_WINDOW => *running = false,
                CTX_LNTRN => app.navigate_to_home(),
                CTX_PREV_TAB => {
                    let n = app.tabs.len();
                    if n > 1 {
                        let i = if app.current_tab == 0 {
                            n - 1
                        } else {
                            app.current_tab - 1
                        };
                        app.switch_tab(i);
                    }
                }
                CTX_NEXT_TAB => {
                    let n = app.tabs.len();
                    if n > 1 {
                        app.switch_tab((app.current_tab + 1) % n);
                    }
                }
                CTX_OPEN => {
                    if let Some(ContextTarget::Favorite(idx)) = app.context_target.clone() {
                        app.on_favorite_click(idx);
                    } else
                    // In search mode, open the search result directly
                    if let Some(ContextTarget::SearchItem(idx)) = &app.context_target {
                        if let Some(entry) = app.search_results.get(*idx) {
                            let path = entry.path.clone();
                            if entry.is_dir {
                                app.close_search();
                                app.navigate_to(path);
                            } else {
                                app.open_file(path);
                            }
                        }
                    } else if let Some(ContextTarget::Path(path)) = app.context_target.clone() {
                        // Nested tree row: dirs navigate, files launch.
                        if app.is_folder(&path) {
                            app.navigate_to(path);
                        } else {
                            app.open_file(path);
                        }
                    } else {
                        app.open_selected();
                    }
                }
                CTX_OPEN_LOCATION => {
                    if let Some(ContextTarget::SearchItem(idx)) = &app.context_target {
                        if let Some(entry) = app.search_results.get(*idx) {
                            if let Some(parent) = entry.path.parent() {
                                let parent = parent.to_path_buf();
                                app.close_search();
                                app.navigate_to(parent);
                            }
                        }
                    }
                }
                CTX_CUT => app.cut_selected(),
                CTX_COPY => app.copy_selected(),
                CTX_PASTE => app.paste(),
                CTX_RENAME => {
                    if let Some(ContextTarget::Item(idx)) = app.context_target {
                        app.start_rename(idx);
                    }
                }
                // Decides per item: a trashed item is deleted for good
                // (after asking), anything else goes to the Trash.
                CTX_TRASH => app.trash_selected(),
                crate::CTX_RESTORE => app.restore_selected(),
                CTX_COPY_PATH => {
                    let text = match &app.context_target {
                        Some(ContextTarget::Item(idx)) => {
                            app.entries.get(*idx).map(|e| e.path.display().to_string())
                        }
                        Some(ContextTarget::SearchItem(idx)) => app
                            .search_results
                            .get(*idx)
                            .map(|e| e.path.display().to_string()),
                        Some(ContextTarget::Path(path)) => Some(path.display().to_string()),
                        _ => None,
                    };
                    if let Some(text) = text {
                        if let Some(clip) = &app.wayland_clipboard {
                            clip.set_text(&text);
                        }
                    }
                }
                CTX_COPY_NAME => {
                    let text = match &app.context_target {
                        Some(ContextTarget::Item(idx)) => {
                            app.entries.get(*idx).map(|e| e.name.clone())
                        }
                        Some(ContextTarget::SearchItem(idx)) => {
                            app.search_results.get(*idx).map(|e| e.name.clone())
                        }
                        Some(ContextTarget::Path(path)) => {
                            path.file_name().map(|n| n.to_string_lossy().to_string())
                        }
                        _ => None,
                    };
                    if let Some(text) = text {
                        if let Some(clip) = &app.wayland_clipboard {
                            clip.set_text(&text);
                        }
                    }
                }
                crate::CTX_SET_WALLPAPER => {
                    let path = match &app.context_target {
                        Some(ContextTarget::Item(idx)) => {
                            app.entries.get(*idx).map(|e| e.path.clone())
                        }
                        Some(ContextTarget::SearchItem(idx)) => {
                            app.search_results.get(*idx).map(|e| e.path.clone())
                        }
                        Some(ContextTarget::Path(path)) => Some(path.clone()),
                        _ => None,
                    };
                    if let Some(path) = path {
                        if let Err(why) = crate::lantern_config::set_wallpaper(&path) {
                            app.show_message("Couldn\u{2019}t set the wallpaper", why);
                        }
                    }
                }
                CTX_DUPLICATE => app.duplicate_selected(),
                CTX_COMPRESS => app.compress_selected(),
                CTX_EXTRACT => app.extract_selected(),
                CTX_OPEN_AS_ROOT => app.open_as_root(),
                CTX_ROOT_MODE => app.toggle_root_mode(),
                CTX_CHANGE_ICON => {
                    // Spawn a file picker to choose an icon image
                    let folder_path = match app.context_target.clone() {
                        Some(crate::app::ContextTarget::Item(idx))
                            if idx < app.entries.len() && app.entries[idx].is_dir =>
                        {
                            Some(app.entries[idx].path.clone())
                        }
                        // A nested tree row: its entry says what it is,
                        // without asking the disk.
                        Some(crate::app::ContextTarget::Path(path)) if app.is_folder(&path) => {
                            Some(path)
                        }
                        _ => None,
                    };
                    if let Some(folder_path) = folder_path {
                        let refresh = app.refresh_flag();
                        std::thread::spawn(move || {
                            if crate::pick_output::choose_folder_icon(&folder_path) {
                                // Listings carry the attribute: have them
                                // read again.
                                refresh.store(true, std::sync::atomic::Ordering::SeqCst);
                                crate::bg::wake();
                            }
                        });
                    }
                }
                CTX_PROPERTIES => {
                    // Drive context → drive properties dialog
                    if let Some(ContextTarget::Drive(device)) = app.context_target.clone() {
                        app.open_drive_properties(&device);
                    } else {
                        let path = if let Some(ref target) = app.context_target {
                            match target {
                                crate::app::ContextTarget::Item(idx) => {
                                    if *idx < app.entries.len() {
                                        Some(app.entries[*idx].path.clone())
                                    } else {
                                        None
                                    }
                                }
                                crate::app::ContextTarget::SearchItem(idx) => {
                                    app.search_results.get(*idx).map(|e| e.path.clone())
                                }
                                crate::app::ContextTarget::Path(path) => Some(path.clone()),
                                crate::app::ContextTarget::Empty => Some(app.current_dir.clone()),
                                crate::app::ContextTarget::Drive(_) => None,
                                crate::app::ContextTarget::Favorite(idx) => {
                                    app.sidebar_favorites().get(*idx).map(|p| p.path.clone())
                                }
                            }
                        } else {
                            None
                        };
                        if let Some(path) = path {
                            app.open_properties(&path, file_info);
                        }
                    }
                }
                CTX_DRIVE_EJECT => {
                    if let Some(ContextTarget::Drive(device)) = app.context_target.clone() {
                        app.eject_drive(&device);
                    }
                }
                CTX_DRIVE_FORMAT => {
                    if let Some(ContextTarget::Drive(device)) = app.context_target.clone() {
                        app.open_drive_format_dialog(&device);
                    }
                }
                CTX_DRIVE_PROPERTIES => {
                    if let Some(ContextTarget::Drive(device)) = app.context_target.clone() {
                        app.open_drive_properties(&device);
                    }
                }
                CTX_NEW_FOLDER => {
                    let target = free_name(&app.current_dir, "New Folder");
                    new_folder_or_prompt(app, target, None);
                }
                CTX_NEW_FOLDER_PLAIN
                | CTX_NEW_FOLDER_RED
                | CTX_NEW_FOLDER_ORANGE
                | CTX_NEW_FOLDER_YELLOW
                | CTX_NEW_FOLDER_GREEN
                | CTX_NEW_FOLDER_BLUE
                | CTX_NEW_FOLDER_PURPLE => {
                    let target = free_name(&app.current_dir, "New Folder");
                    let color: Option<&'static str> = match id {
                        CTX_NEW_FOLDER_RED => Some("red"),
                        CTX_NEW_FOLDER_ORANGE => Some("orange"),
                        CTX_NEW_FOLDER_YELLOW => Some("yellow"),
                        CTX_NEW_FOLDER_GREEN => Some("green"),
                        CTX_NEW_FOLDER_BLUE => Some("blue"),
                        CTX_NEW_FOLDER_PURPLE => Some("purple"),
                        _ => None,
                    };
                    new_folder_or_prompt(app, target, color);
                }
                CTX_NEW_FILE => {
                    let target = free_name(&app.current_dir, "New File");
                    new_file_or_prompt(app, target);
                }
                CTX_ADD_FAVORITE => {
                    // Resolve the target path from whatever context fired it.
                    let path = target_path_for_favorite(app);
                    if let Some(p) = path {
                        if app.add_favorite(p) {
                            app.persist_favorites(settings);
                        }
                    }
                }
                CTX_REMOVE_FAVORITE => {
                    let mut changed = false;
                    if let Some(ContextTarget::Favorite(idx)) = app.context_target.clone() {
                        app.remove_favorite(idx);
                        changed = true;
                    } else if let Some(path) = target_path_for_favorite(app) {
                        if app.is_favorite(&path) {
                            app.remove_favorite_by_path(&path);
                            changed = true;
                        }
                    }
                    if changed {
                        app.persist_favorites(settings);
                    }
                }
                CTX_EMPTY_TRASH => app.empty_trash(),
                CTX_SELECT_ALL => app.select_all(),
                CTX_OPEN_TERMINAL => app.open_in_terminal(),
                CTX_SORT_NAME => apply_sort_selection(app, settings, SortBy::Name),
                CTX_SORT_SIZE => apply_sort_selection(app, settings, SortBy::Size),
                CTX_SORT_DATE => apply_sort_selection(app, settings, SortBy::Date),
                CTX_SORT_TYPE => apply_sort_selection(app, settings, SortBy::Type),
                id if id >= CTX_OPEN_WITH_BASE => {
                    let app_idx = (id - CTX_OPEN_WITH_BASE) as usize;
                    if let Some(chosen) = open_with_apps.get(app_idx) {
                        let files = match &app.context_target {
                            Some(ContextTarget::SearchItem(idx)) => app
                                .search_results
                                .get(*idx)
                                .map(|entry| vec![entry.path.clone()])
                                .unwrap_or_default(),
                            // Uses `selected_paths()` so the path override from a
                            // nested-tree-row right-click is honored too.
                            _ => app.selected_paths(),
                        };
                        // All of them in one call: an app that takes a list
                        // of files opens them in one window.
                        if !files.is_empty() {
                            if let Err(why) = desktop::launch_app(chosen, &files) {
                                app.show_message("Could not open", why);
                            }
                        }
                    }
                }
                _ => {}
            }
            // Action consumed — clear the path-override so the next
            // selection-based op (cut/copy/paste from kbd, etc.) uses
            // the entries-based selection again.
            app.context_override_paths.clear();
            if let Some(backend) = popup_backend {
                context_menu.close_popups(backend);
            }
        }
        MenuEvent::CheckboxToggled { id, checked } => {
            if id == CTX_SHOW_HIDDEN {
                app.show_hidden = checked;
                settings.show_hidden = checked;
                if app.pick.is_none() {
                    settings.save();
                }
                app.reload();
                // Menu stays open — it now shares a row group with the icon
                // size slider, so the user can adjust both in one visit.
            }
        }
        _ => {}
    }
}

/// First of `base`, `base 2`, `base 3`… that is not taken in `dir`.
fn free_name(dir: &std::path::Path, base: &str) -> PathBuf {
    let mut target = dir.join(base);
    let mut n = 2u32;
    // symlink_metadata: a dangling symlink still occupies the name.
    while target.symlink_metadata().is_ok() && n < 10_000 {
        target = dir.join(format!("{base} {n}"));
        n += 1;
    }
    target
}

/// Create a folder at `target`. Falls back to the sudo prompt on permission
/// denied. On direct success: push undo, reload, focus rename. On sudo
/// fallback: the file gets created later via sudo + a reload happens then,
/// so we skip undo/rename (no path-of-clean-creation to track).
fn new_folder_or_prompt(app: &mut App, target: PathBuf, color: Option<&'static str>) {
    if app.root_mode() {
        app.priv_run(crate::sudo::PendingPrivOp::NewFolder {
            path: target,
            color,
        });
        return;
    }
    match std::fs::create_dir(&target) {
        Ok(()) => {
            if let Some(c) = color {
                crate::icons::set_folder_color(&target, c);
            }
            app.undo_stack
                .push(crate::undo::UndoAction::Create(vec![(target.clone(), true)]));
            app.reload();
            if let Some(idx) = app.entries.iter().position(|e| e.path == target) {
                app.select_item(idx);
                app.start_rename(idx);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            app.priv_run(crate::sudo::PendingPrivOp::NewFolder {
                path: target,
                color,
            });
        }
        Err(_) => {}
    }
}

fn new_file_or_prompt(app: &mut App, target: PathBuf) {
    if app.root_mode() {
        app.priv_run(crate::sudo::PendingPrivOp::NewFile(target));
        return;
    }
    // create_new: an existing file (or a symlink) with this name is never
    // truncated, whatever raced us to it.
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target);
    match created {
        Ok(_) => {
            app.undo_stack
                .push(crate::undo::UndoAction::Create(vec![(target, false)]));
            app.reload();
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            app.priv_run(crate::sudo::PendingPrivOp::NewFile(target));
        }
        Err(_) => {}
    }
}

/// Resolve the path the user is currently targeting via the context menu.
/// Used by Add/Remove Favorite to pin the actual right-clicked folder rather
/// than something else in the selection.
fn target_path_for_favorite(app: &App) -> Option<PathBuf> {
    match app.context_target.clone()? {
        ContextTarget::Item(idx) => app.entries.get(idx).map(|e| e.path.clone()),
        ContextTarget::Path(p) => Some(p),
        ContextTarget::SearchItem(idx) => app.search_results.get(idx).map(|e| e.path.clone()),
        ContextTarget::Empty => Some(app.current_dir.clone()),
        ContextTarget::Favorite(idx) => app.sidebar_favorites().get(idx).map(|p| p.path.clone()),
        ContextTarget::Drive(_) => None,
    }
}
