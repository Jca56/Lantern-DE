//! Claude-usage button rendered into its outer-strip slot. Opens the
//! Claude-usage panel. Uses the colored `Claude.svg` asset shipped in
//! `~/.lantern/icons/`, routed through the existing icon-request
//! pipeline. The slot rect comes from [`crate::outer_zones`].
//!
//! Carries a small Fable 5 availability pip in the corner (green = usable,
//! red = gated, grey = checking) so the model's status reads at a glance
//! from the toolbar without opening the usage panel. Driven by the same
//! `usage::model_status` poller the panel uses.

use lntrn_render::{Color, Painter, Rect};

use crate::render::IconRequest;
use crate::usage::model_status::ModelHealth;

/// `app_id` we register the Claude icon under in the icon cache.
pub const CLAUDE_ICON_APP_ID: &str = "lntrn-claude-usage";

/// Absolute path to the colored Claude logo in `~/.lantern/icons/`,
/// resolved from `$HOME` so both machines find it.
pub fn claude_icon_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    format!("{home}/.lantern/icons/Claude.svg")
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    r: Rect,
    scale: f32,
    alpha: f32,
    hovered: bool,
    active: bool,
    health: ModelHealth,
    icons: &mut Vec<IconRequest>,
) {
    // The colored Claude SVG dims slightly when idle, full opacity when
    // hovered or active — same convention as the other strip glyphs.
    let icon_alpha = if hovered || active {
        alpha
    } else {
        alpha * 0.75
    };
    icons.push(IconRequest {
        app_id: CLAUDE_ICON_APP_ID.to_string(),
        icon_name: Some(claude_icon_path()),
        x: r.x,
        y: r.y,
        size: r.w,
        opacity: icon_alpha,
        clip: None,
    });

    // Status pip, last so it isn't skipped. Painter draws on layer 1
    // *under* the icon texture, but the logo's corner is transparent so
    // the pip shows through; the dark halo keeps it legible either way.
    draw_status_pip(painter, r, scale, alpha, health);
}

fn draw_status_pip(painter: &mut Painter, r: Rect, scale: f32, alpha: f32, health: ModelHealth) {
    let color = match health {
        ModelHealth::Up => Color::from_rgb8(120, 220, 120),
        ModelHealth::Down => Color::from_rgb8(255, 90, 90),
        ModelHealth::Unknown => Color::from_rgb8(150, 150, 150),
    };

    // Pip scales with the button but stays a "dot". Tucked into the
    // bottom-right corner where the Claude glyph has no opaque pixels.
    let d = (r.w * 0.30).clamp(7.0 * scale, 13.0 * scale);
    let halo = d + 3.0 * scale;
    let cx = r.x + r.w - d * 0.55;
    let cy = r.y + r.h - d * 0.55;

    // Dark halo for contrast, then the colored pip on top.
    painter.rect_filled(
        Rect::new(cx - halo * 0.5, cy - halo * 0.5, halo, halo),
        halo * 0.5,
        Color::from_rgb8(18, 16, 14).with_alpha(0.85 * alpha),
    );
    painter.rect_filled(
        Rect::new(cx - d * 0.5, cy - d * 0.5, d, d),
        d * 0.5,
        color.with_alpha(alpha),
    );
}
