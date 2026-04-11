//! Reusable fade-in scrollbar. Owned per-scrollable-region; the renderer
//! draws it whenever opacity > 0 and the host (editor or sidebar) calls
//! `ping()` to keep it visible while there's activity.

use std::time::Instant;

use lntrn_render::{Color, Painter, Rect};
use lntrn_ui::gpu::InteractionContext;

use crate::editor::Editor;
use crate::sidebar::Sidebar;

/// Bar width in logical pixels (medium tier).
pub const SCROLLBAR_W: f32 = 14.0;
const MIN_THUMB_H: f32 = 32.0;
/// How long after the last activity the bar stays at full opacity.
const FADE_HOLD_MS: f32 = 600.0;
/// How long the fade-out animation takes.
const FADE_DURATION_MS: f32 = 280.0;

/// Per-scrollable-region state for a fade-in scrollbar.
pub struct ScrollbarState {
    pub opacity: f32,
    pub last_activity: Option<Instant>,
    pub dragging: bool,
    /// Offset of the click within the thumb (0.0 = top, 1.0 = bottom). Used
    /// so the thumb doesn't jump under the cursor when drag starts.
    pub drag_grip: f32,
}

impl ScrollbarState {
    pub fn new() -> Self {
        Self {
            opacity: 0.0,
            last_activity: None,
            dragging: false,
            drag_grip: 0.0,
        }
    }

    /// Mark the scrollbar as recently active so it stays visible.
    pub fn ping(&mut self) {
        self.last_activity = Some(Instant::now());
    }

    /// Advance the fade animation. Call once per frame.
    pub fn tick(&mut self, hovered: bool) {
        let target = if self.dragging || hovered {
            self.last_activity = Some(Instant::now());
            1.0
        } else if let Some(last) = self.last_activity {
            let elapsed = Instant::now().duration_since(last).as_millis() as f32;
            if elapsed < FADE_HOLD_MS {
                1.0
            } else if elapsed < FADE_HOLD_MS + FADE_DURATION_MS {
                1.0 - (elapsed - FADE_HOLD_MS) / FADE_DURATION_MS
            } else {
                0.0
            }
        } else {
            0.0
        };
        let diff = target - self.opacity;
        if diff.abs() < 0.01 {
            self.opacity = target;
        } else {
            self.opacity += diff * 0.35;
        }
    }

    /// True while the fade animation is still settling — host should keep
    /// requesting redraws.
    pub fn animating(&self) -> bool {
        if self.opacity > 0.005 && self.opacity < 0.995 {
            return true;
        }
        let Some(last) = self.last_activity else {
            return false;
        };
        let elapsed = Instant::now().duration_since(last).as_millis() as f32;
        elapsed < FADE_HOLD_MS + FADE_DURATION_MS
    }
}

/// Geometry of the scrollbar within a scrollable region.
#[derive(Clone, Copy, Debug)]
pub struct ScrollbarLayout {
    pub track: Rect,
    pub thumb: Rect,
}

/// Compute the track and thumb rects for a scrollable region. Returns `None`
/// when the content fits entirely (no scrollbar needed).
pub fn layout(
    viewport: Rect,
    content_h: f32,
    scroll: f32,
    scale: f32,
) -> Option<ScrollbarLayout> {
    if content_h <= viewport.h + 0.5 {
        return None;
    }
    let bar_w = SCROLLBAR_W * scale;
    let track = Rect::new(
        viewport.x + viewport.w - bar_w,
        viewport.y,
        bar_w,
        viewport.h,
    );
    let min_thumb = MIN_THUMB_H * scale;
    let raw_thumb_h = (viewport.h / content_h) * track.h;
    let thumb_h = raw_thumb_h.max(min_thumb).min(track.h);
    let max_scroll = (content_h - viewport.h).max(1.0);
    let scroll_t = (scroll / max_scroll).clamp(0.0, 1.0);
    let thumb_y = track.y + scroll_t * (track.h - thumb_h);
    let thumb = Rect::new(track.x, thumb_y, track.w, thumb_h);
    Some(ScrollbarLayout { track, thumb })
}

