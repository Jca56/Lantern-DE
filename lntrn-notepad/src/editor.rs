use std::path::PathBuf;

/// Font size for editor text (physical pixels, scaled at draw time).
pub const FONT_SIZE: f32 = 24.0;
/// Line height multiplier.
pub const LINE_HEIGHT: f32 = 1.5;
/// Padding inside the editor area.
pub const PAD: f32 = 14.0;

/// Simple text editor state with cursor and content.
pub struct Editor {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub file_path: Option<PathBuf>,
    pub filename: String,
    pub modified: bool,
    pub scroll_offset: f32,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            file_path: None,
            filename: "Untitled".to_string(),
            modified: false,
            scroll_offset: 0.0,
        }
    }

    pub fn title(&self) -> String {
        if self.modified {
            format!("* {} — lntrn-text", self.filename)
        } else {
            format!("{} — lntrn-text", self.filename)
        }
    }

    pub fn load_file(&mut self, path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(&path)?;
        self.lines = content.lines().map(|l| l.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        self.file_path = Some(path);
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.modified = false;
        self.scroll_offset = 0.0;
        Ok(())
    }

    pub fn save_file(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.file_path.as_ref().ok_or("No file path set")?;
        let content: String = self.lines.join("\n");
        std::fs::write(path, &content)?;
        self.modified = false;
        self.filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        Ok(())
    }

    pub fn insert_char(&mut self, ch: char) {
        if ch == '\n' {
            let line = &self.lines[self.cursor_line];
            let rest = line[self.cursor_col..].to_string();
            self.lines[self.cursor_line].truncate(self.cursor_col);
            self.cursor_line += 1;
            self.lines.insert(self.cursor_line, rest);
            self.cursor_col = 0;
        } else {
            self.lines[self.cursor_line].insert(self.cursor_col, ch);
            self.cursor_col += ch.len_utf8();
        }
        self.modified = true;
    }

    pub fn insert_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.insert_char(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let line = &self.lines[self.cursor_line];
            // Find the previous char boundary
            let prev = line[..self.cursor_col]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.lines[self.cursor_line].remove(prev);
            self.cursor_col = prev;
            self.modified = true;
        } else if self.cursor_line > 0 {
            let removed = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&removed);
            self.modified = true;
        }
    }

    pub fn delete(&mut self) {
        let line_len = self.lines[self.cursor_line].len();
        if self.cursor_col < line_len {
            self.lines[self.cursor_line].remove(self.cursor_col);
            self.modified = true;
        } else if self.cursor_line + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.modified = true;
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            let line = &self.lines[self.cursor_line];
            let prev = line[..self.cursor_col]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor_col = prev;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
        }
    }

    pub fn move_right(&mut self) {
        let line_len = self.lines[self.cursor_line].len();
        if self.cursor_col < line_len {
            let ch_len = self.lines[self.cursor_line][self.cursor_col..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.cursor_col += ch_len;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn home(&mut self) {
        self.cursor_col = 0;
    }

    pub fn end(&mut self) {
        self.cursor_col = self.lines[self.cursor_line].len();
    }

    pub fn select_all(&mut self) {
        // For now just move cursor to end of document
        self.cursor_line = self.lines.len() - 1;
        self.cursor_col = self.lines[self.cursor_line].len();
    }

    /// Total content height in physical pixels.
    pub fn content_height(&self, scale: f32) -> f32 {
        let line_h = FONT_SIZE * LINE_HEIGHT * scale;
        self.lines.len() as f32 * line_h + PAD * scale * 2.0
    }

    /// Convert a click position (physical px) to a line/col in the editor.
    pub fn click_to_position(&mut self, cx: f32, cy: f32, wf: f32, hf: f32, scale: f32) {
        let editor_rect = crate::render::editor_rect(wf, hf, scale);
        let s = scale;
        let line_h = FONT_SIZE * LINE_HEIGHT * s;
        let text_y_start = editor_rect.y + PAD * s - self.scroll_offset;

        let rel_y = cy - text_y_start;
        let line_idx = (rel_y / line_h).floor().max(0.0) as usize;
        self.cursor_line = line_idx.min(self.lines.len() - 1);

        // Approximate column from x position
        let text_x = editor_rect.x + PAD * s;
        let rel_x = (cx - text_x).max(0.0);
        let char_w = FONT_SIZE * s * 0.52;
        let col = (rel_x / char_w).round() as usize;
        self.cursor_col = col.min(self.lines[self.cursor_line].len());
    }
}
