//! Alt+Tab window switcher — "spotlight" view.
//!
//! State machine + public API. The actual card geometry lives in
//! [`layout`] (shared with input hit-testing) and icon resolution in
//! [`icons`]. The renderer (`render/surface.rs`) consumes [`card_layouts`]
//! plus the per-card chrome helpers to draw shadows, the gold glow ring,
//! rounded live previews, app-icon badges, and the close button.

use std::collections::HashSet;
use std::time::Instant;

use smithay::{
    backend::renderer::{
        element::{
            solid::{SolidColorBuffer, SolidColorRenderElement},
            Id, Kind,
        },
        utils::CommitCounter,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Size},
};

pub mod icons;
pub mod layout;

pub use layout::{CardLayout, CORNER_RADIUS};

/// How long Alt must be held before the overlay appears.
const HOLD_THRESHOLD_MS: u128 = 220;

/// Minimum pointer travel (logical px) since the last hover-driven pick
/// before hovering may center a different card. This is what breaks the
/// "centering shifts a new card under the stationary cursor → runaway"
/// feedback loop: the strip only steps once per deliberate mouse move.
const HOVER_STEP_DISTANCE: f64 = 140.0;

/// Fade-in duration on promote-to-visible (seconds).
const DURATION_FADE: f32 = 0.22;
/// Spring settle window for the selection slide (seconds). The spring
/// overshoots, so we keep redrawing for this long after a selection change.
const SLIDE_SETTLE: f32 = 0.85;
/// Spring parameters for the slide/scale motion — bouncy & playful. 🦊
const SLIDE_DAMPING: f64 = 0.62;
const SLIDE_FREQUENCY: f64 = 5.5;

/// Backdrop dim (Fox-tinted near-black, a touch warmer than pure black).
const OVERLAY_COLOR: [f32; 4] = [0.02, 0.015, 0.03, 0.28];
/// BRAND_GOLD accent: rgb(200, 134, 10) — matches lntrn_theme::BRAND_GOLD.
pub const BRAND_GOLD: [f32; 4] = [200.0 / 255.0, 134.0 / 255.0, 10.0 / 255.0, 1.0];
/// Dim overlay drawn over non-selected / minimized cards.
const DIM_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Close-button background (translucent dark, turns red on the X glyph bg).
const CLOSE_BTN_COLOR: [f32; 4] = [0.85, 0.20, 0.20, 0.95];

/// Layout info for a single thumbnail slot, consumed by the render loop to
/// position each window's live surface preview.
pub struct ThumbnailSlot {
    pub surface: WlSurface,
    /// Top-left of the preview area in logical output coordinates.
    pub position: Point<i32, Logical>,
    /// Logical size to render the preview at (already spotlight-scaled).
    pub size: Size<i32, Logical>,
    pub selected: bool,
    pub minimized: bool,
    /// Corner radius for the rounded preview, logical px.
    pub corner_radius: f32,
}

pub struct AltTabSwitcher {
    visible: bool,
    /// Silent mode: cycling is active but overlay not yet shown.
    active: bool,
    entries: Vec<WlSurface>,
    /// app_id per entry (parallel to `entries`), for icon resolution.
    app_ids: Vec<String>,
    selected_index: usize,
    original_focus: Option<WlSurface>,
    scroll_offset: usize,
    alt_held_since: Option<Instant>,
    fade_start: Option<Instant>,
    /// Start of the slide/scale spring animation.
    slide_start: Option<Instant>,
    prev_selected_index: usize,
    minimized_surfaces: HashSet<WlSurface>,
    /// Pointer position at the last hover-driven selection change. Used to
    /// require deliberate movement before the carousel re-centers on a new
    /// card (see [`hover_select`]).
    last_hover_pos: Option<Point<f64, Logical>>,
    /// When true, opened via hot corner — Alt-release won't dismiss.
    hot_corner_mode: bool,
    // Full-screen backdrop dim (single size, reused).
    overlay_buf: SolidColorBuffer,
    // Stable Ids for the per-card solid quads (built via
    // SolidColorRenderElement::new with explicit geometry so each card
    // can be a different size — a shared SolidColorBuffer can't do that).
    dim_id: Id,
    close_id: Id,
}