/// Draw the scrollbar and register a hit zone for the thumb. Caller must
/// have already called `state.tick(hovered)`.
pub fn draw_scrollbar(
    state: &ScrollbarState,
    painter: &mut Painter,
    input: &mut InteractionContext,
    layout: ScrollbarLayout,
    zone_id: u32,
) {
    if state.opacity < 0.02 {
        return;
    }
    // Track hit zone for click-jump (registered first so the thumb wins on
    // overlap thanks to last-zone-wins).
    input.add_zone(zone_id + 1, layout.track);

    // Faint track plate.
    let track_alpha = (state.opacity * 50.0) as u8;
    painter.rect_filled(
        layout.track,
        layout.track.w * 0.5,
        Color::from_rgba8(60, 50, 35, track_alpha),
    );
    // Thumb hit zone.
    input.add_zone(zone_id, layout.thumb);
    // Thumb body — slightly darker on drag, lighter at rest.
    let base_alpha = if state.dragging { 220.0 } else { 150.0 };
    let thumb_alpha = (state.opacity * base_alpha) as u8;
    painter.rect_filled(
        layout.thumb,
        layout.thumb.w * 0.5,
        Color::from_rgba8(70, 65, 55, thumb_alpha),
    );
}

/// Convert a vertical cursor position within the track to a scroll offset,
/// accounting for the drag grip so the thumb feels glued to the cursor.
pub fn cursor_to_scroll(
    cy: f32,
    layout: ScrollbarLayout,
    content_h: f32,
    viewport_h: f32,
    drag_grip: f32,
) -> f32 {
    let max_scroll = (content_h - viewport_h).max(1.0);
    let usable = (layout.track.h - layout.thumb.h).max(1.0);
    let thumb_top = (cy - layout.track.y - drag_grip * layout.thumb.h).clamp(0.0, usable);
    (thumb_top / usable) * max_scroll
}

/// Returns true if `(x, y)` is within the cushion zone where the scrollbar
/// should fade in even if not yet drawn (slightly larger than the bar).
pub fn near_track(viewport: Rect, x: f32, y: f32, scale: f32) -> bool {
    let bar_w = SCROLLBAR_W * scale;
    let cushion = 8.0 * scale;
    x >= viewport.x + viewport.w - bar_w - cushion
        && x <= viewport.x + viewport.w
        && y >= viewport.y
        && y <= viewport.y + viewport.h
}

/// High-level helper: tick the editor's scrollbar state and draw it. Used by
/// `render.rs` to keep the scrollbar logic in one place.
pub fn draw_editor_scrollbar(
    editor: &mut Editor,
    painter: &mut Painter,
    input: &mut InteractionContext,
    er: Rect,
    scale: f32,
    zone_id: u32,
) {
    let total_h = editor.content_height(scale);
    let cursor = input.cursor();
    let hovered = cursor
        .map(|(cx, cy)| near_track(er, cx, cy, scale))
        .unwrap_or(false);
    editor.scrollbar.tick(hovered);
    if let Some(layout) = layout(er, total_h, editor.scroll_offset, scale) {
        draw_scrollbar(&editor.scrollbar, painter, input, layout, zone_id);
    }
}

/// High-level helper: tick the sidebar's scrollbar state and draw it.
pub fn draw_sidebar_scrollbar(
    sidebar: &mut Sidebar,
    painter: &mut Painter,
    input: &mut InteractionContext,
    viewport: Rect,
    scale: f32,
    zone_id: u32,
) {
    let total_h = sidebar.content_height(scale);
    let cursor = input.cursor();
    let hovered = cursor
        .map(|(cx, cy)| near_track(viewport, cx, cy, scale))
        .unwrap_or(false);
    sidebar.scrollbar.tick(hovered);
    if let Some(layout) = layout(viewport, total_h, sidebar.scroll, scale) {
        draw_scrollbar(&sidebar.scrollbar, painter, input, layout, zone_id);
    }
}
