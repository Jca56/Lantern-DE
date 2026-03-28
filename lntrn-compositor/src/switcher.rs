use std::collections::HashSet;
use std::time::Instant;

use smithay::{
    backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Size},
};

// Animation helpers (inlined from lntrn_ui::animation to avoid adding a dep)
mod anim {
    pub const DURATION_ENTER: f32 = 0.25;
    pub const DURATION_FAST: f32 = 0.1;

    pub fn progress(elapsed: f32, duration: f32) -> f32 {
        if duration <= 0.0 { return 1.0; }
        (elapsed / duration).clamp(0.0, 1.0)
    }

    pub fn ease_out(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        1.0 - inv * inv * inv
    }

    pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }
}

/// Layout info for a single thumbnail slot, used by the render loop to
/// position each window's actual surface elements.
pub struct ThumbnailSlot {
    /// Which window surface occupies this slot.
    pub surface: WlSurface,
    /// Top-left of the thumbnail area in logical coordinates (relative to output).
    pub position: Point<i32, Logical>,
    /// Logical size the thumbnail should be rendered at.
    pub size: Size<i32, Logical>,
    /// Whether this slot is the currently selected one.
    pub selected: bool,
    /// Whether this window is minimized.
    pub minimized: bool,
}

/// Thumbnail layout constants (logical pixels).
const THUMB_W: i32 = 400;
const THUMB_H: i32 = 280;
const THUMB_GAP: i32 = 20;
const MAX_VISIBLE: usize = 5;
const PANEL_PADDING: i32 = 28;
const HIGHLIGHT_BORDER: i32 = 3;

/// Colors.
const OVERLAY_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const PANEL_COLOR: [f32; 4] = [0.08, 0.08, 0.10, 0.88];
const CARD_COLOR: [f32; 4] = [0.18, 0.18, 0.20, 0.92];
// BRAND_GOLD accent: rgb(200, 134, 10) — matches lntrn_theme::BRAND_GOLD
const HIGHLIGHT_COLOR: [f32; 4] = [200.0 / 255.0, 134.0 / 255.0, 10.0 / 255.0, 1.0];
/// Minimized window dim overlay (semi-transparent dark).
const MINIMIZED_DIM_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.50];
/// Close button background (red-ish).
const CLOSE_BTN_COLOR: [f32; 4] = [0.85, 0.20, 0.20, 0.95];
/// Close button size (logical pixels, square).
const CLOSE_BTN_SIZE: i32 = 28;
/// Close button margin from card top-right corner.
const CLOSE_BTN_MARGIN: i32 = 6;

/// How long Alt must be held before the overlay appears.
const HOLD_THRESHOLD_MS: u128 = 500;

pub struct AltTabSwitcher {
    visible: bool,
    /// Silent mode: cycling is active but overlay not yet shown.
    active: bool,
    entries: Vec<WlSurface>,
    selected_index: usize,
    original_focus: Option<WlSurface>,
    /// Scroll offset when there are more entries than MAX_VISIBLE.
    scroll_offset: usize,
    /// When Alt was first pressed (for hold detection).
    alt_held_since: Option<Instant>,
    /// When the overlay became visible (for fade-in animation).
    fade_start: Option<Instant>,
    /// For highlight slide animation.
    highlight_anim_start: Option<Instant>,
    prev_selected_index: usize,
    /// Set of minimized surfaces (updated each time switcher starts).
    minimized_surfaces: HashSet<WlSurface>,
    /// When true, opened via hot corner — Alt-release won't dismiss.
    hot_corner_mode: bool,
    // Solid color buffers for overlay elements
    overlay_buf: SolidColorBuffer,
    panel_buf: SolidColorBuffer,
    card_buf: SolidColorBuffer,
    highlight_buf: SolidColorBuffer,
    minimized_dim_buf: SolidColorBuffer,
    close_btn_buf: SolidColorBuffer,
}

