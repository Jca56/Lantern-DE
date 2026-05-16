//! Power and idle management.
//!
//! Owns:
//! - Idle timer (resets on input; fires dim/idle actions)
//! - Battery monitor (polls sysfs; fires low warning + critical action)
//! - AC-state detection (for AC-aware lid behavior)
//! - Power action dispatch (suspend/lock/hibernate/shutdown)
//!
//! Settings are cached on `PowerState` and refreshed in the same poll cycle
//! the rest of `[input]` uses. All reads of `/sys/class/power_supply/*` and
//! `/sys/class/backlight/*` are cheap (small text files, kernel-cached).

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How often to poll battery sysfs. Battery percentage changes slowly so 5s
/// is plenty — keeps the wakeup pressure low.
const BATTERY_POLL: Duration = Duration::from_secs(5);

/// Dim amount when `dim_after` fires: backlight is multiplied by this fraction.
const DIM_FACTOR: f32 = 0.3;

pub struct PowerState {
    // Idle tracking
    pub last_input: Instant,
    pub dimmed: bool,
    pub idle_fired: bool,
    /// Backlight value remembered before dim, so we can restore on wake.
    pre_dim_brightness: Option<u32>,

    // Battery tracking
    last_battery_poll: Instant,
    pub low_warned: bool,
    pub critical_fired: bool,

    // Cached config (refreshed by reload_from_config)
    pub dim_after: u32,
    pub idle_timeout: u32,
    pub idle_action: String,
    pub low_threshold: u32,
    pub critical_threshold: u32,
    pub critical_action: String,
    pub lid_close_action: String,
    pub lid_close_on_ac: String,

    /// Cached backlight device path, resolved once at startup.
    backlight: Option<BacklightDevice>,
}

struct BacklightDevice {
    brightness_path: PathBuf,
    max: u32,
}

impl PowerState {
    pub fn new() -> Self {
        let mut state = Self {
            last_input: Instant::now(),
            dimmed: false,
            idle_fired: false,
            pre_dim_brightness: None,
            last_battery_poll: Instant::now() - BATTERY_POLL, // poll on first tick
            low_warned: false,
            critical_fired: false,
            dim_after: 0,
            idle_timeout: 0,
            idle_action: "nothing".into(),
            low_threshold: 15,
            critical_threshold: 5,
            critical_action: "nothing".into(),
            lid_close_action: "suspend".into(),
            lid_close_on_ac: "lock".into(),
            backlight: find_backlight(),
        };
        state.reload_from_config();
        state
    }

    pub fn reload_from_config(&mut self) {
        self.dim_after = read_power_u32("dim_after", 120);
        self.idle_timeout = read_power_u32("idle_timeout", 300);
        self.idle_action = read_power_string("idle_action", "suspend");
        self.low_threshold = read_power_u32("low_battery_threshold", 15);
        self.critical_threshold = read_power_u32("critical_battery_threshold", 5);
        self.critical_action = read_power_string("critical_battery_action", "hibernate");
        self.lid_close_action = read_power_string("lid_close_action", "suspend");
        self.lid_close_on_ac = read_power_string("lid_close_on_ac", "lock");
    }

    /// Called on every input event. Wakes the screen and resets the idle clock.
    pub fn note_input(&mut self) {
        self.last_input = Instant::now();
        if self.dimmed {
            self.restore_brightness();
            self.dimmed = false;
        }
        self.idle_fired = false;
    }

    /// Called from the periodic poll cycle. Drives dim/idle/battery actions.
    pub fn tick(&mut self) {
        self.tick_idle();
        self.tick_battery();
    }

    fn tick_idle(&mut self) {
        let elapsed = self.last_input.elapsed().as_secs() as u32;

        // Dim
        if self.dim_after > 0 && !self.dimmed && elapsed >= self.dim_after {
            self.dim();
            self.dimmed = true;
        }

        // Idle action
        if self.idle_timeout > 0 && !self.idle_fired && elapsed >= self.idle_timeout {
            self.idle_fired = true;
            run_power_action(&self.idle_action);
        }
    }

    fn tick_battery(&mut self) {
        if self.last_battery_poll.elapsed() < BATTERY_POLL {
            return;
        }
        self.last_battery_poll = Instant::now();

        let Some(capacity) = battery_capacity() else { return };
        let discharging = battery_status()
            .map(|s| s.eq_ignore_ascii_case("discharging"))
            .unwrap_or(false);

        // Only act while discharging — charging past the threshold shouldn't
        // re-fire warnings on every poll.
        if !discharging {
            // Plugging in clears the latches so warnings can fire again on the
            // next discharge cycle.
            self.low_warned = false;
            self.critical_fired = false;
            return;
        }

        if capacity <= self.critical_threshold && !self.critical_fired {
            self.critical_fired = true;
            tracing::info!("Battery critical at {}% — running '{}'", capacity, self.critical_action);
            notify(
                "Battery critical",
                &format!("Battery at {}% — running {}", capacity, self.critical_action),
                "critical",
            );
            run_power_action(&self.critical_action);
        } else if capacity <= self.low_threshold && !self.low_warned {
            self.low_warned = true;
            tracing::info!("Battery low at {}%", capacity);
            notify(
                "Battery low",
                &format!("Battery at {}%. Plug in soon.", capacity),
                "normal",
            );
        }
    }

