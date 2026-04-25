//! Run-button logic. Saves the active file, picks a runner from its
//! extension, and pipes the command into the embedded terminal panel
//! so output streams live in the editor.
//!
//! Adding a new language = adding one match arm in `command_for`.

use std::path::{Path, PathBuf};

use crate::actions;
use crate::term_panel::TermPanel;
use crate::TextHandler;

/// Build the shell command line that runs `path`. Returns None if the
/// extension isn't one we know how to handle — caller surfaces that to
/// the user as an error rather than silently no-oping.
pub fn command_for(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let dir = path.parent()?.to_string_lossy().to_string();
    let file = path.file_name()?.to_string_lossy().to_string();

    let cmd = match ext.as_str() {
        "py" => format!("cd {dir:?} && python3 {file:?}"),
        "rs" => match cargo_root(path) {
            Some(root) => {
                let root = root.to_string_lossy().to_string();
                format!("cd {root:?} && cargo run")
            }
            None => {
                let bin = format!("/tmp/lntrn-run-{}", std::process::id());
                format!("cd {dir:?} && rustc {file:?} -o {bin:?} && {bin:?}")
            }
        },
        "js" | "mjs" | "cjs" => format!("cd {dir:?} && node {file:?}"),
        "ts" | "tsx" => format!("cd {dir:?} && npx --yes tsx {file:?}"),
        "sh" | "bash" | "zsh" => format!("cd {dir:?} && bash {file:?}"),
        "go" => format!("cd {dir:?} && go run {file:?}"),
        _ => return None,
    };
    Some(cmd)
}

/// Walk upward from `path` looking for a `Cargo.toml`. Returns the
/// containing directory so we can `cd` into the workspace root before
/// `cargo run`.
fn cargo_root(path: &Path) -> Option<PathBuf> {
    let mut cur = path.parent().map(|p| p.to_path_buf());
    while let Some(dir) = cur {
        if dir.join("Cargo.toml").exists() {
            return Some(dir);
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    None
}

/// Save the active file, ensure the terminal panel is open + focused,
/// and pipe the runner command into the shell's stdin.
pub fn run_active_file(handler: &mut TextHandler) {
    actions::save_file_dialog(handler);

    let Some(path) = handler.editor().file_path.clone() else {
        eprintln!("[lntrn-code] run: file has no path — save it first");
        return;
    };

    let Some(cmd) = command_for(&path) else {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("(none)");
        eprintln!("[lntrn-code] run: don't know how to run .{ext} files");
        return;
    };

    if handler.term_panel.is_none() {
        match TermPanel::new(handler.proxy.clone()) {
            Ok(p) => handler.term_panel = Some(p),
            Err(e) => {
                eprintln!("[lntrn-code] run: terminal spawn failed: {e}");
                return;
            }
        }
    }

    if let Some(panel) = handler.term_panel.as_mut() {
        panel.visible = true;
        panel.focused = true;
        let mut bytes = cmd.into_bytes();
        bytes.push(b'\n');
        panel.pty.write(&bytes);
    }
}
