//! Canvas editor state: selection, drag/resize state machine, view transform.
//!
//! Coordinate spaces:
//! - *screen*: physical pixels (cursor already multiplied by fractional scale `s`)
//! - *canvas*: the document plane items live on (see `doc.rs`)
//!
//! screen = viewport_center + (canvas + pan) * zoom * s

use std::path::PathBuf;

use lntrn_render::Rect;

use super::doc::{CanvasDoc, CanvasItem};

/// Minimum item size in canvas units — keeps a resize from collapsing an
/// image into an unclickable sliver.
const MIN_ITEM: f32 = 24.0;
/// Resize handle square size in logical px.
pub const HANDLE: f32 = 12.0;
/// Below this on-screen size (physical px / s), edge handles hide and only
/// corners show, so handles don't overlap into mush on small items.
const EDGE_HANDLE_MIN: f32 = 60.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ResizeHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl ResizeHandle {
    pub fn is_corner(self) -> bool {
        matches!(
            self,
            ResizeHandle::TopLeft
                | ResizeHandle::TopRight
                | ResizeHandle::BottomLeft
                | ResizeHandle::BottomRight
        )
    }
}

pub enum DragMode {
    Idle,
    PanningCanvas {
        last_x: f32,
        last_y: f32,
    },
    /// `grab_dx/dy`: cursor offset from item origin in canvas units at press.
    MovingItem {
        idx: usize,
        grab_dx: f32,
        grab_dy: f32,
    },
    ResizingItem {
        idx: usize,
        handle: ResizeHandle,
        orig: CanvasItem,
        grab_cx: f32,
        grab_cy: f32,
    },
    /// Dragging a file out of the sidebar; ghost follows the cursor.
    SidebarDrag {
        path: PathBuf,
    },
}

pub enum DialogKind {
    /// Name prompt shown on first save. `quit_after`: save was triggered from
    /// the unsaved-changes flow, so quit once it lands.
    SaveName {
        quit_after: bool,
    },
    ConfirmQuit,
    ConfirmNew,
    Error(String),
}

pub struct CanvasEditor {
    pub doc: CanvasDoc,
    pub dirty: bool,
    pub save_path: Option<PathBuf>,
    pub selected: Option<usize>,
    pub drag: DragMode,
    pub dialog: Option<DialogKind>,
    pub name_buf: String,
    pub name_cursor: usize,
}

impl CanvasEditor {
    pub fn new_empty() -> Self {
        Self::from_doc(CanvasDoc::new_empty(), None)
    }

    pub fn from_doc(doc: CanvasDoc, save_path: Option<PathBuf>) -> Self {
        Self {
            doc,
            dirty: false,
            save_path,
            selected: None,
            drag: DragMode::Idle,
            dialog: None,
            name_buf: String::new(),
            name_cursor: 0,
        }
    }

    pub fn window_title(&self) -> String {
        let name = if self.doc.name.is_empty() {
            "Untitled"
        } else {
            &self.doc.name
        };
        let dot = if self.dirty { " •" } else { "" };
        format!("{name}{dot} — Lantern Canvas")
    }

    // ── Transforms ──────────────────────────────────────────────────────

    pub fn to_screen(&self, cx: f32, cy: f32, vp: &Rect, s: f32) -> (f32, f32) {
        let v = &self.doc.view;
        (
            vp.center_x() + (cx + v.pan_x) * v.zoom * s,
            vp.center_y() + (cy + v.pan_y) * v.zoom * s,
        )
    }

    pub fn to_canvas(&self, sx: f32, sy: f32, vp: &Rect, s: f32) -> (f32, f32) {
        let v = &self.doc.view;
        let zs = (v.zoom * s).max(1e-6);
        (
            (sx - vp.center_x()) / zs - v.pan_x,
            (sy - vp.center_y()) / zs - v.pan_y,
        )
    }

    pub fn item_screen_rect(&self, item: &CanvasItem, vp: &Rect, s: f32) -> Rect {
        let (x, y) = self.to_screen(item.x, item.y, vp, s);
        let zs = self.doc.view.zoom * s;
        Rect::new(x, y, item.w * zs, item.h * zs)
    }

