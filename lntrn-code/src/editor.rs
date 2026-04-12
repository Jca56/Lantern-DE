use std::path::PathBuf;

use crate::auto_pair;
use crate::format::{DocFormats, LineFormats, TextAttrs};
use crate::scrollbar::ScrollbarState;
use crate::syntax::{detect_from_filename, Language};

/// Font size for editor text (physical pixels, scaled at draw time).
pub const FONT_SIZE: f32 = 24.0;
/// Line height multiplier.
pub const LINE_HEIGHT: f32 = 1.5;
/// Padding inside the editor area.
pub const PAD: f32 = 14.0;

/// A (line, byte_col) position in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self { Self { line, col } }
}

impl PartialOrd for Pos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}
impl Ord for Pos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.line.cmp(&other.line).then(self.col.cmp(&other.col))
    }
}

/// Undo/redo snapshot.
#[derive(Clone)]
struct Snapshot {
    lines: Vec<String>,
    formats: DocFormats,
    cursor: Pos,
    sel_anchor: Option<Pos>,
}

const MAX_UNDO: usize = 200;

/// Rich text editor state with cursor, selection, formatting, and undo.
pub struct Editor {
    pub lines: Vec<String>,
    pub formats: DocFormats,
    pub cursor_line: usize,
    pub cursor_col: usize,
    /// Selection anchor — when Some, text between anchor and cursor is selected.
    pub sel_anchor: Option<Pos>,
    /// Pending format attrs for next typed character (set when toggling with no selection).
    pub pending_attrs: Option<TextAttrs>,
    pub file_path: Option<PathBuf>,
    pub filename: String,
    pub language: Language,
    pub modified: bool,
    /// Stable identifier for this tab. Assigned by `TextHandler` when the tab
    /// is created — `Editor::new` returns 0 and the host overwrites it.
    pub tab_id: u64,
    /// `Some(source_tab_id)` if this is a Markdown preview rendering some
    /// other tab. Preview tabs are read-only and re-sync from their source
    /// each frame the source changes.
    pub preview_of: Option<u64>,
    /// Hash of the source's `lines` from the last sync, so we can skip
    /// reformatting when nothing has changed.
    pub preview_source_hash: u64,
    /// Animated scroll position drawn on screen. Eases toward `scroll_target`.
    pub scroll_offset: f32,
    /// Where the editor wants to be scrolled to. Updated by the wheel /
    /// keyboard nav; `scroll_offset` interpolates toward it each frame.
    pub scroll_target: f32,
    /// Per-line word-wrap row starts (byte offsets). Recomputed each frame.
    pub wrap_rows: Vec<Vec<usize>>,
    pub wrap_enabled: bool,
    pub scrollbar: ScrollbarState,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            formats: DocFormats::new(1),
            cursor_line: 0,
            cursor_col: 0,
            sel_anchor: None,
            pending_attrs: None,
            file_path: None,
            filename: "Untitled".to_string(),
            language: Language::None,
            modified: false,
            tab_id: 0,
            preview_of: None,
            preview_source_hash: 0,
            scroll_offset: 0.0,
            scroll_target: 0.0,
            wrap_rows: vec![vec![0]],
            wrap_enabled: true,
            scrollbar: ScrollbarState::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn title(&self) -> String {
        if self.modified {
            format!("* {} — lntrn-text", self.filename)
        } else {
            format!("{} — lntrn-text", self.filename)
        }
    }

    /// Whether this tab is a Markdown preview of another tab. Preview tabs
    /// are read-only and re-rendered from their source each frame.
    pub fn is_preview(&self) -> bool {
        self.preview_of.is_some()
    }

    fn cursor_pos(&self) -> Pos { Pos::new(self.cursor_line, self.cursor_col) }

    fn set_cursor(&mut self, p: Pos) {
        self.cursor_line = p.line;
        self.cursor_col = p.col;
    }

    // ── Selection ──────────────────────────────────────────────────────

    /// Returns the ordered (start, end) of the selection, or None.
    pub fn selection_range(&self) -> Option<(Pos, Pos)> {
        let anchor = self.sel_anchor?;
        let cursor = self.cursor_pos();
        if anchor == cursor { return None; }
        Some(if anchor < cursor { (anchor, cursor) } else { (cursor, anchor) })
    }

