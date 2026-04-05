use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

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
}

// ── Appearance ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: String,
    pub accent_color: String,
    pub font_family: String,
    pub font_size: f32,
    pub wallpaper: String,
    pub wallpaper_directory: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Self {
            theme: "fox".into(),
            accent_color: "#C8860A".into(),
            font_family: "sans-serif".into(),
            font_size: 16.0,
            wallpaper: String::new(),
            wallpaper_directory: format!("{}/Pictures/Wallpapers", home),
        }
    }
}

// ── Window manager ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WmConfig {
    pub border_width: u32,
    pub titlebar_height: u32,
    pub gap: u32,
    pub corner_radius: u32,
    pub focus_follows_mouse: bool,
}

impl Default for WmConfig {
    fn default() -> Self {
        Self {
            border_width: 2,
            titlebar_height: 36,
            gap: 8,
            corner_radius: 10,
            focus_follows_mouse: false,
        }
    }
}

// ── Windows (compositor visual effects) ──────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowsConfig {
    pub window_opacity: f32,
    pub blur_intensity: f32,
    pub blur_tint: f32,
    pub blur_darken: f32,
    pub background_opacity: f32,
}

impl Default for WindowsConfig {
    fn default() -> Self {
        Self {
            window_opacity: 1.0,
            blur_intensity: 0.8,
            blur_tint: 0.15,
            blur_darken: 0.0,
            background_opacity: 1.0,
        }
    }
}

// ── Input ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    pub mouse_speed: f32,
    pub mouse_acceleration: bool,
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    pub keyboard_repeat_delay: u32,
    pub keyboard_repeat_rate: u32,
    pub cursor_theme: String,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            mouse_speed: 0.0,
            mouse_acceleration: true,
            natural_scroll: false,
            tap_to_click: true,
            keyboard_repeat_delay: 300,
            keyboard_repeat_rate: 25,
            cursor_theme: "default".into(),
        }
    }
}

// ── Display ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub resolution: String,
    pub refresh_rate: String,
    pub scale: f32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            resolution: "auto".into(),
            refresh_rate: "auto".into(),
            scale: 1.0,
        }
    }
}

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

    /// Apply WiFi power settings immediately and persist to /etc/modprobe.d/.
    /// Spawns a background thread so pkexec dialogs don't block the Wayland event loop.
    /// Auto-detects the WiFi interface and driver — skips silently if no WiFi hardware found.
    pub fn apply_wifi_power(&self) {
        let power_save = self.power.wifi_power_save;
        let scheme = self.power.wifi_power_scheme.clone();

        std::thread::spawn(move || {
            // Auto-detect wireless interface and driver
            let Some((iface, driver)) = detect_wifi_interface() else {
                eprintln!("[settings] No WiFi interface found, skipping WiFi power settings");
                return;
            };

            let power_save_val = if power_save { 1 } else { 0 };
            let scheme_val = match scheme.as_str() {
                "active" => 1,
                "balanced" => 2,
                "battery" => 3,
                _ => 2,
            };

            // Write modprobe.d config via pkexec sh -c (pkexec doesn't pipe stdin)
            let conf = format!(
                "options {driver} power_save={power_save_val}\noptions {driver} power_scheme={scheme_val}\n",
            );
            let modprobe_path = format!("/etc/modprobe.d/{driver}.conf");
            let script = format!(
                "printf '{}' > {modprobe_path}",
                conf.replace('\'', "'\\''"),
            );
            let _ = Command::new("pkexec")
                .args(["sh", "-c", &script])
                .status();

            // Apply power save immediately via iw
            let ps_arg = if power_save { "on" } else { "off" };
            let _ = Command::new("pkexec")
                .args(["iw", "dev", &iface, "set", "power_save", ps_arg])
                .status();
        });
    }
}

/// Detect the first wireless network interface and its kernel driver.
/// Returns `(interface_name, driver_name)` or None if no WiFi hardware found.
fn detect_wifi_interface() -> Option<(String, String)> {
    let net_dir = std::fs::read_dir("/sys/class/net/").ok()?;
    for entry in net_dir.flatten() {
        let path = entry.path();
        // Wireless interfaces have /sys/class/net/<iface>/wireless/
        if path.join("wireless").exists() {
            let iface = entry.file_name().to_string_lossy().into_owned();
            // Read the driver name from the device symlink
            let driver = std::fs::read_link(path.join("device/driver"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "iwlwifi".to_string());
            return Some((iface, driver));
        }
    }
    None
}

impl LanternConfig {
    fn sanitize(&mut self) {
        self.appearance.font_size = self.appearance.font_size.clamp(10.0, 32.0);
        self.window_manager.border_width = self.window_manager.border_width.clamp(0, 10);
        self.window_manager.titlebar_height = self.window_manager.titlebar_height.clamp(20, 60);
        self.window_manager.gap = self.window_manager.gap.clamp(0, 32);
        self.window_manager.corner_radius = self.window_manager.corner_radius.clamp(0, 20);
        self.windows.window_opacity = self.windows.window_opacity.clamp(0.1, 1.0);
        self.windows.blur_intensity = self.windows.blur_intensity.clamp(0.0, 1.0);
        self.windows.blur_tint = self.windows.blur_tint.clamp(0.0, 1.0);
        self.windows.blur_darken = self.windows.blur_darken.clamp(0.0, 1.0);
        self.windows.background_opacity = self.windows.background_opacity.clamp(0.0, 1.0);
        self.input.mouse_speed = self.input.mouse_speed.clamp(-1.0, 1.0);
        self.input.keyboard_repeat_delay = self.input.keyboard_repeat_delay.clamp(100, 2000);
        self.input.keyboard_repeat_rate = self.input.keyboard_repeat_rate.clamp(1, 100);
        self.display.scale = self.display.scale.clamp(0.5, 3.0);
        if !["active", "balanced", "battery"].contains(&self.power.wifi_power_scheme.as_str()) {
            self.power.wifi_power_scheme = "balanced".into();
        }
    }
}
