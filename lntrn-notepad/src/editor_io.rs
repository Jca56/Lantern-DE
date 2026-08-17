//! Editor file I/O — load/save plumbing split out of `editor.rs` so that
//! file stays focused on state + editing ops.

use std::path::PathBuf;

use crate::editor::Editor;

impl Editor {
    pub fn title(&self) -> String {
        if self.modified {
            format!("* {} — lntrn-notepad", self.filename)
        } else {
            format!("{} — lntrn-notepad", self.filename)
        }
    }

    pub fn load_file(&mut self, path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(&path)?;
        let (lines, formats) = crate::persist::parse(&path, &content);
        self.lines = lines;
        self.formats = formats;
        self.layout.clear();
        self.layout_key = None;
        self.total_h = 0.0;
        self.filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        self.file_path = Some(path);
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.sel_anchor = None;
        self.pending_attrs = None;
        self.modified = false;
        self.scroll_offset = 0.0;
        self.scroll_target = 0.0;
        self.clear_history();
        Ok(())
    }

    pub fn save_file(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.file_path.as_ref().ok_or("No file path set")?;
        // Format picked by extension: .lnote = rich (full spans + paragraph
        // attrs), everything else = plain text with the "- " bullet trick.
        let content = crate::persist::serialize(path, &self.lines, &self.formats);
        std::fs::write(path, &content)?;
        self.modified = false;
        self.filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        Ok(())
    }
}
