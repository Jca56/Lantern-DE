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
    pub lockscreen: LockScreenConfig,
    #[serde(default)]
    pub monitors: Vec<MonitorEntry>,
}

// ── Lock screen ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockScreenConfig {
    /// Absolute path to the lock screen background image. Empty = no override
    /// (the lock screen binary falls back to its built-in default).
    pub background: String,
    /// Password field border color (hex). Empty = use the theme accent.
    #[serde(default)]
    pub border_color: String,
    /// Password field border thickness in pixels.
    #[serde(default = "default_lock_border_thickness")]
    pub border_thickness: f32,
    /// Password field fill color (hex).
    #[serde(default = "default_lock_field_color")]
    pub field_color: String,
    /// Password field fill opacity (0.0–1.0).
    #[serde(default = "default_lock_field_opacity")]
    pub field_opacity: f32,
    /// Password dot color (hex).
    #[serde(default = "default_lock_dot_color")]
    pub dot_color: String,
    /// Scrim opacity darkening the wallpaper behind the lock UI (0.0–1.0).
    #[serde(default = "default_lock_scrim_opacity")]
    pub scrim_opacity: f32,
}

fn default_lock_border_thickness() -> f32 { 2.0 }
fn default_lock_field_color() -> String { "#000000".into() }
fn default_lock_field_opacity() -> f32 { 0.55 }
fn default_lock_dot_color() -> String { "#F5F5F5".into() }
fn default_lock_scrim_opacity() -> f32 { 0.38 }

impl Default for LockScreenConfig {
    fn default() -> Self {
        Self {
            background: String::new(),
            border_color: String::new(),
            border_thickness: default_lock_border_thickness(),
            field_color: default_lock_field_color(),
            field_opacity: default_lock_field_opacity(),
            dot_color: default_lock_dot_color(),
            scrim_opacity: default_lock_scrim_opacity(),
        }
    }
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
    /// True multi-stop window background gradient (3 or 4 hex colors,
    /// evenly spaced top→bottom). When set and length ≥ 2, takes precedence
    /// over `background_color`. Empty vec = disabled (use solid bg).
    /// Read by `lntrn_theme::active_window_gradient()`.
    #[serde(default)]
    pub window_gradient_stops: Vec<String>,
    /// Per-stop alpha multipliers (0.0–1.0). One entry per stop; 0.0 = the
    /// stop is fully transparent (lets the wallpaper / desktop show through
    /// at that position), 1.0 = fully opaque color. Multiplied into the
    /// final stop alpha alongside `[windows].background_opacity`. Missing
    /// or empty = treat as all 1.0. Read by
    /// `lntrn_theme::active_window_gradient_alphas()`.
    #[serde(default)]
    pub window_gradient_stop_alphas: Vec<f32>,
    /// Legacy direction preset (now unused — superseded by per-position
    /// glow toggles). Kept so existing configs deserialize without errors.
    #[serde(default = "default_gradient_direction")]
    pub window_gradient_direction: String,
    /// Glow radius for the per-position window gradient, as a 0..1
    /// fraction of the focused window's half-diagonal. Default 0.5.
    /// Read by `lntrn_theme::active_window_gradient_radius()`.
    #[serde(default = "default_gradient_radius")]
    pub window_gradient_radius: f32,
}

fn default_gradient_direction() -> String { "diagonal".into() }
fn default_gradient_radius() -> f32 { 0.5 }

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
            window_gradient_stops: Vec::new(),
            window_gradient_stop_alphas: Vec::new(),
            window_gradient_direction: default_gradient_direction(),
            window_gradient_radius: default_gradient_radius(),
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
    /// Initial size for newly-opened windows, as a percentage (1..=100) of
    /// the output's work area on each axis.
    #[serde(default = "default_pct_default")]
    pub default_size_pct: u32,
    /// Tiny rung of the Super+Shift+Down ladder.
    #[serde(default = "default_pct_small")]
    pub size_small_pct: u32,
    /// "Normal" / Middle rung — also used as the centered Middle pose rect.
    #[serde(default = "default_pct_medium")]
    pub size_medium_pct: u32,
    /// SoloTile rung — Super+Shift+Up step from Normal.
    #[serde(default = "default_pct_large")]
    pub size_large_pct: u32,
    /// Top rung of the resize ladder — near-fullscreen.
    #[serde(default = "default_pct_xlarge")]
    pub size_xlarge_pct: u32,
}

fn default_pct_default() -> u32 { 60 }
fn default_pct_small() -> u32 { 30 }
fn default_pct_medium() -> u32 { 60 }
fn default_pct_large() -> u32 { 85 }
fn default_pct_xlarge() -> u32 { 100 }

