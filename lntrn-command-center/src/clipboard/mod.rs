//! Clipboard History overlay page.
//!
//! Talks to the compositor over `/run/user/{uid}/lntrn-clipboard.sock`
//! (see `lntrn-compositor::clipboard_ipc`). Caches the entry list locally
//! and re-fetches on open / after every mutation.

pub mod ipc;
pub mod render;

use lntrn_render::Rect;

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: u64,
    /// "text" or "image".
    pub kind: String,
    pub preview: Option<String>,
    pub timestamp_ms: u128,
    pub pinned: bool,
    /// Absolute path to the cached PNG for image entries.
    pub image_path: Option<String>,
}

impl Entry {
    pub fn is_image(&self) -> bool {
        self.kind == "image"
    }
}

pub struct ClipboardState {
    pub open: bool,
    pub entries: Vec<Entry>,
    pub filter: crate::search::input::Input,
    pub scroll: f32,
    pub hover_idx: Option<usize>,
    /// Recently-clicked entry id + when, for the gold flash highlight.
    pub recent_copy: Option<(u64, std::time::Instant)>,
    /// Modal flag for the "Clear All" confirm.
    pub confirm_clear: bool,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self {
            open: false,
            entries: Vec::new(),
            filter: crate::search::input::Input::new(),
            scroll: 0.0,
            hover_idx: None,
            recent_copy: None,
            confirm_clear: false,
        }
    }
}

impl ClipboardState {
    pub fn filtered(&self) -> bool {
        !self.filter.is_empty()
    }

    /// Pinned-first ordering of filtered entry indices.
    pub fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.query().to_lowercase();
        let needle = needle.trim();
        let mut pinned: Vec<usize> = Vec::new();
        let mut other: Vec<usize> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if !needle.is_empty() {
                let hay = e
                    .preview
                    .as_deref()
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if !hay.contains(needle) {
                    continue;
                }
            }
            if e.pinned {
                pinned.push(i);
            } else {
                other.push(i);
            }
        }
        pinned.extend(other);
        pinned
    }

    pub fn reset_scroll(&mut self) {
        self.scroll = 0.0;
    }
}

// ── Layout (logical px) ─────────────────────────────────────────────────────

pub const FILTER_BAR_H: f32 = 56.0;
pub const CLEAR_BTN_W: f32 = 110.0;
/// Text-entry row height (logical px).
pub const TEXT_ROW_H: f32 = 76.0;
/// Image-entry row height (logical px) — taller so the image actually
/// reads as a preview, not a postage stamp.
pub const IMAGE_ROW_H: f32 = 180.0;
pub const ROW_GAP: f32 = 6.0;
pub const PAD: f32 = 16.0;
pub const SECTION_GAP: f32 = 8.0;

/// Height for a single entry in logical px.
pub fn entry_row_height(entry: &Entry, scale: f32) -> f32 {
    if entry.is_image() {
        IMAGE_ROW_H * scale
    } else {
        TEXT_ROW_H * scale
    }
}

pub fn filter_bar_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = PAD * scale;
    let clear_w = CLEAR_BTN_W * scale;
    let gap = 12.0 * scale;
    Rect::new(
        panel.x + pad,
        top_y + pad * 0.5,
        panel.w - pad * 2.0 - clear_w - gap,
        FILTER_BAR_H * scale,
    )
}

pub fn clear_btn_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = PAD * scale;
    let bar = filter_bar_rect(panel, top_y, scale);
    let gap = 12.0 * scale;
    Rect::new(
        bar.x + bar.w + gap,
        bar.y,
        CLEAR_BTN_W * scale,
        FILTER_BAR_H * scale,
    )
    .clamp_to_right(panel.x + panel.w - pad)
}

pub fn list_rect(panel: Rect, top_y: f32, scale: f32, panel_bottom: f32) -> Rect {
    let pad = PAD * scale;
    let bar = filter_bar_rect(panel, top_y, scale);
    let top = bar.y + bar.h + SECTION_GAP * scale;
    let h = (panel_bottom - top - pad * 0.5).max(0.0);
    Rect::new(panel.x + pad, top, panel.w - pad * 2.0, h)
}

