//! Surgical edits to the shared compositor config (~/.lantern/config/
//! lantern.toml). Line-based patching only — the file manager doesn't
//! mirror the compositor's full config struct, and a partial-struct
//! rewrite would silently delete every key it doesn't know about.

use std::path::Path;

/// Point `[appearance] wallpaper` at `image` and drop any per-monitor
/// `wallpaper` overrides so the new image shows on every output. The
/// compositor polls the file's mtime and live-reloads within ~500ms —
/// no IPC or restart needed.
pub fn set_wallpaper(image: &Path) -> Result<(), &'static str> {
    // The compositor reads this line without unescaping it, so a name that
    // needs escaping cannot be expressed at all. Refuse BEFORE patching:
    // patching would already have dropped the per-monitor overrides and the
    // compositor would fall back to its default picture.
    let shown = image.to_string_lossy();
    if shown.contains(['"', '\\']) || shown.chars().any(char::is_control) {
        return Err(
            "This file\u{2019}s name contains a quote, a backslash or a line break, \
             which the wallpaper setting can\u{2019}t hold. Rename the file and try again.",
        );
    }
    let cfg_path = lntrn_theme::lantern_config_path().ok_or("Lantern\u{2019}s config file wasn\u{2019}t found.")?;
    let content = std::fs::read_to_string(&cfg_path)
        .map_err(|_| "Lantern\u{2019}s config file couldn\u{2019}t be read.")?;
    let patched = patch_wallpaper_toml(&content, image);
    std::fs::write(&cfg_path, patched).map_err(|_| "Lantern\u{2019}s config file couldn\u{2019}t be written.")
}

/// Escape a value for a TOML basic (double-quoted) string. A file name can
/// hold quotes, backslashes and newlines; written raw they would end the
/// string early and let the rest of the name become config.
fn toml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn patch_wallpaper_toml(content: &str, image: &Path) -> String {
    let new_line = format!(
        "wallpaper = \"{}\"",
        toml_escape(&image.to_string_lossy())
    );
    let mut out: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut wrote = false;
    let mut has_appearance = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // Leaving [appearance] without having seen the key → insert it.
            if section == "[appearance]" && !wrote {
                out.push(new_line.clone());
                wrote = true;
            }
            section = trimmed.to_string();
            if section == "[appearance]" {
                has_appearance = true;
            }
            out.push(line.to_string());
            continue;
        }
        let is_wallpaper_key = trimmed
            .strip_prefix("wallpaper")
            .is_some_and(|rest| rest.trim_start().starts_with('='));
        if is_wallpaper_key {
            match section.as_str() {
                "[appearance]" => {
                    out.push(new_line.clone());
                    wrote = true;
                }
                // Per-output override would hide the new global wallpaper
                // on that monitor — drop it.
                "[[monitors]]" => {}
                _ => out.push(line.to_string()),
            }
            continue;
        }
        out.push(line.to_string());
    }

    if section == "[appearance]" && !wrote {
        out.push(new_line.clone());
        wrote = true;
    }
    if !has_appearance && !wrote {
        out.push(String::new());
        out.push("[appearance]".to_string());
        out.push(new_line);
    }

    let mut text = out.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn replaces_existing_key_and_clears_monitor_overrides() {
        let toml = "\
[appearance]
theme = \"dark\"
wallpaper = \"/old/wall.png\"

[[monitors]]
name = \"DP-1\"
wallpaper = \"/old/per-monitor.png\"
scale = 1.3

[window_manager]
border_width = 2
";
        let got = patch_wallpaper_toml(toml, &PathBuf::from("/new/pic.jpg"));
        assert!(got.contains("wallpaper = \"/new/pic.jpg\""));
        assert!(!got.contains("/old/wall.png"));
        assert!(!got.contains("/old/per-monitor.png"));
        // Untouched keys survive verbatim.
        assert!(got.contains("theme = \"dark\""));
        assert!(got.contains("scale = 1.3"));
        assert!(got.contains("border_width = 2"));
        // Only the [appearance] occurrence remains.
        assert_eq!(got.matches("wallpaper =").count(), 1);
    }

    #[test]
    fn escapes_names_that_would_break_out_of_the_string() {
        let evil = PathBuf::from("/p/a\"\nevil = \"x\\.png");
        let got = patch_wallpaper_toml("[appearance]\n", &evil);
        // Still exactly one line for the key, nothing injected after it.
        assert_eq!(
            got,
            "[appearance]\nwallpaper = \"/p/a\\\"\\u000Aevil = \\\"x\\\\.png\"\n"
        );
    }

    #[test]
    fn inserts_key_when_appearance_lacks_it_and_appends_section_when_missing() {
        let with_section = "[appearance]\ntheme = \"dark\"\n\n[input]\nspeed = 1\n";
        let got = patch_wallpaper_toml(with_section, &PathBuf::from("/p.png"));
        assert!(got.contains("wallpaper = \"/p.png\""));
        // Inserted inside [appearance], i.e. before the [input] header.
        assert!(got.find("wallpaper =").unwrap() < got.find("[input]").unwrap());

        let no_section = "[input]\nspeed = 1\n";
        let got = patch_wallpaper_toml(no_section, &PathBuf::from("/p.png"));
        assert!(got.ends_with("[appearance]\nwallpaper = \"/p.png\"\n"));
        assert!(got.contains("speed = 1"));
    }
}
