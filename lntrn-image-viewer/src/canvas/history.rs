//! Undo/redo history for the canvas document.
//!
//! Snapshot-based: every undoable step stores the full `items` list. An item
//! is a path plus five floats, so even hundreds of steps cost next to nothing
//! and there's no per-command inverse logic to get wrong. Drag gestures
//! (move/resize) record one step per gesture via `begin_gesture`/`end_gesture`
//! so a 200-event mouse drag is a single undo.

use super::doc::CanvasItem;

const MAX_STEPS: usize = 200;

#[derive(Clone)]
pub struct Snapshot {
    pub items: Vec<CanvasItem>,
    pub selected: Option<usize>,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Snapshot taken when a drag gesture started; committed on release only
    /// if the gesture actually changed something.
    gesture: Option<Snapshot>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the state *before* a discrete edit. Any new edit invalidates
    /// the redo stack.
    pub fn push(&mut self, before: Snapshot) {
        self.gesture = None;
        self.undo.push(before);
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn begin_gesture(&mut self, before: Snapshot) {
        self.gesture = Some(before);
    }

    /// Finish a gesture: records the pre-gesture snapshot if `after` differs
    /// from it (a click without movement leaves no history entry).
    pub fn end_gesture(&mut self, after: &[CanvasItem]) {
        if let Some(before) = self.gesture.take() {
            if before.items != after {
                self.push(before);
            }
        }
    }

    pub fn cancel_gesture(&mut self) {
        self.gesture = None;
    }

    /// Pop the previous state, stashing `current` for redo.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let snap = self.undo.pop()?;
        self.redo.push(current);
        Some(snap)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let snap = self.redo.pop()?;
        self.undo.push(current);
        Some(snap)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
