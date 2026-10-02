use super::app::{App, ClipboardOp};
use crate::ops::{FinishedOp, OpItem, OpKind, OpQueue, OpRequest};
use std::path::{Path, PathBuf};

/// `[Trash Info]` sidecar body. The spec wants `Path=` percent-encoded, and
/// restore decodes it, so a raw '%' in a file name must be escaped here or the
/// item comes back under a different name.
pub(crate) fn trashinfo_contents(original: &std::path::Path, deleted_at: &str) -> String {
    let path = percent_encode_path(original);
    format!("[Trash Info]\nPath={path}\nDeletionDate={deleted_at}\n")
}

/// Percent-encode a path's raw bytes for a `file://` URI or a trashinfo
/// `Path=`: unreserved characters and '/' stay, everything else (spaces, '%',
/// '#', non-ASCII, non-UTF-8 bytes) becomes `%XX`.
pub(crate) fn percent_encode_path(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// True when both paths name the same directory (by inode, following
/// symlinks: `current_dir` can be a symlinked path, or a bind-mounted twin).
fn same_dir(a: Option<&std::path::Path>, b: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(a) = a else { return false };
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.is_dir() && a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// `target` is a real directory that physically contains `src` — pasting
/// `/a/foo/foo` into `/a` collides with `/a/foo`. "Replace" there would move
/// the whole of `target` to the Trash, the source (and all its siblings)
/// with it. A symlink target is safe: only the link itself would go.
pub(crate) fn target_holds(target: &std::path::Path, src: &std::path::Path) -> bool {
    if !std::fs::symlink_metadata(target).is_ok_and(|m| m.is_dir()) {
        return false;
    }
    let src_parent = src.parent().and_then(|p| p.canonicalize().ok());
    match (src_parent, target.canonicalize().ok()) {
        (Some(p), Some(t)) => p.starts_with(t),
        _ => src != target && src.starts_with(target),
    }
}

/// A copy or move of `src` into `dest` where `dest` is `src` itself or sits
/// inside it. The recursive copy would list its own output and nest until the
/// path is too long.
fn dest_is_inside(dest: &std::path::Path, src: &std::path::Path) -> bool {
    if !src.is_dir() {
        return false;
    }
    match (dest.canonicalize(), src.canonicalize()) {
        (Ok(d), Ok(s)) => d.starts_with(s),
        _ => dest.starts_with(src),
    }
}

/// A "Keep Both" name that is free on disk, not claimed by an earlier item
/// of the same paste and not on its way from a running operation.
fn keep_both(paste: &crate::conflict::PendingPaste, ops: &OpQueue, target: &Path) -> PathBuf {
    crate::conflict::unique_keep_both_path(target, |p| {
        paste.is_reserved(p) || ops.is_reserved(p)
    })
}

/// Returns true if the path looks like an extractable archive.
pub fn is_archive(path: &std::path::Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tbz2")
        || name.ends_with(".tar.xz")
        || name.ends_with(".txz")
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
        let Some(op) = self.clipboard.take() else {
            return;
        };
        let dest = self.current_dir.clone();
        let (mode, sources) = match op {
            ClipboardOp::Copy(paths) => (crate::conflict::PasteMode::Copy, paths),
            ClipboardOp::Cut(paths) => (crate::conflict::PasteMode::Cut, paths),
        };
        self.start_paste(mode, sources, dest, None, true, true);
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
        in_pane: bool,
    ) {
        self.start_paste(mode, sources, dest, reload_tab, false, in_pane);
    }

    fn start_paste(
        &mut self,
        mode: crate::conflict::PasteMode,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        reload_tab: Option<usize>,
        from_clipboard: bool,
        // Aimed at the focused pane's own list (a clipboard paste, a drop on
        // one of its rows or its empty space).
        in_pane: bool,
    ) {
        use crate::conflict::PasteMode;
        if sources.is_empty() {
            return;
        }
        // Into the Trash (its sidebar place, or a paste while viewing it):
        // a move there IS "Move to Trash", restore record and all. A plain
        // rename into the folder would leave items nothing can restore.
        if crate::trash::locate(&dest).is_some() {
            match mode {
                PasteMode::Cut => self.drop_on_trash(sources),
                PasteMode::Copy => {
                    if from_clipboard {
                        self.clipboard = Some(ClipboardOp::Copy(sources));
                    }
                    self.show_notice(
                        "Nothing was copied",
                        vec!["Items can be moved to the Trash, not copied into it.".into()],
                    );
                }
            }
            return;
        }
        let mut paste = crate::conflict::PendingPaste::new(mode, dest, sources);
        paste.from_clipboard = from_clipboard;
        paste.reload_tab = reload_tab;
        // Root mode goes through the same conflict questions; only the last
        // step differs (sudo with the resolved targets instead of the worker).
        // It reaches no further than the pane it is on: a drop onto another
        // pane, a tab or a sidebar place is an ordinary one, also when that
        // place happens to lie below the root-mode folder.
        paste.privileged = in_pane && self.root_covers(&paste.dest);
        self.pending_paste = Some(paste);
        self.advance_paste();
    }

    /// Drive the pending paste queue forward until either the queue is
    /// drained or we hit a conflict that needs the dialog. Nothing on disk
    /// changes here: every source only gets its target and its Replace flag.
    pub fn advance_paste(&mut self) {
        use crate::conflict::{ConflictAction, ConflictDialog, PasteMode};

        loop {
            let ops = &self.ops;
            let Some(paste) = self.pending_paste.as_mut() else {
                return;
            };
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

            // A folder can't go into itself or its own subfolder.
            if dest_is_inside(&paste.dest, &src) {
                eprintln!("[fox] not pasting {} into itself", src.display());
                paste.remaining.remove(0);
                continue;
            }

            // Resolve any collision before attempting the op.
            // "Onto itself" = same inode AND same folder. Inode alone would
            // also catch a hard link of the same file in another folder,
            // where a normal Replace is correct and harmless.
            let onto_itself = crate::app::is_same_entry(&src, &target)
                && same_dir(src.parent(), &paste.dest);
            // (target, replace what is there). `None` skips the source.
            let resolved: Option<(PathBuf, bool)> = if onto_itself {
                // The item is being pasted onto itself (Ctrl+C, Ctrl+V in its
                // own folder). There is nothing to replace: Replace would
                // trash the source. A copy becomes a duplicate, a move is
                // already where it is going.
                match paste.mode {
                    PasteMode::Copy => Some((keep_both(paste, ops, &target), false)),
                    PasteMode::Cut => None,
                }
            } else if paste.originals.iter().any(|o| target_holds(&target, o)) {
                // The colliding folder CONTAINS this source (or another
                // item of the same paste). Never offer Replace for it; land
                // the item beside it instead.
                Some((keep_both(paste, ops, &target), false))
            } else if paste.is_reserved(&target) {
                // An earlier item of this same paste already claimed the name
                // and nothing has run yet, so the disk can't tell us.
                Some((keep_both(paste, ops, &target), false))
            } else if std::fs::symlink_metadata(&target).is_ok() {
                match paste.apply_to_all {
                    Some(ConflictAction::Skip) => None,
                    Some(ConflictAction::Replace) => Some((target.clone(), true)),
                    Some(ConflictAction::KeepBoth) => Some((keep_both(paste, ops, &target), false)),
                    None => {
                        // Pop the conflict dialog. Leave src at the head of
                        // the queue so the dialog's choice handler can
                        // re-enter advance_paste and pick up here.
                        let remaining = paste.remaining.len().saturating_sub(1);
                        self.conflict_dialog = Some(ConflictDialog::new(&src, &target, remaining));
                        return;
                    }
                }
            } else if ops.is_reserved(&target) {
                // Free on disk, but an operation that is still running (or
                // queued) is about to create it.
                Some((keep_both(paste, ops, &target), false))
            } else {
                Some((target.clone(), false))
            };

            // Past resolution. Pop the head and note what is to be done.
            paste.remaining.remove(0);
            if let Some((target, replace)) = resolved {
                paste.resolved.push(OpItem {
                    src,
                    target,
                    replace,
                });
            }
        }
    }

    fn finalize_paste(&mut self) {
        let Some(paste) = self.pending_paste.take() else {
            return;
        };
        use crate::conflict::PasteMode;

        let reload_tab = paste.reload_tab;
        let copy = matches!(paste.mode, PasteMode::Copy);
        // Only a clipboard copy hands the clipboard back (a drop never had
        // it, and a cut's sources are gone once it has run).
        let rearm = (copy && paste.from_clipboard).then_some(paste.originals);
        if paste.resolved.is_empty() || paste.privileged {
            if !paste.resolved.is_empty() {
                self.run_privileged_items(copy, &paste.resolved);
            }
            if let Some(originals) = rearm {
                if self.clipboard.is_none() {
                    self.clipboard = Some(ClipboardOp::Copy(originals));
                }
            }
        } else {
            // The worker does the I/O; the main loop polls it and
            // finalizes undo / sudo fallback / failures when it ends.
            self.ops.push(OpRequest {
                kind: if copy { OpKind::Copy } else { OpKind::Move },
                label: if copy { "Copying" } else { "Moving" },
                items: paste.resolved,
                dest: paste.dest,
                rearm_clipboard: rearm,
            });
        }
        self.reload();
        if let Some(idx) = reload_tab {
            self.reload_tab(idx);
        }
    }

    /// Run resolved copy/move items through sudo, each to the exact target
    /// the conflict flow chose. Items that cannot be done safely (a Replace
    /// with no Trash slot for the old item) are reported, not attempted.
    pub(crate) fn run_privileged_items(&mut self, copy: bool, items: &[OpItem]) {
        let mut priv_items = Vec::new();
        let mut refused = Vec::new();
        for item in items {
            // A move ends by deleting its source as root, mounted devices
            // inside it included.
            if !copy && crate::mount_guard::holds_mount(&item.src) {
                refused.push(crate::ops::OpFailure {
                    path: item.src.clone(),
                    reason: crate::mount_guard::REASON.into(),
                });
                continue;
            }
            match crate::sudo::PrivItem::from_op(item) {
                Ok(p) => priv_items.push(p),
                Err(failure) => refused.push(failure),
            }
        }
        if !refused.is_empty() {
            self.report_failures(if copy { "copied" } else { "moved" }, &refused);
        }
        if priv_items.is_empty() {
            return;
        }
        self.priv_run(if copy {
            crate::sudo::PendingPrivOp::Copy { items: priv_items }
        } else {
            crate::sudo::PendingPrivOp::Move { items: priv_items }
        });
    }

    /// Drain progress from the ops worker. Called every frame by the main
    /// loop. Returns true if state changed (forces a redraw).
    pub fn poll_op_progress(&mut self) -> bool {
        let (mut dirty, finished) = self.ops.poll();
        for op in finished {
            match op.kind {
                OpKind::History => self.finish_history(op.outcome),
                _ => self.finish_op(op),
            }
            dirty = true;
        }
        if dirty {
            self.after_ops_changed();
        }
        dirty
    }

    /// An operation ended: record what can be undone, report what failed,
    /// send permission failures to sudo, refresh the views.
    fn finish_op(&mut self, op: FinishedOp) {
        use crate::undo::UndoAction;
        let FinishedOp {
            kind,
            dest,
            rearm_clipboard,
            outcome,
        } = op;

        // Old items a Replace moved to the Trash. Pushed BEFORE the copy or
        // move itself, so they are undone after it: by then the name is free
        // again and the old item can come back. Without an undoable
        // companion (a cross-device move) there is no entry; the old item
        // is still in the Trash and can be restored from there.
        let undoable = match kind {
            OpKind::Copy => !outcome.created.is_empty(),
            OpKind::Move => !outcome.renamed.is_empty(),
            OpKind::Delete | OpKind::History => false,
        };
        if undoable && !outcome.replaced.is_empty() {
            self.undo_stack.push(UndoAction::Trash(
                outcome
                    .replaced
                    .iter()
                    .map(|t| (t.original.clone(), t.trashed.clone(), t.info.clone()))
                    .collect(),
            ));
        }
        let verb = match kind {
            OpKind::Copy => {
                if !outcome.created.is_empty() {
                    self.undo_stack.push(UndoAction::Copy(outcome.created));
                }
                // Hand the clipboard back only if nothing was copied or cut
                // while the worker ran.
                if let Some(originals) = rearm_clipboard {
                    if self.clipboard.is_none() {
                        self.clipboard = Some(ClipboardOp::Copy(originals));
                    }
                }
                "copied"
            }
            // A move across filesystems has no undo: it would have to copy
            // the data back over the device boundary.
            OpKind::Move => {
                // An item copied out of a trash to another drive (Restore
                // across drives, a drag out of the Trash view): there is no
                // undo for it, so its restore record has nothing left to
                // describe once the item is out.
                for (src, _) in &outcome.created {
                    if std::fs::symlink_metadata(src).is_err() {
                        crate::trash::forget(src);
                    }
                }
                if !outcome.renamed.is_empty() {
                    self.undo_stack.push(UndoAction::Move(outcome.renamed));
                }
                "moved"
            }
            OpKind::Delete => "deleted",
            // Finalised by `finish_history`, never here.
            OpKind::History => "undone",
        };
        if !outcome.failures.is_empty() {
            // The window may be on its way out and never show the notice.
            for f in &outcome.failures {
                eprintln!("[fox] not {verb}: {}: {}", f.path.display(), f.reason);
            }
            self.report_failures(verb, &outcome.failures);
        }
        // Not on the way out: no password prompt while the window closes.
        if !outcome.perm_fails.is_empty() && !self.closing {
            match kind {
                OpKind::Copy => self.run_privileged_items(true, &outcome.perm_fails),
                OpKind::Move => self.run_privileged_items(false, &outcome.perm_fails),
                OpKind::Delete => self.offer_root_delete(outcome.perm_fails),
                OpKind::History => {}
            }
        }
        self.reload();
        self.reload_background_tabs(&dest);
    }

    /// Refresh every background tab that shows `dir`. By path: the tab an
    /// operation was started for can have moved or closed while it ran.
    pub(crate) fn reload_background_tabs(&mut self, dir: &Path) {
        for i in 0..self.tabs.len() {
            if i != self.current_tab && self.tabs[i].path == dir {
                self.reload_tab(i);
            }
        }
    }

    /// The status strip's cancel button: stop the operation it shows. The
    /// item in flight is rolled back; the other operations carry on.
    pub fn cancel_op(&mut self) {
        self.ops.cancel_shown();
    }

    /// Apply a user's conflict-dialog choice to a pending rename. The dialog
    /// is shared with paste; this branch fires when `pending_rename` is set.
    pub fn resolve_rename_conflict(&mut self, action: crate::conflict::ConflictAction) {
        self.root_question_fronted();
        use crate::conflict::ConflictAction;
        if matches!(action, ConflictAction::Replace)
            && self.conflict_dialog.as_ref().is_some_and(|d| !d.replace_allowed())
        {
            return;
        }
        self.conflict_dialog = None;
        let Some(pending) = self.pending_rename.take() else {
            return;
        };
        match action {
            ConflictAction::Skip => {}
            ConflictAction::Replace => {
                // commit_rename never raises the dialog for these; this is
                // the last line of defence before the target is trashed.
                let onto_itself = crate::app::is_same_entry(&pending.from, &pending.to)
                    || pending.from.starts_with(&pending.to);
                if onto_itself {
                    self.perform_rename(pending.from, pending.to);
                } else {
                    self.rename_replacing(pending.from, pending.to);
                }
            }
            ConflictAction::KeepBoth => {
                let target = crate::conflict::unique_keep_both_path(&pending.to, |p| {
                    self.ops.is_reserved(p)
                });
                self.perform_rename(pending.from, target);
            }
        }
        self.reload();
    }

    /// Rename `from` onto the existing `to`: the old `to` goes to the Trash
    /// first and comes back if the rename does not happen.
    fn rename_replacing(&mut self, from: PathBuf, to: PathBuf) {
        let name = to
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if std::fs::symlink_metadata(&from).is_err() {
            self.show_notice(
                "Not renamed",
                vec![format!("The item to rename is gone. \u{201C}{name}\u{201D} was left alone.")],
            );
            return;
        }
        // Root mode: the same Replace through sudo. The old item gets a
        // place in the Trash first and goes there in the same command that
        // renames; it comes back if the rename does not happen.
        if self.root_covers_item(&from) {
            let item = OpItem {
                src: from,
                target: to,
                replace: true,
            };
            match crate::sudo::PrivItem::from_op(&item) {
                Ok(item) => self.priv_run(crate::sudo::PendingPrivOp::Rename {
                    item,
                    case_only: false,
                }),
                Err(failure) => self.show_notice(
                    "Not renamed",
                    vec![format!("\u{201C}{name}\u{201D} was {}.", failure.reason)],
                ),
            }
            return;
        }
        let old = match crate::trash::trash(&to) {
            Ok(t) => t,
            Err(e) => {
                self.show_notice(
                    "Not renamed",
                    vec![format!(
                        "The existing \u{201C}{name}\u{201D} can't be moved to the Trash ({}), so it was not replaced.",
                        e.reason()
                    )],
                );
                return;
            }
        };
        match crate::copy_tree::rename_noreplace(&from, &to) {
            Ok(()) => {
                // The old item's entry goes below the rename's: it is undone
                // after it, when the name is free again.
                self.undo_stack.push(crate::undo::UndoAction::Trash(vec![(
                    old.original.clone(),
                    old.trashed.clone(),
                    old.info.clone(),
                )]));
                self.undo_stack
                    .push(crate::undo::UndoAction::Rename { from, to });
            }
            Err(e) => {
                // The rename did not happen: the old item comes back.
                let back = crate::trash::restore(&old).is_ok();
                let mut lines = vec![format!(
                    "Renaming to \u{201C}{name}\u{201D} failed: {}.",
                    crate::copy_tree::reason_of(&e)
                )];
                if !back {
                    lines.push("The item it was replacing is in the Trash.".into());
                }
                self.show_notice("Not renamed", lines);
            }
        }
    }

    /// Cancel an in-progress rename waiting on the conflict dialog.
    pub fn cancel_rename_conflict(&mut self) {
        self.root_question_fronted();
        self.conflict_dialog = None;
        self.pending_rename = None;
    }

    /// Handle a user choice on the conflict dialog. Closes the dialog,
    /// optionally promotes the action to apply-to-all, and resumes the
    /// pending paste walk.
    pub fn resolve_conflict(&mut self, action: crate::conflict::ConflictAction) {
        self.root_question_fronted();
        if matches!(action, crate::conflict::ConflictAction::Replace)
            && self.conflict_dialog.as_ref().is_some_and(|d| !d.replace_allowed())
        {
            return;
        }
        let apply_to_all = self
            .conflict_dialog
            .as_ref()
            .map(|d| d.apply_to_all)
            .unwrap_or(false);
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
        use crate::conflict::ConflictAction;
        let ops = &self.ops;
        let Some(paste) = self.pending_paste.as_mut() else {
            return;
        };
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

        let resolved = match action {
            ConflictAction::Skip => None,
            // Last line of defence, as in advance_paste: never trash a
            // folder that holds the very item being pasted.
            ConflictAction::Replace
                if paste.originals.iter().any(|o| target_holds(&target, o)) =>
            {
                Some((keep_both(paste, ops, &target), false))
            }
            // Only the intent is recorded. The old item is trashed by the
            // worker, after the new one has fully arrived.
            ConflictAction::Replace => Some((target, true)),
            ConflictAction::KeepBoth => Some((keep_both(paste, ops, &target), false)),
        };

        paste.remaining.remove(0);
        if let Some((target, replace)) = resolved {
            paste.resolved.push(OpItem {
                src,
                target,
                replace,
            });
        }
        self.advance_paste();
    }

    /// Cancel an in-progress paste (closes the dialog, discards remaining
    /// work). Nothing has been touched yet: the answers so far were only
    /// recorded, so everything is as it was before the paste.
    pub fn cancel_paste(&mut self) {
        self.root_question_fronted();
        self.conflict_dialog = None;
        if let Some(paste) = self.pending_paste.take() {
            use crate::conflict::PasteMode;
            if paste.from_clipboard && self.clipboard.is_none() {
                self.clipboard = Some(match paste.mode {
                    PasteMode::Copy => ClipboardOp::Copy(paste.originals),
                    PasteMode::Cut => ClipboardOp::Cut(paste.originals),
                });
            }
        }
        self.reload();
    }

    pub fn open_selected(&mut self) {
        let selected: Vec<_> = self
            .entries
            .iter()
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
        // Our extension → MIME → default-app lookup (which beats xdg-open's
        // content-sniffing for short code files that get mis-classified as
        // text/plain), then the app's own launch rules.
        let items: Vec<(PathBuf, bool)> = selected
            .iter()
            .map(|&i| (self.entries[i].path.clone(), self.entries[i].is_dir))
            .collect();
        for (path, is_dir) in items {
            if is_dir {
                // One folder of several selected items: whatever the
                // desktop opens folders with (a folder has no extension to
                // go by, whatever its name looks like).
                crate::desktop::xdg_open(path);
            } else {
                self.open_file(path);
            }
        }
    }

    #[allow(dead_code)]
    pub fn open_with(&self, app_name: &str) {
        for entry in &self.entries {
            if !entry.selected {
                continue;
            }
            let path = entry.path.clone();
            let app = app_name.to_string();
            let mut cmd = std::process::Command::new(&app);
            cmd.arg(&path);
            crate::desktop::spawn_reaped(cmd);
        }
    }

    #[allow(dead_code)]
    pub fn copy_path_to_clipboard(&self) {
        let paths: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.path.display().to_string())
            .collect();
        if paths.is_empty() {
            return;
        }
        let text = paths.join("\n");
        if let Some(clip) = &self.wayland_clipboard {
            clip.set_text(&text);
        }
    }

    #[allow(dead_code)]
    pub fn copy_name_to_clipboard(&self) {
        let names: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.name.clone())
            .collect();
        if names.is_empty() {
            return;
        }
        let text = names.join("\n");
        if let Some(clip) = &self.wayland_clipboard {
            clip.set_text(&text);
        }
    }

    /// Unpack every selected archive (or the one the context menu was
    /// opened on) into a new folder next to it.
    pub fn extract_selected(&mut self) {
        let selected = self.selected_paths();
        if selected.is_empty() || self.refuse_in_trash(&selected, "extracted") {
            return;
        }
        if selected[0].parent().is_some_and(|dir| self.root_covers(dir)) {
            self.extract_as_root(&selected);
            return;
        }
        let done = self.refresh_flag();
        std::thread::spawn(move || {
            for path in &selected {
                // Next to the archive: for a row inside an expanded folder
                // that is that folder, not the one being shown.
                let Some(dir) = path.parent() else {
                    continue;
                };
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                // Lowercased like is_archive, so BACKUP.TAR.GZ is recognised.
                let name = path.to_string_lossy().to_lowercase();
                if !is_archive(path) {
                    continue;
                }

                let stem = archive_stem(path);
                // Always a folder of its own: unpacking on top of an
                // existing one would overwrite whatever is in it.
                let Some(extract_dir) = fresh_dir(dir, &stem) else {
                    eprintln!("[fox] extract: no folder could be created for {stem}");
                    continue;
                };

                // Build (program, args) for each archive type
                let (prog, args): (&str, Vec<std::ffi::OsString>) =
                    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
                        (
                            "tar",
                            vec![
                                "xzf".into(),
                                path.as_os_str().into(),
                                "-C".into(),
                                extract_dir.as_os_str().into(),
                            ],
                        )
                    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
                        (
                            "tar",
                            vec![
                                "xjf".into(),
                                path.as_os_str().into(),
                                "-C".into(),
                                extract_dir.as_os_str().into(),
                            ],
                        )
                    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
                        (
                            "tar",
                            vec![
                                "xJf".into(),
                                path.as_os_str().into(),
                                "-C".into(),
                                extract_dir.as_os_str().into(),
                            ],
                        )
                    } else if name.ends_with(".tar") {
                        (
                            "tar",
                            vec![
                                "xf".into(),
                                path.as_os_str().into(),
                                "-C".into(),
                                extract_dir.as_os_str().into(),
                            ],
                        )
                    } else if ext == "zip" {
                        (
                            "unzip",
                            // No -o: nothing here is ever overwritten. (The
                            // folder is new, so only an archive listing the
                            // same name twice could ask; stdin is closed, so
                            // unzip then keeps the first.)
                            vec![
                                path.as_os_str().into(),
                                "-d".into(),
                                extract_dir.as_os_str().into(),
                            ],
                        )
                    } else if ext == "7z" {
                        let out_flag: std::ffi::OsString =
                            format!("-o{}", extract_dir.display()).into();
                        ("7z", vec!["x".into(), path.as_os_str().into(), out_flag])
                    } else {
                        continue;
                    };

                let _ = std::process::Command::new(prog)
                    .args(&args)
                    .stdin(std::process::Stdio::null())
                    .status();
                // An archive that could not be unpacked leaves no empty
                // folder behind (remove_dir refuses a non-empty one).
                let _ = std::fs::remove_dir(&extract_dir);
            }
            // The watcher does not see inside expanded tree folders.
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    /// Extract Here in root mode: the same new-folder-per-archive rule,
    /// through the sudo flow. The folder names are chosen here; `mkdir` in
    /// the privileged command refuses one that turned up in between.
    fn extract_as_root(&mut self, selected: &[PathBuf]) {
        let mut items: Vec<(PathBuf, PathBuf)> = Vec::new();
        for path in selected.iter().filter(|p| is_archive(p)) {
            let Some(dir) = path.parent() else {
                continue;
            };
            let stem = archive_stem(path);
            let into = numbered_free(dir, &stem, |p| {
                std::fs::symlink_metadata(p).is_ok() || items.iter().any(|(_, taken)| taken == p)
            });
            items.push((path.clone(), into));
        }
        if !items.is_empty() {
            self.priv_run(crate::sudo::PendingPrivOp::Extract(items));
        }
    }

    pub fn open_in_terminal(&self) {
        let mut cmd = std::process::Command::new("lntrn-terminal");
        cmd.current_dir(&self.current_dir);
        crate::desktop::spawn_reaped(cmd);
    }

    /// A permanent delete stopped at items this account may not remove.
    /// Local ones are offered to sudo, which asks before it deletes
    /// anything. Root cannot see into a user's phone or network mount, so
    /// those are simply reported.
    fn offer_root_delete(&mut self, items: Vec<OpItem>) {
        let (mounted, items): (Vec<OpItem>, Vec<OpItem>) = items
            .into_iter()
            .partition(|i| crate::mount_guard::holds_mount(&i.src));
        if !mounted.is_empty() {
            let failures: Vec<crate::ops::OpFailure> = mounted
                .into_iter()
                .map(|i| crate::ops::OpFailure {
                    path: i.src,
                    reason: crate::mount_guard::REASON.into(),
                })
                .collect();
            self.report_failures("deleted", &failures);
        }
        let (remote, local): (Vec<PathBuf>, Vec<PathBuf>) = items
            .into_iter()
            .map(|i| i.src)
            .partition(|p| crate::fs::is_slow_path(p));
        if !remote.is_empty() {
            let failures: Vec<crate::ops::OpFailure> = remote
                .into_iter()
                .map(|path| crate::ops::OpFailure {
                    path,
                    reason: "permission denied".into(),
                })
                .collect();
            self.report_failures("deleted", &failures);
        }
        if !local.is_empty() {
            self.priv_run(crate::sudo::PendingPrivOp::Remove(local));
        }
    }
}

/// The folder name an archive unpacks into: its file name without the
/// archive extension(s).
fn archive_stem(path: &Path) -> String {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Strip compound extensions like .tar.gz, .tar.bz2, etc.
    let s = file_name.as_str();
    let lower = s.to_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tar.bz2") || lower.ends_with(".tar.xz") {
        s.rsplitn(3, '.').last().unwrap_or(s).to_string()
    } else {
        Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(file_name)
    }
}