    /// Zoom toward a screen point, keeping the canvas point under it fixed.
    pub fn zoom_at(&mut self, factor: f32, sx: f32, sy: f32, vp: &Rect, s: f32) {
        let old = self.doc.view.zoom;
        let new = (old * factor).clamp(0.01, 100.0);
        if (new - old).abs() < f32::EPSILON {
            return;
        }
        let dx = (sx - vp.center_x()) / s;
        let dy = (sy - vp.center_y()) / s;
        self.doc.view.pan_x += dx * (1.0 / new - 1.0 / old);
        self.doc.view.pan_y += dy * (1.0 / new - 1.0 / old);
        self.doc.view.zoom = new;
    }

    pub fn pan_by_screen(&mut self, dx: f32, dy: f32, s: f32) {
        let zs = (self.doc.view.zoom * s).max(1e-6);
        self.doc.view.pan_x += dx / zs;
        self.doc.view.pan_y += dy / zs;
    }

    pub fn reset_view(&mut self) {
        self.doc.view = Default::default();
    }

    // ── Hit testing ─────────────────────────────────────────────────────

    /// Topmost item under a screen point (items are bottom-to-top).
    pub fn hit_item(&self, sx: f32, sy: f32, vp: &Rect, s: f32) -> Option<usize> {
        let (cx, cy) = self.to_canvas(sx, sy, vp, s);
        self.doc.items.iter().rposition(|it| it.contains(cx, cy))
    }

    /// Handle rects (screen space) for an item. Edge handles drop out when the
    /// item is too small on screen.
    pub fn handle_rects(&self, item: &CanvasItem, vp: &Rect, s: f32) -> Vec<(ResizeHandle, Rect)> {
        use ResizeHandle::*;
        let r = self.item_screen_rect(item, vp, s);
        let hs = HANDLE * s;
        let small = r.w < EDGE_HANDLE_MIN * s || r.h < EDGE_HANDLE_MIN * s;
        let mk = |x: f32, y: f32| Rect::new(x - hs * 0.5, y - hs * 0.5, hs, hs);
        let mut out = vec![
            (TopLeft, mk(r.x, r.y)),
            (TopRight, mk(r.x + r.w, r.y)),
            (BottomLeft, mk(r.x, r.y + r.h)),
            (BottomRight, mk(r.x + r.w, r.y + r.h)),
        ];
        if !small {
            out.push((Top, mk(r.x + r.w * 0.5, r.y)));
            out.push((Bottom, mk(r.x + r.w * 0.5, r.y + r.h)));
            out.push((Left, mk(r.x, r.y + r.h * 0.5)));
            out.push((Right, mk(r.x + r.w, r.y + r.h * 0.5)));
        }
        out
    }

    /// Resize handle of the *selected* item under a screen point. Handles are
    /// checked before item bodies so a drag on one always resizes.
    pub fn hit_handle(&self, sx: f32, sy: f32, vp: &Rect, s: f32) -> Option<(usize, ResizeHandle)> {
        let idx = self.selected?;
        let item = self.doc.items.get(idx)?;
        let pad = 4.0 * s;
        for (h, r) in self.handle_rects(item, vp, s) {
            if r.expand(pad).contains(sx, sy) {
                return Some((idx, h));
            }
        }
        None
    }

    // ── Item management ─────────────────────────────────────────────────

    /// Add an image centered at a canvas point. Natural size is capped to
    /// `max_w/max_h` (canvas units) so a 6000px photo doesn't swallow the
    /// whole view on import — handles can always grow it back.
    pub fn add_item(
        &mut self,
        path: PathBuf,
        center_x: f32,
        center_y: f32,
        nat_w: f32,
        nat_h: f32,
        max_w: f32,
        max_h: f32,
    ) {
        let (nw, nh) = (nat_w.max(1.0), nat_h.max(1.0));
        let scale = (max_w / nw).min(max_h / nh).min(1.0);
        let w = (nw * scale).max(MIN_ITEM);
        let h = (nh * scale).max(MIN_ITEM);
        self.doc.items.push(CanvasItem {
            path: path.to_string_lossy().into_owned(),
            x: center_x - w * 0.5,
            y: center_y - h * 0.5,
            w,
            h,
            angle: 0.0,
        });
        self.selected = Some(self.doc.items.len() - 1);
        self.dirty = true;
    }

