//! Canvas editor state: selection, drag/resize state machine, view transform,
//! and the undo/redo history.
//!
//! Coordinate spaces:
//! - *screen*: physical pixels (cursor already multiplied by fractional scale `s`)
//! - *canvas*: the document plane items live on (see `doc.rs`)
//!
//! screen = viewport_center + (canvas + pan) * zoom * s

use std::path::PathBuf;

use lntrn_render::Rect;

use super::doc::{CanvasDoc, CanvasItem};
use super::history::{History, Snapshot};
use super::snap::{guides_for, SnapGuides, SnapTargets};

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

    pub fn moves_left(self) -> bool {
        matches!(
            self,
            ResizeHandle::TopLeft | ResizeHandle::Left | ResizeHandle::BottomLeft
        )
    }

    pub fn moves_right(self) -> bool {
        matches!(
            self,
            ResizeHandle::TopRight | ResizeHandle::Right | ResizeHandle::BottomRight
        )
    }

    pub fn moves_top(self) -> bool {
        matches!(
            self,
            ResizeHandle::TopLeft | ResizeHandle::Top | ResizeHandle::TopRight
        )
    }

    pub fn moves_bottom(self) -> bool {
        matches!(
            self,
            ResizeHandle::BottomLeft | ResizeHandle::Bottom | ResizeHandle::BottomRight
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
    pub history: History,
    /// Alignment guides for the in-flight drag (cleared on release).
    pub guides: SnapGuides,
    /// Items as of the last save/load — undo/redo compares against this to
    /// decide whether the document is still dirty.
    saved_items: Vec<CanvasItem>,
}

impl CanvasEditor {
    pub fn new_empty() -> Self {
        Self::from_doc(CanvasDoc::new_empty(), None)
    }

    pub fn from_doc(doc: CanvasDoc, save_path: Option<PathBuf>) -> Self {
        let saved_items = doc.items.clone();
        Self {
            doc,
            dirty: false,
            save_path,
            selected: None,
            drag: DragMode::Idle,
            dialog: None,
            name_buf: String::new(),
            name_cursor: 0,
            history: History::new(),
            guides: SnapGuides::default(),
            saved_items,
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

    // ── History ─────────────────────────────────────────────────────────

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            items: self.doc.items.clone(),
            selected: self.selected,
        }
    }

    /// Record the current state as an undo step. Call *before* a discrete edit.
    pub fn record(&mut self) {
        let snap = self.snapshot();
        self.history.push(snap);
    }

    /// Start a drag gesture; `end_gesture` turns it into one undo step if
    /// anything moved.
    pub fn begin_gesture(&mut self) {
        let snap = self.snapshot();
        self.history.begin_gesture(snap);
    }

    pub fn end_gesture(&mut self) {
        self.history.end_gesture(&self.doc.items);
    }

    pub fn undo(&mut self) {
        let current = self.snapshot();
        if let Some(snap) = self.history.undo(current) {
            self.apply_snapshot(snap);
        }
    }

    pub fn redo(&mut self) {
        let current = self.snapshot();
        if let Some(snap) = self.history.redo(current) {
            self.apply_snapshot(snap);
        }
    }

    fn apply_snapshot(&mut self, snap: Snapshot) {
        self.doc.items = snap.items;
        self.selected = snap.selected.filter(|&i| i < self.doc.items.len());
        self.drag = DragMode::Idle;
        self.guides.clear();
        self.dirty = self.doc.items != self.saved_items;
    }

    /// The document just hit disk — this state is the new clean baseline.
    pub fn mark_saved(&mut self) {
        self.saved_items = self.doc.items.clone();
        self.dirty = false;
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
    /// Callers `record()` first so multi-file drops are one undo step.
    #[allow(clippy::too_many_arguments)]
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
        if let Some(i) = self.selected {
            if i < self.doc.items.len() {
                self.record();
                self.selected = None;
                self.doc.items.remove(i);
                self.dirty = true;
            }
        }
    }

    /// Apply a resize drag: cursor at canvas (ccx, ccy), against the original
    /// item geometry captured at press time. With `snap` (targets + threshold
    /// in canvas units) the moving edge(s) snap to other items' edges; for
    /// aspect-locked corners the snapped axis drives the scale. Returns guides
    /// for every edge that landed on a target.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_resize(
        &mut self,
        idx: usize,
        handle: ResizeHandle,
        orig: &CanvasItem,
        ccx: f32,
        ccy: f32,
        grab_cx: f32,
        grab_cy: f32,
        snap: Option<(&SnapTargets, f32)>,
    ) -> SnapGuides {
        use ResizeHandle::*;
        let Some(item) = self.doc.items.get_mut(idx) else {
            return SnapGuides::default();
        };
        let mut dx = ccx - grab_cx;
        let mut dy = ccy - grab_cy;

        // Where the moving edges would land unsnapped → nudge to a target.
        let (mut snap_dx, mut snap_dy) = (None, None);
        if let Some((targets, thr)) = snap {
            if handle.moves_left() {
                snap_dx = targets.nearest_x(&[orig.x + dx], thr);
            } else if handle.moves_right() {
                snap_dx = targets.nearest_x(&[orig.x + orig.w + dx], thr);
            }
            if handle.moves_top() {
                snap_dy = targets.nearest_y(&[orig.y + dy], thr);
            } else if handle.moves_bottom() {
                snap_dy = targets.nearest_y(&[orig.y + orig.h + dy], thr);
            }
        }

        if handle.is_corner() {
            // Aspect-preserving: a snapped axis drives; otherwise whichever
            // axis moved most. The opposite corner stays anchored.
            let kx = |dx: f32| {
                if handle.moves_right() {
                    (orig.w + dx) / orig.w
                } else {
                    (orig.w - dx) / orig.w
                }
            };
            let ky = |dy: f32| {
                if handle.moves_bottom() {
                    (orig.h + dy) / orig.h
                } else {
                    (orig.h - dy) / orig.h
                }
            };
            let mut k = match (snap_dx, snap_dy) {
                (Some(ax), Some(ay)) if ax.abs() <= ay.abs() => kx(dx + ax),
                (_, Some(ay)) => ky(dy + ay),
                (Some(ax), None) => kx(dx + ax),
                (None, None) => {
                    let (a, b) = (kx(dx), ky(dy));
                    if (a - 1.0).abs() >= (b - 1.0).abs() {
                        a
                    } else {
                        b
                    }
                }
            };
            k = k.max(MIN_ITEM / orig.w.min(orig.h));
            item.w = orig.w * k;
            item.h = orig.h * k;
            item.x = if handle.moves_right() {
                orig.x
            } else {
                orig.x + orig.w - item.w
            };
            item.y = if handle.moves_bottom() {
                orig.y
            } else {
                orig.y + orig.h - item.h
            };
        } else {
            // Edge stretch: single axis, opposite edge anchored.
            dx += snap_dx.unwrap_or(0.0);
            dy += snap_dy.unwrap_or(0.0);
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

        // Guides only for the edges that actually moved.
        let Some((targets, _)) = snap else {
            return SnapGuides::default();
        };
        let item = &self.doc.items[idx];
        let mut px = Vec::with_capacity(1);
        let mut py = Vec::with_capacity(1);
        if handle.moves_left() {
            px.push(item.x);
        } else if handle.moves_right() {
            px.push(item.x + item.w);
        }
        if handle.moves_top() {
            py.push(item.y);
        } else if handle.moves_bottom() {
            py.push(item.y + item.h);
        }
        guides_for(targets, item.x, item.y, item.w, item.h, &px, &py)
    }

    // ── Z-order ─────────────────────────────────────────────────────────

    pub fn bring_to_front(&mut self) {
        if let Some(i) = self.selected {
            if i + 1 < self.doc.items.len() {
                self.record();
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
                self.record();
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
                self.record();
                self.doc.items.swap(i, i + 1);
                self.selected = Some(i + 1);
                self.dirty = true;
            }
        }
    }

    pub fn lower(&mut self) {
        if let Some(i) = self.selected {
            if i > 0 {
                self.record();
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
