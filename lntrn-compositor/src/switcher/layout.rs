//! Spotlight layout math for the Alt+Tab switcher.
//!
//! Single source of truth shared by the renderer (`render/surface.rs`)
//! and input hit-testing (`input/pointer.rs`). Produces a [`CardLayout`]
//! per visible window: where the card sits, how big it is, whether it's
//! the spotlighted (selected) one, and its dim level.
//!
//! Geometry, in logical pixels:
//! - The **selected** card is full size (`CARD_W × CARD_H`).
//! - Non-selected cards are scaled by `SIDE_SCALE` and dimmed.
//! - Cards are laid out left→right with `GAP` between footprints; the
//!   whole strip is translated so the selected card's center lands on the
//!   output's horizontal center. The animated `slide` (0..=1, springy)
//!   interpolates that translation from the previous selection's center,
//!   so cycling glides instead of snapping.
//! - Vertically every card is centered on the output's mid-line (taller
//!   selected card and shorter side cards share a center line).

use smithay::utils::{Logical, Point, Size};

/// Full (selected) card footprint, logical px.
pub const CARD_W: i32 = 600;
pub const CARD_H: i32 = 390;
/// Scale applied to non-selected cards.
pub const SIDE_SCALE: f32 = 0.72;
/// Gap between adjacent card footprints, logical px.
pub const GAP: i32 = 56;
/// Max cards visible at once (scroll for more).
pub const MAX_VISIBLE: usize = 5;
/// Corner radius for card previews, logical px.
pub const CORNER_RADIUS: f32 = 18.0;
/// Dim alpha applied over non-selected cards (0 = none).
pub const SIDE_DIM: f32 = 0.32;

/// App-icon badge size (logical px) on the selected card. Side cards get
/// `ICON_SIZE * SIDE_SCALE`.
pub const ICON_SIZE: i32 = 52;
/// Badge inset from the card's bottom-left corner, logical px.
pub const ICON_MARGIN: i32 = 12;

/// Close-button size on the selected card, logical px.
pub const CLOSE_BTN_SIZE: i32 = 30;
/// Close-button inset from the card's top-right corner, logical px.
pub const CLOSE_BTN_MARGIN: i32 = 8;

/// Resolved geometry for one visible card.
#[derive(Clone, Copy)]
pub struct CardLayout {
    /// Index into the switcher's `entries`.
    pub entry_index: usize,
    /// Top-left of the card in logical output coordinates.
    pub position: Point<i32, Logical>,
    /// Card size in logical px (already scaled for side cards).
    pub size: Size<i32, Logical>,
    /// True for the spotlighted (selected) card.
    pub selected: bool,
    /// Dim overlay alpha to draw over the preview (0 = none).
    pub dim: f32,
    /// Scale factor relative to the full card (1.0 selected, `SIDE_SCALE` else).
    pub scale: f32,
}

impl CardLayout {
    /// Centered icon-badge rect (bottom-left of the card).
    pub fn icon_rect(&self) -> (Point<i32, Logical>, i32) {
        let sz = (ICON_SIZE as f32 * self.scale).round() as i32;
        let margin = (ICON_MARGIN as f32 * self.scale).round() as i32;
        let x = self.position.x + margin;
        let y = self.position.y + self.size.h - sz - margin;
        (Point::from((x, y)), sz)
    }

    /// Close-button rect (top-right of the card).
    pub fn close_rect(&self) -> (Point<i32, Logical>, i32) {
        let x = self.position.x + self.size.w - CLOSE_BTN_SIZE - CLOSE_BTN_MARGIN;
        let y = self.position.y + CLOSE_BTN_MARGIN;
        (Point::from((x, y)), CLOSE_BTN_SIZE)
    }
}

/// Compute the spotlight layout for the currently-visible window range.
///
/// `entry_count` is the total number of windows; `selected` and `scroll`
/// index into that list. `slide` (0..=1) animates the strip translation
/// between `prev_selected` and `selected` for the slide effect.
pub fn compute(
    output_size: Size<i32, Logical>,
    entry_count: usize,
    selected: usize,
    prev_selected: usize,
    scroll: usize,
    slide: f32,
) -> Vec<CardLayout> {
    if entry_count == 0 {
        return Vec::new();
    }
    let visible_count = entry_count.min(MAX_VISIBLE);
    let center_x = output_size.w as f32 / 2.0;
    let center_y = output_size.h as f32 / 2.0;

    // Width of a card footprint at index `i` within the visible range
    // depends on whether it's the selected card (full) or a side card.
    let footprint_w = |real_index: usize| -> f32 {
        if real_index == selected { CARD_W as f32 } else { CARD_W as f32 * SIDE_SCALE }
    };

    // X-offset (left edge) of each visible card's footprint, laid out in a
    // row starting at 0. We then translate the whole row so the *target*
    // card center sits at the output center.
    let mut left_edges = Vec::with_capacity(visible_count);
    let mut cursor = 0.0f32;
    for i in 0..visible_count {
        let real_index = scroll + i;
        left_edges.push(cursor);
        cursor += footprint_w(real_index) + GAP as f32;
    }

    // Center of a given visible slot in row-local space.
    let slot_center = |vis: usize| -> f32 {
        let real_index = scroll + vis;
        left_edges[vis] + footprint_w(real_index) / 2.0
    };

    // Translate so the (animated) selected center lands on output center.
    let sel_vis = selected.saturating_sub(scroll).min(visible_count.saturating_sub(1));
    let prev_vis = prev_selected.saturating_sub(scroll).min(visible_count.saturating_sub(1));
    let target_center = slot_center(sel_vis);
    let from_center = slot_center(prev_vis);
    // NOTE: `slide` is a spring value that intentionally overshoots past
    // 1.0 — do NOT clamp it here, or the bounce is lost. It starts at 0
    // (strip at the previous selection) and settles at 1 (strip centered
    // on the new selection), wobbling past on the way.
    let anim_center = from_center + (target_center - from_center) * slide;
    let translate = center_x - anim_center;

    (0..visible_count)
        .map(|i| {
            let real_index = scroll + i;
            let is_selected = real_index == selected;
            let scale = if is_selected { 1.0 } else { SIDE_SCALE };
            let w = (CARD_W as f32 * scale).round() as i32;
            let h = (CARD_H as f32 * scale).round() as i32;
            let x = (left_edges[i] + translate).round() as i32;
            let y = (center_y - h as f32 / 2.0).round() as i32;
            CardLayout {
                entry_index: real_index,
                position: Point::from((x, y)),
                size: Size::from((w, h)),
                selected: is_selected,
                dim: if is_selected { 0.0 } else { SIDE_DIM },
                scale,
            }
        })
        .collect()
}