impl AltTabSwitcher {
    pub fn new() -> Self {
        Self {
            visible: false,
            active: false,
            entries: Vec::new(),
            selected_index: 0,
            original_focus: None,
            scroll_offset: 0,
            alt_held_since: None,
            fade_start: None,
            highlight_anim_start: None,
            prev_selected_index: 0,
            minimized_surfaces: HashSet::new(),
            hot_corner_mode: false,
            overlay_buf: SolidColorBuffer::new((1, 1), OVERLAY_COLOR),
            panel_buf: SolidColorBuffer::new((1, 1), PANEL_COLOR),
            card_buf: SolidColorBuffer::new((1, 1), CARD_COLOR),
            highlight_buf: SolidColorBuffer::new((1, 1), HIGHLIGHT_COLOR),
            minimized_dim_buf: SolidColorBuffer::new((1, 1), MINIMIZED_DIM_COLOR),
            close_btn_buf: SolidColorBuffer::new((1, 1), CLOSE_BTN_COLOR),
        }
    }

    /// Begin cycling in silent mode (no overlay). Records the time Alt was
    /// pressed so we can promote to visible after the hold threshold.
    pub fn start_silent(
        &mut self,
        entries: Vec<WlSurface>,
        original_focus: Option<WlSurface>,
        minimized: HashSet<WlSurface>,
    ) -> Option<WlSurface> {
        if entries.is_empty() {
            self.hide();
            return None;
        }

        self.entries = entries;
        self.original_focus = original_focus.clone();
        self.minimized_surfaces = minimized;
        self.scroll_offset = 0;
        self.active = true;
        self.visible = false;
        self.alt_held_since = Some(Instant::now());
        self.fade_start = None;
        self.highlight_anim_start = None;

        // Find the focused window and select the NEXT one in spawn order
        let focused_idx = original_focus.as_ref().and_then(|f| {
            self.entries.iter().position(|s| s == f)
        });
        self.selected_index = match focused_idx {
            Some(idx) => (idx + 1) % self.entries.len(),
            None => 0,
        };
        self.prev_selected_index = self.selected_index;

        self.update_scroll();
        self.selected_surface().cloned()
    }

    /// Open the switcher immediately in visible mode (hot corner trigger).
    /// Unlike `start_silent`, skips the hold-threshold and shows the overlay
    /// right away. Alt-release will NOT dismiss the switcher — only a click
    /// or ESC will.
    pub fn start_visible(
        &mut self,
        entries: Vec<WlSurface>,
        original_focus: Option<WlSurface>,
        minimized: HashSet<WlSurface>,
    ) {
        if entries.is_empty() {
            return;
        }

        self.entries = entries;
        self.original_focus = original_focus;
        self.minimized_surfaces = minimized;
        self.scroll_offset = 0;
        self.active = true;
        self.visible = true;
        self.hot_corner_mode = true;
        self.alt_held_since = None;
        self.fade_start = Some(Instant::now());
        self.highlight_anim_start = None;
        self.selected_index = 0;
        self.prev_selected_index = 0;
        self.update_scroll();
    }

    /// Move selection to the next entry. Wraps around. Works in both silent
    /// and visible modes.
    pub fn advance(&mut self) -> Option<WlSurface> {
        if !self.active || self.entries.is_empty() {
            return None;
        }

        self.prev_selected_index = self.selected_index;
        self.selected_index = (self.selected_index + 1) % self.entries.len();
        if self.visible {
            self.highlight_anim_start = Some(Instant::now());
        }
        self.update_scroll();
        self.selected_surface().cloned()
    }

    /// Check if Alt has been held long enough to show the overlay.
    pub fn should_promote(&self) -> bool {
        self.active
            && !self.visible
            && self
                .alt_held_since
                .map_or(false, |t| t.elapsed().as_millis() >= HOLD_THRESHOLD_MS)
    }

    /// Promote from silent cycling to visible overlay with fade-in.
    pub fn promote_to_visible(&mut self) {
        if !self.active || self.visible {
            return;
        }
        self.visible = true;
        self.fade_start = Some(Instant::now());
        self.highlight_anim_start = None;
        self.prev_selected_index = self.selected_index;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.active = false;
        self.entries.clear();
        self.selected_index = 0;
        self.prev_selected_index = 0;
        self.original_focus = None;
        self.scroll_offset = 0;
        self.alt_held_since = None;
        self.fade_start = None;
        self.highlight_anim_start = None;
        self.minimized_surfaces.clear();
        self.hot_corner_mode = false;
    }

    /// Returns true when opened via hot corner (Alt-release won't dismiss).
    pub fn is_hot_corner_mode(&self) -> bool {
        self.hot_corner_mode
    }

