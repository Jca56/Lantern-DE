//! Quick Notes tab — multiple named notes with create/delete, auto-save.

use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::input::InteractionState;
use lntrn_ui::gpu::scroll::{ScrollArea, Scrollbar};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

fn notes_dir() -> std::path::PathBuf { crate::bar_config_dir().join("notes") }
const NOTE_FONT: f32 = 18.0;
const TITLE_FONT: f32 = 20.0;
const LIST_ITEM_H: f32 = 36.0;
const HEADER_H: f32 = 40.0;

pub(crate) const ZONE_NOTE_BASE: u32 = 0xBE_0000;
pub(crate) const ZONE_NOTE_NEW: u32 = 0xBE_0F00;
pub(crate) const ZONE_NOTE_BACK: u32 = 0xBE_0F01;
pub(crate) const ZONE_NOTE_DEL: u32 = 0xBE_0F02;
pub(crate) const ZONE_NOTE_NAME: u32 = 0xBE_0F03;

pub struct Notes {
    entries: Vec<NoteEntry>,
    /// Which note is being edited (index), or None for list view
    editing: Option<usize>,
    /// Whether we're editing the note's name
    editing_name: bool,
    pub scroll_offset: f32,
    loaded: bool,
}

struct NoteEntry {
    name: String,
    content: String,
    path: PathBuf,
    dirty: bool,
}

impl Notes {
    pub fn new() -> Self {
        Self { entries: Vec::new(), editing: None, editing_name: false, scroll_offset: 0.0, loaded: false }
    }

