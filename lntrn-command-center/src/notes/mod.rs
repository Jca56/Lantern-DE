//! Quick Notes overlay page.
//!
//! Notes are persisted as `~/.lantern/state/notes/{id}.json` — one file
//! per note, atomic write on every edit. All notes load into memory at
//! CC startup; from then on the in-memory `Vec<Note>` is authoritative
//! and disk is a write-through mirror.

pub mod editor;
pub mod export;
pub mod render;
pub mod store;

use std::sync::{Arc, Mutex};

use lntrn_render::{Rect, TextRenderer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub created_ms: u128,
    pub modified_ms: u128,
    #[serde(default)]
    pub pinned: bool,
}

impl Note {
    pub fn new(id: u64) -> Self {
        let now = now_ms();
        Self {
            id,
            title: String::new(),
            body: String::new(),
            created_ms: now,
            modified_ms: now,
            pinned: false,
        }
    }
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Which text field is the user currently dragging the mouse in to
/// drive a text selection? None when no drag is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragField {
    Filter,
    Title,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteFocus {
    /// No editor field focused — typing goes to the filter input.
    Filter,
    Title,
    Body,
}

#[derive(Debug, Clone, Copy)]
pub enum Hit {
    None,
    Filter,
    NewBtn,
    ListRow(usize),
    PinAction,
    DeleteAction,
    ExportAction,
    TitleInput,
    BodyEditor,
}

pub struct NotesState {
    pub open: bool,
    pub notes: Vec<Note>,
    pub selected_id: Option<u64>,
    pub filter: crate::search::input::Input,
    pub title: crate::search::input::Input,
    pub body: editor::Editor,
    pub focus: NoteFocus,
    pub list_scroll: f32,
    pub body_scroll: f32,
    pub hover_idx: Option<usize>,
    pub confirm_delete: bool,
    /// Brief on-export flash text in the editor area.
    pub flash_text: Option<(String, std::time::Instant)>,
    pub next_id: u64,
    /// Result slot for an async export → file-picker run. The render
    /// loop drains it into `flash_text` on each tick.
    pub export_result: Arc<Mutex<Option<Result<std::path::PathBuf, String>>>>,
    /// Active mouse-drag selection: which field, set on mousedown,
    /// cleared on mouseup or focus change.
    pub drag_field: Option<DragField>,
}

impl Default for NotesState {
    fn default() -> Self {
        Self {
            open: false,
            notes: Vec::new(),
            selected_id: None,
            filter: crate::search::input::Input::new(),
            title: crate::search::input::Input::new(),
            body: editor::Editor::new(),
            focus: NoteFocus::Filter,
            list_scroll: 0.0,
            body_scroll: 0.0,
            hover_idx: None,
            confirm_delete: false,
            flash_text: None,
            next_id: 1,
            export_result: Arc::new(Mutex::new(None)),
            drag_field: None,
        }
    }
}

impl NotesState {
    /// Load all notes from disk into memory. Called on CC startup.
    pub fn load_from_disk(&mut self) {
        let notes = store::load_all();
        self.next_id = notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        self.notes = notes;
        // Pinned first, then by modified desc.
        self.sort_notes();
        // Auto-select the first note if any so the editor isn't empty.
        if self.selected_id.is_none() {
            if let Some(first) = self.notes.first() {
                self.select(first.id);
            }
        }
    }

