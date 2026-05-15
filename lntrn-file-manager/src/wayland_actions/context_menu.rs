use std::path::PathBuf;

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{ContextMenu, InteractionContext, MenuEvent, MenuItem, WaylandPopupBackend};

use crate::app::{App, ContextTarget};
use crate::desktop::{self, DesktopApp};
use crate::fs::SortBy;
use crate::layout::{build_sidebar_layout, content_rect};
use crate::settings::Settings;
use crate::wayland::State;
use crate::{
    CTX_ADD_FAVORITE, CTX_CHANGE_ICON, CTX_COMPRESS, CTX_COPY, CTX_COPY_NAME, CTX_COPY_PATH,
    CTX_CUT, CTX_DRIVE_EJECT, CTX_DRIVE_FORMAT, CTX_DRIVE_PROPERTIES, CTX_DUPLICATE,
    CTX_EMPTY_TRASH, CTX_EXTRACT, CTX_NEW_FILE, CTX_NEW_FOLDER, CTX_NEW_FOLDER_BLUE,
    CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE, CTX_NEW_FOLDER_PLAIN, CTX_NEW_FOLDER_PURPLE,
    CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW, CTX_OPEN, CTX_OPEN_AS_ROOT, CTX_OPEN_LOCATION,
    CTX_OPEN_TERMINAL, CTX_OPEN_WITH, CTX_OPEN_WITH_BASE, CTX_PASTE, CTX_PROPERTIES,
    CTX_REMOVE_FAVORITE, CTX_RENAME, CTX_SELECT_ALL, CTX_SHOW_HIDDEN, CTX_SORT_BY, CTX_SORT_DATE,
    CTX_SORT_NAME, CTX_SORT_SIZE, CTX_SORT_TYPE, CTX_TRASH,
};

use super::{apply_sort_selection, sort_menu_items};

