//! Visualizer color themes for the spectrum bars, plus tiny persistence so the
//! user's pick survives restarts. Each theme is a 5-stop gradient sampled across
//! the frequency range (bass → highs), matching `draw_classic_bars`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// A named 5-stop gradient (bass → highs).
pub struct VisTheme {
    pub name: &'static str,
    pub stops: [(u8, u8, u8); 5],
}

/// All selectable visualizer themes. Index into this is what gets persisted.
pub const VIS_THEMES: &[VisTheme] = &[
    VisTheme {
        name: "Sunset",
        stops: [
            (250, 180, 0),   // Lantern gold — bass
            (255, 130, 30),  // warm orange
            (255, 90, 140),  // coral / pink
            (200, 100, 230), // magenta
            (130, 120, 250), // violet — highs
        ],
    },
    VisTheme {
        name: "Lantern Gold",
        stops: [
            (250, 200, 0),
            (250, 175, 20),
            (240, 150, 30),
            (220, 130, 40),
            (200, 110, 50),
        ],
    },
    VisTheme {
        name: "Ocean",
        stops: [
            (40, 220, 200),
            (30, 180, 220),
            (40, 130, 230),
            (60, 90, 220),
            (110, 80, 230),
        ],
    },
    VisTheme {
        name: "Aurora",
        stops: [
            (60, 230, 140),
            (50, 220, 190),
            (70, 200, 230),
            (130, 150, 240),
            (200, 120, 240),
        ],
    },
    VisTheme {
        name: "Magma",
        stops: [
            (255, 230, 120),
            (255, 170, 40),
            (240, 90, 30),
            (200, 40, 60),
            (140, 20, 70),
        ],
    },
    VisTheme {
        name: "Neon",
        stops: [
            (0, 255, 170),
            (60, 230, 255),
            (120, 130, 255),
            (220, 90, 255),
            (255, 70, 200),
        ],
    },
    VisTheme {
        name: "Mono",
        stops: [
            (240, 240, 245),
            (210, 210, 220),
            (180, 180, 195),
            (150, 150, 170),
            (120, 120, 145),
        ],
    },
    VisTheme {
        name: "Forest",
        stops: [
            (210, 230, 120),
            (150, 215, 90),
            (90, 195, 90),
            (50, 165, 110),
            (30, 130, 120),
        ],
    },
];

pub fn theme_count() -> usize {
    VIS_THEMES.len()
}

pub fn theme(index: usize) -> &'static VisTheme {
    &VIS_THEMES[index.min(VIS_THEMES.len() - 1)]
}

// ── Persistence ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct VisSettings {
    theme_index: usize,
}

impl Default for VisSettings {
    fn default() -> Self {
        Self { theme_index: 0 }
    }
}

fn settings_path() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("lntrn-media-player");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("vis-theme.json"))
}

/// Load the persisted visualizer theme index (defaults to 0 = Sunset).
pub fn load_theme_index() -> usize {
    settings_path()
        .and_then(|p| fs::read_to_string(&p).ok())
        .and_then(|d| serde_json::from_str::<VisSettings>(&d).ok())
        .map(|s| s.theme_index.min(VIS_THEMES.len() - 1))
        .unwrap_or(0)
}

/// Persist the chosen visualizer theme index.
pub fn save_theme_index(index: usize) {
    if let Some(p) = settings_path() {
        let s = VisSettings {
            theme_index: index.min(VIS_THEMES.len() - 1),
        };
        if let Ok(d) = serde_json::to_string_pretty(&s) {
            let _ = fs::write(&p, d);
        }
    }
}
