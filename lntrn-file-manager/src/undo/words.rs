//! What an undo or redo did, in the user's words (the line the status bar
//! shows when it ends).

use std::path::Path;

use super::{Direction, Entry, UndoAction};

pub(super) fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// "“a.txt”" for one item, "3 items" for several.
fn count_of(paths: &[&Path], one: &str, many: &str) -> String {
    match paths {
        [path] => format!("{one}\u{201C}{}\u{201D}", name_of(path)),
        _ => format!("{} {many}", paths.len()),
    }
}

/// What a finished undo or redo did, in the user's words. `entry` is the
/// part that was reversed.
pub(super) fn describe(dir: Direction, entry: &Entry) -> String {
    use Direction::{Redo, Undo};
    let text = match (&entry.action, dir) {
        (UndoAction::Rename { from, to }, Undo) => format!(
            "renamed \u{201C}{}\u{201D} back to \u{201C}{}\u{201D}",
            name_of(to),
            name_of(from)
        ),
        (UndoAction::Rename { from, to }, Redo) => format!(
            "renamed \u{201C}{}\u{201D} to \u{201C}{}\u{201D}",
            name_of(from),
            name_of(to)
        ),
        (UndoAction::Trash(v), _) => {
            let paths: Vec<&Path> = v.iter().map(|(original, _, _)| original.as_path()).collect();
            let what = count_of(&paths, "", "items");
            match dir {
                Undo => format!("{what} restored from the Trash"),
                Redo => format!("{what} moved to the Trash"),
            }
        }
        (UndoAction::Create(v), _) => {
            let paths: Vec<&Path> = v.iter().map(|(path, _)| path.as_path()).collect();
            let what = count_of(&paths, "", "items");
            // An empty folder where there is no Trash was simply removed.
            let trashed = entry.notes.iter().all(|n| n.parked.is_some());
            match dir {
                Undo if trashed => format!("{what} moved to the Trash"),
                Undo => format!("{what} removed"),
                Redo => format!("{what} put back"),
            }
        }
        (UndoAction::Move(v), _) => {
            let paths: Vec<&Path> = v.iter().map(|(_, dst)| dst.as_path()).collect();
            let what = count_of(&paths, "", "items");
            match dir {
                Undo => format!("{what} moved back"),
                Redo => format!("{what} moved again"),
            }
        }
        (UndoAction::Copy(v), _) => {
            let paths: Vec<&Path> = v.iter().map(|(_, created)| created.as_path()).collect();
            let what = count_of(&paths, "the copy ", "copies");
            match dir {
                Undo => format!("{what} moved to the Trash"),
                Redo => format!("{what} put back"),
            }
        }
    };
    match dir {
        Undo => format!("Undo: {text}"),
        Redo => format!("Redo: {text}"),
    }
}
