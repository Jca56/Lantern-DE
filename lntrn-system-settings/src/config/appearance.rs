//! `[appearance]`, `[window_manager]` and `[windows]`: how the desktop
//! looks. Colours are hex strings on disk, as the compositor reads them;
//! an empty colour means "not set".

use lntrn_props::props;

props! {
    /// The desktop's look: window style, accent, font, background.
    pub struct Appearance {
        /// `"lantern"` or `"fox-dark"`: the window chrome every app draws.
        pub theme: String = "fox-dark".to_owned() => { id: 1 },
        /// Accent colour, hex.
        pub accent: String = "#FFC800".to_owned() => { id: 2 },
        /// Proportional font family for every Lantern app.
        pub font_family: String = "sans-serif".to_owned() => { id: 3 },
        pub font_size: f64 = 16.0 => { id: 4, hard: 10.0..=32.0, step: 1.0 },
        /// Global wallpaper; per-monitor entries override it.
        pub wallpaper: String = String::new() => { id: 5 },
        /// Slug of the theme preset last applied; empty when none.
        pub active_theme: String = String::new() => { id: 6 },
        /// Window background override, hex; empty for the style's own.
        pub background_color: String = String::new() => { id: 7 },
        /// Five glow positions (top-left, top-right, bottom-left,
        /// bottom-right, centre) as hex colours; empty = off.
        pub window_gradient_stops: Vec<String> = Vec::new() => { id: 8 },
        /// One opacity per glow position, 0–1.
        pub window_gradient_stop_alphas: Vec<f64> = Vec::new() => { id: 9 },
        /// Legacy; kept so old configs stay whole.
        pub window_gradient_direction: String = "diagonal".to_owned() => { id: 10 },
        /// Glow radius as a fraction of the window's half-diagonal.
        pub window_gradient_radius: f64 = 0.5 => { id: 11, hard: 0.1..=1.0, step: 0.05 },
    }
}

/// How many glow positions `window_gradient_stops` holds.
pub const GRADIENT_STOPS: usize = 5;

impl Appearance {
    /// Grow the glow lists to their five slots without changing what is set.
    pub fn normalize_gradient(&mut self) {
        self.window_gradient_stops.resize(GRADIENT_STOPS, String::new());
        let alpha = self.window_gradient_stop_alphas.first().copied().unwrap_or(1.0);
        self.window_gradient_stop_alphas.resize(GRADIENT_STOPS, alpha);
    }

    pub fn clamp(&mut self) {
        self.font_size = self.font_size.clamp(10.0, 32.0);
        self.window_gradient_radius = self.window_gradient_radius.clamp(0.1, 1.0);
        for a in &mut self.window_gradient_stop_alphas {
            *a = a.clamp(0.0, 1.0);
        }
    }
}

props! {
    /// Window borders, title bars, gaps and the focus glow.
    pub struct WindowManager {
        pub border_width: i64 = 2 => { id: 1, hard: 0..=10 },
        pub border_color: String = "#4A9EFF".to_owned() => { id: 2 },
        pub titlebar_height: i64 = 36 => { id: 3, hard: 20..=60 },
        pub gap: i64 = 8 => { id: 4, hard: 0..=32 },
        pub corner_radius: i64 = 10 => { id: 5, hard: 0..=20 },
        pub focus_follows_mouse: bool = false => { id: 6 },
        pub focus_glow: bool = true => { id: 7 },
        pub focus_glow_color: String = "#4A9EFF".to_owned() => { id: 8 },
        pub focus_glow_intensity: f64 = 0.2 => { id: 9, hard: 0.0..=0.6, step: 0.05 },
    }
}

impl WindowManager {
    pub fn clamp(&mut self) {
        self.border_width = self.border_width.clamp(0, 10);
        self.titlebar_height = self.titlebar_height.clamp(20, 60);
        self.gap = self.gap.clamp(0, 32);
        self.corner_radius = self.corner_radius.clamp(0, 20);
        self.focus_glow_intensity = self.focus_glow_intensity.clamp(0.0, 0.6);
    }
}

props! {
    /// Compositor effects behind windows, and the sizes new windows open at.
    pub struct Windows {
        pub blur_intensity: f64 = 0.8 => { id: 1, hard: 0.0..=1.0 },
        pub blur_tint: f64 = 0.15 => { id: 2, hard: 0.0..=1.0 },
        pub blur_tint_color: String = "#4A9EFF".to_owned() => { id: 3 },
        pub blur_darken: f64 = 0.0 => { id: 4, hard: 0.0..=1.0 },
        pub background_opacity: f64 = 1.0 => { id: 5, hard: 0.0..=1.0 },
        /// App ids the blur skips.
        pub blur_exclude: Vec<String> = Vec::new() => { id: 6 },
        /// Size new windows open at, percent of the work area on each axis.
        pub default_size_pct: i64 = 60 => { id: 7, hard: 1..=100 },
        /// Rungs of the Super+Shift+Up/Down resize ladder.
        pub size_small_pct: i64 = 30 => { id: 8, hard: 1..=100 },
        pub size_medium_pct: i64 = 60 => { id: 9, hard: 1..=100 },
        pub size_large_pct: i64 = 85 => { id: 10, hard: 1..=100 },
        pub size_xlarge_pct: i64 = 100 => { id: 11, hard: 1..=100 },
    }
}

impl Windows {
    pub fn clamp(&mut self) {
        self.blur_intensity = self.blur_intensity.clamp(0.0, 1.0);
        self.blur_tint = self.blur_tint.clamp(0.0, 1.0);
        self.blur_darken = self.blur_darken.clamp(0.0, 1.0);
        self.background_opacity = self.background_opacity.clamp(0.0, 1.0);
        for v in [&mut self.default_size_pct, &mut self.size_small_pct, &mut self.size_medium_pct, &mut self.size_large_pct, &mut self.size_xlarge_pct] {
            *v = (*v).clamp(1, 100);
        }
    }
}