    fn sort_notes(&mut self) {
        self.notes.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.modified_ms.cmp(&a.modified_ms))
        });
    }

    pub fn filtered(&self) -> bool {
        !self.filter.is_empty()
    }

    /// Indices into `notes` matching the current filter, pinned-first.
    pub fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.query().to_lowercase();
        let needle = needle.trim();
        self.notes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                if needle.is_empty() {
                    return true;
                }
                n.title.to_lowercase().contains(needle)
                    || n.body.to_lowercase().contains(needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected_id?;
        self.notes.iter().position(|n| n.id == id)
    }

    /// Move editor state into the note with the given id.
    pub fn select(&mut self, id: u64) {
        // Flush in-progress edits to whatever was previously selected.
        self.flush_edits_to_selected();
        let Some(idx) = self.notes.iter().position(|n| n.id == id) else {
            return;
        };
        self.selected_id = Some(id);
        self.title.set_text(&self.notes[idx].title);
        self.body.set_text(&self.notes[idx].body);
        self.body_scroll = 0.0;
        self.focus = NoteFocus::Body;
    }

    /// Pull the current title/body inputs back into the selected note in
    /// memory + write the file.
    pub fn flush_edits_to_selected(&mut self) {
        let Some(idx) = self.selected_index() else {
            return;
        };
        let new_title = self.title.query().to_string();
        let new_body = self.body.text();
        let note = &mut self.notes[idx];
        let mut changed = false;
        if note.title != new_title {
            note.title = new_title;
            changed = true;
        }
        if note.body != new_body {
            note.body = new_body;
            changed = true;
        }
        if changed {
            note.modified_ms = now_ms();
            let _ = store::save_one(note);
        }
    }

    pub fn new_note(&mut self) {
        self.flush_edits_to_selected();
        let id = self.next_id;
        self.next_id += 1;
        let note = Note::new(id);
        let _ = store::save_one(&note);
        self.notes.insert(0, note);
        self.selected_id = Some(id);
        self.title.clear();
        self.body.set_text("");
        self.body_scroll = 0.0;
        self.focus = NoteFocus::Title;
        self.sort_notes();
    }

    pub fn delete_selected(&mut self) {
        let Some(id) = self.selected_id else { return };
        if let Some(idx) = self.notes.iter().position(|n| n.id == id) {
            let removed = self.notes.remove(idx);
            let _ = store::delete_one(removed.id);
        }
        self.selected_id = self.notes.first().map(|n| n.id);
        if let Some(id) = self.selected_id {
            self.select(id);
        } else {
            self.title.clear();
            self.body.set_text("");
        }
    }

    pub fn toggle_pin_selected(&mut self) {
        let Some(idx) = self.selected_index() else {
            return;
        };
        self.notes[idx].pinned = !self.notes[idx].pinned;
        self.notes[idx].modified_ms = now_ms();
        let _ = store::save_one(&self.notes[idx]);
        let id = self.notes[idx].id;
        self.sort_notes();
        self.selected_id = Some(id);
    }

    /// Spawn `lntrn-file-manager --pick-save` in a background thread and
    /// write the selected note to whichever path the user picks. Result
    /// is shuttled back via `export_result`; `poll_export` drains it
    /// into `flash_text` on the next render tick.
    pub fn export_selected(&mut self) {
        let Some(idx) = self.selected_index() else {
            return;
        };
        let note = self.notes[idx].clone();
        let slot = Arc::clone(&self.export_result);
        std::thread::Builder::new()
            .name("notes-export".into())
            .spawn(move || {
                let result = export::run_picker_and_export(&note);
                if let Ok(mut g) = slot.lock() {
                    *g = Some(result);
                }
            })
            .ok();
        self.flash_text = Some((
            "Pick a destination…".to_string(),
            std::time::Instant::now(),
        ));
    }

    /// Drain the export result slot into `flash_text`. Cheap; safe to
    /// call every render tick.
    pub fn poll_export(&mut self) {
        let result = match self.export_result.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        if let Some(r) = result {
            self.flash_text = Some(match r {
                Ok(path) => (
                    format!("Exported to {}", path.display()),
                    std::time::Instant::now(),
                ),
                Err(e) => (
                    format!("Export: {}", e),
                    std::time::Instant::now(),
                ),
            });
        }
    }
}

// ── Layout (logical px) ─────────────────────────────────────────────────────

pub const TOOLBAR_H: f32 = 56.0;
pub const NEW_BTN_W: f32 = 92.0;
pub const LIST_WIDTH_FRAC: f32 = 0.36;
pub const ROW_H: f32 = 72.0;
pub const ROW_GAP: f32 = 4.0;
pub const PAD: f32 = 16.0;
pub const SECTION_GAP: f32 = 10.0;
pub const TITLE_FIELD_H: f32 = 44.0;
pub const ACTION_ROW_H: f32 = 44.0;

pub fn toolbar_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = PAD * scale;
    Rect::new(
        panel.x + pad,
        top_y + pad * 0.5,
        panel.w - pad * 2.0,
        TOOLBAR_H * scale,
    )
}

pub fn filter_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let tb = toolbar_rect(panel, top_y, scale);
    let new_w = NEW_BTN_W * scale;
    let gap = 12.0 * scale;
    Rect::new(tb.x, tb.y, tb.w - new_w - gap, tb.h)
}

pub fn new_btn_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let tb = toolbar_rect(panel, top_y, scale);
    let new_w = NEW_BTN_W * scale;
    Rect::new(tb.x + tb.w - new_w, tb.y, new_w, tb.h)
}

/// Width of the left column (which holds the action row and the notes list).
pub fn left_col_width(panel: Rect, scale: f32) -> f32 {
    let pad = PAD * scale;
    let w = (panel.w - pad * 3.0) * LIST_WIDTH_FRAC;
    w.max(220.0 * scale)
}