impl AltTabSwitcher {
    pub fn new() -> Self {
        Self {
            visible: false,
            active: false,
            entries: Vec::new(),
            app_ids: Vec::new(),
            selected_index: 0,
            original_focus: None,
            scroll_offset: 0,
            alt_held_since: None,
            fade_start: None,
            slide_start: None,
            prev_selected_index: 0,
            minimized_surfaces: HashSet::new(),
            last_hover_pos: None,
            hot_corner_mode: false,
            overlay_buf: SolidColorBuffer::new((1, 1), OVERLAY_COLOR),
            dim_id: Id::new(),
            close_id: Id::new(),
        }
    }

    /// Build a solid-color quad at an explicit logical rect (any size),
    /// using a stable Id so damage tracking stays sane across frames.
    fn solid(
        &self,
        id: &Id,
        loc: Point<i32, Logical>,
        size: Size<i32, Logical>,
        color: [f32; 4],
        alpha: f32,
        scale: f64,
    ) -> SolidColorRenderElement {
        let geo = Rectangle::new(
            to_phys(loc.x, loc.y, scale),
            Size::<i32, Physical>::from((
                (size.w as f64 * scale).round() as i32,
                (size.h as f64 * scale).round() as i32,
            )),
        );
        let mut c = color;
        c[3] *= alpha;
        SolidColorRenderElement::new(id.clone(), geo, CommitCounter::default(), c, Kind::Unspecified)
    }

    /// Begin cycling in silent mode (no overlay yet).
    pub fn start_silent(
        &mut self,
        entries: Vec<WlSurface>,
        app_ids: Vec<String>,
        original_focus: Option<WlSurface>,
        minimized: HashSet<WlSurface>,
    ) -> Option<WlSurface> {
        if entries.is_empty() {
            self.hide();
            return None;
        }
        self.entries = entries;
        self.app_ids = app_ids;
        self.original_focus = original_focus.clone();
        self.minimized_surfaces = minimized;
        self.scroll_offset = 0;
        self.active = true;
        self.visible = false;
        self.alt_held_since = Some(Instant::now());
        self.fade_start = None;
        self.slide_start = None;
        self.last_hover_pos = None;

        // A quick tap cycles live windows only — skip minimized entries.
        // (They still appear once the overlay is held open; see `advance`.)
        let focused_idx = original_focus
            .as_ref()
            .and_then(|f| self.entries.iter().position(|s| s == f));
        self.selected_index = match focused_idx {
            Some(idx) => self.step_index(idx, false, true),
            None if !self.minimized_surfaces.contains(&self.entries[0]) => 0,
            None => self.step_index(0, false, true),
        };
        self.prev_selected_index = self.selected_index;
        self.update_scroll();
        self.selected_surface().cloned()
    }

    /// Open immediately in visible mode (hot-corner trigger).
    pub fn start_visible(
        &mut self,
        entries: Vec<WlSurface>,
        app_ids: Vec<String>,
        original_focus: Option<WlSurface>,
        minimized: HashSet<WlSurface>,
    ) {
        if entries.is_empty() {
            return;
        }
        self.entries = entries;
        self.app_ids = app_ids;
        self.original_focus = original_focus;
        self.minimized_surfaces = minimized;
        self.scroll_offset = 0;
        self.active = true;
        self.visible = true;
        self.hot_corner_mode = true;
        self.alt_held_since = None;
        self.fade_start = Some(Instant::now());
        self.slide_start = None;
        self.last_hover_pos = None;
        self.selected_index = 0;
        self.prev_selected_index = 0;
        self.update_scroll();
    }

    /// Move selection to the next entry (wraps). While the overlay is still
    /// silent (a quick tap) minimized windows are skipped; once it's held
    /// open they're cyclable so you can pick one to restore.
    pub fn advance(&mut self) -> Option<WlSurface> {
        if !self.active || self.entries.is_empty() {
            return None;
        }
        self.last_hover_pos = None;
        self.prev_selected_index = self.selected_index;
        self.selected_index = self.step_index(self.selected_index, false, !self.visible);
        if self.visible {
            self.slide_start = Some(Instant::now());
        }
        self.update_scroll();
        self.selected_surface().cloned()
    }

