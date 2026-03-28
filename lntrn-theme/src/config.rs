use crate::ThemeVariant;
use std::path::PathBuf;

/// Returns the root of the Lantern home directory: `~/.lantern`.
pub fn lantern_home() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".lantern"))
}

/// Returns the path to the shared DE config: `~/.lantern/config/lantern.toml`.
/// Falls back to old `~/.config/lantern/lantern.toml` if the new file doesn't exist yet.
pub fn lantern_config_path() -> Option<PathBuf> {
    let new_path = lantern_home()?.join("config/lantern.toml");
    if new_path.exists() {
        return Some(new_path);
    }
    // Old-path fallback for migration
    let home = std::env::var("HOME").ok()?;
    let old_path = PathBuf::from(home).join(".config/lantern/lantern.toml");
    if old_path.exists() {
        return Some(old_path);
    }
    // Neither exists — return canonical new path for first-time creation
    Some(lantern_home()?.join("config/lantern.toml"))
}

/// Parse a theme name string into a `ThemeVariant`.
pub fn parse_variant(name: &str) -> Option<ThemeVariant> {
    match name.trim() {
        "fox-dark" => Some(ThemeVariant::FoxDark),
        "fox-light" => Some(ThemeVariant::FoxLight),
        "lantern" => Some(ThemeVariant::Lantern),
        _ => None,
    }
}

/// Read the active theme variant from the Lantern config.
///
/// Looks for `theme = "..."` under `[appearance]`. Falls back to `FoxDark`
/// if the file is missing, unreadable, or the value is unrecognized.
pub fn active_variant() -> ThemeVariant {
    let Some(path) = lantern_config_path() else {
        return ThemeVariant::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return ThemeVariant::default();
    };

    // We're in the [appearance] section when we see that header,
    // and we leave it when we hit another [section].
    let mut in_appearance = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_appearance = trimmed == "[appearance]";
            continue;
        }
        if in_appearance {
            if let Some(value) = trimmed.strip_prefix("theme") {
                let value = value.trim_start();
                if let Some(value) = value.strip_prefix('=') {
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    if let Some(variant) = parse_variant(value) {
                        return variant;
                    }
                }
            }
        }
    }

    ThemeVariant::default()
}