/// Action row sits at the top of the left column, just below the
/// toolbar. Holds Pin / Export / Delete.
pub fn action_row_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = PAD * scale;
    let tb = toolbar_rect(panel, top_y, scale);
    let y = tb.y + tb.h + SECTION_GAP * scale;
    let w = left_col_width(panel, scale);
    Rect::new(panel.x + pad, y, w, ACTION_ROW_H * scale)
}

pub fn list_rect(panel: Rect, top_y: f32, scale: f32, panel_bottom: f32) -> Rect {
    let pad = PAD * scale;
    let actions = action_row_rect(panel, top_y, scale);
    let top = actions.y + actions.h + SECTION_GAP * scale;
    let w = left_col_width(panel, scale);
    let h = (panel_bottom - top - pad * 0.5).max(0.0);
    Rect::new(panel.x + pad, top, w, h)
}

pub fn editor_rect(panel: Rect, top_y: f32, scale: f32, panel_bottom: f32) -> Rect {
    let pad = PAD * scale;
    let tb = toolbar_rect(panel, top_y, scale);
    let top = tb.y + tb.h + SECTION_GAP * scale;
    let x = panel.x + pad + left_col_width(panel, scale) + pad;
    let w = (panel.x + panel.w - pad - x).max(0.0);
    let h = (panel_bottom - top - pad * 0.5).max(0.0);
    Rect::new(x, top, w, h)
}

pub fn title_field_rect(editor: Rect, scale: f32) -> Rect {
    Rect::new(editor.x, editor.y, editor.w, TITLE_FIELD_H * scale)
}

pub fn body_rect(editor: Rect, scale: f32) -> Rect {
    let title = title_field_rect(editor, scale);
    let gap = 10.0 * scale;
    let y = title.y + title.h + gap;
    let h = (editor.y + editor.h - y).max(0.0);
    Rect::new(editor.x, y, editor.w, h)
}

/// Left-side action buttons (pin / export / delete), packed left to
/// right inside the action row.
pub fn action_buttons(panel: Rect, top_y: f32, scale: f32) -> (Rect, Rect, Rect) {
    let row = action_row_rect(panel, top_y, scale);
    let gap = 8.0 * scale;
    let btn_w = ((row.w - gap * 2.0) / 3.0).max(60.0 * scale);
    let pin = Rect::new(row.x, row.y, btn_w, row.h);
    let export = Rect::new(row.x + btn_w + gap, row.y, btn_w, row.h);
    let delete = Rect::new(row.x + (btn_w + gap) * 2.0, row.y, btn_w, row.h);
    (pin, export, delete)
}

pub fn list_row_rect(list: Rect, scale: f32, visible_idx: usize) -> Rect {
    let row_h = ROW_H * scale;
    let gap = ROW_GAP * scale;
    let y = list.y + visible_idx as f32 * (row_h + gap);
    Rect::new(list.x, y, list.w, row_h)
}

/// Visible content height inside the body editor's rounded plate (i.e.
/// body rect minus internal padding).
pub fn body_inner_height(body: Rect, scale: f32) -> f32 {
    (body.h - 24.0 * scale).max(0.0)
}

/// Line height (logical px) used by the body editor at the given text
/// size. Mirrors the constants used in `render::draw_body_text` — note
/// the body uses a ~12% bump over the global text size for readability.
pub fn body_line_height(text_size: f32, scale: f32) -> f32 {
    let font = (text_size * scale * 1.12).max(16.0);
    font * 1.25
}

pub fn body_max_scroll(line_count: usize, body: Rect, scale: f32, text_size: f32) -> f32 {
    let lh = body_line_height(text_size, scale);
    let total = (line_count as f32) * lh;
    let visible = body_inner_height(body, scale);
    (total - visible).max(0.0)
}

/// Clamp the body scroll so the caret line stays visible. Returns the
/// new scroll value (in logical px).
pub fn body_scroll_for_caret(
    state: &NotesState,
    body: Rect,
    scale: f32,
    text_size: f32,
) -> f32 {
    let lh = body_line_height(text_size, scale);
    let caret_line = state.body.cursor_line() as f32;
    let scroll_top_line = state.body_scroll / lh;
    let visible_lines = body_inner_height(body, scale) / lh;
    let new_top_line = if caret_line < scroll_top_line {
        caret_line
    } else if caret_line >= scroll_top_line + visible_lines - 1.0 {
        (caret_line - visible_lines + 1.0).max(0.0)
    } else {
        scroll_top_line
    };
    new_top_line * lh
}

