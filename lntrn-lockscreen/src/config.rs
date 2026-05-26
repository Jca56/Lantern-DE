use lntrn_render::Color;
use std::path::PathBuf;

/// Valid built-in background color names.
pub const COLORS: [&str; 4] = ["blue", "green", "purple", "red"];

/// Resolved visual styling for the lock screen UI, read from `[lockscreen]`
/// in lantern.toml. Every field is independently configurable; see
/// [`style`] for keys and defaults.
pub struct Style {
    pub border_color: Color,
    pub border_thickness: f32,
    /// Field fill color with opacity already baked into the alpha channel.
    pub field_color: Color,
    pub dot_color: Color,
    pub scrim_opacity: f32,
}

fn parse_hex_or(s: &str, fallback: Color) -> Color {
    if s.is_empty() {
        return fallback;
    }
    Color::from_hex(s).unwrap_or(fallback)
}

/// Resolve all lock screen styling from `[lockscreen]` in lantern.toml.
///
/// Keys (all optional): `border_color` (hex, default = theme accent),
/// `border_thickness` (px, default 2.0), `field_color` (hex, default #000000),
/// `field_opacity` (0..1, default 0.55), `dot_color` (hex, default #F5F5F5),
/// `scrim_opacity` (0..1, default 0.38).
pub fn style() -> Style {
    let accent = accent();
    let border_color = parse_hex_or(
        &lntrn_theme::read_config_string("lockscreen", "border_color", ""),
        Color::from_rgb8(accent.r, accent.g, accent.b),
    );
    let border_thickness =
        lntrn_theme::read_config_f32("lockscreen", "border_thickness", 2.0).clamp(0.0, 16.0);

    let field_opacity =
        lntrn_theme::read_config_f32("lockscreen", "field_opacity", 0.55).clamp(0.0, 1.0);
    let field_rgb = parse_hex_or(
        &lntrn_theme::read_config_string("lockscreen", "field_color", "#000000"),
        Color::from_rgb8(0, 0, 0),
    );
    let field_color = Color::rgba(field_rgb.r, field_rgb.g, field_rgb.b, field_opacity);

    let dot_color = parse_hex_or(
        &lntrn_theme::read_config_string("lockscreen", "dot_color", "#F5F5F5"),
        Color::from_rgb8(245, 245, 245),
    );
    let scrim_opacity =
        lntrn_theme::read_config_f32("lockscreen", "scrim_opacity", 0.38).clamp(0.0, 1.0);

    Style { border_color, border_thickness, field_color, dot_color, scrim_opacity }
}

/// Directory where installed lockscreen backgrounds live.
pub fn share_dir() -> Option<PathBuf> {
    lntrn_theme::lantern_home().map(|h| h.join("share/lockscreen"))
}

/// Resolve the background image path from `[lockscreen] background` in lantern.toml.
///
/// The value may be a built-in color name ("blue", "green", "purple", "red")
/// or an absolute path to a custom image. Falls back to "blue".
pub fn background_path() -> Option<PathBuf> {
    let value = lntrn_theme::read_config_string("lockscreen", "background", "blue");

    // Absolute path → use directly.
    let p = PathBuf::from(&value);
    if p.is_absolute() && p.exists() {
        return Some(p);
    }

    // Otherwise treat as a color name, normalized to a known one.
    let color = if COLORS.contains(&value.as_str()) { value } else { "blue".to_string() };
    let dir = share_dir()?;
    let path = dir.join(format!("lockscreen-{color}.png"));
    if path.exists() {
        Some(path)
    } else {
        // Dev fallback: load straight from the crate's Backgrounds/ dir.
        dev_background(&color)
    }
}

/// Dev-only fallback: load from the repo's `Backgrounds/` directory.
/// The red asset is named differently (`lock-screen-red.png`).
fn dev_background(color: &str) -> Option<PathBuf> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Backgrounds");
    let candidates = [
        base.join(format!("lockscreen-{color}.png")),
        base.join(format!("lock-screen-{color}.png")),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// The active theme accent color.
pub fn accent() -> lntrn_theme::Rgba {
    lntrn_theme::active_variant().accent()
}
