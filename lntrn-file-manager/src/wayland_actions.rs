use std::time::Instant;

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{
    ContextMenu, InteractionContext, MenuEvent, MenuItem,
    WaylandPopupBackend,
};
use wayland_protocols::xdg::shell::client::xdg_toplevel;

use crate::app::{App, ContextTarget};
use crate::desktop::{self, DesktopApp};
use crate::fs::SortBy;
use crate::layout::{content_rect, file_item_rect, grid_columns};
use crate::settings::Settings;
use crate::wayland::State;
use crate::{
    ClickAction, CTX_CHANGE_ICON, CTX_COMPRESS, CTX_COPY, CTX_COPY_NAME, CTX_COPY_PATH, CTX_CUT,
    CTX_DUPLICATE, CTX_EXTRACT, CTX_NEW_FILE, CTX_NEW_FOLDER, CTX_NEW_FOLDER_BLUE,
    CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE, CTX_NEW_FOLDER_PLAIN, CTX_NEW_FOLDER_PURPLE,
    CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW, CTX_OPEN, CTX_OPEN_AS_ROOT,
    CTX_OPEN_TERMINAL, CTX_OPEN_WITH, CTX_OPEN_WITH_BASE, CTX_PASTE, CTX_PROPERTIES,
    CTX_RENAME, CTX_SELECT_ALL, CTX_SHOW_HIDDEN, CTX_SORT_BY, CTX_SORT_DATE, CTX_SORT_NAME,
    CTX_SORT_SIZE, CTX_SORT_TYPE, CTX_TRASH, SORT_RADIO_GROUP, VIEW_SLIDER_ID, VIEW_OPACITY_SLIDER_ID, VIEW_SHOW_HIDDEN_ID,
    ZONE_CLOSE, ZONE_FILE_ITEM_BASE, ZONE_MAXIMIZE, ZONE_MENU_VIEW, ZONE_MINIMIZE,
    ZONE_NAV_BACK, ZONE_NAV_FORWARD, ZONE_NAV_UP, ZONE_NAV_SEARCH, ZONE_NAV_VIEW_TOGGLE,
    ZONE_PATH_INPUT, ZONE_SIDEBAR_ITEM_BASE, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE, ZONE_TAB_NEW,
    ZONE_DRIVE_ITEM_BASE, ZONE_TREE_ITEM_BASE,
};

// ── Helper functions ────────────────────────────────────────────────────────

pub(crate) fn edge_resize(cx: f32, cy: f32, w: f32, h: f32, border: f32) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(xdg_toplevel::ResizeEdge::TopLeft),
        (_, true, true, _) => Some(xdg_toplevel::ResizeEdge::TopRight),
        (true, _, _, true) => Some(xdg_toplevel::ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(xdg_toplevel::ResizeEdge::BottomRight),
        (true, _, _, _) => Some(xdg_toplevel::ResizeEdge::Left),
        (_, true, _, _) => Some(xdg_toplevel::ResizeEdge::Right),
        (_, _, true, _) => Some(xdg_toplevel::ResizeEdge::Top),
        (_, _, _, true) => Some(xdg_toplevel::ResizeEdge::Bottom),
        _ => None,
    }
}

