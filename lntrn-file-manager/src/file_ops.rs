use std::path::PathBuf;
use super::app::{App, ClipboardOp, dirs_home};

fn trash_dir() -> PathBuf {
    dirs_home().join(".local/share/Trash")
}

/// Simple ISO-ish timestamp for trash info (no chrono crate).
fn chrono_now() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let mins = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if remaining < year_days { break; }
        remaining -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    for &md in &month_days {
        if remaining < md { break; }
        remaining -= md;
        m += 1;
    }
    format!("{y}-{:02}-{:02}T{hours:02}:{mins:02}:{s:02}", m + 1, remaining + 1)
}

pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Returns true if the path looks like an extractable archive.
pub fn is_archive(path: &std::path::Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    name.ends_with(".tar.gz") || name.ends_with(".tgz")
        || name.ends_with(".tar.bz2") || name.ends_with(".tbz2")
        || name.ends_with(".tar.xz") || name.ends_with(".txz")
        || name.ends_with(".tar")
        || name.ends_with(".zip")
        || name.ends_with(".7z")
}

/// File operation methods for App.
impl App {
    pub fn copy_selected(&mut self) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            self.clipboard = Some(ClipboardOp::Copy(paths));
        }
    }

    pub fn cut_selected(&mut self) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            self.clipboard = Some(ClipboardOp::Cut(paths));
        }
    }

    pub fn paste(&mut self) {
        let Some(op) = self.clipboard.take() else { return };
        let dest = self.current_dir.clone();

        // Root mode: skip the direct attempt, route straight to sudo. The
        // sudo path doesn't need a conflict dialog — `cp -r` / `mv` handle
        // overwrites natively (and the user already opted into elevated
        // semantics by toggling Open as Root).
        if self.root_mode {
            let priv_op = match &op {
                ClipboardOp::Copy(paths) => crate::sudo::PendingPrivOp::Copy {
                    sources: paths.clone(), dest: dest.clone(),
                },
                ClipboardOp::Cut(paths) => crate::sudo::PendingPrivOp::Move {
                    sources: paths.clone(), dest: dest.clone(),
                },
            };
            if let ClipboardOp::Copy(_) = &op {
                self.clipboard = Some(op);
            }
            self.priv_run(priv_op);
            return;
        }

        let (mode, sources) = match op {
            ClipboardOp::Copy(paths) => (crate::conflict::PasteMode::Copy, paths),
            ClipboardOp::Cut(paths) => (crate::conflict::PasteMode::Cut, paths),
        };
        self.pending_paste = Some(crate::conflict::PendingPaste::new(mode, dest, sources));
        self.advance_paste();
    }

    /// Entry point for drag-drop. Routes the operation through the same
    /// conflict-resolution flow as paste so an overwrite pops the Replace /
    /// Keep Both / Skip dialog instead of silently clobbering the target.
    pub fn start_drag_drop(
        &mut self,
        mode: crate::conflict::PasteMode,
        sources: Vec<std::path::PathBuf>,
        dest: std::path::PathBuf,
        reload_tab: Option<usize>,
    ) {
        if sources.is_empty() { return; }

        // Mirror the root-mode bypass from paste(): elevated cp/mv handle
        // overwrites natively, so we skip the conflict dialog there.
        if self.root_mode {
            let priv_op = match mode {
                crate::conflict::PasteMode::Copy => crate::sudo::PendingPrivOp::Copy {
                    sources, dest,
                },
                crate::conflict::PasteMode::Cut => crate::sudo::PendingPrivOp::Move {
                    sources, dest,
                },
            };
            self.priv_run(priv_op);
            if let Some(idx) = reload_tab { self.reload_tab(idx); }
            return;
        }

        let mut paste = crate::conflict::PendingPaste::new(mode, dest, sources);
        paste.reload_tab = reload_tab;
        self.pending_paste = Some(paste);
        self.advance_paste();
    }

    /// Drive the pending paste queue forward until either the queue is
    /// drained or we hit a conflict that needs the dialog.
    pub fn advance_paste(&mut self) {
        use crate::conflict::{ConflictDialog, ConflictMeta, PasteMode, ConflictAction};

        loop {
            let Some(paste) = self.pending_paste.as_mut() else { return };
            let Some(src) = paste.remaining.first().cloned() else {
                // Drained — finalize.
                self.finalize_paste();
                return;
            };

            let Some(name) = src.file_name() else {
                paste.remaining.remove(0);
                continue;
            };
            let target = paste.dest.join(name);

            // Resolve any collision before attempting the op.
            let (effective_target, skip) = if target.exists() {
                match paste.apply_to_all {
                    Some(ConflictAction::Skip) => (target.clone(), true),
                    Some(ConflictAction::Replace) => {
                        let _ = if target.is_dir() {
                            std::fs::remove_dir_all(&target)
                        } else {
                            std::fs::remove_file(&target)
                        };
                        (target.clone(), false)
                    }
                    Some(ConflictAction::KeepBoth) => {
                        (crate::conflict::unique_keep_both_path(&target), false)
                    }
                    None => {
                        // Pop the conflict dialog. Leave src at the head of
                        // the queue so the dialog's choice handler can
                        // re-enter advance_paste and pick up here.
                        let dialog = ConflictDialog {
                            source: src.clone(),
                            target: target.clone(),
                            source_meta: ConflictMeta::read(&src),
                            target_meta: ConflictMeta::read(&target),
                            apply_to_all: false,
                            remaining_count: paste.remaining.len().saturating_sub(1),
                            mode: paste.mode,
                        };
                        self.conflict_dialog = Some(dialog);
                        return;
                    }
                }
            } else {
                (target.clone(), false)
            };

            // Past resolution. Pop the head and perform the op.
            paste.remaining.remove(0);
            if skip { continue; }

            match paste.mode {
                PasteMode::Copy => {
                    // Defer the actual copy I/O to a worker thread — we
                    // only collect the resolved (src, target) pairs here.
                    paste.resolved_pairs.push((src, effective_target));
                }
                PasteMode::Cut => {
                    // Rename is atomic and fast; apply inline.
                    match std::fs::rename(&src, &effective_target) {
                        Ok(()) => paste.moves.push((src.clone(), effective_target)),
                        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                            paste.perm_fails.push(src);
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    }

    fn finalize_paste(&mut self) {
        let Some(paste) = self.pending_paste.take() else { return };
        use crate::conflict::PasteMode;

        let reload_tab = paste.reload_tab;
        match paste.mode {
            PasteMode::Cut => {
                if !paste.moves.is_empty() {
                    self.undo_stack.push(crate::undo::UndoAction::Move(paste.moves));
                }
                if !paste.perm_fails.is_empty() {
                    self.priv_run(crate::sudo::PendingPrivOp::Move {
                        sources: paste.perm_fails, dest: paste.dest,
                    });
                }
                self.reload();
                if let Some(idx) = reload_tab { self.reload_tab(idx); }
            }
            PasteMode::Copy => {
                // Spawn the copy worker. The main loop will poll its
                // progress channel and finalize undo/perm_fails when Done.
                if paste.resolved_pairs.is_empty() {
                    // Nothing to copy (all skipped, etc.) — re-arm clipboard, done.
                    self.clipboard = Some(ClipboardOp::Copy(paste.originals));
                    self.reload();
                    if let Some(idx) = reload_tab { self.reload_tab(idx); }
                    return;
                }
                let mut handle = crate::ops::spawn_copy_worker(
                    paste.resolved_pairs,
                    paste.originals,
                    paste.dest,
                    "Copying",
                );
                handle.reload_tab = reload_tab;
                self.op_progress = Some(handle);
            }
        }
    }

    /// Drain progress from the running copy worker. Called every frame by
    /// the main loop. Returns true if state changed (forces a redraw).
    pub fn poll_op_progress(&mut self) -> bool {
        let Some(handle) = self.op_progress.as_mut() else { return false; };
        let dirty = handle.poll();
        if handle.finished {
            // Finalize: push undo entries, route perm_fails through sudo.
            let handle = self.op_progress.take().unwrap();
            let reload_tab = handle.reload_tab;
            if let Some((created, perm_fails, _cancelled)) = handle.done_payload {
                if !created.is_empty() {
                    self.undo_stack.push(crate::undo::UndoAction::Copy {
                        sources: handle.originals.clone(),
                        created,
                    });
                }
                self.clipboard = Some(ClipboardOp::Copy(handle.originals));
                if !perm_fails.is_empty() {
                    self.priv_run(crate::sudo::PendingPrivOp::Copy {
                        sources: perm_fails, dest: handle.dest,
                    });
                }
            }
            self.reload();
            if let Some(idx) = reload_tab { self.reload_tab(idx); }
            return true;
        }
        dirty
    }

    pub fn cancel_op(&mut self) {
        if let Some(h) = self.op_progress.as_ref() {
            h.request_cancel();
        }
    }

    /// Apply a user's conflict-dialog choice to a pending rename. The dialog
    /// is shared with paste; this branch fires when `pending_rename` is set.
    pub fn resolve_rename_conflict(&mut self, action: crate::conflict::ConflictAction) {
        use crate::conflict::ConflictAction;
        self.conflict_dialog = None;
        let Some(pending) = self.pending_rename.take() else { return; };
        match action {
            ConflictAction::Skip => {}
            ConflictAction::Replace => {
                let _ = if pending.to.is_dir() {
                    std::fs::remove_dir_all(&pending.to)
                } else {
                    std::fs::remove_file(&pending.to)
                };
                self.perform_rename(pending.from, pending.to);
            }
            ConflictAction::KeepBoth => {
                let target = crate::conflict::unique_keep_both_path(&pending.to);
                self.perform_rename(pending.from, target);
            }
        }
        self.reload();
    }

    /// Cancel an in-progress rename waiting on the conflict dialog.
    pub fn cancel_rename_conflict(&mut self) {
        self.conflict_dialog = None;
        self.pending_rename = None;
    }

    /// Handle a user choice on the conflict dialog. Closes the dialog,
    /// optionally promotes the action to apply-to-all, and resumes the
    /// pending paste walk.
    pub fn resolve_conflict(&mut self, action: crate::conflict::ConflictAction) {
        let apply_to_all = self.conflict_dialog.as_ref().map(|d| d.apply_to_all).unwrap_or(false);
        self.conflict_dialog = None;
        if let Some(paste) = self.pending_paste.as_mut() {
            if apply_to_all {
                paste.apply_to_all = Some(action);
            } else {
                // One-shot resolution: handle the head source with this
                // action, then continue with no apply_to_all override.
                self.apply_single_conflict_action(action);
                return;
            }
        }
        self.advance_paste();
    }

    /// Apply `action` to just the source at the head of the paste queue,
    /// then resume the walk. Used when the "Apply to all" checkbox is off.
    fn apply_single_conflict_action(&mut self, action: crate::conflict::ConflictAction) {
        use crate::conflict::{PasteMode, ConflictAction};
        let Some(paste) = self.pending_paste.as_mut() else { return; };
        let Some(src) = paste.remaining.first().cloned() else {
            self.finalize_paste();
            return;
        };
        let Some(name) = src.file_name() else {
            paste.remaining.remove(0);
            self.advance_paste();
            return;
        };
        let target = paste.dest.join(name);

        let effective_target = match action {
            ConflictAction::Skip => {
                paste.remaining.remove(0);
                self.advance_paste();
                return;
            }
            ConflictAction::Replace => {
                let _ = if target.is_dir() {
                    std::fs::remove_dir_all(&target)
                } else {
                    std::fs::remove_file(&target)
                };
                target
            }
            ConflictAction::KeepBoth => crate::conflict::unique_keep_both_path(&target),
        };

        paste.remaining.remove(0);
        match paste.mode {
            PasteMode::Copy => {
                paste.resolved_pairs.push((src, effective_target));
            }
            PasteMode::Cut => {
                match std::fs::rename(&src, &effective_target) {
                    Ok(()) => paste.moves.push((src.clone(), effective_target)),
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                        paste.perm_fails.push(src);
                    }
                    Err(_) => {}
                }
            }
        }
        self.advance_paste();
    }

    /// Cancel an in-progress paste (closes the dialog, discards remaining work).
    pub fn cancel_paste(&mut self) {
        self.conflict_dialog = None;
        if let Some(paste) = self.pending_paste.take() {
            // Finalize whatever already happened so the user sees those
            // files in the destination + has undo for them.
            use crate::conflict::PasteMode;
            match paste.mode {
                PasteMode::Copy => {
                    if !paste.created.is_empty() {
                        self.undo_stack.push(crate::undo::UndoAction::Copy {
                            sources: paste.originals.clone(),
                            created: paste.created,
                        });
                    }
                    self.clipboard = Some(ClipboardOp::Copy(paste.originals));
                }
                PasteMode::Cut => {
                    if !paste.moves.is_empty() {
                        self.undo_stack.push(crate::undo::UndoAction::Move(paste.moves));
                    }
                }
            }
        }
        self.reload();
    }

    /// True if the current directory lives inside the user's trash.
    pub fn in_trash(&self) -> bool {
        let trash_files = trash_dir().join("files");
        self.current_dir.starts_with(&trash_files)
    }

    /// Restore every selected item back to its original location, using the
    /// XDG Trash spec's `info/{name}.trashinfo` sidecar to recover the path.
    /// If a destination collides, append a counter suffix to the basename.
    pub fn restore_selected(&mut self) {
        let trash = trash_dir();
        let info_dir = trash.join("info");
        let files_dir = trash.join("files");
        let mut restored = 0usize;

        for entry in &self.entries {
            if !entry.selected {
                continue;
            }
            // Only files that actually sit under .../Trash/files are restorable
            // via the trashinfo flow — defensively skip anything else.
            let Ok(rel) = entry.path.strip_prefix(&files_dir) else { continue };
            let top = match rel.iter().next() {
                Some(c) => c.to_string_lossy().into_owned(),
                None => continue,
            };
            let info_path = info_dir.join(format!("{top}.trashinfo"));
            let Some(original) = read_trashinfo_path(&info_path) else {
                eprintln!("[fox] restore: missing or unreadable {}", info_path.display());
                continue;
            };

            let dest = pick_restore_dest(&original);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let trashed_path = files_dir.join(&top);
            match std::fs::rename(&trashed_path, &dest) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&info_path);
                    restored += 1;
                }
                Err(e) => {
                    eprintln!("[fox] restore failed for {}: {e}", trashed_path.display());
                }
            }
        }

        if restored > 0 {
            eprintln!("[fox] restored {restored} item(s) from trash");
        }
        self.reload();
    }

    pub fn trash_selected(&mut self) {
        if self.root_mode {
            self.delete_selected();
            return;
        }
        let trash_dir = trash_dir();
        let trash_info_dir = trash_dir.join("info");
        let trash_files_dir = trash_dir.join("files");
        let _ = std::fs::create_dir_all(&trash_info_dir);
        let _ = std::fs::create_dir_all(&trash_files_dir);

        let mut undo_entries = Vec::new();
        let mut permission_failures: Vec<PathBuf> = Vec::new();
        for entry in &self.entries {
            if !entry.selected { continue; }
            let name = entry.path.file_name().unwrap_or_default().to_string_lossy().to_string();

            let mut dest_name = name.clone();
            let mut counter = 1u32;
            while trash_files_dir.join(&dest_name).exists() {
                let stem = std::path::Path::new(&name).file_stem()
                    .map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let ext = std::path::Path::new(&name).extension()
                    .map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
                dest_name = format!("{stem}.{counter}{ext}");
                counter += 1;
            }

            let now = chrono_now();
            let info_content = format!(
                "[Trash Info]\nPath={}\nDeletionDate={}\n",
                entry.path.display(), now
            );
            let info_path = trash_info_dir.join(format!("{dest_name}.trashinfo"));
            let file_path = trash_files_dir.join(&dest_name);
            let _ = std::fs::write(&info_path, info_content);
            match std::fs::rename(&entry.path, &file_path) {
                Ok(()) => {
                    undo_entries.push((entry.path.clone(), file_path, info_path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    permission_failures.push(entry.path.clone());
                    let _ = std::fs::remove_file(&info_path);
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&info_path);
                }
            }
        }
        if !undo_entries.is_empty() {
            self.undo_stack.push(crate::undo::UndoAction::Trash(undo_entries));
        }
        // Items we couldn't trash because they're owned by root → route
        // through the sudo prompt as a permanent delete (mv into trash would
        // leave them root-owned in the trash dir, which is just kicking the
        // permission can down the road).
        if !permission_failures.is_empty() {
            self.priv_run(crate::sudo::PendingPrivOp::Remove(permission_failures));
        }
        self.reload();
    }

    /// Permanently delete every item in the XDG trash. Wipes both
    /// `~/.local/share/Trash/files/` (the actual contents) and
    /// `~/.local/share/Trash/info/` (the .trashinfo sidecars), so nothing
    /// is left to "restore" afterwards. Falls back to the sudo prompt if
    /// any item is root-owned (e.g. after a `sudo mv` into Trash earlier).
    pub fn empty_trash(&mut self) {
        let trash = trash_dir();
        let mut hit_permission_denied = false;
        for sub in ["files", "info"] {
            let dir = trash.join(sub);
            let Ok(entries) = std::fs::read_dir(&dir) else { continue; };
            for entry in entries.flatten() {
                let path = entry.path();
                let res = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                if let Err(e) = res {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        hit_permission_denied = true;
                    }
                }
            }
        }
        if hit_permission_denied {
            self.priv_run(crate::sudo::PendingPrivOp::EmptyTrash);
        }
        if self.in_trash() {
            self.reload();
        }
    }

    pub fn delete_selected(&mut self) {
        let paths: Vec<PathBuf> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect();
        let mut permission_failures: Vec<PathBuf> = Vec::new();
        // In root_mode we skip the direct attempt — we're already trying to
        // operate on protected paths, so just go straight to sudo.
        if !self.root_mode {
            for path in &paths {
                let res = if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
                if let Err(e) = res {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        permission_failures.push(path.clone());
                    }
                }
            }
        } else {
            permission_failures = paths;
        }
        if !permission_failures.is_empty() {
            self.priv_run(crate::sudo::PendingPrivOp::Remove(permission_failures));
        }
        self.reload();
    }

    pub fn open_selected(&mut self) {
        let selected: Vec<_> = self.entries.iter()
            .enumerate()
            .filter(|(_, e)| e.selected)
            .map(|(i, _)| i)
            .collect();
        if selected.len() == 1 {
            let entry = &self.entries[selected[0]];
            if entry.is_dir {
                let path = entry.path.clone();
                self.navigate_to(path);
                return;
            }
        }
        for &i in &selected {
            let path = self.entries[i].path.clone();
            let ext = self.entries[i].extension();
            // Try our extension → MIME → default-app lookup first; this beats
            // xdg-open's content-sniffing for short code files that get
            // mis-classified as text/plain.
            if let Some(app) = crate::desktop::default_app_for_extension(&ext) {
                crate::desktop::launch_app(&app.exec, &path);
            } else {
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                });
            }
        }
    }

    #[allow(dead_code)]
    pub fn open_with(&self, app_name: &str) {
        for entry in &self.entries {
            if !entry.selected { continue; }
            let path = entry.path.clone();
            let app = app_name.to_string();
            std::thread::spawn(move || {
                let _ = std::process::Command::new(&app).arg(&path).spawn();
            });
        }
    }

    #[allow(dead_code)]
    pub fn copy_path_to_clipboard(&self) {
        let paths: Vec<String> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.path.display().to_string())
            .collect();
        if paths.is_empty() { return; }
        let text = paths.join("\n");
        if let Some(clip) = &self.wayland_clipboard {
            clip.set_text(&text);
        }
    }

    #[allow(dead_code)]
    pub fn copy_name_to_clipboard(&self) {
        let names: Vec<String> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.name.clone())
            .collect();
        if names.is_empty() { return; }
        let text = names.join("\n");
        if let Some(clip) = &self.wayland_clipboard {
            clip.set_text(&text);
        }
    }

    pub fn duplicate_selected(&mut self) {
        let selected: Vec<_> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| (e.path.clone(), e.name.clone(), e.is_dir))
            .collect();
        let root_mode = self.root_mode;
        for (path, name, is_dir) in selected {
            let parent = path.parent().unwrap_or(&self.current_dir).to_path_buf();
            let stem = std::path::Path::new(&name).file_stem()
                .map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let ext = std::path::Path::new(&name).extension()
                .map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();

            let mut dest_name = format!("{stem} (copy){ext}");
            let mut counter = 2u32;
            while parent.join(&dest_name).exists() {
                dest_name = format!("{stem} (copy {counter}){ext}");
                counter += 1;
            }
            let dest = parent.join(&dest_name);
            if root_mode {
                let src = path.clone();
                let d = dest.clone();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("pkexec")
                        .args(["cp", "-r", "--"])
                        .arg(&src).arg(&d)
                        .status();
                });
            } else if is_dir {
                let src = path.clone();
                let d = dest.clone();
                std::thread::spawn(move || { let _ = copy_dir_recursive(&src, &d); });
            } else {
                let _ = std::fs::copy(&path, &dest);
            }
            self.undo_stack.push(crate::undo::UndoAction::Copy {
                sources: vec![path], created: vec![dest],
            });
        }
        self.reload();
    }

    pub fn compress_selected(&mut self) {
        let selected: Vec<PathBuf> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect();
        if selected.is_empty() { return; }

        let base_name = selected[0].file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".into());
        let mut archive_name = format!("{base_name}.tar.gz");
        let mut counter = 2u32;
        while self.current_dir.join(&archive_name).exists() {
            archive_name = format!("{base_name} ({counter}).tar.gz");
            counter += 1;
        }

        let dir = self.current_dir.clone();
        let root_mode = self.root_mode;
        std::thread::spawn(move || {
            let file_args: Vec<String> = selected.iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .collect();
            if root_mode {
                let mut args = vec!["tar".to_string(), "czf".to_string(), archive_name];
                args.extend(file_args);
                let _ = std::process::Command::new("pkexec")
                    .args(&args)
                    .current_dir(&dir)
                    .status();
            } else {
                let _ = std::process::Command::new("tar")
                    .arg("czf").arg(&archive_name)
                    .args(&file_args)
                    .current_dir(&dir)
                    .status();
            }
        });
    }

    pub fn extract_selected(&mut self) {
        let selected: Vec<PathBuf> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect();
        let dir = self.current_dir.clone();
        let root_mode = self.root_mode;
        std::thread::spawn(move || {
            for path in &selected {
                let ext = path.extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let name = path.to_string_lossy();

                // Derive subfolder name from archive filename (strip extensions)
                let stem = {
                    let file_name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    // Strip compound extensions like .tar.gz, .tar.bz2, etc.
                    let s = file_name.as_str();
                    if s.ends_with(".tar.gz") || s.ends_with(".tar.bz2") || s.ends_with(".tar.xz") {
                        s.rsplitn(3, '.').last().unwrap_or(s).to_string()
                    } else {
                        std::path::Path::new(&file_name).file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or(file_name)
                    }
                };
                let extract_dir = dir.join(&stem);
                let _ = std::fs::create_dir_all(&extract_dir);

                // Build (program, args) for each archive type
                let (prog, args): (&str, Vec<std::ffi::OsString>) = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
                    ("tar", vec!["xzf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
                    ("tar", vec!["xjf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
                    ("tar", vec!["xJf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if name.ends_with(".tar") {
                    ("tar", vec!["xf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if ext == "zip" {
                    ("unzip", vec!["-o".into(), path.as_os_str().into(), "-d".into(), extract_dir.as_os_str().into()])
                } else if ext == "7z" {
                    let out_flag: std::ffi::OsString = format!("-o{}", extract_dir.display()).into();
                    ("7z", vec!["x".into(), path.as_os_str().into(), out_flag])
                } else {
                    continue;
                };

                if root_mode {
                    let mut cmd = std::process::Command::new("pkexec");
                    cmd.arg(prog).args(&args);
                    let _ = cmd.status();
                } else {
                    let _ = std::process::Command::new(prog)
                        .args(&args)
                        .status();
                }
            }
        });
    }

    pub fn open_as_root(&mut self) {
        // Navigate into the selected folder with root mode enabled
        let selected: Vec<PathBuf> = self.entries.iter()
            .filter(|e| e.selected && e.is_dir)
            .map(|e| e.path.clone())
            .collect();
        if let Some(path) = selected.into_iter().next() {
            self.navigate_to(path);
            self.root_mode = true; // set after navigate_to (which resets it)
        } else {
            // No folder selected — just toggle root mode for current dir
            self.root_mode = !self.root_mode;
        }
    }

    pub fn open_in_terminal(&self) {
        let dir = self.current_dir.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("lntrn-terminal")
                .current_dir(&dir)
                .spawn();
        });
    }

    // ── Privileged op runner ────────────────────────────────────────────
    //
    // priv_run is the entry point for ops that may need root. Flow:
    //   1. Try `sudo -n <cmd>` (uses cached ticket — no prompt).
    //   2. If the ticket is expired/missing, stash the op into `sudo_prompt`
    //      so the modal opens and the user can type their password.
    //   3. On modal submit, `resume_sudo_op` runs `sudo -S <cmd>`.
    //
    // Callers that already attempted the direct (non-sudo) path and got
    // PermissionDenied should call this with the op they wanted to run.

    /// Attempt a privileged op. If sudo's ticket is cached, runs immediately
    /// and reloads. Otherwise queues the op behind the password modal.
    pub fn priv_run(&mut self, op: crate::sudo::PendingPrivOp) {
        use crate::sudo::CachedResult;
        match crate::sudo::try_cached(&op) {
            CachedResult::Ok => {
                self.reload();
            }
            CachedResult::NeedPassword => {
                self.sudo_prompt = Some(crate::dialogs::SudoPrompt::new(op));
            }
            CachedResult::Failed(msg) => {
                eprintln!("[fox] privileged op failed: {msg}");
            }
        }
    }

    /// Called from the modal Submit handler. Runs the queued op with the
    /// supplied password. On success closes the modal + reloads; on failure
    /// surfaces the error message in the modal.
    pub fn submit_sudo_prompt(&mut self) {
        let Some(prompt) = self.sudo_prompt.as_ref() else { return; };
        if !prompt.can_submit() { return; }
        let password = prompt.password.clone();
        let op = prompt.op.clone();
        // Mark submitting so the button re-labels.
        if let Some(p) = self.sudo_prompt.as_mut() { p.submitting = true; }
        match crate::sudo::run_with_password(&op, &password) {
            Ok(()) => {
                self.sudo_prompt = None;
                self.reload();
            }
            Err(msg) => {
                if let Some(p) = self.sudo_prompt.as_mut() {
                    p.submitting = false;
                    p.password.clear();
                    p.cursor = 0;
                    p.error = Some(msg);
                }
            }
        }
    }

    pub fn cancel_sudo_prompt(&mut self) {
        self.sudo_prompt = None;
    }
}

pub(crate) fn _wl_copy(_text: String) {
    // Deprecated — native Wayland clipboard (clipboard.rs) is used instead.
}

/// Parse the Path= line out of a `.trashinfo` file (XDG Trash spec).
/// Values are URL-encoded per spec; we decode percent-escapes so paths with
/// spaces and unicode come back correctly.
fn read_trashinfo_path(info_path: &std::path::Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(info_path).ok()?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Path=") {
            let decoded = percent_decode(rest);
            return Some(PathBuf::from(decoded));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// If the original path already exists, append " (restored N)" before the
/// extension to avoid clobbering whatever's there now.
fn pick_restore_dest(original: &std::path::Path) -> PathBuf {
    if !original.exists() {
        return original.to_path_buf();
    }
    let parent = original.parent().unwrap_or(std::path::Path::new("/"));
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = original.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    for n in 1u32..1000 {
        let candidate = parent.join(format!("{stem} (restored {n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} (restored){ext}"))
}