    /// Returns true when the overlay is visible (not just silently cycling).
    pub fn is_visible(&self) -> bool {
        self.visible && !self.entries.is_empty()
    }

    /// Returns true when cycling is active (silent or visible).
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

    /// Hit-test a logical point against thumbnail slots. Returns the entry
    /// index if the point falls within a card. Used for mouse hover/click.
    pub fn hit_test(&self, point: Point<f64, Logical>, output_size: Size<i32, Logical>) -> Option<usize> {
        if !self.is_visible() {
            return None;
        }
        let visible_count = self.entries.len().min(MAX_VISIBLE);
        let strip_w = visible_count as i32 * THUMB_W
            + (visible_count as i32 - 1).max(0) * THUMB_GAP;
        let panel_w = strip_w + 2 * PANEL_PADDING;
        let panel_h = THUMB_H + 2 * PANEL_PADDING;
        let panel_x = ((output_size.w - panel_w) / 2).max(0);
        let panel_y = ((output_size.h - panel_h) / 2).max(0);
        let start_x = panel_x + PANEL_PADDING;
        let start_y = panel_y + PANEL_PADDING;

        let px = point.x as i32;
        let py = point.y as i32;

        for i in 0..visible_count {
            let x = start_x + i as i32 * (THUMB_W + THUMB_GAP);
            if px >= x && px < x + THUMB_W && py >= start_y && py < start_y + THUMB_H {
                return Some(self.scroll_offset + i);
            }
        }
        None
    }

    /// Select a specific entry by index (from hit_test). Triggers highlight
    /// animation if the selection changed.
    pub fn select(&mut self, index: usize) {
        if index >= self.entries.len() || index == self.selected_index {
            return;
        }
        self.prev_selected_index = self.selected_index;
        self.selected_index = index;
        if self.visible {
            self.highlight_anim_start = Some(Instant::now());
        }
        self.update_scroll();
    }

    /// Hit-test the close button on the selected thumbnail. Returns the entry
    /// index if the point falls within the close button area.
    pub fn hit_test_close(&self, point: Point<f64, Logical>, output_size: Size<i32, Logical>) -> Option<usize> {
        if !self.is_visible() {
            return None;
        }
        let visible_count = self.entries.len().min(MAX_VISIBLE);
        let strip_w = visible_count as i32 * THUMB_W
            + (visible_count as i32 - 1).max(0) * THUMB_GAP;
        let panel_w = strip_w + 2 * PANEL_PADDING;
        let panel_h = THUMB_H + 2 * PANEL_PADDING;
        let panel_x = ((output_size.w - panel_w) / 2).max(0);
        let panel_y = ((output_size.h - panel_h) / 2).max(0);
        let start_x = panel_x + PANEL_PADDING;
        let start_y = panel_y + PANEL_PADDING;

        let px = point.x as i32;
        let py = point.y as i32;

        // Only check the selected slot
        let sel_vis = self.selected_index.checked_sub(self.scroll_offset)?;
        if sel_vis >= visible_count {
            return None;
        }
        let card_x = start_x + sel_vis as i32 * (THUMB_W + THUMB_GAP);
        let btn_x = card_x + THUMB_W - CLOSE_BTN_SIZE - CLOSE_BTN_MARGIN;
        let btn_y = start_y + CLOSE_BTN_MARGIN;

        if px >= btn_x && px < btn_x + CLOSE_BTN_SIZE
            && py >= btn_y && py < btn_y + CLOSE_BTN_SIZE
        {
            Some(self.selected_index)
        } else {
            None
        }
    }

    /// Remove an entry from the switcher (after closing a window). Returns the
    /// removed surface. Adjusts selected_index if needed.
    pub fn remove_entry(&mut self, index: usize) -> Option<WlSurface> {
        if index >= self.entries.len() {
            return None;
        }
        let surface = self.entries.remove(index);
        if self.entries.is_empty() {
            self.hide();
            return Some(surface);
        }
        // Adjust selection
        if self.selected_index >= self.entries.len() {
            self.selected_index = 0;
        } else if self.selected_index > index {
            self.selected_index -= 1;
        }
        self.prev_selected_index = self.selected_index;
        self.update_scroll();
        Some(surface)
    }

