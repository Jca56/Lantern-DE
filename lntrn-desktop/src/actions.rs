//! Execution of `PendingAction`s queued by input/menu handlers. Kept separate
//! from the `wayland` event loop so that loop stays focused on surface +
//! frame lifecycle.

use crate::state::{DesktopState, PendingAction, RenameState};

/// Consume one pending action. Sets `*needs_rescan` when the desktop contents
/// changed and the icon grid must be rebuilt by the caller.
pub fn apply_action(app: &mut DesktopState, action: PendingAction, needs_rescan: &mut bool) {
    match action {
        PendingAction::Open(idx) => {
            if let Some(item) = app.items.get(idx) {
                crate::icons::open_with_default(&item.path);
            }
        }
        PendingAction::Trash(idxs) => {
            let mut paths: Vec<std::path::PathBuf> = idxs
                .iter()
                .filter_map(|&i| app.items.get(i).map(|it| it.path.clone()))
                .collect();
            paths.sort();
            paths.dedup();
            for p in &paths {
                crate::icons::move_to_trash(p);
            }
            app.selection.clear();
            *needs_rescan = true;
        }
        PendingAction::StartRename(idx) => {
            if let Some(item) = app.items.get(idx) {
                app.renaming = Some(RenameState {
                    idx,
                    buffer: item.name.clone(),
                    cursor: item.name.chars().count(),
                });
                app.selection.clear();
                app.selection.insert(idx);
            }
        }
        PendingAction::SubmitRename => {
            let Some(rn) = app.renaming.take() else {
                return;
            };
            let old = match app.items.get(rn.idx) {
                Some(it) => it.path.clone(),
                None => return,
            };
            let trimmed = rn.buffer.trim();
            if !trimmed.is_empty() && trimmed != old.file_name().unwrap_or_default().to_string_lossy()
            {
                if crate::icons::rename(&old, trimmed) {
                    // Carry over the position to the new name.
                    if let Some(cell) = app.positions.get(&app.items[rn.idx].name) {
                        app.positions.remove(&app.items[rn.idx].name);
                        app.positions.set(trimmed, cell);
                        app.dirty_positions = true;
                    }
                    *needs_rescan = true;
                }
            }
        }
        PendingAction::CancelRename => {
            app.renaming = None;
        }
        PendingAction::NewFolder => {
            if let Some(new_path) = crate::icons::new_folder(&app.desktop_dir) {
                // Place the new folder where the menu was opened (anchor) — if known.
                let anchor_cell = app
                    .menu
                    .as_ref()
                    .map(|m| crate::layout::pixel_to_cell(m.anchor_x, m.anchor_y));
                if let (Some(cell), Some(name)) = (anchor_cell, new_path.file_name()) {
                    app.positions.set(&name.to_string_lossy(), cell);
                    app.dirty_positions = true;
                }
                *needs_rescan = true;
            }
        }
        PendingAction::Refresh => {
            *needs_rescan = true;
        }
        PendingAction::SelectAll => {
            app.selection = (0..app.items.len()).collect();
        }
        PendingAction::OpenTerminal => {
            let dir = app.desktop_dir.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("lntrn-terminal")
                    .current_dir(&dir)
                    .spawn();
            });
        }
        PendingAction::Launch(bin) => {
            // Spawn a Lantern app binary by name (detached), cwd = desktop dir.
            let dir = app.desktop_dir.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new(bin).current_dir(&dir).spawn();
            });
        }
        PendingAction::CopyName(idx) => {
            if let Some(item) = app.items.get(idx) {
                let s = item.name.clone();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("wl-copy").arg(&s).spawn();
                });
            }
        }
        PendingAction::CopyPath(idx) => {
            if let Some(item) = app.items.get(idx) {
                let s = item.path.display().to_string();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("wl-copy").arg(&s).spawn();
                });
            }
        }
    }
}
