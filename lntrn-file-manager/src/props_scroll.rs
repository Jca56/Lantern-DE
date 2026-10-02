//! Scrolling inside the Properties dialog.
//!
//! The dialog can be taller than the window (a plain file needs about 820
//! logical px, an image or an audio file more, and a laptop screen has
//! fewer). Its panel was clamped to the window but its rows were not: they
//! were drawn at their natural positions, over the backdrop and off the
//! window, with no way to reach them. The header (icon, name, close button)
//! now stays put and what is below it scrolls: the information rows, or in
//! the icon picker the grid of icons between its tabs and its buttons.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, InteractionState};

use crate::properties::FileProperties;

/// The part of the dialog that scrolls, as it was last drawn.
#[derive(Clone, Copy, Debug)]
pub struct ScrollView {
    pub viewport: Rect,
    pub content_h: f32,
}

/// Height of the panel: its natural height, or what the window has room
/// for. `fixed_h` is what does not scroll (the header); a little of the
/// scrolling part is kept even in a window that is all header.
pub(crate) fn panel_height(natural_h: f32, fixed_h: f32, screen_h: f32, s: f32) -> f32 {
    let room = (screen_h - 40.0 * s).max(fixed_h + 100.0 * s).min(screen_h);
    natural_h.min(room)
}

impl FileProperties {
    /// The scroll offset of what is showing: the picker's grid while the
    /// picker is open, the information rows otherwise.
    pub(crate) fn scroll_slot(&mut self) -> &mut f32 {
        if self.picker_open && self.is_dir {
            &mut self.picker_scroll
        } else {
            &mut self.scroll_offset
        }
    }

    /// Start drawing the scrolling part: `content_h` of rows seen through
    /// `viewport`. Returns the y the first row is drawn at. Everything drawn
    /// until `end_scroll` is clipped to the viewport.
    pub(crate) fn begin_scroll(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        viewport: Rect,
        content_h: f32,
    ) -> f32 {
        let max = (content_h - viewport.h).max(0.0);
        let slot = self.scroll_slot();
        *slot = if slot.is_finite() {
            slot.clamp(0.0, max)
        } else {
            0.0
        };
        let offset = *slot;
        self.scroll_view = Some(ScrollView {
            viewport,
            content_h,
        });
        painter.push_clip(viewport);
        text.push_clip([viewport.x, viewport.y, viewport.w, viewport.h]);
        viewport.y - offset
    }

    /// Done with the scrolling part: lift the clip and, when there is more
    /// than fits, draw the scrollbar at the viewport's right edge.
    pub(crate) fn end_scroll(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        fox: &FoxPalette,
        s: f32,
    ) {
        painter.pop_clip();
        text.pop_clip();
        let Some(view) = self.scroll_view else {
            return;
        };
        // Half a pixel of rounding is not something to scroll.
        if view.content_h <= view.viewport.h + 0.5 {
            return;
        }
        let offset = *self.scroll_slot();
        let bar = crate::scrollbar::bar(&view.viewport, view.content_h, offset, s);
        let zone = crate::scrollbar::hover_zone(&bar, s);
        let state = match zone.intersect(&view.viewport) {
            Some(visible) => ix.add_zone(crate::ZONE_PROPS_SCROLLBAR, visible),
            None => InteractionState::Idle,
        };
        crate::scrollbar::draw(painter, &bar, state, fox, true);
    }

    /// The part of `rect` inside the scrolling viewport: what a row may
    /// register as its zone. A row scrolled under the header must not be
    /// clickable through it.
    pub(crate) fn visible_part(&self, rect: Rect) -> Option<Rect> {
        match self.scroll_view {
            Some(view) => rect.intersect(&view.viewport),
            None => Some(rect),
        }
    }

    /// Clip for the textures drawn inside the scrolling part (cover art,
    /// picker thumbnails): they are drawn in a later pass, which the
    /// painter's clip does not reach.
    pub fn texture_clip(&self) -> Option<[f32; 4]> {
        self.scroll_view
            .map(|v| [v.viewport.x, v.viewport.y, v.viewport.w, v.viewport.h])
    }
}

/// Cover every part of the window's column of the panel that is not the
/// scrolling viewport. Rows scrolled out of view still have their zones
/// (the audio tag editor registers its own), and a zone registered later
/// wins: after this, a press above or below the viewport reaches the panel
/// or the backdrop, never a row that is not there. The header's own zones
/// are registered again after this by the caller.
pub(crate) fn mask_outside_viewport(
    ix: &mut InteractionContext,
    panel: Rect,
    viewport: Rect,
    screen_h: f32,
    panel_zone: u32,
    backdrop_zone: u32,
) {
    let mut cover = |zone: u32, top: f32, bottom: f32| {
        if bottom > top {
            ix.add_zone(zone, Rect::new(panel.x, top, panel.w, bottom - top));
        }
    };
    let panel_bottom = panel.y + panel.h;
    cover(backdrop_zone, 0.0, panel.y);
    cover(panel_zone, panel.y, viewport.y);
    cover(panel_zone, viewport.y + viewport.h, panel_bottom);
    cover(backdrop_zone, panel_bottom, screen_h);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_is_its_natural_height_until_the_window_is_shorter() {
        // Room to spare: natural height.
        assert_eq!(panel_height(820.0, 165.0, 1000.0, 1.0), 820.0);
        // A laptop-height window: clamped, with the usual margin.
        assert_eq!(panel_height(880.0, 165.0, 860.0, 1.0), 820.0);
        // A very short window: the margin gives way before the body does...
        assert_eq!(panel_height(820.0, 165.0, 290.0, 1.0), 265.0);
        // ...and the panel never grows past the window itself.
        assert_eq!(panel_height(820.0, 165.0, 200.0, 1.0), 200.0);
        // The margin scales.
        assert_eq!(panel_height(2000.0, 231.0, 1000.0, 1.4), 944.0);
    }

    #[test]
    fn the_mask_leaves_only_the_viewport_open() {
        let mut ix = InteractionContext::new();
        let panel = Rect::new(300.0, 100.0, 520.0, 600.0);
        let viewport = Rect::new(300.0, 265.0, 520.0, 427.0);
        // A row scrolled up under the header, and one below the panel.
        ix.add_zone(810, Rect::new(324.0, 180.0, 472.0, 34.0));
        ix.add_zone(811, Rect::new(324.0, 720.0, 472.0, 34.0));
        // One that is in view.
        ix.add_zone(812, Rect::new(324.0, 400.0, 472.0, 34.0));
        mask_outside_viewport(&mut ix, panel, viewport, 800.0, 802, 801);
        assert_eq!(ix.zone_at(400.0, 190.0), Some(802), "under the header");
        assert_eq!(ix.zone_at(400.0, 730.0), Some(801), "below the panel");
        assert_eq!(ix.zone_at(400.0, 410.0), Some(812), "in view");
        assert_eq!(
            ix.zone_at(400.0, 695.0),
            Some(802),
            "the panel's bottom edge"
        );
        assert_eq!(ix.zone_at(400.0, 50.0), Some(801), "above the panel");
    }
}