    pub fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    /// Get the selected text as a String. Defensively clamps the selection
    /// to valid line bounds so a stale anchor (e.g. left over from a
    /// find/replace operation) cannot cause a panic.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let last_line = self.lines.len().saturating_sub(1);
        let s_line = start.line.min(last_line);
        let e_line = end.line.min(last_line);
        let clamp_col = |line_idx: usize, col: usize| -> usize {
            let line = &self.lines[line_idx];
            let mut c = col.min(line.len());
            while c > 0 && !line.is_char_boundary(c) {
                c -= 1;
            }
            c
        };
        let s_col = clamp_col(s_line, start.col);
        let e_col = clamp_col(e_line, end.col);
        if s_line == e_line {
            return Some(self.lines[s_line][s_col..e_col].to_string());
        }
        let mut result = String::new();
        result.push_str(&self.lines[s_line][s_col..]);
        for line in &self.lines[s_line + 1..e_line] {
            result.push('\n');
            result.push_str(line);
        }
        result.push('\n');
        result.push_str(&self.lines[e_line][..e_col]);
        Some(result)
    }

    /// Delete the selected text, leaving cursor at the start of the selection.
    pub fn delete_selection(&mut self) {
        let (start, end) = match self.selection_range() {
            Some(r) => r,
            None => return,
        };
        self.push_undo();
        if start.line == end.line {
            self.formats.get_mut(start.line).delete_range(start.col, end.col);
            self.lines[start.line].replace_range(start.col..end.col, "");
        } else {
            // Delete from start.col to end of start line in formats
            let start_line_len = self.lines[start.line].len();
            self.formats.get_mut(start.line).delete_range(start.col, start_line_len);
            // Delete from 0 to end.col in end line, then grab remaining formats
            self.formats.get_mut(end.line).delete_range(0, end.col);
            let end_fmts = self.formats.remove_line(end.line);
            // Remove middle lines' formats
            for _ in (start.line + 1)..end.line {
                self.formats.remove_line(start.line + 1);
            }
            // Append end line formats to start line
            let start_len_after = start.col; // start line was truncated to start.col
            self.formats.get_mut(start.line).append(end_fmts, start_len_after);

            let tail = self.lines[end.line][end.col..].to_string();
            self.lines[start.line].truncate(start.col);
            self.lines[start.line].push_str(&tail);
            self.lines.drain(start.line + 1..=end.line);
        }
        self.set_cursor(start);
        self.sel_anchor = None;
        self.modified = true;
    }

    pub fn clear_selection(&mut self) {
        self.sel_anchor = None;
    }

    /// Start or extend selection from the current cursor.
    pub fn begin_selection(&mut self) {
        if self.sel_anchor.is_none() {
            self.sel_anchor = Some(self.cursor_pos());
        }
    }

    pub fn select_all(&mut self) {
        self.sel_anchor = Some(Pos::new(0, 0));
        self.cursor_line = self.lines.len() - 1;
        self.cursor_col = self.lines[self.cursor_line].len();
    }

    // ── Undo / Redo ────────────────────────────────────────────────────

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            formats: self.formats.clone(),
            cursor: self.cursor_pos(),
            sel_anchor: self.sel_anchor,
        }
    }

    pub fn push_undo(&mut self) {
        let snap = self.snapshot();
        self.undo_stack.push(snap);
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        if let Some(snap) = self.undo_stack.pop() {
            self.redo_stack.push(self.snapshot());
            self.restore(snap);
        }
    }

    pub fn redo(&mut self) {
        if let Some(snap) = self.redo_stack.pop() {
            self.undo_stack.push(self.snapshot());
            self.restore(snap);
        }
    }

    fn restore(&mut self, snap: Snapshot) {
        self.lines = snap.lines;
        self.formats = snap.formats;
        self.set_cursor(snap.cursor);
        self.sel_anchor = snap.sel_anchor;
        self.modified = true;
    }

    // ── File I/O ───────────────────────────────────────────────────────

    pub fn load_file(&mut self, path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(&path)?;
        self.lines = content.lines().map(|l| l.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.formats = DocFormats::new(self.lines.len());
        self.wrap_rows = vec![vec![0]; self.lines.len()];
        self.filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        self.language = detect_from_filename(&self.filename);
        self.file_path = Some(path);
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.sel_anchor = None;
        self.pending_attrs = None;
        self.modified = false;
        self.scroll_offset = 0.0;
        self.scroll_target = 0.0;
        self.undo_stack.clear();
        self.redo_stack.clear();
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

    // ── Text editing ───────────────────────────────────────────────────

    pub fn insert_char(&mut self, ch: char) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            self.push_undo();
        }
        let pending = self.pending_attrs.clone();
        if ch == '\n' {
            // Capture indent from the line we're splitting.
            let indent: String = self.lines[self.cursor_line]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let opens_block = auto_pair::is_code_file(&self.filename)
                && self.lines[self.cursor_line][..self.cursor_col]
                    .trim_end()
                    .ends_with('{');

            let mut right_fmts =
                self.formats.get_mut(self.cursor_line).split_at(self.cursor_col);
            let rest = self.lines[self.cursor_line][self.cursor_col..].to_string();
            self.lines[self.cursor_line].truncate(self.cursor_col);
            self.cursor_line += 1;

            let closes_block = opens_block && rest.starts_with('}');

            if closes_block {
                // Between `{` and `}` — blank indented line + closing brace line.
                let extra = format!("{}    ", indent);
                self.lines.insert(self.cursor_line, extra.clone());
                self.formats.insert_line(self.cursor_line, LineFormats::new());
                right_fmts.insert_at(0, indent.len());
                self.lines.insert(self.cursor_line + 1, format!("{}{}", indent, rest));
                self.formats.insert_line(self.cursor_line + 1, right_fmts);
                self.cursor_col = extra.len();
            } else {
                let new_indent = if opens_block {
                    format!("{}    ", indent)
                } else {
                    indent
                };
                right_fmts.insert_at(0, new_indent.len());
                self.lines.insert(self.cursor_line, format!("{}{}", new_indent, rest));
                self.formats.insert_line(self.cursor_line, right_fmts);
                self.cursor_col = new_indent.len();
            }
        } else {
            let len = ch.len_utf8();
            if let Some(attrs) = pending {
                self.formats.get_mut(self.cursor_line).insert_formatted(self.cursor_col, len, attrs);
            } else {
                self.formats.get_mut(self.cursor_line).insert_at(self.cursor_col, len);
            }
            self.lines[self.cursor_line].insert(self.cursor_col, ch);
            self.cursor_col += len;
        }
        self.modified = true;
    }

    pub fn insert_str(&mut self, s: &str) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            self.push_undo();
        }
        self.pending_attrs = None;
        for ch in s.chars() {
            if ch == '\n' {
                let right_fmts = self.formats.get_mut(self.cursor_line).split_at(self.cursor_col);
                let rest = self.lines[self.cursor_line][self.cursor_col..].to_string();
                self.lines[self.cursor_line].truncate(self.cursor_col);
                self.cursor_line += 1;
                self.lines.insert(self.cursor_line, rest);
                self.formats.insert_line(self.cursor_line, right_fmts);
                self.cursor_col = 0;
            } else {
                let len = ch.len_utf8();
                self.formats.get_mut(self.cursor_line).insert_at(self.cursor_col, len);
                self.lines[self.cursor_line].insert(self.cursor_col, ch);
                self.cursor_col += len;
            }
        }
        self.modified = true;
    }

    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        self.push_undo();
        if self.cursor_col > 0 {
            let prev = self.lines[self.cursor_line][..self.cursor_col]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.formats.get_mut(self.cursor_line).delete_range(prev, self.cursor_col);
            self.lines[self.cursor_line].remove(prev);
            self.cursor_col = prev;
            self.modified = true;
        } else if self.cursor_line > 0 {
            let removed_fmts = self.formats.remove_line(self.cursor_line);
            let removed = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.formats.get_mut(self.cursor_line).append(removed_fmts, self.cursor_col);
            self.lines[self.cursor_line].push_str(&removed);
            self.modified = true;
        }
    }

    pub fn delete(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        self.push_undo();
        let line_len = self.lines[self.cursor_line].len();
        if self.cursor_col < line_len {
            let ch_len = self.lines[self.cursor_line][self.cursor_col..]
                .chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            self.formats.get_mut(self.cursor_line)
                .delete_range(self.cursor_col, self.cursor_col + ch_len);
            self.lines[self.cursor_line].remove(self.cursor_col);
            self.modified = true;
        } else if self.cursor_line + 1 < self.lines.len() {
            let next_fmts = self.formats.remove_line(self.cursor_line + 1);
            let next = self.lines.remove(self.cursor_line + 1);
            let cur_len = self.lines[self.cursor_line].len();
            self.formats.get_mut(self.cursor_line).append(next_fmts, cur_len);
            self.lines[self.cursor_line].push_str(&next);
            self.modified = true;
        }
    }

    // ── Cursor movement ────────────────────────────────────────────────

    pub fn move_left(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        if self.cursor_col > 0 {
            let prev = self.lines[self.cursor_line][..self.cursor_col]
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

    pub fn move_right(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
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

    pub fn move_up(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn move_down(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn home(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        self.cursor_col = 0;
    }

    pub fn end(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        self.cursor_col = self.lines[self.cursor_line].len();
    }

    // ── Formatting ─────────────────────────────────────────────────────

    /// Toggle a format attribute on the selection. If no selection, sets
    /// pending_attrs so the next typed character gets the toggled format.
    pub fn toggle_format(&mut self, toggle_fn: impl Fn(&mut TextAttrs)) {
        if let Some((start, end)) = self.selection_range() {
            self.push_undo();
            let line_lens: Vec<usize> = self.lines.iter().map(|l| l.len()).collect();
            self.formats.apply_format_range(
                start.line, start.col, end.line, end.col, &line_lens, &toggle_fn,
            );
            self.modified = true;
        } else {
            // No selection — toggle pending attrs for next character
            let base = self.pending_attrs.unwrap_or_else(|| {
                self.formats.get(self.cursor_line).attrs_at(self.cursor_col)
            });
            let mut attrs = base;
            toggle_fn(&mut attrs);
            self.pending_attrs = Some(attrs);
        }
    }

    /// Set font size on the selection. If no selection, sets pending_attrs.
    pub fn set_font_size(&mut self, size: f32) {
        self.toggle_format(|a| a.font_size = Some(size));
    }

    /// Query the uniform format state across the current selection.
    /// Returns default if no selection.
    pub fn selection_format_state(&self) -> TextAttrs {
        if let Some((start, end)) = self.selection_range() {
            let line_lens: Vec<usize> = self.lines.iter().map(|l| l.len()).collect();
            self.formats.query_uniform_range(
                start.line, start.col, end.line, end.col, &line_lens,
            )
        } else if let Some(pending) = self.pending_attrs {
            pending
        } else {
            self.formats.get(self.cursor_line).attrs_at(self.cursor_col)
        }
    }

    /// Total content height in physical pixels (accounts for word-wrap rows).
    pub fn content_height(&self, scale: f32) -> f32 {
        let line_h = FONT_SIZE * LINE_HEIGHT * scale;
        let rows = if self.wrap_rows.len() == self.lines.len() {
            self.wrap_rows.iter().map(|w| w.len()).sum::<usize>()
        } else {
            self.lines.len()
        };
        rows as f32 * line_h + PAD * scale * 2.0
    }

    /// Resolve which doc line and wrap-row byte range a click y falls on.
    /// Returns `(doc_line, row_start_byte, row_end_byte)`.
    pub fn wrap_row_at_y(
        &self,
        cy: f32,
        editor_rect: lntrn_render::Rect,
        scale: f32,
    ) -> (usize, usize, usize) {
        let line_h = FONT_SIZE * LINE_HEIGHT * scale;
        let text_y_start = editor_rect.y + PAD * scale - self.scroll_offset;
        let vis_row = ((cy - text_y_start) / line_h).floor().max(0.0) as usize;

        let mut cum = 0;
        for (i, wraps) in self.wrap_rows.iter().enumerate() {
            if cum + wraps.len() > vis_row {
                let row_idx = vis_row - cum;
                let row_start = wraps[row_idx];
                let row_end = wraps.get(row_idx + 1).copied().unwrap_or(self.lines[i].len());
                return (i, row_start, row_end);
            }
            cum += wraps.len();
        }
        let last = self.lines.len() - 1;
        let last_start = *self.wrap_rows.get(last).and_then(|w| w.last()).unwrap_or(&0);
        (last, last_start, self.lines[last].len())
    }

    /// Find the byte column closest to click x within a wrap-row byte range.
    pub fn col_at_x(
        &self,
        cx: f32,
        line_idx: usize,
        row_start: usize,
        row_end: usize,
        editor_rect: lntrn_render::Rect,
        scale: f32,
        mut measure_fn: impl FnMut(usize) -> f32,
    ) -> usize {
        let line_num_w = 50.0 * scale;
        let content_x = editor_rect.x + PAD * scale + line_num_w;
        let rel_x = (cx - content_x).max(0.0);

        let line = &self.lines[line_idx];
        let char_offsets: Vec<usize> = line[row_start..row_end]
            .char_indices()
            .map(|(i, _)| row_start + i)
            .chain(std::iter::once(row_end))
            .collect();

        let mut best_col = row_start;
        let mut best_dist = f32::MAX;
        for &byte_off in &char_offsets {
            let dist = (measure_fn(byte_off) - rel_x).abs();
            if dist < best_dist {
                best_dist = dist;
                best_col = byte_off;
            }
        }
        best_col
    }
}
