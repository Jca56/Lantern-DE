//! The unified "ribbon" — the top chrome band that fuses the inline title bar,
//! the tab strip, and the formatting toolbar into one cohesive panel (Word's
//! ribbon). Instead of three flat plates, it's a single top-lit gradient with a
//! tight drop shadow and a 1px top sheen, sitting above the document.
//!
//! Pulled out of `render.rs` so that file stays under the size limit. The
//! ribbon↔desk seam hairline is drawn separately in `render.rs` AFTER the tab
//! strip, so the active tab can cover its slice of the seam.

use lntrn_render::{Painter, Rect};
use lntrn_ui::gpu::FoxPalette;

use crate::render::TOOLBAR_H;
use crate::tab_strip::TAB_STRIP_H;
use crate::title_bar::TITLE_BAR_H;
use crate::tokens;

/// Total ribbon height in physical pixels (title + tabs + toolbar).
pub fn ribbon_height(s: f32) -> f32 {
    (TITLE_BAR_H + TAB_STRIP_H + TOOLBAR_H) * s
}

/// Paint the ribbon panel background. Call right after the window-bg fill and
/// BEFORE the window controls / menu / tabs / toolbar draw on top of it.
pub fn draw_ribbon_bg(painter: &mut Painter, pal: &FoxPalette, wf: f32, s: f32) {
    let ribbon_h = ribbon_height(s);
    let ribbon_r = Rect::new(0.0, 0.0, wf, ribbon_h);

    // 1. Tight drop shadow UNDER the ribbon, before the gradient paints over it.
    painter.shadow(
        ribbon_r,
        0.0,
        tokens::PANEL_SHADOW_SIGMA * s,
        tokens::panel_shadow_color(),
        0.0,
        tokens::PANEL_SHADOW_OFFSET_Y * s,
    );

    // 2. Unified vertical gradient — top lit, bottom settled.
    let (top, bot) = tokens::ribbon_gradient(pal.surface);
    painter.rect_gradient_linear(ribbon_r, 0.0, tokens::GRAD_ANGLE, top, bot);

    // 3. 1px top sheen highlight.
    painter.rect_filled(
        Rect::new(0.0, 0.0, wf, 1.0 * s),
        0.0,
        tokens::ribbon_sheen(pal.surface),
    );
}