    /// Move selection to the previous entry (wraps) — Shift+Tab / Left.
    /// Mirrors [`advance`]'s skip-minimized-while-silent behaviour.
    pub fn retreat(&mut self) -> Option<WlSurface> {
        if !self.active || self.entries.is_empty() {
            return None;
        }
        self.last_hover_pos = None;
        self.prev_selected_index = self.selected_index;
        self.selected_index = self.step_index(self.selected_index, true, !self.visible);
        if self.visible {
            self.slide_start = Some(Instant::now());
        }
        self.update_scroll();
        self.selected_surface().cloned()
    }

    /// Step one entry from `start` in `backward`'s direction. When
    /// `skip_minimized` is set, keep stepping over minimized entries so a
    /// quick Alt+Tab tap only visits live windows. Falls back to `start`
    /// when every other entry is minimized.
    fn step_index(&self, start: usize, backward: bool, skip_minimized: bool) -> usize {
        let n = self.entries.len();
        if n == 0 {
            return 0;
        }
        let mut idx = start;
        for _ in 0..n {
            idx = if backward {
                if idx == 0 { n - 1 } else { idx - 1 }
            } else {
                (idx + 1) % n
            };
            if !skip_minimized || !self.minimized_surfaces.contains(&self.entries[idx]) {
                return idx;
            }
        }
        start
    }

    pub fn should_promote(&self) -> bool {
        self.active
            && !self.visible
            && self
                .alt_held_since
                .map_or(false, |t| t.elapsed().as_millis() >= HOLD_THRESHOLD_MS)
    }

    pub fn promote_to_visible(&mut self) {
        if !self.active || self.visible {
            return;
        }
        self.visible = true;
        self.fade_start = Some(Instant::now());
        self.slide_start = None;
        self.last_hover_pos = None;
        self.prev_selected_index = self.selected_index;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.active = false;
        self.entries.clear();
        self.app_ids.clear();
        self.selected_index = 0;
        self.prev_selected_index = 0;
        self.original_focus = None;
        self.scroll_offset = 0;
        self.alt_held_since = None;
        self.fade_start = None;
        self.slide_start = None;
        self.last_hover_pos = None;
        self.minimized_surfaces.clear();
        self.hot_corner_mode = false;
    }

    pub fn is_hot_corner_mode(&self) -> bool {
        self.hot_corner_mode
    }

    pub fn is_visible(&self) -> bool {
        self.visible && !self.entries.is_empty()
    }

