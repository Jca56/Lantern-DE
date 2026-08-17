use std::path::PathBuf;

use crate::format::{Alignment, DocFormats, LineFormats, ParagraphAttrs, TextAttrs};
use crate::layout::LineLayout;
use crate::scrollbar::ScrollbarState;

/// Default font size for editor text (logical pixels, scaled at draw time).
/// Spans may override this per-run via `TextAttrs::font_size`.
pub const FONT_SIZE: f32 = 24.0;
/// Padding inside the editor area.
pub const PAD: f32 = 14.0;
/// Hanging indent (logical px) for bullet-list paragraphs: the text is pushed
/// right by this much and the • glyph sits in the gap.
pub const BULLET_INDENT: f32 = 28.0;

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
    pub modified: bool,
    /// Stable identifier for this tab. Assigned by `TextHandler` when the
    /// tab is created — `Editor::new` returns 0 and the host overwrites it.
    pub tab_id: u64,
    /// Animated scroll position drawn on screen. Eases toward `scroll_target`.
    pub scroll_offset: f32,
    /// Where the editor wants to be scrolled to. Updated by the wheel /
    /// keyboard nav; `scroll_offset` interpolates toward it each frame.
    pub scroll_target: f32,
    /// Set when the caret moved and the view should follow it. Consumed by the
    /// renderer, which is the only place wrap rows are guaranteed fresh — an
    /// input handler running between an edit and the next frame would measure
    /// against stale rows and scroll to the wrong place.
    pub follow_caret: bool,
    /// Per-line geometry cache — wrap rows, advances, row sizes, stacking.
    /// Maintained by `layout::compute`, which rebuilds a line only when its
    /// content signature changes, NOT the whole document every frame.
    pub layout: Vec<LineLayout>,
    /// Global layout inputs (width/scale/font-size bits) the cache was built
    /// against. Any change invalidates every line.
    pub layout_key: Option<(u32, u32, u32)>,
    /// Stacked height of every line including paragraph spacing, excluding the
    /// editor's own padding. Maintained by `layout::compute`.
    pub total_h: f32,
    pub scrollbar: ScrollbarState,
    pub(crate) undo_stack: Vec<crate::history::Snapshot>,
    pub(crate) redo_stack: Vec<crate::history::Snapshot>,
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
            modified: false,
            tab_id: 0,
            scroll_offset: 0.0,
            scroll_target: 0.0,
            follow_caret: false,
            layout: Vec::new(),
            layout_key: None,
            total_h: 0.0,
            scrollbar: ScrollbarState::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
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
    /// Like `selected_text`, defensively clamps to valid line bounds and char
    /// boundaries so a stale anchor cannot cause a slicing panic.
    pub fn delete_selection(&mut self) {
        let (raw_start, raw_end) = match self.selection_range() {
            Some(r) => r,
            None => return,
        };
        let last_line = self.lines.len().saturating_sub(1);
        let clamp = |p: Pos| -> Pos {
            let line_idx = p.line.min(last_line);
            let line = &self.lines[line_idx];
            let mut c = p.col.min(line.len());
            while c > 0 && !line.is_char_boundary(c) {
                c -= 1;
            }
            Pos::new(line_idx, c)
        };
        let start = clamp(raw_start);
        let end = clamp(raw_end);
        if start >= end {
            self.sel_anchor = None;
            return;
        }
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

    // ── Text editing ───────────────────────────────────────────────────

    /// The attrs newly typed text at the cursor inherits: the char left of
    /// the cursor's formatting, falling back to the char at the cursor at
    /// the start of a line. Standard word-processor insertion behavior —
    /// typing at the end of a 32px run continues at 32px, click or no click.
    pub fn typing_attrs(&self) -> TextAttrs {
        let lf = self.formats.get(self.cursor_line);
        // Any byte inside the previous char resolves its span.
        lf.attrs_at(self.cursor_col.saturating_sub(1))
    }

    pub fn insert_char(&mut self, ch: char) {
        if self.has_selection() {
            self.delete_selection();
        } else {
            self.push_undo();
        }
        let attrs = self.pending_attrs.unwrap_or_else(|| self.typing_attrs());
        if ch == '\n' {
            let right_fmts = self.formats.get_mut(self.cursor_line).split_at(self.cursor_col);
            let rest = self.lines[self.cursor_line][self.cursor_col..].to_string();
            self.lines[self.cursor_line].truncate(self.cursor_col);
            self.cursor_line += 1;
            self.lines.insert(self.cursor_line, rest);
            self.formats.insert_line(self.cursor_line, right_fmts);
            self.cursor_col = 0;
            // Carry the insertion format onto the new line, where there is
            // no left-hand char to inherit from.
            self.pending_attrs = if attrs.is_default() { None } else { Some(attrs) };
        } else {
            let len = ch.len_utf8();
            self.formats.get_mut(self.cursor_line).insert_formatted(self.cursor_col, len, attrs);
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
        if s.is_empty() {
            return;
        }
        self.modified = true;
        // Pasted plain text takes the insertion point's format, same as typing.
        let inherited = self.typing_attrs();

        // Bulk insertion. The old per-char loop was O(line²) per pasted line
        // (String::insert shifts the whole tail every char) and froze the app
        // for minutes on big pastes. Here each segment is spliced in whole.
        let mut segments = s.split('\n');
        let first = segments.next().unwrap_or("");
        if !first.is_empty() {
            self.formats
                .get_mut(self.cursor_line)
                .insert_formatted(self.cursor_col, first.len(), inherited);
            self.lines[self.cursor_line].insert_str(self.cursor_col, first);
            self.cursor_col += first.len();
        }
        let rest: Vec<&str> = segments.collect();
        if rest.is_empty() {
            return;
        }

        // Newlines present: split the current line once at the cursor; the
        // tail (text + formats) moves to the end of the last pasted segment.
        // Every new line inherits the origin line's paragraph attrs, matching
        // what repeated `insert_char('\n')` calls produced.
        let right_fmts = self.formats.get_mut(self.cursor_line).split_at(self.cursor_col);
        let tail = self.lines[self.cursor_line][self.cursor_col..].to_string();
        self.lines[self.cursor_line].truncate(self.cursor_col);
        let para = self.formats.get(self.cursor_line).para;

        let n = rest.len();
        let mut new_lines: Vec<String> = Vec::with_capacity(n);
        let mut new_fmts: Vec<LineFormats> = Vec::with_capacity(n);
        for seg in &rest[..n - 1] {
            let mut lf = LineFormats::new();
            lf.para = para;
            lf.insert_formatted(0, seg.len(), inherited);
            new_lines.push((*seg).to_string());
            new_fmts.push(lf);
        }
        let last = rest[n - 1];
        let mut lf = LineFormats::new();
        lf.para = para;
        lf.insert_formatted(0, last.len(), inherited);
        lf.append(right_fmts, last.len());
        let mut text = String::with_capacity(last.len() + tail.len());
        text.push_str(last);
        text.push_str(&tail);
        new_lines.push(text);
        new_fmts.push(lf);

        let at = self.cursor_line + 1;
        self.lines.splice(at..at, new_lines);
        self.formats.insert_lines(at, new_fmts);
        self.cursor_line += n;
        self.cursor_col = last.len();
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

    // ── Cursor movement — every move drops `pending_attrs`: a not-yet-typed
    // format toggle belongs to the position where it was toggled. ──────────

    pub fn move_left(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        self.pending_attrs = None;
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
        self.pending_attrs = None;
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
        self.pending_attrs = None;
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn move_down(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        self.pending_attrs = None;
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn home(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        self.pending_attrs = None;
        self.cursor_col = 0;
    }

    pub fn end(&mut self, selecting: bool) {
        if selecting { self.begin_selection(); } else { self.clear_selection(); }
        self.pending_attrs = None;
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
            // No selection — toggle pending attrs for next typed character,
            // starting from what typing would inherit at this position.
            let base = self.pending_attrs.unwrap_or_else(|| self.typing_attrs());
            let mut attrs = base;
            toggle_fn(&mut attrs);
            self.pending_attrs = Some(attrs);
        }
    }

    /// Set font size on the selection. If no selection, sets pending_attrs.
    pub fn set_font_size(&mut self, size: f32) {
        self.toggle_format(|a| a.font_size = Some(size));
    }

    /// Set the font family (a `fonts::FONTS` index, or `None` for default) on
    /// the selection. If no selection, applies to pending_attrs.
    pub fn set_font_family(&mut self, font: Option<u8>) {
        self.toggle_format(|a| a.font = font);
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
            // Show what typing here would produce, not the char to the right.
            self.typing_attrs()
        }
    }

    // ── Paragraph formatting ────────────────────────────────────────────

    /// Apply a paragraph attribute change to the current line or all lines
    /// touched by the selection.
    pub fn set_paragraph_attr(&mut self, apply_fn: impl Fn(&mut ParagraphAttrs)) {
        self.push_undo();
        if let Some((start, end)) = self.selection_range() {
            for i in start.line..=end.line {
                apply_fn(&mut self.formats.get_mut(i).para);
            }
        } else {
            apply_fn(&mut self.formats.get_mut(self.cursor_line).para);
        }
        self.modified = true;
    }

    pub fn set_alignment(&mut self, align: Alignment) {
        self.set_paragraph_attr(|p| p.alignment = align);
    }

    /// Toggle bullet-list state on the current paragraph(s). If any touched
    /// line is not yet a bullet, turn all on; otherwise turn all off.
    pub fn toggle_bullet(&mut self) {
        let (lo, hi) = if let Some((start, end)) = self.selection_range() {
            (start.line, end.line)
        } else {
            (self.cursor_line, self.cursor_line)
        };
        let all_bullet = (lo..=hi).all(|i| self.formats.get(i).para.bullet);
        let target = !all_bullet;
        self.push_undo();
        for i in lo..=hi {
            self.formats.get_mut(i).para.bullet = target;
        }
        self.modified = true;
    }

    /// True if the cursor line is an empty (text-less) bullet item.
    pub fn cursor_on_empty_bullet(&self) -> bool {
        let lf = self.formats.get(self.cursor_line);
        lf.para.bullet && self.lines[self.cursor_line].is_empty()
    }

    /// True if the cursor is at the very start of a bullet line.
    pub fn cursor_at_bullet_start(&self) -> bool {
        self.cursor_col == 0 && self.formats.get(self.cursor_line).para.bullet
    }

    /// Clear bullet state on the cursor's line (used to "exit" the list).
    pub fn clear_bullet_here(&mut self) {
        self.push_undo();
        self.formats.get_mut(self.cursor_line).para.bullet = false;
        self.modified = true;
    }

    pub fn set_first_indent(&mut self, indent: f32) {
        self.set_paragraph_attr(|p| p.first_indent = indent);
    }

    /// Get the paragraph attrs of the line the cursor is on.
    pub fn current_para(&self) -> ParagraphAttrs {
        self.formats.get(self.cursor_line).para
    }
}
