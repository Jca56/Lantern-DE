use std::path::PathBuf;

/// Valid built-in background color names.
pub const COLORS: [&str; 4] = ["blue", "green", "purple", "red"];

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
