use lntrn_render::{Painter, Rect, TextRenderer};

use super::input::InteractionState;
use super::palette::FoxPalette;

/// Vertical scroll area — a clip container with offset.
///
/// Wraps `Painter::push_clip` / `pop_clip` and `TextRenderer::push_clip` / `pop_clip`
/// so both shapes and text are clipped to the viewport.
/// Usage:
/// ```ignore
/// let area = ScrollArea::new(visible_rect, content_height, &mut scroll_offset);
/// area.begin(painter, text);
/// // draw content at (x, visible_rect.y - offset) ...
/// area.end(painter, text);
/// ```
pub struct ScrollArea {
    pub viewport: Rect,
    pub content_height: f32,
    pub offset: f32,
}

impl ScrollArea {
    pub fn new(viewport: Rect, content_height: f32, offset: &mut f32) -> Self {
        // Clamp offset to valid range
        let max_offset = (content_height - viewport.h).max(0.0);
        *offset = offset.clamp(0.0, max_offset);
        Self {
            viewport,
            content_height,
            offset: *offset,
        }
    }

    /// Maximum scroll offset.
    pub fn max_offset(&self) -> f32 {
        (self.content_height - self.viewport.h).max(0.0)
    }

    /// Whether the content overflows the viewport.
    pub fn is_scrollable(&self) -> bool {
        self.content_height > self.viewport.h
    }

    /// The Y position at which to start drawing content (accounts for scroll offset).
    pub fn content_y(&self) -> f32 {
        self.viewport.y - self.offset
    }

    /// Push the clip rect onto the painter and text renderer.
    pub fn begin(&self, painter: &mut Painter, text: &mut TextRenderer) {
        painter.push_clip(self.viewport);
        let v = self.viewport;
        text.push_clip([v.x, v.y, v.w, v.h]);
    }

    /// Pop the clip rect from both painter and text renderer.
    pub fn end(&self, painter: &mut Painter, text: &mut TextRenderer) {
        painter.pop_clip();
        text.pop_clip();
    }

    /// Apply scroll-wheel delta (positive = scroll down).
    pub fn apply_scroll(offset: &mut f32, delta: f32, content_height: f32, viewport_h: f32) {
        let max = (content_height - viewport_h).max(0.0);
        *offset = (*offset + delta).clamp(0.0, max);
    }
}

/// Animated wheel scrolling — deltas move a target and the rendered offset
/// eases toward it each frame, instead of the rigid 1:1 jump per event.
///
/// Matches the file-manager scroll feel: exponential ease with
/// `k = 1 - exp(-dt * 12)`, snapping once within half a pixel. Callers keep
/// scheduling ~16ms frames while [`SmoothScroll::tick`] returns true.
#[derive(Default)]
pub struct SmoothScroll {
    /// The rendered offset — pass `&mut .offset` to [`ScrollArea::new`].
    pub offset: f32,
    target: Option<f32>,
}

impl SmoothScroll {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a wheel delta (positive = down), clamped to the content bounds.
    pub fn scroll_by(&mut self, delta: f32, content_height: f32, viewport_h: f32) {
        let max = (content_height - viewport_h).max(0.0);
        let base = self.target.unwrap_or(self.offset);
        self.target = Some((base + delta).clamp(0.0, max));
    }

    /// Jump straight to `offset` and cancel any in-flight animation
    /// (view resets, scrollbar drags).
    pub fn set(&mut self, offset: f32) {
        self.offset = offset;
        self.target = None;
    }

    /// Clamp both the offset and any pending target to the content bounds.
    /// Call from draw once real sizes are known, so a shrinking list can't
    /// leave the animation chasing an out-of-range target forever.
    pub fn clamp_to(&mut self, content_height: f32, viewport_h: f32) {
        let max = (content_height - viewport_h).max(0.0);
        self.offset = self.offset.clamp(0.0, max);
        if let Some(t) = self.target {
            self.target = Some(t.clamp(0.0, max));
        }
    }

    /// Advance the ease toward the target. Returns true while still moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        let Some(target) = self.target else {
            return false;
        };
        let k = 1.0 - (-dt * 12.0).exp();
        self.offset += (target - self.offset) * k;
        if (target - self.offset).abs() < 0.5 {
            self.offset = target;
            self.target = None;
            return false;
        }
        true
    }

    pub fn is_animating(&self) -> bool {
        self.target.is_some()
    }
}

const MIN_THUMB_H: f32 = 30.0;
const SCROLLBAR_W: f32 = 14.0;
const SCROLLBAR_PAD: f32 = 4.0;

/// Vertical scrollbar with track + thumb rendering and hit-test math.
pub struct Scrollbar {
    pub track: Rect,
    pub thumb: Rect,
    pub visible_ratio: f32,
}

impl Scrollbar {
    /// Compute track and thumb geometry from a viewport, content height, and current offset.
    pub fn new(viewport: &Rect, content_height: f32, offset: f32) -> Self {
        let track = Rect::new(
            viewport.x + viewport.w - SCROLLBAR_W - SCROLLBAR_PAD,
            viewport.y + SCROLLBAR_PAD,
            SCROLLBAR_W,
            viewport.h - SCROLLBAR_PAD * 2.0,
        );

        let visible_ratio = if content_height > 0.0 {
            (viewport.h / content_height).min(1.0)
        } else {
            1.0
        };

        let thumb_h = (track.h * visible_ratio).max(MIN_THUMB_H).min(track.h);
        let max_offset = (content_height - viewport.h).max(0.0);
        let scroll_fraction = if max_offset > 0.0 {
            offset / max_offset
        } else {
            0.0
        };
        let thumb_travel = track.h - thumb_h;
        let thumb_y = track.y + thumb_travel * scroll_fraction;

        Self {
            track,
            thumb: Rect::new(track.x, thumb_y, track.w, thumb_h),
            visible_ratio,
        }
    }

    /// Map a drag Y position on the track to a scroll offset.
    pub fn offset_for_thumb_y(
        &self,
        thumb_center_y: f32,
        content_height: f32,
        viewport_h: f32,
    ) -> f32 {
        let thumb_travel = self.track.h - self.thumb.h;
        if thumb_travel <= 0.0 {
            return 0.0;
        }
        let fraction =
            ((thumb_center_y - self.thumb.h * 0.5 - self.track.y) / thumb_travel).clamp(0.0, 1.0);
        let max_offset = (content_height - viewport_h).max(0.0);
        fraction * max_offset
    }

    /// Width of the hover zone (wider than the visible bar for easy targeting).
    pub fn hover_zone(&self) -> Rect {
        let pad = 10.0;
        Rect::new(
            self.track.x - pad,
            self.track.y,
            self.track.w + pad * 2.0,
            self.track.h,
        )
    }

    /// Draw the scrollbar track and thumb.
    /// Hidden when Idle — only appears on hover or drag.
    pub fn draw(&self, painter: &mut Painter, state: InteractionState, palette: &FoxPalette) {
        if self.visible_ratio >= 1.0 {
            return; // No scrollbar needed
        }

        // Hidden when idle — auto-hide until hover
        if matches!(state, InteractionState::Idle) {
            return;
        }

        // Track
        painter.rect_filled(self.track, SCROLLBAR_W * 0.5, palette.bg.with_alpha(0.3));

        // Thumb
        let thumb_color = match state {
            InteractionState::Pressed | InteractionState::Dragging => palette.accent,
            InteractionState::Hovered => palette.text_secondary.with_alpha(0.7),
            InteractionState::Idle => unreachable!(),
        };
        painter.rect_filled(self.thumb, SCROLLBAR_W * 0.5, thumb_color);
    }
}
