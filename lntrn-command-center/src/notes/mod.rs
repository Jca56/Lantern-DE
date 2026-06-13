//! Quick Notes overlay page.
//!
//! Notes are persisted as `~/.lantern/state/notes/{id}.json` — one file
//! per note, atomic write on every edit. All notes load into memory at
//! CC startup; from then on the in-memory `Vec<Note>` is authoritative
//! and disk is a write-through mirror.

pub mod editor;
pub mod export;
pub mod render;
pub mod sticky;
pub mod store;
pub mod wrap;

use std::sync::{Arc, Mutex};

use lntrn_render::Rect;
use serde::{Deserialize, Serialize};

// Mouse → byte-offset hit-testing lives next to the editor it serves;
// re-exported so call sites keep their `crate::notes::` paths.
pub use editor::{body_byte_at, input_byte_at};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub created_ms: u128,
    pub modified_ms: u128,
    #[serde(default)]
    pub pinned: bool,
    /// Floating sticky-note state — geometry in logical px, kept when
    /// unsticking so a re-stick lands where the paper used to be.
    #[serde(default)]
    pub sticky: bool,
    #[serde(default)]
    pub sticky_x: f32,
    #[serde(default)]
    pub sticky_y: f32,
    #[serde(default)]
    pub sticky_w: f32,
    #[serde(default)]
    pub sticky_h: f32,
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
            sticky: false,
            sticky_x: 0.0,
            sticky_y: 0.0,
            sticky_w: 0.0,
            sticky_h: 0.0,
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
    StickyAction,
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
        let removed_idx = self.notes.iter().position(|n| n.id == id);
        if let Some(idx) = removed_idx {
            let removed = self.notes.remove(idx);
            let _ = store::delete_one(removed.id);
        }
        // Clear the editor BEFORE selecting the next note: select() flushes
        // the editor fields into the current selection, and right now they
        // still hold the deleted note's text — flushing would overwrite the
        // next note's content with the dead note's.
        self.selected_id = None;
        self.title.clear();
        self.body.set_text("");
        // Select the note that slid into the deleted note's slot (or the
        // new last note when the deleted one was at the bottom).
        let next_id = removed_idx
            .and_then(|idx| {
                let last = self.notes.len().checked_sub(1)?;
                self.notes.get(idx.min(last))
            })
            .map(|n| n.id);
        if let Some(id) = next_id {
            self.select(id);
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

/// Left-side action buttons (pin / stick / export / delete), packed
/// left to right inside the action row.
pub fn action_buttons(panel: Rect, top_y: f32, scale: f32) -> (Rect, Rect, Rect, Rect) {
    let row = action_row_rect(panel, top_y, scale);
    let gap = 6.0 * scale;
    let btn_w = ((row.w - gap * 3.0) / 4.0).max(52.0 * scale);
    let at = |i: f32| Rect::new(row.x + (btn_w + gap) * i, row.y, btn_w, row.h);
    (at(0.0), at(1.0), at(2.0), at(3.0))
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

/// Inner text area of the body editor (plate minus padding) — the rect
/// visual lines are laid out, drawn, and hit-tested in. Render and
/// every input path must use this same rect or clicks drift.
pub fn body_inner_rect(body: Rect, scale: f32) -> Rect {
    let pad = 12.0 * scale;
    Rect::new(
        body.x + pad,
        body.y + pad,
        (body.w - pad * 2.0).max(0.0),
        body_inner_height(body, scale),
    )
}

/// Line height used by the body editor at the given text size. Built
/// on `wrap::body_font` so it can't drift from what the renderer and
/// hit-testing measure with.
pub fn body_line_height(text_size: f32, scale: f32) -> f32 {
    wrap::body_font(text_size, scale) * 1.25
}

/// `line_count` is the *visual* (soft-wrapped) line count from
/// `wrap::layout`, not the '\n' count.
pub fn body_max_scroll(line_count: usize, body: Rect, scale: f32, text_size: f32) -> f32 {
    let lh = body_line_height(text_size, scale);
    let total = (line_count as f32) * lh;
    let visible = body_inner_height(body, scale);
    (total - visible).max(0.0)
}

/// Clamp the body scroll so the caret's visual line stays visible.
/// Returns the new scroll value (physical px).
pub fn body_scroll_for_caret(
    state: &NotesState,
    body: Rect,
    scale: f32,
    text_size: f32,
    text: &mut lntrn_render::TextRenderer,
) -> f32 {
    let inner = body_inner_rect(body, scale);
    let font = wrap::body_font(text_size, scale);
    let vlines = wrap::layout(state.body.raw(), font, inner.w, text);
    let lh = body_line_height(text_size, scale);
    let caret_line = wrap::caret_vline(&vlines, state.body.cursor_byte()) as f32;
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
    let (pin, stick, export, delete) = action_buttons(panel, top_y, scale);
    if point_in(pin, px, py) {
        return Hit::PinAction;
    }
    if point_in(stick, px, py) {
        return Hit::StickyAction;
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

pub fn point_in(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

/// Confirm-delete modal geometry: `(dialog, cancel_btn, delete_btn)`.
/// Shared by the renderer and the click handler so the buttons land
/// exactly where they're drawn.
pub fn confirm_delete_rects(panel: Rect, scale: f32) -> (Rect, Rect, Rect) {
    let w = 380.0 * scale;
    let h = 180.0 * scale;
    let x = panel.x + (panel.w - w) / 2.0;
    let y = panel.y + (panel.h - h) / 2.0;
    let pad = 18.0 * scale;
    let btn_h = 38.0 * scale;
    let btn_w = (w - pad * 2.0 - 12.0 * scale) / 2.0;
    let by = y + h - pad - btn_h;
    let cancel = Rect::new(x + pad, by, btn_w, btn_h);
    let delete = Rect::new(x + pad + btn_w + 12.0 * scale, by, btn_w, btn_h);
    (Rect::new(x, y, w, h), cancel, delete)
}

/// Logical horizontal pad from the filter bar's left edge to where the
/// query text actually starts (i.e. past the magnifier glyph).
pub fn filter_text_left_pad(bar: Rect, scale: f32) -> f32 {
    let glyph_pad = 14.0 * scale;
    let glyph_r = (bar.h * 0.18).min(10.0 * scale);
    glyph_pad + glyph_r * 2.0 + 14.0 * scale
}

// Mouse → byte-offset hit-testing (`body_byte_at`, `byte_at_x_in_line`,
// `input_byte_at`) lives in `editor.rs`, re-exported at the top of this
// module.
