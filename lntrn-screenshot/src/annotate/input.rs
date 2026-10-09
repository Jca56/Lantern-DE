//! The annotation side of the selection UI: the bar's buttons, dragging a
//! mark out, typing text, and undo.

use super::bar::{BarAction, BarLayout, BarState};
use super::{Mark, Shape, Tool};
use crate::selection::DragMode;
use crate::wayland::Typed;
use crate::{SelectionUi, UiMode};

impl SelectionUi {
    /// The bar by the region. It is there once a region is set and at
    /// rest: never while one is being dragged out, moved or sized, so it
    /// is not in the way of setting the region.
    pub(crate) fn bar_layout(&self, screen_w: f32, screen_h: f32, scale: f32) -> Option<BarLayout> {
        if self.mode != UiMode::Normal || !matches!(self.drag_mode, DragMode::None) {
            return None;
        }
        let region = self.selection.as_ref()?.normalized();
        let readout_h = crate::render::readout_height(scale);
        Some(BarLayout::compute(
            screen_w, screen_h, scale, region, readout_h,
        ))
    }

    pub(crate) fn bar_state(&self) -> BarState {
        BarState {
            tool: self.tool,
            color: self.color,
            size: self.size,
            can_undo: !self.marks.is_empty() || self.typing.is_some(),
        }
    }

    pub(crate) fn on_bar_action(&mut self, action: BarAction) {
        match action {
            // Clicking the tool in hand puts it down, so drags move and
            // size the region again.
            BarAction::Tool(tool) => {
                self.commit_typing();
                self.tool = if self.tool == Some(tool) {
                    None
                } else {
                    Some(tool)
                };
            }
            // A colour or a thickness is for the next mark; text being
            // typed is finished as it is.
            BarAction::Color(color) => {
                self.commit_typing();
                self.color = color;
            }
            BarAction::Size(size) => {
                self.commit_typing();
                self.size = size;
            }
            BarAction::Undo => self.undo(),
        }
    }

    /// The button went down at `at` with `tool` in hand.
    pub(crate) fn start_mark(&mut self, tool: Tool, at: (f32, f32), scale: f32) {
        self.commit_typing();
        let mark = Mark::begin(tool, at, self.color, self.size, scale);
        if tool == Tool::Text {
            self.typing = Some(mark);
        } else {
            self.drawing = Some(mark);
        }
    }

    pub(crate) fn drag_mark(&mut self, to: (f32, f32)) {
        if let Some(mark) = self.drawing.as_mut() {
            mark.drag_to(to);
        }
    }

    /// The button came up: keep the mark, unless there is nothing to it.
    pub(crate) fn finish_mark(&mut self) {
        if let Some(mark) = self.drawing.take().filter(|mark| !mark.is_empty()) {
            self.marks.push(mark);
        }
    }

    /// Put what was typed into the text being written.
    pub(crate) fn type_text(&mut self, typed: &[Typed]) {
        let Some(Mark {
            shape: Shape::Text { text, .. },
            ..
        }) = self.typing.as_mut()
        else {
            return;
        };
        for key in typed {
            match key {
                Typed::Text(s) => text.push_str(s),
                Typed::Backspace => {
                    text.pop();
                }
            }
        }
    }

    /// The text being written becomes a mark, unless it is blank.
    pub(crate) fn commit_typing(&mut self) {
        if let Some(mark) = self.typing.take().filter(|mark| !mark.is_empty()) {
            self.marks.push(mark);
        }
    }

    /// Take back the text being written, or else the newest mark.
    pub(crate) fn undo(&mut self) {
        if self.typing.take().is_none() {
            self.marks.pop();
        }
    }
}