    fn dim(&mut self) {
        let Some(bl) = &self.backlight else { return };
        let current = read_brightness(&bl.brightness_path).unwrap_or(bl.max);
        self.pre_dim_brightness = Some(current);
        let dimmed = ((current as f32) * DIM_FACTOR).round() as u32;
        let dimmed = dimmed.max(1); // never fully off — that looks like the screen broke
        let _ = write_brightness(&bl.brightness_path, dimmed);
    }

    fn restore_brightness(&mut self) {
        let Some(bl) = &self.backlight else { return };
        if let Some(prev) = self.pre_dim_brightness.take() {
            let _ = write_brightness(&bl.brightness_path, prev);
        }
    }
}

// ── sysfs helpers ───────────────────────────────────────────────────────────

fn find_backlight() -> Option<BacklightDevice> {
    let entries = std::fs::read_dir("/sys/class/backlight").ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let max_path = dir.join("max_brightness");
        let brightness_path = dir.join("brightness");
        if let Ok(s) = std::fs::read_to_string(&max_path) {
            if let Ok(max) = s.trim().parse::<u32>() {
                return Some(BacklightDevice { brightness_path, max });
            }
        }
    }
    None
}

fn read_brightness(path: &PathBuf) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn write_brightness(path: &PathBuf, value: u32) -> std::io::Result<()> {
    std::fs::write(path, value.to_string())
}

pub fn is_on_ac() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return true; // assume AC when unsure (desktop)
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("AC") || name.starts_with("ADP")) {
            continue;
        }
        if let Ok(s) = std::fs::read_to_string(entry.path().join("online")) {
            return s.trim() == "1";
        }
    }
    true
}

fn battery_capacity() -> Option<u32> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("BAT") {
            continue;
        }
        if let Ok(s) = std::fs::read_to_string(entry.path().join("capacity")) {
            if let Ok(c) = s.trim().parse::<u32>() {
                return Some(c);
            }
        }
    }
    None
}

fn battery_status() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("BAT") {
            continue;
        }
        if let Ok(s) = std::fs::read_to_string(entry.path().join("status")) {
            return Some(s.trim().to_string());
        }
    }
    None
}

// ── config readers ──────────────────────────────────────────────────────────

fn read_power_string(key: &str, default: &str) -> String {
    let contents = crate::cached_lantern_toml();
    if contents.is_empty() {
        return default.to_string();
    }
    let mut in_power = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_power = trimmed == "[power]";
            continue;
        }
        if in_power {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == key {
                    return v.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    default.to_string()
}

fn read_power_u32(key: &str, default: u32) -> u32 {
    let s = read_power_string(key, "");
    if s.is_empty() {
        return default;
    }
    s.parse::<u32>().unwrap_or(default)
}

// ── action dispatch ─────────────────────────────────────────────────────────

/// Are we booted under systemd? Canonical `sd_booted(3)` check: PID1
/// creates `/run/systemd/system` on boot. Absent on elogind hosts.
fn systemd_booted() -> bool {
    std::path::Path::new("/run/systemd/system").is_dir()
}

/// Pick the right binary for a power-state action. `lock` is always
/// `loginctl lock-session` (both logind impls). The rest split:
/// systemd-logind exposes them on `systemctl` only, elogind exposes
/// them on `loginctl` only.
fn power_bin(action: &str) -> Option<(&'static str, &'static str)> {
    match action {
        "lock" => Some(("loginctl", "lock-session")),
        "suspend" | "hibernate" => Some((
            if systemd_booted() { "systemctl" } else { "loginctl" },
            // verb is identical on both
            if action == "suspend" { "suspend" } else { "hibernate" },
        )),
        "shutdown" => Some((
            if systemd_booted() { "systemctl" } else { "loginctl" },
            "poweroff",
        )),
        _ => None,
    }
}

/// Run a power action: "suspend", "lock", "hibernate", "shutdown", "nothing".
/// Spawns the command and detaches — does not wait.
///
/// Auto-targets systemd (Arch) or elogind (Gentoo OpenRC, Artix, Void,
/// Alpine) at runtime so the binary works on both.
pub fn run_power_action(action: &str) {
    if action == "nothing" || action.is_empty() {
        return;
    }
    let Some((bin, verb)) = power_bin(action) else {
        tracing::warn!("Unknown power action '{}'", action);
        return;
    };
    match std::process::Command::new(bin).arg(verb).spawn() {
        Ok(_) => tracing::info!("Power action '{}' dispatched via {}", action, bin),
        Err(e) => tracing::warn!("Failed to dispatch '{}': {}", action, e),
    }
}

fn notify(summary: &str, body: &str, urgency: &str) {
    let _ = std::process::Command::new("notify-send")
        .arg("--urgency").arg(urgency)
        .arg("--app-name").arg("Lantern Power")
        .arg(summary)
        .arg(body)
        .spawn();
}
