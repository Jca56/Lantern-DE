use std::time::Instant;

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{
    ContextMenu, InteractionContext, MenuEvent, MenuItem,
};

use crate::app::{App, ContextTarget};
use crate::desktop::{self, DesktopApp};
use crate::fs::SortBy;
use crate::layout::{content_rect, file_item_rect, grid_columns};
use crate::settings::Settings;
use crate::{
    ClickAction, CTX_CHANGE_ICON, CTX_COMPRESS, CTX_COPY, CTX_COPY_NAME, CTX_COPY_PATH, CTX_CUT,
    CTX_DUPLICATE, CTX_EXTRACT, CTX_NEW_FILE, CTX_NEW_FOLDER, CTX_NEW_FOLDER_BLUE,
    CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE, CTX_NEW_FOLDER_PLAIN, CTX_NEW_FOLDER_PURPLE,
    CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW, CTX_OPEN, CTX_OPEN_AS_ROOT,
    CTX_OPEN_TERMINAL, CTX_OPEN_WITH, CTX_OPEN_WITH_BASE, CTX_PASTE, CTX_PROPERTIES,
    CTX_RENAME, CTX_SELECT_ALL, CTX_SHOW_HIDDEN, CTX_SORT_BY, CTX_SORT_DATE, CTX_SORT_NAME,
    CTX_SORT_SIZE, CTX_SORT_TYPE, CTX_TRASH, SORT_RADIO_GROUP, VIEW_SLIDER_ID, VIEW_OPACITY_SLIDER_ID,
    ZONE_CLOSE, ZONE_FILE_ITEM_BASE, ZONE_MENU_VIEW,
    ZONE_NAV_BACK, ZONE_NAV_FORWARD, ZONE_NAV_UP, ZONE_NAV_SEARCH, ZONE_NAV_VIEW_TOGGLE,
    ZONE_PATH_INPUT, ZONE_SIDEBAR_ITEM_BASE, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE, ZONE_TAB_NEW,
    ZONE_DRIVE_ITEM_BASE, ZONE_TREE_ITEM_BASE,
};

pub(crate) fn handle_click(
    input: &mut InteractionContext,
    app: &mut App,
    view_menu: &mut ContextMenu,
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
        // Global tab bar clicks
        if zone_id >= crate::ZONE_GLOBAL_TAB_BASE && zone_id < crate::ZONE_GLOBAL_TAB_BASE + 10 {
            let panels = [crate::DesktopPanel::Files, crate::DesktopPanel::Blank];
            let idx = (zone_id - crate::ZONE_GLOBAL_TAB_BASE) as usize;
            if idx < panels.len() {
                return ClickAction::SwitchPanel(panels[idx]);
            }
        }
        match zone_id {
            ZONE_CLOSE => return ClickAction::Close,
            ZONE_MENU_VIEW => {
                if !view_menu.is_open() {
                    let label_x = 10.0;
                    let label_y = crate::layout::panel_top(s) / s;
                    view_menu.set_scale(s);
                    // Desktop mode: open inline
                    let px = label_x * s;
                    let py = label_y * s;
                    view_menu.open(px, py, vec![
                        MenuItem::slider(VIEW_SLIDER_ID, "Icon Size", app.icon_zoom),
                        MenuItem::slider(VIEW_OPACITY_SLIDER_ID, "Opacity", bg_opacity),
                    ]);
                } else {
                    view_menu.close();
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
            _ => {}
        }
    }
    ClickAction::None
}

pub(crate) fn handle_right_click(
    app: &mut App,
    context_menu: &mut ContextMenu,
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
        if !is_dir && is_archive(&app.entries[idx].path) {
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
    // Desktop mode: open inline (rendered on same surface)
    context_menu.open(cx, cy, items);
}

pub(crate) fn handle_ctx_event(
    app: &mut App, settings: &mut Settings,
    context_menu: &mut ContextMenu,
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
                    // Properties removed — stub for now
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
            context_menu.close();
        }
        MenuEvent::CheckboxToggled { id, checked } => {
            if id == CTX_SHOW_HIDDEN {
                app.show_hidden = checked;
                settings.show_hidden = checked;
                app.reload();
                context_menu.close();
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
            context_menu.close();
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

/// Check if a file path looks like an archive.
fn is_archive(path: &std::path::Path) -> bool {
    let name = path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name.ends_with(".zip") || name.ends_with(".tar") || name.ends_with(".tar.gz")
        || name.ends_with(".tgz") || name.ends_with(".tar.bz2") || name.ends_with(".tar.xz")
        || name.ends_with(".7z") || name.ends_with(".rar")
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
        26 => if shift { b'{' } else { b'[' },
        27 => if shift { b'}' } else { b']' },
        39 => if shift { b':' } else { b';' },
        40 => if shift { b'"' } else { b'\'' },
        41 => if shift { b'~' } else { b'`' },
        43 => if shift { b'|' } else { b'\\' },
        51 => if shift { b'<' } else { b',' },
        52 => if shift { b'>' } else { b'.' },
        53 => if shift { b'?' } else { b'/' },
        57 => b' ',
        _ => return None,
    };
    Some(ch as char)
}

pub(crate) fn handle_key(
    app: &mut App, _settings: &mut Settings,
    context_menu: &mut ContextMenu,
    key: u32, ctrl: bool, shift: bool,
    running: &mut bool,
) {
    if context_menu.is_open() {
        if key == KEY_ESC {
            context_menu.close();
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
        match key {
            KEY_ENTER => app.commit_path_edit(),
            KEY_ESC => app.cancel_path_edit(),
            KEY_BACKSPACE => {
                if app.path_cursor > 0 {
                    app.path_cursor -= 1;
                    app.path_buf.remove(app.path_cursor);
                }
            }
            KEY_DELETE => {
                if app.path_cursor < app.path_buf.len() {
                    app.path_buf.remove(app.path_cursor);
                }
            }
            KEY_LEFT => {
                if app.path_cursor > 0 { app.path_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.path_cursor < app.path_buf.len() { app.path_cursor += 1; }
            }
            KEY_HOME => app.path_cursor = 0,
            KEY_END => app.path_cursor = app.path_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.path_buf.insert(app.path_cursor, ch);
                    app.path_cursor += 1;
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

    if ctrl {
        match key {
            KEY_A => app.select_all(),
            KEY_C => app.copy_selected(),
            KEY_X => app.cut_selected(),
            KEY_V => app.paste(),
            KEY_T => app.new_tab(),
            KEY_W => app.close_tab(app.current_tab),
            _ => {}
        }
    } else {
        match key {
            KEY_BACKSPACE => app.go_up(),
            KEY_ESC => app.clear_selection(),
            KEY_F2 => {
                if let Some(idx) = app.entries.iter().position(|e| e.selected) {
                    app.start_rename(idx);
                }
            }
            KEY_DELETE => app.trash_selected(),
            _ => {}
        }
    }
}