pub(crate) fn handle_click(
    input: &mut InteractionContext,
    app: &mut App,
    view_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    last_tab_click: &mut Option<(usize, Instant)>,
    tab_drag_press: &mut Option<(usize, f32)>,
    s: f32,
    bg_opacity: f32,
) -> ClickAction {
    if let Some(zone_id) = input.on_left_pressed() {
        // If path editing, commit on any click outside the path input
        if app.path_editing && zone_id != ZONE_PATH_INPUT {
            app.commit_path_edit();
            return ClickAction::None;
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
                                MenuItem::slider(VIEW_SLIDER_ID, "Icon Size", app.icon_zoom),
                                MenuItem::slider(VIEW_OPACITY_SLIDER_ID, "Opacity", bg_opacity),
                                MenuItem::checkbox(VIEW_SHOW_HIDDEN_ID, "Show Hidden Files", app.show_hidden),
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
            ZONE_PATH_INPUT => {
                if !app.path_editing {
                    app.start_path_edit();
                }
            }
            ZONE_NAV_BACK => app.go_back(),
            ZONE_NAV_FORWARD => app.go_forward(),
            ZONE_NAV_UP => app.go_up(),
            ZONE_NAV_SEARCH => {
                if app.searching {
                    app.close_search();
                } else {
                    app.start_search();
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
                    if te.entry.is_dir {
                        let path = te.entry.path.clone();
                        app.toggle_tree_expand(path);
                    } else {
                        let path = te.entry.path.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                        });
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
                            std::thread::spawn(move || {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(&path).spawn();
                            });
                        }
                    }
                } else {
                    if idx < app.entries.len() {
                        app.select_item(idx);
                        app.pending_open = Some(idx);
                        if let Some((cx, cy)) = input.cursor() {
                            app.press_pos = Some((cx, cy));
                        }
                    }
                }
            }
            id if id >= ZONE_SIDEBAR_ITEM_BASE && id < ZONE_DRIVE_ITEM_BASE => {
                let idx = (id - ZONE_SIDEBAR_ITEM_BASE) as usize;
                app.on_sidebar_click(idx);
            }
            id if id >= ZONE_DRIVE_ITEM_BASE && id < ZONE_TAB_BASE => {
                let idx = (id - ZONE_DRIVE_ITEM_BASE) as usize;
                app.on_drive_click(idx);
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

pub(crate) fn handle_right_click(
    app: &mut App,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    input: &InteractionContext,
    open_with_apps: &mut Vec<DesktopApp>,
    wf: f32, hf: f32, s: f32,
) {
    let Some((cx, cy)) = input.cursor() else { return };
    let cr = content_rect(wf, hf, s);
    if !cr.contains(cx, cy) { return; }

    let zoom = app.icon_zoom;
    let cols = grid_columns(cr.w, s, zoom);
    let base_y = cr.y - app.scroll_offset;

    let clicked_item = (0..app.entries.len()).find(|&i| {
        file_item_rect(i, cols, cr.x, base_y, s, zoom).contains(cx, cy)
    });

    let has_clipboard = app.clipboard.is_some();
    let items = if let Some(idx) = clicked_item {
        app.select_item(idx);
        app.context_target = Some(ContextTarget::Item(idx));
        let is_dir = app.entries[idx].is_dir;
        let mut v = vec![MenuItem::action(CTX_OPEN, "Open")];
        if !is_dir {
            // Discover apps for this file's MIME type
            let ext = app.entries[idx].extension();
            *open_with_apps = desktop::apps_for_extension(&ext);
            if !open_with_apps.is_empty() {
                let children: Vec<MenuItem> = open_with_apps.iter().enumerate()
                    .map(|(i, a)| MenuItem::action(CTX_OPEN_WITH_BASE + i as u32, &a.name))
                    .collect();
                v.push(MenuItem::submenu(CTX_OPEN_WITH, "Open With", children));
            }
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
        if !is_dir && crate::file_ops::is_archive(&app.entries[idx].path) {
            v.push(MenuItem::action(CTX_EXTRACT, "Extract Here"));
        }
        v.push(MenuItem::action(CTX_COMPRESS, "Compress"));
        v.push(MenuItem::separator());
        v.push(MenuItem::action(CTX_RENAME, "Rename"));
        v.push(MenuItem::action_danger(CTX_TRASH, "Move to Trash"));
        v.push(MenuItem::separator());
        if is_dir {
            v.push(MenuItem::action(CTX_CHANGE_ICON, "Change Icon"));
        }
        v.push(MenuItem::action(CTX_PROPERTIES, "Properties"));
        v
    } else {
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
        v.push(MenuItem::submenu(CTX_SORT_BY, "Sort By", vec![
            MenuItem::radio(CTX_SORT_NAME, SORT_RADIO_GROUP, "Name", app.sort_by == SortBy::Name),
            MenuItem::radio(CTX_SORT_SIZE, SORT_RADIO_GROUP, "Size", app.sort_by == SortBy::Size),
            MenuItem::radio(CTX_SORT_DATE, SORT_RADIO_GROUP, "Date Modified", app.sort_by == SortBy::Date),
            MenuItem::radio(CTX_SORT_TYPE, SORT_RADIO_GROUP, "Type", app.sort_by == SortBy::Type),
        ]));
        v.push(MenuItem::separator());
        v.push(MenuItem::action(CTX_SELECT_ALL, "Select All"));
        v.push(MenuItem::action(CTX_OPEN_TERMINAL, "Open Terminal Here"));
        v.push(MenuItem::separator());
        v.push(MenuItem::checkbox(CTX_SHOW_HIDDEN, "Show Hidden Files", app.show_hidden));
        v
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
    event: MenuEvent,
) {
    match event {
        MenuEvent::Action(id) => {
            match id {
                CTX_OPEN => app.open_selected(),
                CTX_CUT => app.cut_selected(),
                CTX_COPY => app.copy_selected(),
                CTX_PASTE => app.paste(),
                CTX_RENAME => {
                    if let Some(ContextTarget::Item(idx)) = app.context_target {
                        app.start_rename(idx);
                    }
                }
                CTX_TRASH => app.trash_selected(),
                CTX_COPY_PATH => app.copy_path_to_clipboard(),
                CTX_COPY_NAME => app.copy_name_to_clipboard(),
                CTX_DUPLICATE => app.duplicate_selected(),
                CTX_COMPRESS => app.compress_selected(),
                CTX_EXTRACT => app.extract_selected(),
                CTX_OPEN_AS_ROOT => app.open_as_root(),
                CTX_CHANGE_ICON => {
                    // Spawn a file picker to choose an icon image
                    if let Some(crate::app::ContextTarget::Item(idx)) = app.context_target.clone() {
                        if idx < app.entries.len() && app.entries[idx].is_dir {
                            let folder_path = app.entries[idx].path.clone();
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
                }
                CTX_PROPERTIES => {
                    let path = if let Some(ref target) = app.context_target {
                        match target {
                            crate::app::ContextTarget::Item(idx) => {
                                if *idx < app.entries.len() {
                                    Some(app.entries[*idx].path.clone())
                                } else { None }
                            }
                            crate::app::ContextTarget::Empty => {
                                Some(app.current_dir.clone())
                            }
                        }
                    } else { None };
                    if let Some(path) = path {
                        app.properties = crate::properties::FileProperties::from_path(&path);
                    }
                }
                CTX_NEW_FOLDER => {
                    let target = app.current_dir.join("New Folder");
                    if app.root_mode {
                        let _ = std::process::Command::new("pkexec")
                            .args(["mkdir", "--"]).arg(&target).status();
                    } else {
                        let _ = std::fs::create_dir(&target);
                    }
                    app.reload();
                }
                CTX_NEW_FOLDER_PLAIN | CTX_NEW_FOLDER_RED | CTX_NEW_FOLDER_ORANGE
                | CTX_NEW_FOLDER_YELLOW | CTX_NEW_FOLDER_GREEN | CTX_NEW_FOLDER_BLUE
                | CTX_NEW_FOLDER_PURPLE => {
                    let target = app.current_dir.join("New Folder");
                    if app.root_mode {
                        let _ = std::process::Command::new("pkexec")
                            .args(["mkdir", "--"]).arg(&target).status();
                    } else {
                        let _ = std::fs::create_dir(&target);
                    }
                    let color = match id {
                        CTX_NEW_FOLDER_RED => "red",
                        CTX_NEW_FOLDER_ORANGE => "orange",
                        CTX_NEW_FOLDER_YELLOW => "yellow",
                        CTX_NEW_FOLDER_GREEN => "green",
                        CTX_NEW_FOLDER_BLUE => "blue",
                        CTX_NEW_FOLDER_PURPLE => "purple",
                        _ => "",
                    };
                    if !color.is_empty() {
                        crate::icons::set_folder_color(&target, color);
                    }
                    app.reload();
                }
                CTX_NEW_FILE => {
                    let target = app.current_dir.join("New File");
                    if app.root_mode {
                        let _ = std::process::Command::new("pkexec")
                            .args(["touch", "--"]).arg(&target).status();
                    } else {
                        let _ = std::fs::write(&target, "");
                    }
                    app.reload();
                }
                CTX_SELECT_ALL => app.select_all(),
                CTX_OPEN_TERMINAL => app.open_in_terminal(),
                id if id >= CTX_OPEN_WITH_BASE => {
                    let app_idx = (id - CTX_OPEN_WITH_BASE) as usize;
                    if app_idx < open_with_apps.len() {
                        let selected: Vec<_> = app.entries.iter()
                            .filter(|e| e.selected)
                            .map(|e| e.path.clone())
                            .collect();
                        for file_path in &selected {
                            desktop::launch_app(&open_with_apps[app_idx].exec, file_path);
                        }
                    }
                }
                _ => {}
            }
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
        MenuEvent::RadioSelected { id, .. } => {
            let sort = match id {
                CTX_SORT_NAME => SortBy::Name,
                CTX_SORT_SIZE => SortBy::Size,
                CTX_SORT_DATE => SortBy::Date,
                CTX_SORT_TYPE => SortBy::Type,
                _ => return,
            };
            app.sort_by = sort;
            settings.set_sort_by(sort);
            app.reload();
            if let Some(backend) = popup_backend {
                context_menu.close_popups(backend);
            }
        }
        _ => {}
    }
}

pub(crate) fn update_rubber_band(app: &mut App, wf: f32, hf: f32, s: f32) {
    let (Some(start), Some(end)) = (app.rubber_band_start, app.rubber_band_end) else { return };
    let cr = content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    let cols = grid_columns(cr.w, s, zoom);
    let base_y = cr.y - app.scroll_offset;
    let band = Rect::new(
        start.0.min(end.0), start.1.min(end.1),
        (start.0 - end.0).abs(), (start.1 - end.1).abs(),
    );
    for i in 0..app.entries.len() {
        let ir = file_item_rect(i, cols, cr.x, base_y, s, zoom);
        app.entries[i].selected = ir.intersect(&band).is_some();
    }
}

pub(crate) fn handle_drop(app: &mut App, input: &InteractionContext, wf: f32, hf: f32, s: f32, drag_idx: usize) {
    use crate::app::PendingDrop;
    let Some((cx, cy)) = input.cursor() else { return };

    // Collect all selected paths (or just the dragged one if not selected)
    let sources: Vec<std::path::PathBuf> = {
        let selected = app.selected_paths();
        if selected.is_empty() || !app.entries[drag_idx].selected {
            vec![app.entries[drag_idx].path.clone()]
        } else {
            selected
        }
    };

    // Check if dropped on a zone (tab, sidebar, or file item)
    if let Some(zone_id) = input.zone_at(cx, cy) {
        // ── Drop on a tab ───────────────────────────────────────────
        if zone_id >= ZONE_TAB_BASE && zone_id < ZONE_TAB_CLOSE_BASE {
            let tab_idx = (zone_id - ZONE_TAB_BASE) as usize;
            if tab_idx < app.tabs.len() {
                let dest_dir = app.tabs[tab_idx].path.clone();
                app.pending_drop = Some(PendingDrop {
                    sources, dest_dir, reload_tab: Some(tab_idx),
                });
            }
            return;
        }
        // ── Drop on a sidebar place ─────────────────────────────────
        if zone_id >= ZONE_SIDEBAR_ITEM_BASE && zone_id < ZONE_DRIVE_ITEM_BASE {
            let place_idx = (zone_id - ZONE_SIDEBAR_ITEM_BASE) as usize;
            let places = app.sidebar_places();
            if place_idx < places.len() {
                let dest_dir = places[place_idx].path.clone();
                app.pending_drop = Some(PendingDrop {
                    sources, dest_dir, reload_tab: None,
                });
            }
            return;
        }
    }

    // ── Drop on a folder in the content grid ────────────────────────
    let cr = content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    let cols = grid_columns(cr.w, s, zoom);
    let base_y = cr.y - app.scroll_offset;
    for i in 0..app.entries.len() {
        if i == drag_idx { continue; }
        if sources.iter().any(|s| s == &app.entries[i].path) { continue; }
        let ir = file_item_rect(i, cols, cr.x, base_y, s, zoom);
        if ir.contains(cx, cy) && app.entries[i].is_dir {
            let dest_dir = app.entries[i].path.clone();
            app.pending_drop = Some(PendingDrop {
                sources, dest_dir, reload_tab: None,
            });
            return;
        }
    }
}

pub(crate) fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) {
    let _ = std::fs::create_dir_all(dest);
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let from = entry.path();
            let to = dest.join(entry.file_name());
            if from.is_dir() {
                copy_dir_recursive(&from, &to);
            } else {
                let _ = std::fs::copy(&from, &to);
            }
        }
    }
}

// Linux keycodes
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;
const KEY_A: u32 = 30;
const KEY_C: u32 = 46;
const KEY_V: u32 = 47;
const KEY_X: u32 = 45;
const KEY_T: u32 = 20;
const KEY_W: u32 = 17;
const KEY_F2: u32 = 60;
const KEY_DELETE: u32 = 111;
const KEY_HOME: u32 = 102;
const KEY_END: u32 = 107;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;

/// Map an evdev keycode to a character for filename entry.
fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    // Number row: keycodes 2=1, 3=2, ..., 10=9, 11=0
    let ch = match key {
        2..=11 => {
            let base = b"1234567890"[(key - 2) as usize];
            if shift {
                b"!@#$%^&*()"[(key - 2) as usize]
            } else {
                base
            }
        }
        12 => if shift { b'_' } else { b'-' },
        13 => if shift { b'+' } else { b'=' },
        // Letters (a=30..z)
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        // Punctuation
        26 => if shift { b'{' } else { b'[' },
        27 => if shift { b'}' } else { b']' },
        39 => if shift { b':' } else { b';' },
        40 => if shift { b'"' } else { b'\'' },
        41 => if shift { b'~' } else { b'`' },
        43 => if shift { b'|' } else { b'\\' },
        51 => if shift { b'<' } else { b',' },
        52 => if shift { b'>' } else { b'.' },
        53 => if shift { b'?' } else { b'/' },
        57 => b' ', // space
        _ => return None,
    };
    Some(ch as char)
}

pub(crate) fn handle_key(
    app: &mut App, _settings: &mut Settings,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    key: u32, ctrl: bool, shift: bool,
    running: &mut bool,
) {
    // Drop confirmation modal — ESC cancels
    if app.pending_drop.is_some() {
        if key == KEY_ESC {
            app.pending_drop = None;
        }
        return;
    }

    if context_menu.is_open() {
        if key == KEY_ESC {
            if let Some(backend) = popup_backend {
                context_menu.close_popups(backend);
            }
        }
        return;
    }

    // ── Search mode ──────────────────────────────────────────────────
    if app.searching {
        match key {
            KEY_ESC => app.close_search(),
            KEY_BACKSPACE => {
                if app.search_cursor > 0 {
                    app.search_cursor -= 1;
                    app.search_buf.remove(app.search_cursor);
                    app.run_search();
                }
            }
            KEY_LEFT => {
                if app.search_cursor > 0 { app.search_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.search_cursor < app.search_buf.len() { app.search_cursor += 1; }
            }
            KEY_HOME => app.search_cursor = 0,
            KEY_END => app.search_cursor = app.search_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.search_buf.insert(app.search_cursor, ch);
                    app.search_cursor += 1;
                    app.run_search();
                }
            }
        }
        return;
    }

    // ── Path bar editing ─────────────────────────────────────────────
    if app.path_editing {
        if ctrl {
            match key {
                KEY_A => {
                    let len = app.path_buf.chars().count();
                    app.path_selection = Some((0, len));
                    app.path_cursor = len;
                }
                KEY_C => {
                    if let Some(text) = app.path_selected_text() {
                        crate::file_ops::wl_copy(text);
                    }
                }
                _ => {}
            }
            return;
        }
        // Helper: delete selected range and place cursor at selection start
        let delete_selection = |app: &mut App| -> bool {
            if let Some((a, b)) = app.path_selection.take() {
                let s = a.min(b);
                let e = a.max(b);
                if s != e {
                    let byte_start = app.path_buf.char_indices().nth(s).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                    let byte_end = app.path_buf.char_indices().nth(e).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                    app.path_buf.replace_range(byte_start..byte_end, "");
                    app.path_cursor = s;
                    return true;
                }
            }
            false
        };
        match key {
            KEY_ENTER => app.commit_path_edit(),
            KEY_ESC => app.cancel_path_edit(),
            KEY_BACKSPACE => {
                if !delete_selection(app) && app.path_cursor > 0 {
                    let byte_pos = app.path_buf.char_indices().nth(app.path_cursor - 1).map(|(i,_)| i).unwrap_or(0);
                    app.path_buf.remove(byte_pos);
                    app.path_cursor -= 1;
                }
                app.path_selection = None;
            }
            KEY_DELETE => {
                if !delete_selection(app) {
                    let char_len = app.path_buf.chars().count();
                    if app.path_cursor < char_len {
                        let byte_pos = app.path_buf.char_indices().nth(app.path_cursor).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                        app.path_buf.remove(byte_pos);
                    }
                }
                app.path_selection = None;
            }
            KEY_LEFT => {
                if app.path_cursor > 0 { app.path_cursor -= 1; }
                app.path_selection = None;
            }
            KEY_RIGHT => {
                let char_len = app.path_buf.chars().count();
                if app.path_cursor < char_len { app.path_cursor += 1; }
                app.path_selection = None;
            }
            KEY_HOME => { app.path_cursor = 0; app.path_selection = None; }
            KEY_END => { app.path_cursor = app.path_buf.chars().count(); app.path_selection = None; }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    delete_selection(app);
                    let byte_pos = app.path_buf.char_indices().nth(app.path_cursor).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                    app.path_buf.insert(byte_pos, ch);
                    app.path_cursor += 1;
                    app.path_selection = None;
                }
            }
        }
        return;
    }

    // ── Rename mode ─────────────────────────────────────────────────
    if app.renaming.is_some() {
        match key {
            KEY_ENTER => app.commit_rename(),
            KEY_ESC => app.cancel_rename(),
            KEY_BACKSPACE => {
                if app.rename_cursor > 0 {
                    app.rename_cursor -= 1;
                    app.rename_buf.remove(app.rename_cursor);
                }
            }
            KEY_DELETE => {
                if app.rename_cursor < app.rename_buf.len() {
                    app.rename_buf.remove(app.rename_cursor);
                }
            }
            KEY_LEFT => {
                if app.rename_cursor > 0 { app.rename_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.rename_cursor < app.rename_buf.len() { app.rename_cursor += 1; }
            }
            KEY_HOME => app.rename_cursor = 0,
            KEY_END => app.rename_cursor = app.rename_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.rename_buf.insert(app.rename_cursor, ch);
                    app.rename_cursor += 1;
                }
            }
        }
        return;
    }

    // ── Pick mode: save name editing ───────────────────────────────
    if app.save_name_editing {
        match key {
            KEY_ENTER => {
                app.save_name_editing = false;
                app.confirm_pick();
                *running = false;
            }
            KEY_ESC => {
                app.save_name_editing = false;
            }
            KEY_BACKSPACE => {
                if app.save_name_cursor > 0 {
                    app.save_name_cursor -= 1;
                    app.save_name_buf.remove(app.save_name_cursor);
                }
            }
            KEY_DELETE => {
                if app.save_name_cursor < app.save_name_buf.len() {
                    app.save_name_buf.remove(app.save_name_cursor);
                }
            }
            KEY_LEFT => {
                if app.save_name_cursor > 0 { app.save_name_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.save_name_cursor < app.save_name_buf.len() { app.save_name_cursor += 1; }
            }
            KEY_HOME => app.save_name_cursor = 0,
            KEY_END => app.save_name_cursor = app.save_name_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.save_name_buf.insert(app.save_name_cursor, ch);
                    app.save_name_cursor += 1;
                }
            }
        }
        return;
    }

    if ctrl {
        match key {
            KEY_A => app.select_all(),
            KEY_C => app.copy_selected(),
            KEY_X => app.cut_selected(),
            KEY_V => app.paste(),
            KEY_T if app.pick.is_none() => app.new_tab(),
            KEY_W if app.pick.is_none() => app.close_tab(app.current_tab),
            _ => {}
        }
    } else {
        match key {
            KEY_BACKSPACE => app.go_up(),
            KEY_ESC if app.pick.is_some() => {
                app.cancel_pick();
                *running = false;
            }
            KEY_ESC => app.clear_selection(),
            KEY_ENTER if app.pick.is_some() => {
                app.confirm_pick();
                *running = false;
            }
            KEY_F2 if app.pick.is_none() => {
                if let Some(idx) = app.entries.iter().position(|e| e.selected) {
                    app.start_rename(idx);
                }
            }
            KEY_DELETE if app.pick.is_none() => app.trash_selected(),
            _ => {}
        }
    }
}
