//! Claude-usage button rendered into the top strip's "Usage" slot.
//! Opens the Claude-usage panel. Uses the colored `Claude.svg` asset
//! shipped in `~/.lantern/icons/`, routed through the existing
//! icon-request pipeline.

use lntrn_render::Rect;

use crate::render::IconRequest;

/// Absolute path to the colored Claude logo we ship in `~/.lantern/icons/`.
pub const CLAUDE_ICON_PATH: &str = "/home/alva/.lantern/icons/Claude.svg";
/// `app_id` we register the Claude icon under in the icon cache.
pub const CLAUDE_ICON_APP_ID: &str = "lntrn-claude-usage";

/// Hit-rect for the usage button — owned by `view_indicator` so it
/// lines up with the rest of the top strip.
pub fn button_rect(panel: Rect, scale: f32) -> Rect {
    crate::view_indicator::usage_rect(panel, scale)
}

pub fn hit_test(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    let r = button_rect(panel, scale);
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

pub fn draw(
    _painter: &mut lntrn_render::Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered: bool,
    active: bool,
    icons: &mut Vec<IconRequest>,
) {
    let r = button_rect(panel, scale);
    // The colored Claude SVG dims slightly when idle, full opacity when
    // hovered or active — same convention as the other top-strip glyphs.
    let icon_alpha = if hovered || active { alpha } else { alpha * 0.75 };
    icons.push(IconRequest {
        app_id: CLAUDE_ICON_APP_ID.to_string(),
        icon_name: Some(CLAUDE_ICON_PATH.to_string()),
        x: r.x,
        y: r.y,
        size: r.w,
        opacity: icon_alpha,
        clip: None,
    });
}
