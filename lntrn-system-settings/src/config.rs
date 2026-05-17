use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Window chrome mode ───────────────────────────────────────────────────────

/// Visual style of the system-settings window chrome. Maps from the unified
/// `[appearance].theme` key onto a chrome mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    Fox,
    Lantern,
}

impl Default for WindowMode {
    fn default() -> Self { Self::Fox }
}

// ── Top-level Lantern config ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanternConfig {
    pub appearance: AppearanceConfig,
    pub window_manager: WmConfig,
    pub windows: WindowsConfig,
    pub input: InputConfig,
    pub display: DisplayConfig,
    pub power: PowerConfig,
    pub notifications: NotificationsConfig,
    pub animations: AnimationsConfig,
    #[serde(default)]
    pub monitors: Vec<MonitorEntry>,
}

// ── Animations ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimationsConfig {
    /// Master toggle. When false, all window animations complete instantly.
    pub enabled: bool,
    /// Speed multiplier. Higher = faster. 1.0 = stock cinematic.
    pub speed: f32,
    /// Named curve set. One of: "cinematic", "snappy", "springy", "linear".
    /// Each preset maps to a specific easing per category in
    /// `lntrn-compositor::animations`.
    pub preset: String,
    /// Per-category enables. When a category is false, that animation
    /// completes in a single frame (1ms) — same path as the master toggle.
    pub open_close: bool,
    pub state: bool,
    pub minimize: bool,
    pub tiling: bool,
    pub workspace: bool,
}

impl Default for AnimationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
            preset: "cinematic".into(),
            open_close: true,
            state: true,
            minimize: true,
            tiling: true,
            workspace: true,
        }
    }
}

// ── Appearance ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Active theme variant — `"fox-dark"` or `"lantern"`. Read by every
    /// Lantern app via `lntrn_theme::active_variant()`.
    pub theme: String,
    /// Accent color override (hex). Read by `lntrn_theme::active_accent()`
    /// and applied to `FoxPalette::current().accent` everywhere.
    pub accent: String,
    pub font_family: String,
    pub font_size: f32,
    pub wallpaper: String,
    /// Slug of the currently-applied theme preset (set by Themes UI). Empty
    /// when no theme has ever been applied or the preset was deleted. We
    /// don't try to detect drift from theme — see `themes.rs` for why.
    pub active_theme: String,
    /// Custom window background hex override. Empty = use the variant default
    /// from `theme`. Set by the Background Color swatch picker; chrome.rs
    /// reads it via lntrn_theme so other apps can opt in.
    pub background_color: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "fox-dark".into(),
            accent: "#C8860A".into(),
            font_family: "sans-serif".into(),
            font_size: 16.0,
            wallpaper: String::new(),
            active_theme: String::new(),
            background_color: String::new(),
        }
    }
}

impl AppearanceConfig {
    pub fn window_mode(&self) -> WindowMode {
        // Map the unified theme key onto the chrome's two-mode enum.
        match self.theme.as_str() {
            "lantern" => WindowMode::Lantern,
            _ => WindowMode::Fox,
        }
    }
}

// ── Window manager ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WmConfig {
    pub border_width: u32,
    pub border_color: String,
    pub titlebar_height: u32,
    pub gap: u32,
    pub corner_radius: u32,
    pub focus_follows_mouse: bool,
    pub focus_glow: bool,
    pub focus_glow_color: String,
    pub focus_glow_intensity: f32,
}

impl Default for WmConfig {
    fn default() -> Self {
        Self {
            border_width: 2,
            border_color: "#4A9EFF".into(),
            titlebar_height: 36,
            gap: 8,
            corner_radius: 10,
            focus_follows_mouse: false,
            focus_glow: true,
            focus_glow_color: "#4A9EFF".into(),
            focus_glow_intensity: 0.2,
        }
    }
}

// ── Windows (compositor visual effects) ──────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowsConfig {
    pub blur_intensity: f32,
    pub blur_tint: f32,
    pub blur_tint_color: String,
    pub blur_darken: f32,
    pub background_opacity: f32,
    pub blur_exclude: Vec<String>,
}

impl Default for WindowsConfig {
    fn default() -> Self {
        Self {
            blur_intensity: 0.8,
            blur_tint: 0.15,
            blur_tint_color: "#4A9EFF".into(),
            blur_darken: 0.0,
            background_opacity: 1.0,
            blur_exclude: Vec::new(),
        }
    }
}

// ── Input ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    pub mouse_speed: f32,
    /// libinput acceleration profile: true = adaptive, false = flat.
    pub pointer_acceleration: bool,
    /// Scroll wheel speed multiplier (0.25 – 3.0, default 1.0).
    pub scroll_speed: f32,
    /// File-manager click behavior: true = require double-click to open files
    /// and folders, false = single-click opens.
    pub double_click_to_open: bool,
    /// Cursor size in pixels (16 – 64, default 24).
    pub cursor_size: u32,
    pub cursor_theme: String,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            mouse_speed: 0.0,
            pointer_acceleration: true,
            scroll_speed: 1.0,
            double_click_to_open: false,
            cursor_size: 24,
            cursor_theme: "default".into(),
        }
    }
}

// ── Display ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Global default scale, used by the compositor when an output has no
    /// per-monitor `[[monitors]] scale` entry.
    pub scale: f32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            scale: 1.0,
        }
    }
}

// ── Monitor arrangement ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorEntry {
    pub name: String,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub resolution: String,
    #[serde(default)]
    pub refresh_rate: String,
    #[serde(default = "default_monitor_scale")]
    pub scale: f32,
    #[serde(default)]
    pub wallpaper: String,
}

