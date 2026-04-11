//! Find & Replace overlay. Slides down at the top of the editor area when
//! Ctrl+F (find) or Ctrl+H (replace) is pressed.
//!
//! Match scanning is line-based: queries cannot span newlines for now. Replace
//! operations group into a single undo entry.

use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::editor::Editor;

pub const FIND_ROW_H: f32 = 38.0;

/// Bar mode: hidden, find-only, or find+replace.
#[derive(Clone, Copy, PartialEq)]
pub enum FindMode {
    Hidden,
    Find,
    Replace,
}

#[derive(Clone, Copy, PartialEq)]
pub enum FocusField {
    Query,
    Replace,
}

#[derive(Clone, Copy, Debug)]
pub struct MatchSpan {
    pub line: usize,
    pub start: usize, // byte offset within the line
    pub end: usize,
}

pub struct FindBar {
    pub mode: FindMode,
    pub focus: FocusField,
    pub query: String,
    pub query_cursor: usize,
    pub replace: String,
    pub replace_cursor: usize,
    pub matches: Vec<MatchSpan>,
    pub current: usize,
    pub case_sensitive: bool,
}

impl FindBar {
    pub fn new() -> Self {
        Self {
            mode: FindMode::Hidden,
            focus: FocusField::Query,
            query: String::new(),
            query_cursor: 0,
            replace: String::new(),
            replace_cursor: 0,
            matches: Vec::new(),
            current: 0,
            case_sensitive: false,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.mode != FindMode::Hidden
    }

    /// Total bar height in physical pixels.
    pub fn height(&self, scale: f32) -> f32 {
        match self.mode {
            FindMode::Hidden => 0.0,
            FindMode::Find => FIND_ROW_H * scale,
            FindMode::Replace => FIND_ROW_H * 2.0 * scale,
        }
    }

    pub fn open_find(&mut self, prefill: Option<String>) {
        self.mode = FindMode::Find;
        self.focus = FocusField::Query;
        if let Some(text) = prefill {
            self.query = text;
            self.query_cursor = self.query.len();
        }
    }

    pub fn open_replace(&mut self) {
        self.mode = FindMode::Replace;
        self.focus = FocusField::Query;
    }

    pub fn close(&mut self) {
        self.mode = FindMode::Hidden;
        self.matches.clear();
        self.current = 0;
    }

    /// Walk the document looking for occurrences of `query`. Updates
    /// `self.matches` and resets `current` to 0.
    pub fn recompute(&mut self, editor: &Editor) {
        self.matches.clear();
        self.current = 0;
        if self.query.is_empty() {
            return;
        }
        let needle = if self.case_sensitive {
            self.query.clone()
        } else {
            self.query.to_lowercase()
        };
        for (line_idx, line) in editor.lines.iter().enumerate() {
            let haystack: String = if self.case_sensitive {
                line.clone()
            } else {
                line.to_lowercase()
            };
            let mut from = 0usize;
            while let Some(pos) = haystack[from..].find(&needle) {
                let start = from + pos;
                let end = start + needle.len();
                self.matches.push(MatchSpan {
                    line: line_idx,
                    start,
                    end,
                });
                from = end;
                if needle.is_empty() {
                    break;
                }
            }
        }
    }

    /// Move editor cursor to the current match (no scroll math here — the
    /// next render frame will reveal it via natural scroll).
    pub fn focus_current(&self, editor: &mut Editor) {
        if let Some(m) = self.matches.get(self.current) {
            editor.cursor_line = m.line;
            editor.cursor_col = m.start;
            editor.sel_anchor = Some(crate::editor::Pos {
                line: m.line,
                col: m.end,
            });
        }
    }

    pub fn next(&mut self, editor: &mut Editor) {
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.focus_current(editor);
    }

    pub fn prev(&mut self, editor: &mut Editor) {
        if self.matches.is_empty() {
            return;
        }
        self.current = if self.current == 0 {
            self.matches.len() - 1
        } else {
            self.current - 1
        };
        self.focus_current(editor);
    }

    /// Replace the current match with the replace text, then advance.
    pub fn replace_one(&mut self, editor: &mut Editor) {
        if self.matches.is_empty() {
            return;
        }
        let m = self.matches[self.current];
        editor.push_undo();
        single_line_replace(editor, m, &self.replace);
        editor.modified = true;
        // Drop the previous match's selection anchor — it pointed at the old
        // text and may now sit past the line end.
        editor.sel_anchor = None;
        clamp_cursor(editor);
        // Re-scan after the change so offsets are valid.
        self.recompute(editor);
        if !self.matches.is_empty() && self.current >= self.matches.len() {
            self.current = self.matches.len() - 1;
        }
        // focus_current will set a fresh selection on the next match (or
        // leave sel_anchor at None if we replaced the last match).
        self.focus_current(editor);
    }

    /// Replace every match. One undo step.
    pub fn replace_all(&mut self, editor: &mut Editor) {
        if self.matches.is_empty() {
            return;
        }
        editor.push_undo();
        // Apply in reverse so earlier offsets stay valid.
        let to_apply: Vec<MatchSpan> = self.matches.clone();
        for m in to_apply.iter().rev() {
            single_line_replace(editor, *m, &self.replace);
        }
        editor.modified = true;
        editor.sel_anchor = None;
        clamp_cursor(editor);
        self.recompute(editor);
    }

    /// Returns true if the key was consumed.
    pub fn handle_key(
        &mut self,
        key: &Key,
        mods: ModifiersState,
        editor: &mut Editor,
    ) -> bool {
        let ctrl = mods.contains(ModifiersState::CONTROL);
        let shift = mods.contains(ModifiersState::SHIFT);

        match key {
            Key::Named(NamedKey::Escape) => {
                self.close();
                true
            }
            Key::Named(NamedKey::Enter) => {
                // Ctrl+Enter → replace all (any field, in Replace mode).
                // Enter in the Replace field → replace one and advance.
                // Otherwise: Enter = next, Shift+Enter = prev.
                if ctrl && self.mode == FindMode::Replace {
                    self.replace_all(editor);
                } else if self.mode == FindMode::Replace
                    && self.focus == FocusField::Replace
                    && !shift
                {
                    self.replace_one(editor);
                } else if shift {
                    self.prev(editor);
                } else {
                    self.next(editor);
                }
                true
            }
            Key::Named(NamedKey::Tab) => {
                if self.mode == FindMode::Replace {
                    self.focus = match self.focus {
                        FocusField::Query => FocusField::Replace,
                        FocusField::Replace => FocusField::Query,
                    };
                }
                true
            }
            Key::Named(NamedKey::Backspace) => {
                let (text, cursor) = self.active_field_mut();
                if *cursor > 0 {
                    let prev = prev_char_boundary(text, *cursor);
                    text.replace_range(prev..*cursor, "");
                    *cursor = prev;
                }
                if self.focus == FocusField::Query {
                    self.recompute(editor);
                    self.focus_current(editor);
                }
                true
            }
            Key::Named(NamedKey::ArrowLeft) => {
                let (text, cursor) = self.active_field_mut();
                if *cursor > 0 {
                    *cursor = prev_char_boundary(text, *cursor);
                }
                true
            }
            Key::Named(NamedKey::ArrowRight) => {
                let (text, cursor) = self.active_field_mut();
                if *cursor < text.len() {
                    *cursor = next_char_boundary(text, *cursor);
                }
                true
            }
            Key::Named(NamedKey::Home) => {
                let (_, cursor) = self.active_field_mut();
                *cursor = 0;
                true
            }
            Key::Named(NamedKey::End) => {
                let (text, cursor) = self.active_field_mut();
                *cursor = text.len();
                true
            }
            Key::Named(NamedKey::Space) => {
                self.insert_str(" ");
                if self.focus == FocusField::Query {
                    self.recompute(editor);
                    self.focus_current(editor);
                }
                true
            }
            Key::Character(s) if !ctrl => {
                self.insert_str(s.as_str());
                if self.focus == FocusField::Query {
                    self.recompute(editor);
                    self.focus_current(editor);
                }
                true
            }
            _ => false,
        }
    }

    fn active_field_mut(&mut self) -> (&mut String, &mut usize) {
        match self.focus {
            FocusField::Query => (&mut self.query, &mut self.query_cursor),
            FocusField::Replace => (&mut self.replace, &mut self.replace_cursor),
        }
    }

    fn insert_str(&mut self, s: &str) {
        let (text, cursor) = self.active_field_mut();
        text.insert_str(*cursor, s);
        *cursor += s.len();
    }
}

fn prev_char_boundary(s: &str, i: usize) -> usize {
    let mut p = i.saturating_sub(1);
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn next_char_boundary(s: &str, i: usize) -> usize {
    let mut p = (i + 1).min(s.len());
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

/// Replace the byte range `[m.start, m.end)` on `m.line` with `replacement`.
/// Adjusts format spans on that line to keep them valid. Snaps offsets to
/// char boundaries so we never panic on multi-byte text.
fn single_line_replace(editor: &mut Editor, m: MatchSpan, replacement: &str) {
    if m.line >= editor.lines.len() {
        return;
    }
    let line_len = editor.lines[m.line].len();
    let mut end = m.end.min(line_len);
    let mut start = m.start.min(end);
    while start < line_len && !editor.lines[m.line].is_char_boundary(start) {
        start += 1;
    }
    while end < line_len && !editor.lines[m.line].is_char_boundary(end) {
        end += 1;
    }
    if start >= end {
        return;
    }
    editor.lines[m.line].replace_range(start..end, replacement);
    let fmts = editor.formats.get_mut(m.line);
    fmts.delete_range(start, end);
    fmts.insert_at(start, replacement.len());
}

/// Clamp `cursor_line` and `cursor_col` to be within the editor's current
/// line bounds. Snaps `cursor_col` to the nearest preceding char boundary so
/// the next render frame can safely measure text up to the cursor.
fn clamp_cursor(editor: &mut Editor) {
    if editor.lines.is_empty() {
        editor.cursor_line = 0;
        editor.cursor_col = 0;
        return;
    }
    if editor.cursor_line >= editor.lines.len() {
        editor.cursor_line = editor.lines.len() - 1;
    }
    let line = &editor.lines[editor.cursor_line];
    if editor.cursor_col > line.len() {
        editor.cursor_col = line.len();
    }
    while editor.cursor_col > 0 && !line.is_char_boundary(editor.cursor_col) {
        editor.cursor_col -= 1;
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Draw the find bar overlay at the top of the editor area, starting at
/// `editor_left` and spanning `editor_width` (so the sidebar is respected).
pub fn draw_find_bar(
    bar: &FindBar,
    painter: &mut Painter,
    text: &mut TextRenderer,
    _input: &mut InteractionContext,
    palette: &FoxPalette,
    editor_top: f32,
    editor_left: f32,
    editor_width: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    if bar.mode == FindMode::Hidden {
        return;
    }
    let h = bar.height(s);
    let bar_rect = Rect::new(editor_left, editor_top, editor_width, h);

    // Background — slightly inset paper plate.
    painter.rect_filled(bar_rect, 0.0, palette.surface_2);
    // Bottom hairline separator.
    painter.line(
        editor_left,
        editor_top + h,
        editor_left + editor_width,
        editor_top + h,
        1.0 * s,
        Color::from_rgba8(60, 50, 35, 60),
    );

    let row_h = FIND_ROW_H * s;
    let pad_x = 14.0 * s;
    let font_px = 18.0 * s;
    let label_y_offset = (row_h - font_px) * 0.5;

    // ── Find row ─────────────────────────────────────────────────────
    draw_field_row(
        painter,
        text,
        palette,
        Rect::new(editor_left, editor_top, editor_width, row_h),
        "Find",
        &bar.query,
        bar.query_cursor,
        bar.focus == FocusField::Query,
        s,
        sw,
        sh,
        font_px,
        pad_x,
        label_y_offset,
    );

    // ── Match counter on the right of the find row ─────────────────
    let counter = if bar.matches.is_empty() {
        if bar.query.is_empty() {
            String::new()
        } else {
            "no matches".to_string()
        }
    } else {
        format!("{} of {}", bar.current + 1, bar.matches.len())
    };
    if !counter.is_empty() {
        let cw = text.measure_width(&counter, font_px);
        text.queue_styled(
            &counter,
            font_px,
            editor_left + editor_width - cw - pad_x,
            editor_top + label_y_offset,
            palette.text_secondary,
            cw,
            FontWeight::Normal,
            FontStyle::Normal,
            sw,
            sh,
        );
    }

    // ── Replace row ─────────────────────────────────────────────────
    if bar.mode == FindMode::Replace {
        draw_field_row(
            painter,
            text,
            palette,
            Rect::new(editor_left, editor_top + row_h, editor_width, row_h),
            "Replace",
            &bar.replace,
            bar.replace_cursor,
            bar.focus == FocusField::Replace,
            s,
            sw,
            sh,
            font_px,
            pad_x,
            label_y_offset,
        );
    }
}

fn draw_field_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    row: Rect,
    label: &str,
    value: &str,
    cursor_byte: usize,
    focused: bool,
    s: f32,
    sw: u32,
    sh: u32,
    font_px: f32,
    pad_x: f32,
    label_y_offset: f32,
) {
    // Label
    let label_w = text.measure_width(label, font_px);
    text.queue_styled(
        label,
        font_px,
        row.x + pad_x,
        row.y + label_y_offset,
        palette.text_secondary,
        label_w,
        FontWeight::Normal,
        FontStyle::Normal,
        sw,
        sh,
    );

    // Input rect
    let input_x = row.x + pad_x + label_w + 12.0 * s;
    let input_w = (row.w - input_x - 140.0 * s).max(60.0 * s);
    let input_h = 26.0 * s;
    let input_y = row.y + (row.h - input_h) * 0.5;
    let input_rect = Rect::new(input_x, input_y, input_w, input_h);

    // Input background
    let input_bg = palette.bg;
    painter.rect_filled(input_rect, 4.0 * s, input_bg);
    let border_color = if focused {
        palette.accent
    } else {
        Color::from_rgba8(60, 50, 35, 60)
    };
    painter.rect_stroke(input_rect, 4.0 * s, 1.5 * s, border_color);

    // Value text
    let text_x = input_rect.x + 8.0 * s;
    let text_y = input_rect.y + (input_h - font_px) * 0.5;
    text.queue_styled(
        value,
        font_px,
        text_x,
        text_y,
        palette.text,
        input_rect.w - 16.0 * s,
        FontWeight::Normal,
        FontStyle::Normal,
        sw,
        sh,
    );

    // Caret
    if focused {
        let prefix = &value[..cursor_byte.min(value.len())];
        let caret_x = text_x + text.measure_width(prefix, font_px);
        painter.rect_filled(
            Rect::new(caret_x, text_y, 2.0 * s, font_px),
            0.0,
            palette.accent,
        );
    }
}

/// Paint match highlights inside the editor body. Caller passes the offsets
/// computed by render.rs (since wrap-aware measurement lives there).
pub fn match_color(current: bool) -> Color {
    if current {
        Color::from_rgba8(255, 100, 0, 180)
    } else {
        Color::from_rgba8(255, 140, 40, 90)
    }
}
