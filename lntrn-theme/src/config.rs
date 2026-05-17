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
        "fox-dark" | "fox" => Some(ThemeVariant::FoxDark),
        "fox-light" => Some(ThemeVariant::FoxLight),
        "lantern" => Some(ThemeVariant::Lantern),
        "night-sky" | "nightsky" | "night_sky" => Some(ThemeVariant::NightSky),
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

/// Read a float value from a `[section]` in `lantern.toml`.
/// Returns `default` if the file/section/key is missing or unparseable.
pub fn read_config_f32(section: &str, key: &str, default: f32) -> f32 {
    let path = match lantern_config_path() {
        Some(p) => p,
        None => return default,
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return default,
    };
    let header = format!("[{}]", section);
    let mut in_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == header;
            continue;
        }
        if in_section {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == key {
                    return v.trim().trim_matches('"').parse().unwrap_or(default);
                }
            }
        }
    }
    default
}

/// Read a bool value from a `[section]` in `lantern.toml`.
/// Returns `default` if the file/section/key is missing or unparseable.
pub fn read_config_bool(section: &str, key: &str, default: bool) -> bool {
    let path = match lantern_config_path() {
        Some(p) => p,
        None => return default,
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return default,
    };
    let header = format!("[{}]", section);
    let mut in_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == header;
            continue;
        }
        if in_section {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == key {
                    return match v.trim() {
                        "true" => true,
                        "false" => false,
                        _ => default,
                    };
                }
            }
        }
    }
    default
}

/// Read a string value from a `[section]` in `lantern.toml`.
/// Returns `default` if the file/section/key is missing.
pub fn read_config_string(section: &str, key: &str, default: &str) -> String {
    let path = match lantern_config_path() {
        Some(p) => p,
        None => return default.to_string(),
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return default.to_string(),
    };
    let header = format!("[{}]", section);
    let mut in_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == header;
            continue;
        }
        if in_section {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == key {
                    return v.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    default.to_string()
}

/// Read the global background opacity from `[windows] background_opacity`.
/// Apps use this to make their background transparent while keeping text opaque.
pub fn background_opacity() -> f32 {
    read_config_f32("windows", "background_opacity", 1.0)
}

/// Read the user-configured background color from
/// `[appearance].background_color`. Returns `None` when the key is missing,
/// the value is empty/unparseable, or the config file is unavailable —
/// callers fall back to the variant's built-in surface color in that case.
pub fn active_background_color() -> Option<crate::Rgba> {
    let path = lantern_config_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let mut in_appearance = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_appearance = trimmed == "[appearance]";
            continue;
        }
        if in_appearance {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == "background_color" {
                    let hex = v.trim().trim_matches('"').trim_matches('\'');
                    if hex.is_empty() { return None; }
                    return parse_hex_rgb(hex);
                }
            }
        }
    }
    None
}

/// Read the user-configured accent color from `[appearance].accent`. Returns
/// `None` when the key is missing, the value is not a parseable hex string,
/// or the config file is unavailable — callers fall back to the variant's
/// built-in accent in that case.
pub fn active_accent() -> Option<crate::Rgba> {
    let path = lantern_config_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let mut in_appearance = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_appearance = trimmed == "[appearance]";
            continue;
        }
        if in_appearance {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == "accent" {
                    let hex = v.trim().trim_matches('"').trim_matches('\'');
                    return parse_hex_rgb(hex);
                }
            }
        }
    }
    None
}

/// Parse a `#RGB`, `#RRGGBB`, or `#RRGGBBAA` hex string into `Rgba`. Returns
/// `None` for malformed input. Alpha defaults to 255 when absent.
fn parse_hex_rgb(s: &str) -> Option<crate::Rgba> {
    let s = s.strip_prefix('#').unwrap_or(s);
    let (r, g, b, a) = match s.len() {
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).ok()?;
            let g = u8::from_str_radix(&s[1..2], 16).ok()?;
            let b = u8::from_str_radix(&s[2..3], 16).ok()?;
            (r * 17, g * 17, b * 17, 255)
        }
        6 => (
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
            255,
        ),
        8 => (
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
            u8::from_str_radix(&s[6..8], 16).ok()?,
        ),
        _ => return None,
    };
    Some(crate::Rgba::rgba(r, g, b, a))
}