fn default_monitor_scale() -> f32 { 1.25 }


// ── Power ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerConfig {
    pub lid_close_action: String,       // "suspend", "hibernate", "lock", "nothing"
    pub lid_close_on_ac: String,        // same options, when plugged in
    pub dim_after: u32,                 // seconds before screen dims (0 = never)
    pub idle_timeout: u32,              // seconds before idle action
    pub idle_action: String,            // "suspend", "lock", "nothing"
    pub low_battery_threshold: u32,     // percentage for warning
    pub critical_battery_threshold: u32, // percentage for critical action
    pub critical_battery_action: String, // "suspend", "hibernate", "shutdown", "nothing"
    pub wifi_power_save: bool,          // true = power saving on, false = always active
    pub wifi_power_scheme: String,      // "active", "balanced", "battery"
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            lid_close_action: "suspend".into(),
            lid_close_on_ac: "lock".into(),
            dim_after: 120,
            idle_timeout: 300,
            idle_action: "suspend".into(),
            low_battery_threshold: 15,
            critical_battery_threshold: 5,
            critical_battery_action: "hibernate".into(),
            wifi_power_save: true,
            wifi_power_scheme: "balanced".into(),
        }
    }
}

// ── Notifications ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// Master mute. When true, no toasts show and no sound plays.
    pub do_not_disturb: bool,
    /// Show toast popups when notifications arrive.
    pub show_toasts: bool,
    /// Play the notification chime sound.
    pub play_sound: bool,
    /// Notification chime volume (0.0 – 1.0).
    pub volume: f32,
    /// Default display duration for a toast in seconds (clamp 1.0–30.0).
    /// Apps that pass an explicit `expire_timeout > 0` override this.
    pub default_duration_secs: f32,
    /// Screen corner the toast stack anchors to.
    /// One of: "top-right", "top-left", "bottom-right", "bottom-left".
    pub position: String,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            do_not_disturb: false,
            show_toasts: true,
            play_sound: true,
            volume: 0.8,
            default_duration_secs: 5.0,
            position: "top-right".into(),
        }
    }
}

// ── Top-level default ────────────────────────────────────────────────────────

impl Default for LanternConfig {
    fn default() -> Self {
        Self {
            appearance: AppearanceConfig::default(),
            window_manager: WmConfig::default(),
            windows: WindowsConfig::default(),
            input: InputConfig::default(),
            display: DisplayConfig::default(),
            power: PowerConfig::default(),
            notifications: NotificationsConfig::default(),
            animations: AnimationsConfig::default(),
            monitors: Vec::new(),
        }
    }
}

// ── Load / Save ──────────────────────────────────────────────────────────────

impl LanternConfig {
    pub fn path() -> PathBuf {
        lntrn_theme::lantern_config_path().unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".lantern/config/lantern.toml")
        })
    }

    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let mut config: Self = toml::from_str(&contents).unwrap_or_default();
            config.sanitize();
            config
        } else {
            let config = Self::default();
            config.save();
            config
        }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(toml_str) = toml::to_string_pretty(self) {
            std::fs::write(&path, toml_str).ok();
        }
    }
}

impl LanternConfig {
    fn sanitize(&mut self) {
        self.appearance.font_size = self.appearance.font_size.clamp(10.0, 32.0);
        self.window_manager.border_width = self.window_manager.border_width.clamp(0, 10);
        self.window_manager.titlebar_height = self.window_manager.titlebar_height.clamp(20, 60);
        self.window_manager.gap = self.window_manager.gap.clamp(0, 32);
        self.window_manager.corner_radius = self.window_manager.corner_radius.clamp(0, 20);
        if lntrn_render::Color::from_hex(&self.window_manager.focus_glow_color).is_none() {
            self.window_manager.focus_glow_color = "#4A9EFF".into();
        }
        self.window_manager.focus_glow_intensity =
            self.window_manager.focus_glow_intensity.clamp(0.0, 0.6);
        self.windows.blur_intensity = self.windows.blur_intensity.clamp(0.0, 1.0);
        self.windows.blur_tint = self.windows.blur_tint.clamp(0.0, 1.0);
        self.windows.blur_darken = self.windows.blur_darken.clamp(0.0, 1.0);
        self.windows.background_opacity = self.windows.background_opacity.clamp(0.0, 1.0);
        self.input.mouse_speed = self.input.mouse_speed.clamp(-1.0, 1.0);
        self.input.scroll_speed = self.input.scroll_speed.clamp(0.25, 3.0);
        self.input.cursor_size = self.input.cursor_size.clamp(16, 64);
        self.display.scale = self.display.scale.clamp(0.5, 3.0);
        if !["active", "balanced", "battery"].contains(&self.power.wifi_power_scheme.as_str()) {
            self.power.wifi_power_scheme = "balanced".into();
        }
        self.animations.speed = self.animations.speed.clamp(0.25, 3.0);
        if !["cinematic", "snappy", "springy", "linear"]
            .contains(&self.animations.preset.as_str())
        {
            self.animations.preset = "cinematic".into();
        }
        self.notifications.volume = self.notifications.volume.clamp(0.0, 1.0);
        self.notifications.default_duration_secs =
            self.notifications.default_duration_secs.clamp(1.0, 30.0);
        if !["top-right", "top-left", "bottom-right", "bottom-left"]
            .contains(&self.notifications.position.as_str())
        {
            self.notifications.position = "top-right".into();
        }
    }
}
