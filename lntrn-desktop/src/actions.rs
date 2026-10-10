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
            if !trimmed.is_empty()
                && trimmed != old.file_name().unwrap_or_default().to_string_lossy()
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
        PendingAction::Launch(cmd) => {
            // Spawn a command (program + args) detached, cwd = desktop dir.
            // Args let ring entries do e.g.
            // "lntrn-system-settings --panel wallpaper".
            let mut parts = split_command(&cmd).into_iter();
            if let Some(prog) = parts.next() {
                let mut command = std::process::Command::new(prog);
                command.args(parts).current_dir(&app.desktop_dir);
                spawn_reaped(command);
            }
        }
        PendingAction::CommandCenter => spawn_reaped(command_center_toggle()),
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

/// Run `command` on a thread that waits for it. Nothing else here reaps
/// children, so one that was only spawned would stay a zombie for as long as
/// the desktop runs — and the Command Center's toggle exits at once.
fn spawn_reaped(mut command: std::process::Command) {
    std::thread::spawn(move || match command.spawn() {
        Ok(mut child) => {
            let _ = child.wait();
        }
        Err(e) => tracing::warn!("[desktop] could not run {:?}: {e}", command.get_program()),
    });
}

/// `lntrn-command-center --toggle`, set up the way the compositor does it for
/// a Super tap: when no daemon is running yet this very process becomes it, so
/// it gets the daemon's log file, a process group of its own and the home
/// directory rather than whatever the desktop has.
fn command_center_toggle() -> std::process::Command {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
    let log_dir = home.join(".lantern/log");
    let _ = std::fs::create_dir_all(&log_dir);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("lntrn-command-center.log"));
    let (out, err) = match log.and_then(|f| Ok((f.try_clone()?, f))) {
        Ok((out, err)) => (Stdio::from(out), Stdio::from(err)),
        Err(_) => (Stdio::null(), Stdio::null()),
    };

    let mut command = std::process::Command::new("lntrn-command-center");
    command
        .arg("--toggle")
        .current_dir(&home)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0);
    command
}

/// Split a command line into its program and arguments the way a `.desktop`
/// `Exec=` is read: whitespace separates, double or single quotes keep a run
/// together, and a backslash (outside single quotes) takes the next character
/// as it is. No shell is involved: `$VAR`, `~` and `|` mean nothing here.
fn split_command(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    // Quotes can make an empty word (""), so "in a word" is tracked on its own.
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"') | None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    in_word = true;
                }
            }
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split_command;

    #[test]
    fn commands_split_on_spaces_and_quotes_keep_runs_together() {
        assert_eq!(split_command("lntrn-terminal"), ["lntrn-terminal"]);
        assert_eq!(
            split_command("  lntrn-system-settings   --panel wallpaper "),
            ["lntrn-system-settings", "--panel", "wallpaper"]
        );
        assert_eq!(
            split_command(r#""/home/a/Ember Nights/run.sh" --fast"#),
            ["/home/a/Ember Nights/run.sh", "--fast"]
        );
        assert_eq!(
            split_command(r#"env "WINEPREFIX=/home/a/.wine b" wine start 'C:\Games\x.exe'"#),
            ["env", "WINEPREFIX=/home/a/.wine b", "wine", "start", r"C:\Games\x.exe"]
        );
        assert_eq!(
            split_command(r#"say "a \"b\"" c\ d "" end"#),
            ["say", r#"a "b""#, "c d", "", "end"]
        );
        assert!(split_command("   ").is_empty());
    }
}