/// `stem`, or `stem (2)`, `stem (3)`... in `parent`: the first that `taken`
/// does not claim.
pub(crate) fn numbered_free(parent: &Path, stem: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let mut candidate = parent.join(stem);
    let mut n = 2u32;
    while taken(&candidate) && n < 10_000 {
        candidate = parent.join(format!("{stem} ({n})"));
        n += 1;
    }
    candidate
}

/// Create a new, empty folder for an archive's contents: `stem`, or
/// `stem (2)`, `stem (3)`... when that name is taken. `create_dir` is the
/// test, so an existing folder is never reused.
fn fresh_dir(parent: &Path, stem: &str) -> Option<PathBuf> {
    for n in 1u32..1000 {
        let candidate = if n == 1 {
            parent.join(stem)
        } else {
            parent.join(format!("{stem} ({n})"))
        };
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Some(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

pub(crate) fn _wl_copy(_text: String) {
    // Deprecated — native Wayland clipboard (clipboard.rs) is used instead.
}

#[cfg(test)]
mod paste_guard_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lntrn-fm-paste-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn target_that_contains_the_source_is_recognised() {
        let a = scratch("holds");
        std::fs::create_dir_all(a.join("foo/foo")).unwrap();
        std::fs::create_dir_all(a.join("other")).unwrap();
        std::fs::write(a.join("foo/note.txt"), b"x").unwrap();
        // /a/foo/foo pasted into /a collides with /a/foo, which holds it.
        assert!(target_holds(&a.join("foo"), &a.join("foo/foo")));
        assert!(target_holds(&a.join("foo"), &a.join("foo/note.txt")));
        // Same, reached through a symlinked spelling of the folder.
        std::os::unix::fs::symlink(&a, a.join("link")).unwrap();
        assert!(target_holds(&a.join("link/foo"), &a.join("foo/foo")));
        // An unrelated folder, the item itself, and a symlink target do not.
        assert!(!target_holds(&a.join("other"), &a.join("foo/foo")));
        assert!(!target_holds(&a.join("foo"), &a.join("foo")));
        std::os::unix::fs::symlink(a.join("foo"), a.join("foo-link")).unwrap();
        assert!(!target_holds(&a.join("foo-link"), &a.join("foo/foo")));
        let _ = std::fs::remove_dir_all(&a);
    }

    #[test]
    fn dest_inside_source_and_same_dir() {
        let a = scratch("inside");
        std::fs::create_dir_all(a.join("src/sub")).unwrap();
        std::fs::create_dir_all(a.join("elsewhere")).unwrap();
        assert!(dest_is_inside(&a.join("src"), &a.join("src")));
        assert!(dest_is_inside(&a.join("src/sub"), &a.join("src")));
        assert!(!dest_is_inside(&a.join("elsewhere"), &a.join("src")));
        std::os::unix::fs::symlink(&a, a.join("link")).unwrap();
        assert!(same_dir(Some(&a.join("link")), &a));
        assert!(!same_dir(Some(&a.join("src")), &a));
        assert!(!same_dir(None, &a));
        let _ = std::fs::remove_dir_all(&a);
    }

    #[test]
    fn extract_folder_is_always_a_new_one() {
        let a = scratch("extract");
        std::fs::create_dir_all(a.join("project")).unwrap();
        std::fs::write(a.join("project/work.txt"), b"current").unwrap();
        // The name is taken: a second folder is made, the first is untouched.
        assert_eq!(fresh_dir(&a, "project").unwrap(), a.join("project (2)"));
        assert_eq!(fresh_dir(&a, "project").unwrap(), a.join("project (3)"));
        assert_eq!(fresh_dir(&a, "other").unwrap(), a.join("other"));
        assert_eq!(std::fs::read(a.join("project/work.txt")).unwrap(), b"current");
        let _ = std::fs::remove_dir_all(&a);
    }

    #[test]
    fn root_extract_picks_the_same_names_without_touching_the_disk() {
        assert_eq!(archive_stem(Path::new("/opt/Backup.TAR.GZ")), "Backup");
        assert_eq!(archive_stem(Path::new("/opt/photos.zip")), "photos");
        assert_eq!(archive_stem(Path::new("/opt/a.b.tar.xz")), "a.b");
        let opt = Path::new("/opt");
        let taken = [opt.join("photos"), opt.join("photos (2)")];
        let free = numbered_free(opt, "photos", |p| taken.iter().any(|t| t == p));
        assert_eq!(free, opt.join("photos (3)"));
        assert_eq!(numbered_free(opt, "new", |_| false), opt.join("new"));
    }
}