    pub fn is_active(&self) -> bool {
        self.active && !self.entries.is_empty()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn selected_surface(&self) -> Option<&WlSurface> {
        self.entries.get(self.selected_index)
    }

    pub fn original_focus(&self) -> Option<&WlSurface> {
        self.original_focus.as_ref()
    }

    /// app_id for an entry index (for icon resolution).
    pub fn app_id_at(&self, index: usize) -> Option<&str> {
        self.app_ids.get(index).map(|s| s.as_str())
    }

    /// Select a specific entry by index (from hit_test).
    pub fn select(&mut self, index: usize) {
        if index >= self.entries.len() || index == self.selected_index {
            return;
        }
        self.prev_selected_index = self.selected_index;
        self.selected_index = index;
        if self.visible {
            self.slide_start = Some(Instant::now());
        }
        self.update_scroll();
    }

    /// Remove an entry (after closing its window). Adjusts selection.
    pub fn remove_entry(&mut self, index: usize) -> Option<WlSurface> {
        if index >= self.entries.len() {
            return None;
        }
        let surface = self.entries.remove(index);
        if index < self.app_ids.len() {
            self.app_ids.remove(index);
        }
        if self.entries.is_empty() {
            self.hide();
            return Some(surface);
        }
        if self.selected_index >= self.entries.len() {
            self.selected_index = 0;
        } else if self.selected_index > index {
            self.selected_index -= 1;
        }
        self.prev_selected_index = self.selected_index;
        self.update_scroll();
        Some(surface)
    }

    fn update_scroll(&mut self) {
        if self.entries.len() <= layout::MAX_VISIBLE {
            self.scroll_offset = 0;
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + layout::MAX_VISIBLE {
            self.scroll_offset = self.selected_index + 1 - layout::MAX_VISIBLE;
        }
    }

    /// Current slide progress (0..=1, springy) for the selection animation.
    fn slide_progress(&self) -> f32 {
        let Some(start) = self.slide_start else {
            return 1.0;
        };
        let elapsed = start.elapsed().as_secs_f32();
        if elapsed >= SLIDE_SETTLE {
            return 1.0;
        }
        let t = (elapsed / SLIDE_SETTLE) as f64;
        crate::easing::spring(t, SLIDE_DAMPING, SLIDE_FREQUENCY) as f32
    }

    /// Current fade-in alpha (0..=1).
    fn fade_alpha(&self) -> f32 {
        let Some(start) = self.fade_start else {
            return 1.0;
        };
        let elapsed = start.elapsed().as_secs_f32();
        let t = (elapsed / DURATION_FADE).clamp(0.0, 1.0);
        crate::easing::ease_out_cubic_f32(t)
    }

    /// The animated card layout for the current frame.
    pub fn card_layouts(&self, output_size: Size<i32, Logical>) -> Vec<CardLayout> {
        if !self.is_visible() {
            return Vec::new();
        }
        layout::compute(
            output_size,
            self.entries.len(),
            self.selected_index,
            self.prev_selected_index,
            self.scroll_offset,
            self.slide_progress(),
        )
    }

    /// Thumbnail slots for the live-preview render path. Derived from
    /// [`card_layouts`] so geometry stays in one place.
    pub fn thumbnail_slots(&self, output_size: Size<i32, Logical>) -> Vec<ThumbnailSlot> {
        self.card_layouts(output_size)
            .into_iter()
            .filter_map(|c| {
                let surface = self.entries.get(c.entry_index)?.clone();
                Some(ThumbnailSlot {
                    minimized: self.minimized_surfaces.contains(&surface),
                    surface,
                    position: c.position,
                    size: c.size,
                    selected: c.selected,
                    corner_radius: CORNER_RADIUS * c.scale,
                })
            })
            .collect()
    }

    /// Hit-test a logical point against the cards. Returns entry index.
    pub fn hit_test(
        &self,
        point: Point<f64, Logical>,
        output_size: Size<i32, Logical>,
    ) -> Option<usize> {
        if !self.is_visible() {
            return None;
        }
        let cards = self.card_layouts(output_size);
        layout::hit_test(&cards, point.x as i32, point.y as i32)
    }

    /// Pointer-hover selection: center the window under the cursor, then
    /// hold steady. Returns true if the selection changed (caller re-renders).
    ///
    /// Two things make this stop instead of running the carousel to the end:
    /// 1. We hit-test the *settled* layout (cards at their resting spots for
    ///    the current selection) rather than the mid-slide layout, so an
    ///    in-flight animation never drags a different card under the cursor.
    /// 2. We only step selection once the pointer has travelled
    ///    [`HOVER_STEP_DISTANCE`] since the last hover pick. Centering a card
    ///    shifts the strip so a *new* card lands under the (now stationary)
    ///    cursor — without this gate that alone would trigger another pick,
    ///    and another, cascading to the edge. Requiring fresh travel means
    ///    the strip moves one window per deliberate mouse move.
    pub fn hover_select(
        &mut self,
        point: Point<f64, Logical>,
        output_size: Size<i32, Logical>,
    ) -> bool {
        if !self.is_visible() {
            return false;
        }
        // Settled layout for the current selection (prev == selected makes
        // the slide a no-op, pinning cards at their resting positions).
        let cards = layout::compute(
            output_size,
            self.entries.len(),
            self.selected_index,
            self.selected_index,
            self.scroll_offset,
            1.0,
        );
        let hit = layout::hit_test(&cards, point.x as i32, point.y as i32);

        let changed = match hit {
            Some(idx) if idx != self.selected_index => {
                // Distance accumulates from the last *committed* pick, so a
                // slow drag eventually crosses the threshold and steps once.
                let moved_enough = self.last_hover_pos.map_or(false, |last| {
                    let dx = point.x - last.x;
                    let dy = point.y - last.y;
                    dx * dx + dy * dy >= HOVER_STEP_DISTANCE * HOVER_STEP_DISTANCE
                });
                if moved_enough {
                    self.select(idx);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };

        if changed || self.last_hover_pos.is_none() {
            // Re-anchor on a real pick, or seed the baseline on first hover
            // (so the overlay appearing under the cursor can't insta-jump).
            self.last_hover_pos = Some(point);
        }
        changed
    }

    /// Hit-test the close button on the selected card.
    pub fn hit_test_close(
        &self,
        point: Point<f64, Logical>,
        output_size: Size<i32, Logical>,
    ) -> Option<usize> {
        if !self.is_visible() {
            return None;
        }
        let cards = self.card_layouts(output_size);
        layout::hit_test_close(&cards, point.x as i32, point.y as i32)
    }

    /// True if any animation is still running (needs re-render).
    pub fn needs_redraw(&self) -> bool {
        if !self.is_visible() {
            return false;
        }
        if let Some(start) = self.fade_start {
            if start.elapsed().as_secs_f32() < DURATION_FADE {
                return true;
            }
        }
        if let Some(start) = self.slide_start {
            if start.elapsed().as_secs_f32() < SLIDE_SETTLE {
                return true;
            }
        }
        false
    }

    /// The full-screen backdrop dim element. Drawn behind everything else.
    /// `_output_size` is unused (the buffer is sized in `update_sizes`) but
    /// kept for call-site symmetry with the other element builders.
    pub fn backdrop_element(
        &self,
        _output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Option<SolidColorRenderElement> {
        if !self.is_visible() {
            return None;
        }
        let kind = smithay::backend::renderer::element::Kind::Unspecified;
        Some(SolidColorRenderElement::from_buffer(
            &self.overlay_buf,
            Point::<i32, Physical>::from((0, 0)),
            scale,
            self.fade_alpha(),
            kind,
        ))
    }

    /// Per-card dim overlays (non-selected + minimized), drawn above the
    /// preview. Returns (element) list in arbitrary order.
    pub fn dim_elements(
        &self,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Vec<SolidColorRenderElement> {
        if !self.is_visible() {
            return Vec::new();
        }
        let alpha = self.fade_alpha();
        let mut out = Vec::new();
        for c in self.card_layouts(output_size) {
            let surface = match self.entries.get(c.entry_index) {
                Some(s) => s,
                None => continue,
            };
            let minimized = self.minimized_surfaces.contains(surface);
            // Selected card with a live (non-minimized) window: no dim.
            let dim = if minimized { 0.55_f32.max(c.dim) } else { c.dim };
            if dim <= 0.0 {
                continue;
            }
            let mut color = DIM_COLOR;
            color[3] = dim;
            out.push(self.solid(&self.dim_id, c.position, c.size, color, alpha, scale));
        }
        out
    }

    /// Close-button background element for the selected card (top layer).
    pub fn close_button_element(
        &self,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Option<SolidColorRenderElement> {
        if !self.is_visible() {
            return None;
        }
        let alpha = self.fade_alpha();
        let card = self.card_layouts(output_size).into_iter().find(|c| c.selected)?;
        let (loc, sz) = card.close_rect();
        Some(self.solid(
            &self.close_id,
            loc,
            Size::from((sz, sz)),
            CLOSE_BTN_COLOR,
            alpha,
            scale,
        ))
    }

    pub fn current_fade_alpha(&self) -> f32 {
        self.fade_alpha()
    }

    /// Resize the reused full-screen backdrop buffer for the current frame.
    pub fn update_sizes(&mut self, output_size: Size<i32, Logical>) {
        if !self.is_visible() {
            return;
        }
        self.overlay_buf.resize((output_size.w, output_size.h));
    }
}

fn to_phys(x: i32, y: i32, scale: f64) -> Point<i32, Physical> {
    Point::from((
        (x as f64 * scale).round() as i32,
        (y as f64 * scale).round() as i32,
    ))
}