impl Default for WindowsConfig {
    fn default() -> Self {
        Self {
            blur_intensity: 0.8,
            blur_tint: 0.15,
            blur_tint_color: "#4A9EFF".into(),
            blur_darken: 0.0,
            background_opacity: 1.0,
            blur_exclude: Vec::new(),
            default_size_pct: default_pct_default(),
            size_small_pct: default_pct_small(),
            size_medium_pct: default_pct_medium(),
            size_large_pct: default_pct_large(),
            size_xlarge_pct: default_pct_xlarge(),
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
    /// Five-stop recolor palette for the bundled Lantern cursors. Each
    /// stop maps to a canonical hex code authored into the SVGs:
    /// `#ffffff` body-light, `#ababab` body-dark, `#fab414` accent-light,
    /// `#9a6300` accent-dark, `#0a0a0a` outline. Custom-theme SVGs (in
    /// `~/.lantern/config/cursors/`) are not retinted — themes ship the
    /// look they want.
    #[serde(default = "default_cursor_body_light")]
    pub cursor_body_light: String,
    #[serde(default = "default_cursor_body_dark")]
    pub cursor_body_dark: String,
    #[serde(default = "default_cursor_accent_light")]
    pub cursor_accent_light: String,
    #[serde(default = "default_cursor_accent_dark")]
    pub cursor_accent_dark: String,
    #[serde(default = "default_cursor_outline_color")]
    pub cursor_outline_color: String,
    /// Multiplier on every authored SVG `stroke-width`. 1.0 keeps the
    /// strokes as authored; 2.0 doubles them; 0.0 hides outlines.
    /// Clamped to 0.0..=3.0 by `sanitize`.
    #[serde(default = "default_cursor_outline_scale")]
    pub cursor_outline_scale: f32,
    /// 0..=1 corner-rounding factor for the bundled default pointer. 0 =
    /// stock sharp tip, 1 = max-rounded pebble. Only affects
    /// `lntrn-cursor.svg`; other cursors keep their authored geometry.
    #[serde(default = "default_cursor_corner_radius")]
    pub cursor_corner_radius: f32,
    /// Show a ripple animation around the cursor on left-click. Compositor
    /// reads this live; toggling here takes effect on the next click.
    #[serde(default = "default_click_anim_enabled")]
    pub click_anim_enabled: bool,
    /// Scale multiplier for the click ripple. 1.0 = baseline (~70% of
    /// cursor diameter at peak); clamped to 0.25..=3.0 by `sanitize`.
    #[serde(default = "default_click_anim_size")]
    pub click_anim_size: f32,
    /// Hex color for the ripple. Empty string = inherit from
    /// `cursor_outline` so the ripple matches the cursor stroke.
    #[serde(default = "default_click_anim_color")]
    pub click_anim_color: String,
    /// Ripple style: "rings" today; reserved for future variants
    /// ("burst", "wave", etc.) so we can swap animations without
    /// renaming any config keys.
    #[serde(default = "default_click_anim_style")]
    pub click_anim_style: String,
}

fn default_cursor_body_light() -> String   { "#ffffff".into() }
fn default_cursor_body_dark() -> String    { "#ababab".into() }
fn default_cursor_accent_light() -> String { "#fab414".into() }
fn default_cursor_accent_dark() -> String  { "#9a6300".into() }
fn default_cursor_outline_color() -> String { "#0a0a0a".into() }
fn default_cursor_outline_scale() -> f32   { 1.0 }
fn default_cursor_corner_radius() -> f32   { 0.0 }
fn default_click_anim_enabled() -> bool { true }
fn default_click_anim_size() -> f32 { 1.0 }
fn default_click_anim_color() -> String { String::new() }
fn default_click_anim_style() -> String { "rings".into() }

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            mouse_speed: 0.0,
            pointer_acceleration: true,
            scroll_speed: 1.0,
            double_click_to_open: false,
            cursor_size: 24,
            cursor_theme: "default".into(),
            cursor_body_light:    default_cursor_body_light(),
            cursor_body_dark:     default_cursor_body_dark(),
            cursor_accent_light:  default_cursor_accent_light(),
            cursor_accent_dark:   default_cursor_accent_dark(),
            cursor_outline_color: default_cursor_outline_color(),
            cursor_outline_scale: default_cursor_outline_scale(),
            cursor_corner_radius: default_cursor_corner_radius(),
            click_anim_enabled: default_click_anim_enabled(),
            click_anim_size: default_click_anim_size(),
            click_anim_color: default_click_anim_color(),
            click_anim_style: default_click_anim_style(),
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

fn default_monitor_scale() -> f32 { 1.0 }


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
            lockscreen: LockScreenConfig::default(),
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
        for a in self.appearance.window_gradient_stop_alphas.iter_mut() {
            *a = a.clamp(0.0, 1.0);
        }
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
        self.input.cursor_size = self.input.cursor_size.clamp(16, 128);
        self.input.cursor_outline_scale = self.input.cursor_outline_scale.clamp(0.0, 3.0);
        self.input.cursor_corner_radius = self.input.cursor_corner_radius.clamp(0.0, 1.0);
        self.input.click_anim_size = self.input.click_anim_size.clamp(0.25, 3.0);
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