    /// Keep the selected item visible by adjusting scroll_offset.
    fn update_scroll(&mut self) {
        if self.entries.len() <= MAX_VISIBLE {
            self.scroll_offset = 0;
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + MAX_VISIBLE {
            self.scroll_offset = self.selected_index + 1 - MAX_VISIBLE;
        }
    }

    /// Compute the thumbnail layout for the render loop. Each `ThumbnailSlot`
    /// tells the renderer where to draw a scaled-down copy of a window.
    pub fn thumbnail_slots(&self, output_size: Size<i32, Logical>) -> Vec<ThumbnailSlot> {
        if !self.is_visible() {
            return Vec::new();
        }

        let visible_count = self.entries.len().min(MAX_VISIBLE);
        let strip_w = visible_count as i32 * THUMB_W
            + (visible_count as i32 - 1).max(0) * THUMB_GAP;
        let panel_w = strip_w + 2 * PANEL_PADDING;
        let panel_h = THUMB_H + 2 * PANEL_PADDING;
        let panel_x = ((output_size.w - panel_w) / 2).max(0);
        let panel_y = ((output_size.h - panel_h) / 2).max(0);

        let start_x = panel_x + PANEL_PADDING;
        let start_y = panel_y + PANEL_PADDING;

        self.entries
            .iter()
            .skip(self.scroll_offset)
            .take(visible_count)
            .enumerate()
            .map(|(i, surface)| {
                let real_index = self.scroll_offset + i;
                let x = start_x + i as i32 * (THUMB_W + THUMB_GAP);
                ThumbnailSlot {
                    surface: surface.clone(),
                    position: Point::from((x, start_y)),
                    size: Size::from((THUMB_W, THUMB_H)),
                    selected: real_index == self.selected_index,
                    minimized: self.minimized_surfaces.contains(surface),
                }
            })
            .collect()
    }

    /// Current fade-in alpha (0.0 to 1.0).
    fn fade_alpha(&self) -> f32 {
        let Some(start) = self.fade_start else {
            return 1.0;
        };
        let elapsed = start.elapsed().as_secs_f32();
        let t = anim::progress(elapsed, anim::DURATION_ENTER);
        anim::ease_out(t)
    }

    /// Compute the highlight X position, interpolating between previous and
    /// current selection for smooth sliding.
    fn highlight_x(&self, start_x: i32) -> f32 {
        let current_x = start_x as f32
            + (self.selected_index.saturating_sub(self.scroll_offset)) as f32
                * (THUMB_W + THUMB_GAP) as f32;

        let Some(anim_start) = self.highlight_anim_start else {
            return current_x;
        };

        let elapsed = anim_start.elapsed().as_secs_f32();
        let t = anim::progress(elapsed, anim::DURATION_FAST);
        let t = anim::ease_out(t);

        let prev_x = start_x as f32
            + (self.prev_selected_index.saturating_sub(self.scroll_offset)) as f32
                * (THUMB_W + THUMB_GAP) as f32;

        anim::lerp(prev_x, current_x, t)
    }

    /// Returns true if any animation is still in progress (needs re-render).
    pub fn needs_redraw(&self) -> bool {
        if !self.is_visible() {
            return false;
        }
        // Fade-in still running?
        if let Some(start) = self.fade_start {
            if start.elapsed().as_secs_f32() < anim::DURATION_ENTER {
                return true;
            }
        }
        // Highlight slide still running?
        if let Some(start) = self.highlight_anim_start {
            if start.elapsed().as_secs_f32() < anim::DURATION_FAST {
                return true;
            }
        }
        false
    }

    /// Render overlay chrome split into two layers:
    /// - `base`: dim, panel, highlights, cards (rendered BEHIND thumbnails)
    /// - `top`: minimized dim overlays, close button (rendered ABOVE thumbnails)
    pub fn render_overlay_split(
        &self,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> (Vec<SolidColorRenderElement>, Vec<SolidColorRenderElement>) {
        if !self.is_visible() {
            return (Vec::new(), Vec::new());
        }

        let alpha = self.fade_alpha();

        let to_phys = |x: i32, y: i32| -> Point<i32, Physical> {
            Point::from((
                (x as f64 * scale).round() as i32,
                (y as f64 * scale).round() as i32,
            ))
        };
        let to_phys_f = |x: f32, y: f32| -> Point<i32, Physical> {
            Point::from((
                (x as f64 * scale).round() as i32,
                (y as f64 * scale).round() as i32,
            ))
        };

        let visible_count = self.entries.len().min(MAX_VISIBLE);
        let strip_w = visible_count as i32 * THUMB_W
            + (visible_count as i32 - 1).max(0) * THUMB_GAP;
        let panel_w = strip_w + 2 * PANEL_PADDING;
        let panel_h = THUMB_H + 2 * PANEL_PADDING;
        let panel_x = ((output_size.w - panel_w) / 2).max(0);
        let panel_y = ((output_size.h - panel_h) / 2).max(0);

        let start_x = panel_x + PANEL_PADDING;
        let start_y = panel_y + PANEL_PADDING;

        let kind = smithay::backend::renderer::element::Kind::Unspecified;

        let mut base = Vec::with_capacity(2 + visible_count * 2);
        let mut top = Vec::new();

        // 1) Full-screen dark overlay
        base.push(SolidColorRenderElement::from_buffer(
            &self.overlay_buf, to_phys(0, 0), scale, alpha, kind,
        ));

        // 2) Panel background
        base.push(SolidColorRenderElement::from_buffer(
            &self.panel_buf, to_phys(panel_x, panel_y), scale, alpha, kind,
        ));

        let hl_x = self.highlight_x(start_x);

        for (i, surface) in self.entries.iter().skip(self.scroll_offset).take(visible_count).enumerate() {
            let real_index = self.scroll_offset + i;
            let x = start_x + i as i32 * (THUMB_W + THUMB_GAP);
            let is_selected = real_index == self.selected_index;
            let is_minimized = self.minimized_surfaces.contains(surface);

            // Highlight border (base layer, behind thumbnails)
            if is_selected {
                base.push(SolidColorRenderElement::from_buffer(
                    &self.highlight_buf,
                    to_phys_f(hl_x - HIGHLIGHT_BORDER as f32, (start_y - HIGHLIGHT_BORDER) as f32),
                    scale, alpha, kind,
                ));
            }

            // Card background (base layer)
            base.push(SolidColorRenderElement::from_buffer(
                &self.card_buf, to_phys(x, start_y), scale, alpha, kind,
            ));

            // Minimized dim (top layer, above thumbnails)
            if is_minimized {
                top.push(SolidColorRenderElement::from_buffer(
                    &self.minimized_dim_buf, to_phys(x, start_y), scale, alpha, kind,
                ));
            }

            // Close button (top layer, above thumbnails)
            if is_selected {
                let btn_x = x + THUMB_W - CLOSE_BTN_SIZE - CLOSE_BTN_MARGIN;
                let btn_y = start_y + CLOSE_BTN_MARGIN;
                top.push(SolidColorRenderElement::from_buffer(
                    &self.close_btn_buf, to_phys(btn_x, btn_y), scale, alpha, kind,
                ));
            }
        }

        (base, top)
    }

    /// Resize overlay/panel/card buffers to match the current output size.
    /// Call this once before rendering each frame when the switcher is visible.
    pub fn update_sizes(&mut self, output_size: Size<i32, Logical>) {
        if !self.is_visible() {
            return;
        }

        let visible_count = self.entries.len().min(MAX_VISIBLE);
        let strip_w = visible_count as i32 * THUMB_W
            + (visible_count as i32 - 1).max(0) * THUMB_GAP;
        let panel_w = strip_w + 2 * PANEL_PADDING;
        let panel_h = THUMB_H + 2 * PANEL_PADDING;

        self.overlay_buf.resize((output_size.w, output_size.h));
        self.panel_buf.resize((panel_w, panel_h));
        self.card_buf.resize((THUMB_W, THUMB_H));
        self.highlight_buf.resize((
            THUMB_W + 2 * HIGHLIGHT_BORDER,
            THUMB_H + 2 * HIGHLIGHT_BORDER,
        ));
        self.minimized_dim_buf.resize((THUMB_W, THUMB_H));
        self.close_btn_buf.resize((CLOSE_BTN_SIZE, CLOSE_BTN_SIZE));
    }
}