    pub fn delete_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            if i < self.doc.items.len() {
                self.doc.items.remove(i);
                self.dirty = true;
            }
        }
    }

    /// Apply a resize drag: cursor at canvas (ccx, ccy), against the original
    /// item geometry captured at press time.
    pub fn apply_resize(
        &mut self,
        idx: usize,
        handle: ResizeHandle,
        orig: &CanvasItem,
        ccx: f32,
        ccy: f32,
        grab_cx: f32,
        grab_cy: f32,
    ) {
        use ResizeHandle::*;
        let Some(item) = self.doc.items.get_mut(idx) else {
            return;
        };
        let dx = ccx - grab_cx;
        let dy = ccy - grab_cy;

        if handle.is_corner() {
            // Aspect-preserving: dominant axis wins, opposite corner anchored.
            let sx = match handle {
                TopRight | BottomRight => (orig.w + dx) / orig.w,
                _ => (orig.w - dx) / orig.w,
            };
            let sy = match handle {
                BottomLeft | BottomRight => (orig.h + dy) / orig.h,
                _ => (orig.h - dy) / orig.h,
            };
            let mut k = if (sx - 1.0).abs() >= (sy - 1.0).abs() {
                sx
            } else {
                sy
            };
            k = k.max(MIN_ITEM / orig.w.min(orig.h));
            item.w = orig.w * k;
            item.h = orig.h * k;
            item.x = match handle {
                TopRight | BottomRight => orig.x,
                _ => orig.x + orig.w - item.w,
            };
            item.y = match handle {
                BottomLeft | BottomRight => orig.y,
                _ => orig.y + orig.h - item.h,
            };
        } else {
            // Edge stretch: single axis, opposite edge anchored.
            match handle {
                Right => item.w = (orig.w + dx).max(MIN_ITEM),
                Left => {
                    item.w = (orig.w - dx).max(MIN_ITEM);
                    item.x = orig.x + orig.w - item.w;
                }
                Bottom => item.h = (orig.h + dy).max(MIN_ITEM),
                Top => {
                    item.h = (orig.h - dy).max(MIN_ITEM);
                    item.y = orig.y + orig.h - item.h;
                }
                _ => {}
            }
        }
        self.dirty = true;
    }

    // ── Z-order ─────────────────────────────────────────────────────────

    pub fn bring_to_front(&mut self) {
        if let Some(i) = self.selected {
            if i + 1 < self.doc.items.len() {
                let item = self.doc.items.remove(i);
                self.doc.items.push(item);
                self.selected = Some(self.doc.items.len() - 1);
                self.dirty = true;
            }
        }
    }

    pub fn send_to_back(&mut self) {
        if let Some(i) = self.selected {
            if i > 0 {
                let item = self.doc.items.remove(i);
                self.doc.items.insert(0, item);
                self.selected = Some(0);
                self.dirty = true;
            }
        }
    }

    pub fn raise(&mut self) {
        if let Some(i) = self.selected {
            if i + 1 < self.doc.items.len() {
                self.doc.items.swap(i, i + 1);
                self.selected = Some(i + 1);
                self.dirty = true;
            }
        }
    }

    pub fn lower(&mut self) {
        if let Some(i) = self.selected {
            if i > 0 {
                self.doc.items.swap(i, i - 1);
                self.selected = Some(i - 1);
                self.dirty = true;
            }
        }
    }
}

/// The canvas drawing area: window minus title bar, status bar, and sidebar.
pub fn canvas_viewport(wf: f32, hf: f32, s: f32, sidebar_phys_w: f32) -> Rect {
    let title_h = crate::TITLE_H * s;
    let status_h = crate::STATUS_H * s;
    Rect::new(
        sidebar_phys_w,
        title_h,
        (wf - sidebar_phys_w).max(1.0),
        (hf - title_h - status_h).max(1.0),
    )
}
