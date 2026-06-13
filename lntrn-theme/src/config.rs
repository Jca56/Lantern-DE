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

/// Active proportional font family — `font_family` under `[appearance]`.
///
/// The legacy generic value `"sans-serif"` (and empty/missing) resolves to
/// the theme default (`typography::FAMILY_PROPORTIONAL`), so existing
/// configs pick up the DE font without an edit. Read once at app startup
/// by `lntrn-render`'s font system; a change applies as apps restart.
pub fn active_font_family() -> String {
    let v = read_config_string("appearance", "font_family", "");
    let v = v.trim();
    if v.is_empty() || v == "sans-serif" {
        crate::typography::FAMILY_PROPORTIONAL.to_string()
    } else {
        v.to_string()
    }
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

/// Read the configured window gradient direction. Returns radians for the
/// `[appearance].window_gradient_direction` preset, defaulting to the
/// TL→BR diagonal when unset.
///
/// Accepted values (case-insensitive):
///   * `"diagonal"`         — π/4 (top-left → bottom-right). Default.
///   * `"diagonal-reverse"` — 3π/4 (top-right → bottom-left)
///   * `"vertical"`         — π/2 (top → bottom)
///   * `"horizontal"`       — 0   (left → right)
pub fn active_window_gradient_angle() -> f32 {
    let raw = read_config_string("appearance", "window_gradient_direction", "diagonal");
    window_gradient_angle_from_str(&raw)
}

/// Map a direction preset name to radians. Unknown / empty → diagonal default.
pub fn window_gradient_angle_from_str(s: &str) -> f32 {
    match s.trim().to_ascii_lowercase().as_str() {
        "horizontal" => 0.0,
        "vertical" => std::f32::consts::FRAC_PI_2,
        "diagonal-reverse" => 3.0 * std::f32::consts::FRAC_PI_4,
        // "diagonal" and any unknown value fall through to the TL→BR default
        _ => std::f32::consts::FRAC_PI_4,
    }
}

/// Map the same direction preset to a pair of normalized anchor points
/// (0..1 within the window rect) for the twin-radial gradient overlay.
/// Stop 0 lands at the first anchor, stop 1 at the second.
pub fn window_gradient_anchors_from_str(s: &str) -> ((f32, f32), (f32, f32)) {
    match s.trim().to_ascii_lowercase().as_str() {
        "horizontal"       => ((0.0, 0.5), (1.0, 0.5)),  // left  ↔ right
        "vertical"         => ((0.5, 0.0), (0.5, 1.0)),  // top   ↔ bottom
        "diagonal-reverse" => ((1.0, 0.0), (0.0, 1.0)),  // TR    ↔ BL
        _                  => ((0.0, 0.0), (1.0, 1.0)),  // TL    ↔ BR (default)
    }
}

/// Convenience for callers that just want the active anchors.
pub fn active_window_gradient_anchors() -> ((f32, f32), (f32, f32)) {
    let raw = read_config_string("appearance", "window_gradient_direction", "diagonal");
    window_gradient_anchors_from_str(&raw)
}

/// Position index for the per-corner / center gradient overlay.
/// Order matches the `window_gradient_stops` array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientCorner { TopLeft, TopRight, BottomLeft, BottomRight, Center }

impl GradientCorner {
    pub const ALL: [GradientCorner; 5] = [
        GradientCorner::TopLeft,
        GradientCorner::TopRight,
        GradientCorner::BottomLeft,
        GradientCorner::BottomRight,
        GradientCorner::Center,
    ];

    /// Normalized anchor (0..1 within the window rect).
    pub fn anchor(self) -> (f32, f32) {
        match self {
            GradientCorner::TopLeft     => (0.0, 0.0),
            GradientCorner::TopRight    => (1.0, 0.0),
            GradientCorner::BottomLeft  => (0.0, 1.0),
            GradientCorner::BottomRight => (1.0, 1.0),
            GradientCorner::Center      => (0.5, 0.5),
        }
    }
}

/// Read the per-corner gradient colours. Returns up to 5 entries (one per
/// `GradientCorner` position) where `None` means that position is disabled.
/// Pulled from the same `[appearance].window_gradient_stops` array; index
/// 0 → TopLeft, 1 → TopRight, 2 → BottomLeft, 3 → BottomRight, 4 → Center.
/// Empty strings or "off" mark the slot disabled.
pub fn active_window_gradient_corners() -> [Option<crate::Rgba>; 5] {
    let mut out = [None; 5];
    let Some(path) = lantern_config_path() else { return out; };
    let Ok(contents) = std::fs::read_to_string(&path) else { return out; };
    let mut in_appearance = false;
    let mut lines = contents.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains('=') {
            in_appearance = trimmed == "[appearance]";
            continue;
        }
        if !in_appearance { continue; }
        let Some((k, v)) = trimmed.split_once('=') else { continue; };
        if k.trim() != "window_gradient_stops" { continue; }
        let mut buf = v.trim().to_string();
        while !buf.contains(']') {
            let Some(next) = lines.next() else { break; };
            buf.push(' ');
            buf.push_str(next.trim());
        }
        let Some(rest) = buf.trim().strip_prefix('[') else { break; };
        let Some(inner) = rest.rsplit_once(']').map(|(a, _)| a) else { break; };
        for (i, raw) in inner.split(',').take(5).enumerate() {
            let s = raw.trim().trim_matches('"').trim_matches('\'');
            if s.is_empty() || s.eq_ignore_ascii_case("off") { continue; }
            if let Some(rgba) = parse_hex_rgb(s) {
                out[i] = Some(rgba);
            }
        }
        break;
    }
    out
}

