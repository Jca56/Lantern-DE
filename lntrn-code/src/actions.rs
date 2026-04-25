//! Top-level actions invoked by the menu, keyboard, and mouse handlers —
//! file dialogs and clipboard glue.

use crate::editor::Editor;
use crate::TextHandler;

/// Open a file via the lntrn-file-manager picker. Loads it into a new tab.
pub fn open_file_dialog(handler: &mut TextHandler) {
    let output = std::process::Command::new("lntrn-file-manager")
        .args(["--pick", "--title", "Open File"])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                let mut e = Editor::new();
                e.tab_id = handler.next_tab_id;
                handler.next_tab_id += 1;
                let _ = e.load_file(std::path::PathBuf::from(path));
                handler.tabs.push(e);
                handler.active_tab = handler.tabs.len() - 1;
            }
        }
    }
}

/// Save the active editor. If no file path is set, prompt for one via the
/// file manager.
pub fn save_file_dialog(handler: &mut TextHandler) {
    if handler.editor_mut().file_path.is_some() {
        let _ = handler.editor_mut().save_file();
        crate::lsp::glue::notify_did_save(handler);
        return;
    }
    save_file_as_dialog(handler);
}

/// Always prompt for a destination, then save. Used by the Save As menu.
pub fn save_file_as_dialog(handler: &mut TextHandler) {
    let title = if handler.editor().file_path.is_some() {
        "Save As"
    } else {
        "Save File"
    };
    let output = std::process::Command::new("lntrn-file-manager")
        .args(["--pick-save", "--title", title])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                handler.editor_mut().file_path = Some(std::path::PathBuf::from(path));
                let _ = handler.editor_mut().save_file();
                // Treat as a fresh open so the right LSP picks it up.
                handler.editor_mut().lsp_just_opened = true;
            }
        }
    }
}

pub fn do_copy(handler: &mut TextHandler) {
    if let Some(text) = handler.editor().selected_text() {
        if let Some(cb) = &handler.clipboard {
            cb.set_text(&text);
        }
    }
}

pub fn do_cut(handler: &mut TextHandler) {
    if let Some(text) = handler.editor().selected_text() {
        if let Some(cb) = &handler.clipboard {
            cb.set_text(&text);
        }
        handler.editor_mut().delete_selection();
    }
}

pub fn do_paste(handler: &mut TextHandler) {
    if let Some(cb) = &handler.clipboard {
        if let Some(text) = cb.get_text() {
            handler.editor_mut().insert_str(&text);
        }
    }
}