/// Build the standard right-click menu for a single file/folder. Shared
/// between the Item (entries-index) and Path (nested tree row) branches.
#[allow(clippy::too_many_arguments)]
fn build_item_menu(
    is_dir: bool,
    is_archive: bool,
    allow_rename: bool,
    in_trash: bool,
    has_clipboard: bool,
    open_with_apps: &[DesktopApp],
    fav_state: FavoriteState,
) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::action(CTX_OPEN, "Open")];
    if !is_dir && !open_with_apps.is_empty() {
        let children: Vec<MenuItem> = open_with_apps.iter().enumerate()
            .map(|(i, a)| MenuItem::action(CTX_OPEN_WITH_BASE + i as u32, &a.name))
            .collect();
        v.push(MenuItem::submenu(CTX_OPEN_WITH, "Open With", children));
    }
    v.push(MenuItem::action(CTX_OPEN_AS_ROOT, "Open as Root"));
    v.push(MenuItem::separator());
    v.push(MenuItem::action_with(CTX_CUT, "Cut", "Ctrl+X"));
    v.push(MenuItem::action_with(CTX_COPY, "Copy", "Ctrl+C"));
    if has_clipboard {
        v.push(MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+V"));
    }
    v.push(MenuItem::action(CTX_DUPLICATE, "Duplicate"));
    v.push(MenuItem::separator());
    v.push(MenuItem::action(CTX_COPY_PATH, "Copy Path"));
    v.push(MenuItem::action(CTX_COPY_NAME, "Copy Name"));
    v.push(MenuItem::separator());
    if is_archive {
        v.push(MenuItem::action(CTX_EXTRACT, "Extract Here"));
    }
    v.push(MenuItem::action(CTX_COMPRESS, "Compress"));
    v.push(MenuItem::separator());
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
            FavoriteState::NotFavorite => v.push(MenuItem::action(CTX_ADD_FAVORITE, "Add to Favorites")),
            FavoriteState::Favorite => v.push(MenuItem::action(CTX_REMOVE_FAVORITE, "Remove from Favorites")),
            FavoriteState::NotApplicable => {}
        }
        // Change-icon now lives inside Properties — click the icon at the top
        // of the dialog to open the picker.
    }
    v.push(MenuItem::action(CTX_PROPERTIES, "Properties"));
    v
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
    wf: f32, hf: f32, s: f32,
) {
    let Some((cx, cy)) = input.cursor() else { return };

    // Rebuild the sidebar layout so hit-tests match what's currently on
    // screen (collapsed sections, favorites count, etc.).
    let sb = build_sidebar_layout(
        s,
        app.sidebar_places().len(),
        app.sidebar_favorites().len(),
        app.drives.len(),
        app.phones.len(),
        app.places_collapsed,
        app.favorites_collapsed,
        app.devices_collapsed,
    );

    // ── Sidebar places: right-click → place-specific menu ───────────────
    // Currently only the Trash place gets a menu (Empty Trash). Other places
    // could grow their own actions later.
    for (i, r) in sb.place_items.iter().enumerate() {
        if r.contains(cx, cy) {
            let Some(place) = app.sidebar_places().get(i) else { return; };
            if place.name != "Trash" { return; }
            let items = vec![
                MenuItem::action_danger(CTX_EMPTY_TRASH, "Empty Trash"),
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

    // ── Sidebar favorites: right-click → remove ─────────────────────────
    for (i, r) in sb.favorite_items.iter().enumerate() {
        if r.contains(cx, cy) {
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
        if r.contains(cx, cy) {
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
            app.context_target = Some(ContextTarget::Drive(i));
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

    let cr = content_rect(wf, hf, s);
    if !cr.contains(cx, cy) { return; }

    // Search mode: use list-based hit detection against search_results
    if app.searching && !app.search_buf.is_empty() {
        let row_h = crate::layout::search_list_row_h(s, app.icon_zoom);
        let hdr_h = 32.0 * crate::layout::list_zoom_multiplier(app.icon_zoom) * s;
        let base_y = cr.y - app.scroll_offset;
        let clicked_idx = (0..app.search_results.len()).find(|&i| {
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
                let ext = entry.path.extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                *open_with_apps = desktop::apps_for_extension(&ext);
                if !open_with_apps.is_empty() {
                    let children: Vec<MenuItem> = open_with_apps.iter().enumerate()
                        .map(|(i, a)| MenuItem::action(CTX_OPEN_WITH_BASE + i as u32, &a.name))
                        .collect();
                    items.insert(1, MenuItem::submenu(CTX_OPEN_WITH, "Open With", children));
                }
            }
            context_menu.set_scale(s);
            if let Some(backend) = popup_backend {
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
        NestedPath(PathBuf, bool),  // (path, is_dir) for tree rows not in entries
        None,
    }
    let clicked_row = match input.zone_at(cx, cy) {
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
            if fi < app.entries.len() { ClickedRow::Item(fi) } else { ClickedRow::None }
        }
        _ => ClickedRow::None,
    };

    let has_clipboard = app.clipboard.is_some();
    let items = match clicked_row {
        ClickedRow::Item(idx) => {
            app.select_item(idx);
            app.context_target = Some(ContextTarget::Item(idx));
            let is_dir = app.entries[idx].is_dir;
            let is_archive = !is_dir && crate::file_ops::is_archive(&app.entries[idx].path);
            let ext = if !is_dir { app.entries[idx].extension() } else { String::new() };
            if !is_dir {
                *open_with_apps = desktop::apps_for_extension(&ext);
            }
            let fav_state = if is_dir {
                if app.is_favorite(&app.entries[idx].path) {
                    FavoriteState::Favorite
                } else {
                    FavoriteState::NotFavorite
                }
            } else { FavoriteState::NotApplicable };
            build_item_menu(is_dir, is_archive, true, app.in_trash(), has_clipboard, open_with_apps, fav_state)
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
            } else { String::new() };
            if !is_dir {
                *open_with_apps = desktop::apps_for_extension(&ext);
            }
            let fav_state = if is_dir {
                if app.is_favorite(&path) {
                    FavoriteState::Favorite
                } else {
                    FavoriteState::NotFavorite
                }
            } else { FavoriteState::NotApplicable };
            // `allow_rename = false` — rename UI keys off an entries index and
            // doesn't have a path-based variant yet, so we hide it for nested rows.
            build_item_menu(is_dir, is_archive, false, app.in_trash(), has_clipboard, open_with_apps, fav_state)
        }
        ClickedRow::None => {
            app.clear_selection();
            app.context_target = Some(ContextTarget::Empty);
            let mut v = Vec::new();
            if has_clipboard {
                v.push(MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+V"));
                v.push(MenuItem::separator());
            }
            v.push(MenuItem::action(CTX_NEW_FILE, "New File"));
            v.push(MenuItem::color_swatches("New Folder", vec![
                (CTX_NEW_FOLDER_PLAIN,  Color::from_rgb8(140, 140, 140)),
                (CTX_NEW_FOLDER_RED,    Color::from_rgb8(220, 60, 60)),
                (CTX_NEW_FOLDER_ORANGE, Color::from_rgb8(230, 150, 40)),
                (CTX_NEW_FOLDER_YELLOW, Color::from_rgb8(220, 200, 50)),
                (CTX_NEW_FOLDER_GREEN,  Color::from_rgb8(70, 180, 80)),
                (CTX_NEW_FOLDER_BLUE,   Color::from_rgb8(60, 130, 220)),
                (CTX_NEW_FOLDER_PURPLE, Color::from_rgb8(160, 80, 210)),
            ]));
            v.push(MenuItem::separator());
            v.push(MenuItem::submenu(CTX_SORT_BY, "Sort By", sort_menu_items(app)));
            v.push(MenuItem::separator());
            v.push(MenuItem::action(CTX_SELECT_ALL, "Select All"));
            v.push(MenuItem::action(CTX_OPEN_TERMINAL, "Open Terminal Here"));
            v.push(MenuItem::separator());
            v.push(MenuItem::checkbox(CTX_SHOW_HIDDEN, "Show Hidden Files", app.show_hidden));
            v
        }
    };

    context_menu.set_scale(s);
    if let Some(backend) = popup_backend {
        // Window mode: open as xdg_popup surface
        let lx = (cx / s) as f32;
        let ly = (cy / s) as f32;
        context_menu.open_popup(lx, ly, items, backend);
    } else {
        // Desktop mode: open inline (rendered on same surface)
        context_menu.open(cx, cy, items);
    }
}

pub(crate) fn handle_ctx_event(
    app: &mut App, settings: &mut Settings,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    open_with_apps: &[DesktopApp],
    file_info: &mut crate::file_info::FileInfoCache,
    event: MenuEvent,
) {
    match event {
        MenuEvent::Action(id) => {
            match id {
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
                                std::thread::spawn(move || {
                                    let _ = std::process::Command::new("xdg-open")
                                        .arg(&path).spawn();
                                });
                            }
                        }
                    } else if let Some(ContextTarget::Path(path)) = app.context_target.clone() {
                        // Nested tree row — open via xdg-open (dirs navigate, files launch)
                        if path.is_dir() {
                            app.navigate_to(path);
                        } else {
                            std::thread::spawn(move || {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(&path).spawn();
                            });
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
                CTX_TRASH => {
                    if app.in_trash() {
                        app.delete_selected();
                    } else {
                        app.trash_selected();
                    }
                }
                crate::CTX_RESTORE => app.restore_selected(),
                CTX_COPY_PATH => {
                    let text = match &app.context_target {
                        Some(ContextTarget::Item(idx)) =>
                            app.entries.get(*idx).map(|e| e.path.display().to_string()),
                        Some(ContextTarget::SearchItem(idx)) =>
                            app.search_results.get(*idx).map(|e| e.path.display().to_string()),
                        Some(ContextTarget::Path(path)) =>
                            Some(path.display().to_string()),
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
                        Some(ContextTarget::Item(idx)) =>
                            app.entries.get(*idx).map(|e| e.name.clone()),
                        Some(ContextTarget::SearchItem(idx)) =>
                            app.search_results.get(*idx).map(|e| e.name.clone()),
                        Some(ContextTarget::Path(path)) =>
                            path.file_name().map(|n| n.to_string_lossy().to_string()),
                        _ => None,
                    };
                    if let Some(text) = text {
                        if let Some(clip) = &app.wayland_clipboard {
                            clip.set_text(&text);
                        }
                    }
                }
                CTX_DUPLICATE => app.duplicate_selected(),
                CTX_COMPRESS => app.compress_selected(),
                CTX_EXTRACT => app.extract_selected(),
                CTX_OPEN_AS_ROOT => app.open_as_root(),
                CTX_CHANGE_ICON => {
                    // Spawn a file picker to choose an icon image
                    let folder_path = match app.context_target.clone() {
                        Some(crate::app::ContextTarget::Item(idx))
                            if idx < app.entries.len() && app.entries[idx].is_dir =>
                                Some(app.entries[idx].path.clone()),
                        Some(crate::app::ContextTarget::Path(path)) if path.is_dir() =>
                                Some(path),
                        _ => None,
                    };
                    if let Some(folder_path) = folder_path {
                        std::thread::spawn(move || {
                            let output = std::process::Command::new("lntrn-file-manager")
                                .args([
                                    "--pick",
                                    "--title", "Choose Folder Icon",
                                    "--filters", "Images:*.png,*.svg,*.jpg,*.jpeg,*.webp,*.ico",
                                ])
                                .output();
                            if let Ok(out) = output {
                                if out.status.success() {
                                    let chosen = String::from_utf8_lossy(&out.stdout)
                                        .trim().to_string();
                                    if !chosen.is_empty() {
                                        crate::icons::set_folder_icon(&folder_path, &chosen);
                                    }
                                }
                            }
                        });
                    }
                }
                CTX_PROPERTIES => {
                    // Drive context → drive properties dialog
                    if let Some(ContextTarget::Drive(idx)) = app.context_target.clone() {
                        app.open_drive_properties(idx);
                    } else {
                        let path = if let Some(ref target) = app.context_target {
                            match target {
                                crate::app::ContextTarget::Item(idx) => {
                                    if *idx < app.entries.len() {
                                        Some(app.entries[*idx].path.clone())
                                    } else { None }
                                }
                                crate::app::ContextTarget::SearchItem(idx) => {
                                    app.search_results.get(*idx).map(|e| e.path.clone())
                                }
                                crate::app::ContextTarget::Path(path) => {
                                    Some(path.clone())
                                }
                                crate::app::ContextTarget::Empty => {
                                    Some(app.current_dir.clone())
                                }
                                crate::app::ContextTarget::Drive(_) => None,
                                crate::app::ContextTarget::Favorite(idx) => {
                                    app.sidebar_favorites().get(*idx).map(|p| p.path.clone())
                                }
                            }
                        } else { None };
                        if let Some(path) = path {
                            if let Some(mut props) = crate::properties::FileProperties::from_path(&path) {
                                props.populate_media_info(file_info);
                                app.properties = Some(props);
                            }
                        }
                    }
                }
                CTX_DRIVE_EJECT => {
                    if let Some(ContextTarget::Drive(idx)) = app.context_target.clone() {
                        app.eject_drive(idx);
                    }
                }
                CTX_DRIVE_FORMAT => {
                    if let Some(ContextTarget::Drive(idx)) = app.context_target.clone() {
                        app.open_drive_format_dialog(idx);
                    }
                }
                CTX_DRIVE_PROPERTIES => {
                    if let Some(ContextTarget::Drive(idx)) = app.context_target.clone() {
                        app.open_drive_properties(idx);
                    }
                }
                CTX_NEW_FOLDER => {
                    let target = app.current_dir.join("New Folder");
                    new_folder_or_prompt(app, target, None);
                }
                CTX_NEW_FOLDER_PLAIN | CTX_NEW_FOLDER_RED | CTX_NEW_FOLDER_ORANGE
                | CTX_NEW_FOLDER_YELLOW | CTX_NEW_FOLDER_GREEN | CTX_NEW_FOLDER_BLUE
                | CTX_NEW_FOLDER_PURPLE => {
                    let target = app.current_dir.join("New Folder");
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
                    let target = app.current_dir.join("New File");
                    new_file_or_prompt(app, target);
                }
                CTX_ADD_FAVORITE => {
                    // Resolve the target path from whatever context fired it.
                    let path = target_path_for_favorite(app);
                    if let Some(p) = path {
                        if app.add_favorite(p) {
                            settings.favorites = app.favorites_paths();
                            settings.save();
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
                        settings.favorites = app.favorites_paths();
                        settings.save();
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
                    if app_idx < open_with_apps.len() {
                        if let Some(ContextTarget::SearchItem(idx)) = &app.context_target {
                            if let Some(entry) = app.search_results.get(*idx) {
                                desktop::launch_app(&open_with_apps[app_idx].exec, &entry.path);
                            }
                        } else {
                            // Uses `selected_paths()` so the path override from a
                            // nested-tree-row right-click is honored too.
                            for file_path in app.selected_paths() {
                                desktop::launch_app(&open_with_apps[app_idx].exec, &file_path);
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
                app.reload();
                if let Some(backend) = popup_backend {
                    context_menu.close_popups(backend);
                }
            }
        }
        _ => {}
    }
}

/// Create a folder at `target`. Falls back to the sudo prompt on permission
/// denied. On direct success: push undo, reload, focus rename. On sudo
/// fallback: the file gets created later via sudo + a reload happens then,
/// so we skip undo/rename (no path-of-clean-creation to track).
fn new_folder_or_prompt(app: &mut App, target: PathBuf, color: Option<&'static str>) {
    if app.root_mode {
        app.priv_run(crate::sudo::PendingPrivOp::NewFolder { path: target, color });
        return;
    }
    match std::fs::create_dir(&target) {
        Ok(()) => {
            if let Some(c) = color {
                crate::icons::set_folder_color(&target, c);
            }
            app.undo_stack.push(crate::undo::UndoAction::Create(vec![target.clone()]));
            app.reload();
            if let Some(idx) = app.entries.iter().position(|e| e.path == target) {
                app.select_item(idx);
                app.start_rename(idx);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            app.priv_run(crate::sudo::PendingPrivOp::NewFolder { path: target, color });
        }
        Err(_) => {}
    }
}

fn new_file_or_prompt(app: &mut App, target: PathBuf) {
    if app.root_mode {
        app.priv_run(crate::sudo::PendingPrivOp::NewFile(target));
        return;
    }
    match std::fs::write(&target, "") {
        Ok(()) => {
            app.undo_stack.push(crate::undo::UndoAction::Create(vec![target]));
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
