//! What an app takes from the desktop's settings so it looks like it
//! belongs: the accent, how see-through windows are, and the font. Read
//! from `lantern.toml`, which System Settings writes; [`Follow`] keeps a
//! running app in step with it.

use std::path::PathBuf;
use std::time::SystemTime;

use lntrn_data::{Doc, toml};
use lntrn_math::Color;
use lntrn_ui::{Host, Shell};

use crate::look;

/// The family every Lantern app falls back to.
pub const DEFAULT_FAMILY: &str = "Inter";
/// Seconds between looks at the file for a change.
const LOOK_EVERY: f64 = 1.0;

/// The family a stored `font_family` means: empty and the legacy generic
/// `sans-serif` are the default.
pub fn effective_family(stored: &str) -> String {
    let s = stored.trim();
    if s.is_empty() || s == "sans-serif" { DEFAULT_FAMILY.to_owned() } else { s.to_owned() }
}

/// `~/.lantern/config/lantern.toml`.
pub fn config_path() -> PathBuf {
    lntrn_sys::dirs::lantern_config().unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern/config")).join("lantern.toml")
}

/// The desktop's look as an app needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Desktop {
    pub accent: Color,
    /// How solid window backgrounds are, 0.05 to 1.
    pub opacity: f64,
    /// The proportional font family.
    pub font: String,
}

impl Default for Desktop {
    fn default() -> Self {
        Self { accent: look::GOLD, opacity: 1.0, font: DEFAULT_FAMILY.to_owned() }
    }
}

impl Desktop {
    /// What `lantern.toml` says; the defaults for whatever it doesn't (or
    /// when it is missing or won't parse).
    pub fn read() -> Desktop {
        std::fs::read_to_string(config_path()).ok().and_then(|text| toml::parse(&text).ok()).map(|doc| Self::from_doc(&doc)).unwrap_or_default()
    }

    fn from_doc(doc: &Doc) -> Desktop {
        let text = |path: &str| doc.path(path).and_then(Doc::as_str).unwrap_or("");
        Desktop {
            accent: look::accent(text("appearance.accent")),
            opacity: doc.path("windows.background_opacity").and_then(Doc::as_f64).unwrap_or(1.0).clamp(0.05, 1.0),
            font: effective_family(text("appearance.font_family")),
        }
    }
}

/// Keeps a shell wearing the Lantern look with the desktop's accent and
/// opacity, picking up a change to `lantern.toml` within a second.
pub struct Follow {
    now: Desktop,
    stamp: Option<SystemTime>,
    looked: f64,
}

fn stamp() -> Option<SystemTime> {
    std::fs::metadata(config_path()).ok().and_then(|m| m.modified().ok())
}

impl Default for Follow {
    fn default() -> Self {
        Self { now: Desktop::read(), stamp: stamp(), looked: 0.0 }
    }
}

impl Follow {
    pub fn desktop(&self) -> &Desktop {
        &self.now
    }

    /// Put the look on `shell`, reading the file again when it has
    /// changed. Call from `AppHost::after_rebuild` and return what this
    /// returns: `true` when the shell changed, so the frame is rebuilt
    /// with it.
    pub fn apply<H: Host>(&mut self, shell: &mut Shell<H>) -> bool {
        let t = shell.state.now;
        if t - self.looked >= LOOK_EVERY {
            self.looked = t;
            let on_disk = stamp();
            if on_disk != self.stamp {
                self.stamp = on_disk;
                self.now = Desktop::read();
            }
        }
        // Wake to look again even when nothing else happens.
        shell.state.request_redraw_after(LOOK_EVERY);
        if (shell.opacity - self.now.opacity).abs() > 1e-3 {
            shell.opacity = self.now.opacity;
        }
        let theme = look::theme(self.now.accent);
        let restyled = shell.prefs.theme != theme;
        if restyled {
            shell.prefs.theme = theme;
        }
        restyled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktops_look_is_read_and_what_is_missing_defaults() {
        let doc = toml::parse("[appearance]\naccent = \"#2563EB\"\nfont_family = \"sans-serif\"\n\n[windows]\nbackground_opacity = 0.0\n").unwrap();
        let d = Desktop::from_doc(&doc);
        assert_eq!(d.accent, Color::hex(0x2563EB));
        assert_eq!(d.opacity, 0.05, "never fully see-through");
        assert_eq!(d.font, "Inter");
        assert_eq!(Desktop::from_doc(&toml::parse("").unwrap()), Desktop::default());
        assert_eq!(effective_family("  Fira Sans "), "Fira Sans");
    }
}
