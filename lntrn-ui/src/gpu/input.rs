use lntrn_render::Rect;

/// Current interaction state for a hit zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionState {
    Idle,
    Hovered,
    Pressed,
    Dragging,
}

impl InteractionState {
    pub fn is_hovered(self) -> bool {
        matches!(self, Self::Hovered | Self::Pressed | Self::Dragging)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Pressed | Self::Dragging)
    }
}

/// Named interactive region with current state.
#[derive(Clone, Debug)]
pub struct HitZone {
    pub id: u32,
    pub rect: Rect,
    pub state: InteractionState,
}

/// Manages hit-testing and interaction state for a set of rectangular zones.
///
/// Replaces ad-hoc per-widget hover/pressed/drag booleans with a single
/// unified model. Zones are registered each frame with `add_zone` and then
/// driven by cursor/button events.
pub struct InteractionContext {
    zones: Vec<HitZone>,
    cursor: Option<(f32, f32)>,
    /// Zone that owns the active press/drag capture (if any).
    active_zone: Option<u32>,
    scroll_delta: f32,
}

impl InteractionContext {
    pub fn new() -> Self {
        Self {
            zones: Vec::with_capacity(32),
            cursor: None,
            active_zone: None,
            scroll_delta: 0.0,
        }
    }

    /// Clear zones for a new frame. Call at the start of each render cycle.
    /// Active capture is preserved across frames until released.
    pub fn begin_frame(&mut self) {
        self.zones.clear();
        self.scroll_delta = 0.0;
    }

    /// Register a hit zone. Returns the current `InteractionState` for it.
    pub fn add_zone(&mut self, id: u32, rect: Rect) -> InteractionState {
        let state = self.compute_state(id, &rect);
        self.zones.push(HitZone {
            id,
            rect,
            state,
        });
        state
    }

    /// Get the state of a previously-added zone by id.
    pub fn zone_state(&self, id: u32) -> InteractionState {
        self.zones
            .iter()
            .find(|z| z.id == id)
            .map_or(InteractionState::Idle, |z| z.state)
    }

    /// Current cursor position (if known).
    pub fn cursor(&self) -> Option<(f32, f32)> {
        self.cursor
    }

    /// Accumulated scroll delta this frame (positive = down/forward).
    pub fn scroll_delta(&self) -> f32 {
        self.scroll_delta
    }

    /// The id of the zone that currently owns capture (press/drag), if any.
    pub fn active_zone_id(&self) -> Option<u32> {
        self.active_zone
    }

    // ── Event drivers ────────────────────────────────────────────────
    // Call these from your event loop.

    pub fn on_cursor_moved(&mut self, x: f32, y: f32) {
        self.cursor = Some((x, y));
    }

    pub fn on_cursor_left(&mut self) {
        self.cursor = None;
    }

    /// Returns the id of the zone that was pressed (topmost hit), if any.
    pub fn on_left_pressed(&mut self) -> Option<u32> {
        let (x, y) = self.cursor?;
        // Last-added zone wins (painter's order = front-to-back is reversed
        // but we add back-to-front during rendering, so last = topmost).
        let hit = self.zones.iter().rev().find(|z| z.rect.contains(x, y))?;
        let id = hit.id;
        self.active_zone = Some(id);
        Some(id)
    }

    pub fn on_left_released(&mut self) {
        self.active_zone = None;
    }

    pub fn on_scroll(&mut self, delta: f32) {
        self.scroll_delta += delta;
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Find the zone (from previous frame) that contains the given point.
    pub fn zone_at(&self, x: f32, y: f32) -> Option<u32> {
        self.zones
            .iter()
            .rev()
            .find(|z| z.rect.contains(x, y))
            .map(|z| z.id)
    }

    /// Check if cursor is inside `rect` right now.
    pub fn is_hovered(&self, rect: &Rect) -> bool {
        self.cursor
            .map_or(false, |(x, y)| rect.contains(x, y))
    }

    /// Compute a linear drag value (0.0–1.0) along a horizontal track.
    pub fn drag_fraction_x(&self, track: &Rect) -> Option<f32> {
        let (x, _) = self.cursor?;
        Some(((x - track.x) / track.w.max(1.0)).clamp(0.0, 1.0))
    }

    /// Compute a linear drag value (0.0–1.0) along a vertical track.
    pub fn drag_fraction_y(&self, track: &Rect) -> Option<f32> {
        let (_, y) = self.cursor?;
        Some(((y - track.y) / track.h.max(1.0)).clamp(0.0, 1.0))
    }

    fn compute_state(&self, id: u32, rect: &Rect) -> InteractionState {
        // If this zone owns capture, it's either Pressed or Dragging.
        if self.active_zone == Some(id) {
            let inside = self
                .cursor
                .map_or(false, |(x, y)| rect.contains(x, y));
            return if inside {
                InteractionState::Pressed
            } else {
                InteractionState::Dragging
            };
        }

        // No capture — check hover (only if nothing else is captured).
        if self.active_zone.is_none() {
            if self
                .cursor
                .map_or(false, |(x, y)| rect.contains(x, y))
            {
                return InteractionState::Hovered;
            }
        }

        InteractionState::Idle
    }
}

impl Default for InteractionContext {
    fn default() -> Self {
        Self::new()
    }
}