    pub fn load(&mut self) {
        if self.loaded { return; }
        self.loaded = true;
        let dir = &notes_dir();
        let _ = std::fs::create_dir_all(dir);
        self.entries.clear();

        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut files: Vec<_> = rd.flatten()
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "txt"))
                .collect();
            files.sort_by_key(|e| e.file_name());
            for entry in files {
                let path = entry.path();
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("untitled").to_string();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                self.entries.push(NoteEntry { name, content, path, dirty: false });
            }
        }
        if self.entries.is_empty() {
            self.create_note("Quick Note");
        }
    }

    fn create_note(&mut self, name: &str) {
        let dir = &notes_dir();
        let _ = std::fs::create_dir_all(dir);
        let mut fname = format!("{name}.txt");
        let mut i = 1;
        while dir.join(&fname).exists() {
            fname = format!("{name} {i}.txt");
            i += 1;
        }
        let path = dir.join(&fname);
        let _ = std::fs::write(&path, "");
        let entry_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
        self.entries.push(NoteEntry { name: entry_name, content: String::new(), path, dirty: false });
    }

    fn delete_note(&mut self, idx: usize) {
        if idx < self.entries.len() {
            let entry = self.entries.remove(idx);
            let _ = std::fs::remove_file(&entry.path);
        }
    }

    fn save_note(&mut self, idx: usize) {
        if let Some(entry) = self.entries.get_mut(idx) {
            if entry.dirty {
                let _ = std::fs::write(&entry.path, &entry.content);
                entry.dirty = false;
            }
        }
    }

    fn rename_note(&mut self, idx: usize) {
        if let Some(entry) = self.entries.get_mut(idx) {
            let fallback = notes_dir();
            let dir = entry.path.parent().unwrap_or(&fallback);
            let new_path = dir.join(format!("{}.txt", entry.name));
            if new_path != entry.path {
                let _ = std::fs::rename(&entry.path, &new_path);
                entry.path = new_path;
            }
        }
    }

    pub fn save_all(&mut self) {
        for i in 0..self.entries.len() { self.save_note(i); }
    }

    pub fn on_key(&mut self, key: u32, shift: bool) -> bool {
        // Name editing mode
        if self.editing_name {
            let Some(idx) = self.editing else { return false };
            let Some(entry) = self.entries.get_mut(idx) else { return false };
            match key {
                1 | 28 => { // Esc or Enter — finish name edit
                    self.editing_name = false;
                    self.rename_note(idx);
                    true
                }
                14 => { entry.name.pop(); true } // Backspace
                _ => {
                    if let Some(ch) = super::keycode_to_char(key, shift) {
                        entry.name.push(ch);
                        true
                    } else { false }
                }
            }
        } else {
            // Content editing mode
            let Some(idx) = self.editing else { return false };
            let Some(entry) = self.entries.get_mut(idx) else { return false };
            match key {
                1 => { self.save_note(idx); self.editing = None; self.scroll_offset = 0.0; true }
                14 => { entry.content.pop(); entry.dirty = true; true }
                28 => { entry.content.push('\n'); entry.dirty = true; true }
                _ => {
                    if let Some(ch) = super::keycode_to_char(key, shift) {
                        entry.content.push(ch); entry.dirty = true; true
                    } else { false }
                }
            }
        }
    }

    pub fn wants_keyboard(&self) -> bool { self.editing.is_some() }

    /// Handle left click — all click actions go here (not in draw).
    pub fn on_left_click(&mut self, ix: &InteractionContext, phys_x: f32, phys_y: f32) {
        if let Some(zone) = ix.zone_at(phys_x, phys_y) {
            if self.editing.is_some() {
                // Editor view
                match zone {
                    ZONE_NOTE_BACK => {
                        if let Some(idx) = self.editing {
                            if self.editing_name { self.rename_note(idx); }
                            self.save_note(idx);
                        }
                        self.editing = None;
                        self.editing_name = false;
                        self.scroll_offset = 0.0;
                    }
                    ZONE_NOTE_DEL => {
                        if let Some(idx) = self.editing {
                            self.editing = None;
                            self.editing_name = false;
                            self.scroll_offset = 0.0;
                            self.delete_note(idx);
                        }
                    }
                    ZONE_NOTE_NAME => {
                        self.editing_name = !self.editing_name;
                        if !self.editing_name {
                            if let Some(idx) = self.editing { self.rename_note(idx); }
                        }
                    }
                    _ => {
                        // Click in editor area — stop name editing if active
                        if self.editing_name {
                            self.editing_name = false;
                            if let Some(idx) = self.editing { self.rename_note(idx); }
                        }
                    }
                }
            } else {
                // List view
                match zone {
                    ZONE_NOTE_NEW => { self.create_note("Note"); }
                    z if z >= ZONE_NOTE_BASE && z < ZONE_NOTE_BASE + 0x100 => {
                        let idx = (z - ZONE_NOTE_BASE) as usize;
                        if idx < self.entries.len() {
                            self.editing = Some(idx);
                            self.editing_name = false;
                            self.scroll_offset = 0.0;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ── Drawing ──────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        area: Rect, scale: f32, _screen_w: u32, _screen_h: u32,
    ) {
        self.load();
        if let Some(idx) = self.editing {
            self.draw_editor(painter, text, ix, palette, area, scale, idx);
        } else {
            self.draw_list(painter, text, ix, palette, area, scale);
        }
    }

    fn draw_list(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette, area: Rect, scale: f32,
    ) {
        let pad = 16.0 * scale;
        let tf = TITLE_FONT * scale;
        let nf = NOTE_FONT * scale;
        let item_h = LIST_ITEM_H * scale;
        let header_h = HEADER_H * scale;
        let gr = Rect::new(area.x + pad, area.y + pad, area.w - pad * 2.0, area.h - pad * 2.0);
        let clip = [gr.x, gr.y, gr.w, gr.h];

        // Header
        text.queue_clipped("Notes", tf, gr.x, gr.y + (header_h - tf) * 0.5, palette.text, gr.w, clip);

        // "+ New" button
        let new_label = "+ New";
        let new_w = text.measure_width(new_label, tf) + pad * 2.0;
        let new_rect = Rect::new(gr.x + gr.w - new_w, gr.y, new_w, header_h);
        let new_state = ix.add_zone(ZONE_NOTE_NEW, new_rect);
        if new_state.is_hovered() {
            painter.rect_filled(new_rect, 0.0, palette.surface_2);
        }
        let new_color = if new_state.is_hovered() { palette.accent } else { palette.text_secondary };
        text.queue_clipped(new_label, tf, new_rect.x + pad, gr.y + (header_h - tf) * 0.5, new_color, new_w, clip);

        // Scrollable note list
        let list_y = gr.y + header_h + 8.0 * scale;
        let list_h = gr.h - header_h - 8.0 * scale;
        let content_h = self.entries.len() as f32 * item_h;
        let list_rect = Rect::new(gr.x, list_y, gr.w, list_h);
        // Note: scroll_offset is &mut but we're &self here — use 0.0 for now in list view
        // (list is short enough usually). For proper scroll we'd need &mut self.
        let mut scroll_off = 0.0f32;
        let scroll = ScrollArea::new(list_rect, content_h, &mut scroll_off);
        scroll.begin(painter, text);
        let list_clip = [list_rect.x, list_rect.y, list_rect.w, list_rect.h];

        for (i, entry) in self.entries.iter().enumerate() {
            let iy = scroll.content_y() + i as f32 * item_h;
            if iy + item_h < list_y || iy > list_y + list_h { continue; }

            let item_rect = Rect::new(gr.x, iy, gr.w, item_h);
            let zone_id = ZONE_NOTE_BASE + i as u32;
            let state = ix.add_zone(zone_id, item_rect);

            if state.is_hovered() {
                painter.rect_filled(item_rect, 0.0, palette.surface_2);
            }

            // Note name
            text.queue_clipped(&entry.name, nf, gr.x + 8.0 * scale, iy + (item_h - nf) * 0.5, palette.text, gr.w * 0.6, list_clip);

            // Preview
            let preview = entry.content.lines().next().unwrap_or("(empty)");
            let pstr = if preview.len() > 30 { format!("{}...", &preview[..28]) } else { preview.to_string() };
            let pw = text.measure_width(&pstr, nf * 0.85);
            text.queue_clipped(&pstr, nf * 0.85,
                gr.x + gr.w - pw - 8.0 * scale, iy + (item_h - nf * 0.85) * 0.5,
                palette.muted, pw + 4.0, list_clip);

            // Separator
            painter.rect_filled(Rect::new(gr.x, iy + item_h - 1.0, gr.w, 1.0), 0.0, palette.muted.with_alpha(0.15));
        }

        scroll.end(painter, text);
        if scroll.is_scrollable() {
            let sb = Scrollbar::new(&list_rect, content_h, scroll_off);
            sb.draw(painter, InteractionState::Idle, palette);
        }
    }

    fn draw_editor(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette, area: Rect, scale: f32, idx: usize,
    ) {
        let pad = 16.0 * scale;
        let tf = TITLE_FONT * scale;
        let nf = NOTE_FONT * scale;
        let header_h = HEADER_H * scale;
        let line_h = nf * 1.4;
        let gr = Rect::new(area.x + pad, area.y + pad, area.w - pad * 2.0, area.h - pad * 2.0);
        let clip = [gr.x, gr.y, gr.w, gr.h];
        let Some(entry) = self.entries.get(idx) else { return };

        // Back button
        let back_label = "< Back";
        let back_w = text.measure_width(back_label, tf) + pad;
        let back_rect = Rect::new(gr.x, gr.y, back_w, header_h);
        let back_state = ix.add_zone(ZONE_NOTE_BACK, back_rect);
        if back_state.is_hovered() {
            painter.rect_filled(back_rect, 0.0, palette.surface_2);
        }
        let back_color = if back_state.is_hovered() { palette.text } else { palette.text_secondary };
        text.queue_clipped(back_label, tf, gr.x + 4.0 * scale, gr.y + (header_h - tf) * 0.5, back_color, back_w, clip);

        // Note name (clickable to edit)
        let name_display = if self.editing_name {
            format!("{}|", entry.name) // show cursor
        } else {
            entry.name.clone()
        };
        let name_w = text.measure_width(&name_display, tf).max(80.0 * scale);
        let name_x = gr.x + (gr.w - name_w) * 0.5;
        let name_rect = Rect::new(name_x - 8.0 * scale, gr.y, name_w + 16.0 * scale, header_h);
        let name_state = ix.add_zone(ZONE_NOTE_NAME, name_rect);
        if self.editing_name {
            painter.rect_filled(name_rect, 0.0, palette.surface_2);
            painter.rect_stroke(name_rect, 0.0, 1.0 * scale, palette.accent);
        } else if name_state.is_hovered() {
            painter.rect_filled(name_rect, 0.0, palette.surface_2);
        }
        let name_color = if self.editing_name { palette.accent } else if name_state.is_hovered() { palette.text } else { palette.text };
        text.queue_clipped(&name_display, tf, name_x, gr.y + (header_h - tf) * 0.5, name_color, name_w + 4.0, clip);

        // Delete button
        let del_label = "Delete";
        let del_w = text.measure_width(del_label, tf) + pad;
        let del_rect = Rect::new(gr.x + gr.w - del_w, gr.y, del_w, header_h);
        let del_state = ix.add_zone(ZONE_NOTE_DEL, del_rect);
        if del_state.is_hovered() {
            painter.rect_filled(del_rect, 0.0, Color::from_rgb8(239, 68, 68).with_alpha(0.15));
        }
        let del_color = if del_state.is_hovered() { palette.danger } else { Color::from_rgb8(239, 68, 68).with_alpha(0.5) };
        text.queue_clipped(del_label, tf, del_rect.x + 4.0 * scale, gr.y + (header_h - tf) * 0.5, del_color, del_w, clip);

        // Editor area
        let editor_y = gr.y + header_h + 8.0 * scale;
        let editor_h = gr.h - header_h - 8.0 * scale;
        let lines: Vec<&str> = entry.content.split('\n').collect();
        let content_h = (lines.len() as f32 + 1.0) * line_h;
        let editor_rect = Rect::new(gr.x, editor_y, gr.w, editor_h);

        let mut scroll_off = 0.0f32;
        let scroll = ScrollArea::new(editor_rect, content_h, &mut scroll_off);
        scroll.begin(painter, text);
        let editor_clip = [editor_rect.x, editor_rect.y, editor_rect.w, editor_rect.h];

        for (i, line) in lines.iter().enumerate() {
            let ly = scroll.content_y() + i as f32 * line_h;
            if ly + line_h < editor_y || ly > editor_y + editor_h { continue; }
            text.queue_clipped(line, nf, gr.x + 4.0 * scale, ly, palette.text, gr.w - 8.0 * scale, editor_clip);
        }

        // Cursor
        if !self.editing_name {
            let cursor_line = lines.len().saturating_sub(1);
            let cursor_x = gr.x + 4.0 * scale + text.measure_width(lines.last().unwrap_or(&""), nf);
            let cursor_y = scroll.content_y() + cursor_line as f32 * line_h;
            painter.rect_filled(Rect::new(cursor_x, cursor_y, 2.0 * scale, nf), 0.0, palette.accent);
        }

        scroll.end(painter, text);
        if scroll.is_scrollable() {
            let sb = Scrollbar::new(&editor_rect, content_h, scroll_off);
            sb.draw(painter, InteractionState::Idle, palette);
        }
    }
}
