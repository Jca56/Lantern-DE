//! Claude-usage button rendered into its outer-strip slot. Opens the
//! Claude-usage panel. Uses the colored `Claude.svg` asset shipped in
//! `~/.lantern/icons/`, routed through the existing icon-request
//! pipeline. The slot rect comes from [`crate::outer_zones`].

use lntrn_render::Rect;

use crate::render::IconRequest;

/// `app_id` we register the Claude icon under in the icon cache.
pub const CLAUDE_ICON_APP_ID: &str = "lntrn-claude-usage";

/// Absolute path to the colored Claude logo in `~/.lantern/icons/`,
/// resolved from `$HOME` so both machines find it.
pub fn claude_icon_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    format!("{home}/.lantern/icons/Claude.svg")
}

pub fn draw(r: Rect, alpha: f32, hovered: bool, active: bool, icons: &mut Vec<IconRequest>) {
    // The colored Claude SVG dims slightly when idle, full opacity when
    // hovered or active — same convention as the other strip glyphs.
    let icon_alpha = if hovered || active { alpha } else { alpha * 0.75 };
    icons.push(IconRequest {
        app_id: CLAUDE_ICON_APP_ID.to_string(),
        icon_name: Some(claude_icon_path()),
        x: r.x,
        y: r.y,
        size: r.w,
        opacity: icon_alpha,
        clip: None,
    });
}