/// Read the global gradient-glow radius (fraction of the window's
/// half-diagonal length). Clamped to 0..1. Default 0.5.
pub fn active_window_gradient_radius() -> f32 {
    read_config_f32("appearance", "window_gradient_radius", 0.5).clamp(0.0, 1.0)
}

/// Read the global gradient intensity (cube-curved at render time).
/// Default 1.0 — combined with the per-stop alpha. We continue to pull
/// from `window_gradient_stop_alphas[0]` so the existing slider in
/// settings keeps working while the per-stop alphas concept goes away.
pub fn active_window_gradient_intensity() -> f32 {
    active_window_gradient_alphas()
        .and_then(|v| v.first().copied())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

/// Read the user-configured multi-stop window gradient from
/// `[appearance].window_gradient_stops`. Returns the parsed list of colors
/// when present (typically 3 or 4 hex strings); `None` when missing, empty,
/// or unparseable — callers fall back to the solid background color.
///
/// Expected TOML form:
/// ```toml
/// [appearance]
/// window_gradient_stops = ["#1a1a1a", "#2a1f4f", "#5a2f8f"]
/// ```
pub fn active_window_gradient() -> Option<Vec<crate::Rgba>> {
    let path = lantern_config_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let mut in_appearance = false;
    let mut lines = contents.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains('=') {
            // Section header, e.g. `[appearance]`. Arrays-on-one-line look
            // like `key = [...]` and won't match this branch.
            in_appearance = trimmed == "[appearance]";
            continue;
        }
        if !in_appearance { continue; }
        let Some((k, v)) = trimmed.split_once('=') else { continue; };
        if k.trim() != "window_gradient_stops" { continue; }
        // Collect array body. May be `["a","b"]` on one line, or span
        // multiple lines (toml::to_string_pretty does this past N entries).
        let mut buf = v.trim().to_string();
        while !buf.contains(']') {
            let Some(next) = lines.next() else { break; };
            buf.push(' ');
            buf.push_str(next.trim());
        }
        let inner = buf.trim().strip_prefix('[')?;
        let inner = inner.rsplit_once(']')?.0;
        let mut stops: Vec<crate::Rgba> = inner
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\''))
            .filter(|s| !s.is_empty())
            .filter_map(parse_hex_rgb)
            .collect();
        if stops.len() < 2 { return None; }
        // Lantern caps window gradients at 2 stops — the shader's 2-stop
        // path is a clean single `mix`, which avoids the visible kinks
        // at intermediate stops in 3- and 4-stop gradients. Legacy
        // configs with more stops collapse to first+last so the user's
        // overall color intent is preserved.
        if stops.len() > 2 {
            let last = *stops.last().unwrap();
            stops.truncate(1);
            stops.push(last);
        }
        return Some(stops);
    }
    None
}

/// Read per-stop alpha multipliers from
/// `[appearance].window_gradient_stop_alphas`. Returns the parsed list when
/// present; `None` when missing, empty, or unparseable — callers treat each
/// missing entry as 1.0 (fully opaque). Values are clamped to `0.0..=1.0`.
///
/// Expected TOML form (any length; one entry per stop):
/// ```toml
/// [appearance]
/// window_gradient_stop_alphas = [1.0, 0.6, 0.0, 1.0]
/// ```
pub fn active_window_gradient_alphas() -> Option<Vec<f32>> {
    let path = lantern_config_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let mut in_appearance = false;
    let mut lines = contents.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains('=') {
            in_appearance = trimmed == "[appearance]";
            continue;
        }
        if !in_appearance { continue; }
        let Some((k, v)) = trimmed.split_once('=') else { continue; };
        if k.trim() != "window_gradient_stop_alphas" { continue; }
        let mut buf = v.trim().to_string();
        while !buf.contains(']') {
            let Some(next) = lines.next() else { break; };
            buf.push(' ');
            buf.push_str(next.trim());
        }
        let inner = buf.trim().strip_prefix('[')?;
        let inner = inner.rsplit_once(']')?.0;
        let mut alphas: Vec<f32> = inner
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f32>().ok())
            .map(|a| a.clamp(0.0, 1.0))
            .collect();
        if alphas.is_empty() { return None; }
        // Match the 2-stop cap in `active_window_gradient`: collapse a
        // legacy 4-entry alpha list to first+last so it stays paired.
        if alphas.len() > 2 {
            let last = *alphas.last().unwrap();
            alphas.truncate(1);
            alphas.push(last);
        }
        return Some(alphas);
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