/// Hit-test a logical point against the laid-out cards. Returns the
/// entry index of the card containing the point, if any.
pub fn hit_test(cards: &[CardLayout], px: i32, py: i32) -> Option<usize> {
    // Iterate selected-last isn't needed (cards don't overlap horizontally),
    // but the selected card is larger; check all and take the first hit.
    for c in cards {
        let p = c.position;
        if px >= p.x && px < p.x + c.size.w && py >= p.y && py < p.y + c.size.h {
            return Some(c.entry_index);
        }
    }
    None
}

/// Hit-test the close button on the selected card.
pub fn hit_test_close(cards: &[CardLayout], px: i32, py: i32) -> Option<usize> {
    for c in cards.iter().filter(|c| c.selected) {
        let (loc, sz) = c.close_rect();
        if px >= loc.x && px < loc.x + sz && py >= loc.y && py < loc.y + sz {
            return Some(c.entry_index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out() -> Size<i32, Logical> {
        Size::from((1920, 1080))
    }

    /// The selected card's center lands on the output's horizontal center
    /// once the slide animation has settled (slide = 1.0).
    #[test]
    fn selected_card_is_centered_when_settled() {
        let cards = compute(out(), 4, 2, 2, 0, 1.0);
        let sel = cards.iter().find(|c| c.selected).expect("a selected card");
        let center = sel.position.x + sel.size.w / 2;
        assert!(
            (center - 960).abs() <= 1,
            "selected center {center} should be ~960 (half of 1920)"
        );
    }

    /// The selected card is full-size; side cards are scaled down + dimmed.
    #[test]
    fn selected_is_full_size_sides_are_scaled() {
        let cards = compute(out(), 3, 1, 1, 0, 1.0);
        for c in &cards {
            if c.selected {
                assert_eq!(c.size.w, CARD_W);
                assert_eq!(c.dim, 0.0);
            } else {
                assert!(c.size.w < CARD_W, "side card should be smaller");
                assert!(c.dim > 0.0, "side card should be dimmed");
            }
        }
    }

    /// A point inside the selected card hit-tests to its entry index.
    #[test]
    fn hit_test_finds_selected_card() {
        let cards = compute(out(), 3, 1, 1, 0, 1.0);
        let sel = cards.iter().find(|c| c.selected).unwrap();
        let cx = sel.position.x + sel.size.w / 2;
        let cy = sel.position.y + sel.size.h / 2;
        assert_eq!(hit_test(&cards, cx, cy), Some(1));
        // A point far above the strip hits nothing.
        assert_eq!(hit_test(&cards, cx, 0), None);
    }

    /// The close button sits inside the selected card's top-right and
    /// hit-tests correctly; side cards have no close button.
    #[test]
    fn close_button_only_on_selected() {
        let cards = compute(out(), 3, 0, 0, 0, 1.0);
        let sel = cards.iter().find(|c| c.selected).unwrap();
        let (loc, sz) = sel.close_rect();
        assert_eq!(
            hit_test_close(&cards, loc.x + sz / 2, loc.y + sz / 2),
            Some(0)
        );
        // A point on a side card's body is not a close hit.
        let side = cards.iter().find(|c| !c.selected).unwrap();
        let bx = side.position.x + side.size.w / 2;
        let by = side.position.y + side.size.h / 2;
        assert_eq!(hit_test_close(&cards, bx, by), None);
    }

    /// With more than MAX_VISIBLE windows, only MAX_VISIBLE cards are laid
    /// out and the selected one (kept in range by scroll) stays centered.
    #[test]
    fn scrolling_keeps_selected_centered() {
        // 8 windows, selected index 6, scrolled so it's the last visible.
        let scroll = 6 + 1 - MAX_VISIBLE; // mirrors AltTabSwitcher::update_scroll
        let cards = compute(out(), 8, 6, 6, scroll, 1.0);
        assert_eq!(cards.len(), MAX_VISIBLE);
        let sel = cards.iter().find(|c| c.selected).expect("selected visible");
        let center = sel.position.x + sel.size.w / 2;
        assert!((center - 960).abs() <= 1, "selected center {center} ~960");
    }
}