/// Y of the visible_idx-th row's top edge (before scroll offset).
/// Walks the visible list summing per-entry heights since rows are no
/// longer uniform.
pub fn row_rect_at(list: Rect, state: &ClipboardState, scale: f32, visible_idx: usize) -> Rect {
    let visible = state.visible_indices();
    let gap = ROW_GAP * scale;
    let mut y = list.y;
    for (i, &eidx) in visible.iter().enumerate() {
        let entry = &state.entries[eidx];
        let h = entry_row_height(entry, scale);
        if i == visible_idx {
            return Rect::new(list.x, y, list.w, h);
        }
        y += h + gap;
    }
    // visible_idx out of range — return a zero-h rect at the end.
    Rect::new(list.x, y, list.w, 0.0)
}

/// Pin-star and delete-X sub-rects for a TEXT row (right-aligned along the centerline).
pub fn text_row_actions(row: Rect, scale: f32) -> (Rect, Rect) {
    let btn = (row.h * 0.62).min(36.0 * scale);
    let pad = 12.0 * scale;
    let delete_x = row.x + row.w - pad - btn;
    let pin_x = delete_x - 6.0 * scale - btn;
    let y = row.y + (row.h - btn) / 2.0;
    (
        Rect::new(pin_x, y, btn, btn),
        Rect::new(delete_x, y, btn, btn),
    )
}

/// Pin-star and delete-X sub-rects for an IMAGE row (top-right corner,
/// floating over the image).
pub fn image_row_actions(row: Rect, scale: f32) -> (Rect, Rect) {
    let btn = 36.0 * scale;
    let pad = 10.0 * scale;
    let delete_x = row.x + row.w - pad - btn;
    let pin_x = delete_x - 8.0 * scale - btn;
    let y = row.y + pad;
    (
        Rect::new(pin_x, y, btn, btn),
        Rect::new(delete_x, y, btn, btn),
    )
}

/// Picks the right per-row actions helper based on the entry kind.
pub fn row_actions_for(entry: &Entry, row: Rect, scale: f32) -> (Rect, Rect) {
    if entry.is_image() {
        image_row_actions(row, scale)
    } else {
        text_row_actions(row, scale)
    }
}

pub fn max_scroll(state: &ClipboardState, list_h: f32, scale: f32) -> f32 {
    let visible = state.visible_indices();
    if visible.is_empty() {
        return 0.0;
    }
    let gap = ROW_GAP * scale;
    let mut total = 0.0;
    for (i, &eidx) in visible.iter().enumerate() {
        let entry = &state.entries[eidx];
        total += entry_row_height(entry, scale);
        if i + 1 < visible.len() {
            total += gap;
        }
    }
    (total - list_h).max(0.0)
}

#[derive(Debug, Clone, Copy)]
pub enum Hit {
    None,
    Filter,
    ClearAll,
    Row(usize),
    PinStar(usize),
    Delete(usize),
}

pub fn hit_test(
    state: &ClipboardState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    panel_bottom: f32,
    px: f32,
    py: f32,
) -> Hit {
    if point_in(filter_bar_rect(panel, top_y, scale), px, py) {
        return Hit::Filter;
    }
    if point_in(clear_btn_rect(panel, top_y, scale), px, py) {
        return Hit::ClearAll;
    }
    let list = list_rect(panel, top_y, scale, panel_bottom);
    if point_in(list, px, py) {
        let visible = state.visible_indices();
        let scroll_px = state.scroll * scale;
        let gap = ROW_GAP * scale;
        let mut y = list.y - scroll_px;
        for (vis_idx, &eidx) in visible.iter().enumerate() {
            let entry = &state.entries[eidx];
            let h = entry_row_height(entry, scale);
            let row = Rect::new(list.x, y, list.w, h);
            if point_in(row, px, py) {
                let (pin, del) = row_actions_for(entry, row, scale);
                if point_in(pin, px, py) {
                    return Hit::PinStar(vis_idx);
                }
                if point_in(del, px, py) {
                    return Hit::Delete(vis_idx);
                }
                return Hit::Row(vis_idx);
            }
            y += h + gap;
        }
    }
    Hit::None
}

fn point_in(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

// Rect helper: clamp the right edge to a maximum x without changing height.
trait ClampToRight {
    fn clamp_to_right(self, max_right: f32) -> Self;
}

impl ClampToRight for Rect {
    fn clamp_to_right(self, max_right: f32) -> Self {
        let new_right = (self.x + self.w).min(max_right);
        let new_w = (new_right - self.x).max(0.0);
        Rect::new(self.x, self.y, new_w, self.h)
    }
}