pub fn list_max_scroll(visible_count: usize, list_h: f32, scale: f32) -> f32 {
    if visible_count == 0 {
        return 0.0;
    }
    let row_h = ROW_H * scale;
    let gap = ROW_GAP * scale;
    let total =
        visible_count as f32 * row_h + (visible_count.saturating_sub(1)) as f32 * gap;
    (total - list_h).max(0.0)
}

pub fn hit_test(
    state: &NotesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    panel_bottom: f32,
    px: f32,
    py: f32,
) -> Hit {
    if point_in(filter_rect(panel, top_y, scale), px, py) {
        return Hit::Filter;
    }
    if point_in(new_btn_rect(panel, top_y, scale), px, py) {
        return Hit::NewBtn;
    }
    // Action row buttons (above the list).
    let (pin, export, delete) = action_buttons(panel, top_y, scale);
    if point_in(pin, px, py) {
        return Hit::PinAction;
    }
    if point_in(export, px, py) {
        return Hit::ExportAction;
    }
    if point_in(delete, px, py) {
        return Hit::DeleteAction;
    }

    let list = list_rect(panel, top_y, scale, panel_bottom);
    if point_in(list, px, py) {
        let visible = state.visible_indices();
        let scroll_px = state.list_scroll * scale;
        let row_h = ROW_H * scale;
        let stride = row_h + ROW_GAP * scale;
        let rel_y = py - list.y + scroll_px;
        let idx = (rel_y / stride).floor() as i32;
        if idx >= 0 && (idx as usize) < visible.len() {
            return Hit::ListRow(idx as usize);
        }
    }
    let editor = editor_rect(panel, top_y, scale, panel_bottom);
    if point_in(title_field_rect(editor, scale), px, py) {
        return Hit::TitleInput;
    }
    if point_in(body_rect(editor, scale), px, py) {
        return Hit::BodyEditor;
    }
    Hit::None
}

fn point_in(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

/// Logical horizontal pad from the filter bar's left edge to where the
/// query text actually starts (i.e. past the magnifier glyph).
pub fn filter_text_left_pad(bar: Rect, scale: f32) -> f32 {
    let glyph_pad = 14.0 * scale;
    let glyph_r = (bar.h * 0.18).min(10.0 * scale);
    glyph_pad + glyph_r * 2.0 + 14.0 * scale
}

// ── Mouse → byte offset hit-testing ────────────────────────────────────────

/// Layout-aware byte offset for a mouse position over the body editor.
pub fn body_byte_at(
    body_rect_inner: Rect,
    buf: &str,
    body_scroll: f32,
    text_size: f32,
    scale: f32,
    px: f32,
    py: f32,
    text: &mut TextRenderer,
) -> usize {
    let font = (text_size * scale).max(15.0);
    let line_h = body_line_height(text_size, scale);
    let line_count = buf.split('\n').count().max(1);
    let rel_y = py - body_rect_inner.y + body_scroll;
    let line_idx = ((rel_y / line_h).floor() as i64)
        .clamp(0, (line_count as i64) - 1) as usize;
    // Compute byte offset of line start.
    let mut line_start = 0usize;
    for _ in 0..line_idx {
        if let Some(off) = buf[line_start..].find('\n') {
            line_start += off + 1;
        } else {
            line_start = buf.len();
            break;
        }
    }
    let line_end = buf[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(buf.len());
    let line = &buf[line_start..line_end];
    let target_x = (px - body_rect_inner.x).max(0.0);
    line_start + byte_at_x_in_line(line, target_x, font, text)
}

/// Walk through chars in `line` measuring widths and return the byte
/// offset whose visual gap is closest to `target_x`.
pub fn byte_at_x_in_line(line: &str, target_x: f32, font: f32, text: &mut TextRenderer) -> usize {
    let mut last_w = 0.0f32;
    for (byte_idx, ch) in line.char_indices() {
        let next_byte = byte_idx + ch.len_utf8();
        let w = text.measure_width(&line[..next_byte], font);
        let center = (last_w + w) * 0.5;
        if target_x < center {
            return byte_idx;
        }
        last_w = w;
    }
    line.len()
}

/// Single-line byte offset for inputs with a left padding and font.
pub fn input_byte_at(
    field_rect: Rect,
    text_left_pad: f32,
    buf: &str,
    font: f32,
    px: f32,
    text: &mut TextRenderer,
) -> usize {
    let target_x = (px - field_rect.x - text_left_pad).max(0.0);
    byte_at_x_in_line(buf, target_x, font, text)
}
