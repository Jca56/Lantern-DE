//! Battery widget — reads /sys/class/power_supply and displays icon + percentage.
//! Click to open a popup with charge level, time remaining, and charge limit toggle.

use std::path::{Path, PathBuf};

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Toggle};

use crate::svg_icon::IconCache;

const POLL_INTERVAL_MS: u64 = 15_000; // read sysfs every 15s

/// Battery charge level thresholds.
const LOW_THRESH: u32 = 20;
const MED_THRESH: u32 = 60;

// Hit-test zone IDs
pub const ZONE_BATTERY: u32 = 0xBB_0000;
pub const ZONE_CHARGE_LIMIT_TOGGLE: u32 = 0xBB_0001;

// ── Battery state ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

impl BatteryStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "On Battery",
            Self::Full => "Fully Charged",
            Self::NotCharging => "Not Charging",
            Self::Unknown => "Unknown",
        }
    }
}

pub struct Battery {
    sysfs_path: PathBuf,
    capacity: u32,
    status: BatteryStatus,
    energy_now_uwh: u64,
    energy_full_uwh: u64,
    power_now_uw: u64,
    charge_limit: u32,
    has_charge_limit: bool,
    last_poll: std::time::Instant,
    icons_loaded: bool,
    // Popup state
    pub open: bool,
    toggle_anim: f32, // 0.0 = off, 1.0 = on (for charge limit toggle)
}

impl Battery {
    /// Try to find a battery in /sys/class/power_supply.
    pub fn new() -> Option<Self> {
        let sysfs = find_battery()?;
        let has_charge_limit = sysfs.join("charge_control_end_threshold").exists();
        let mut bat = Self {
            sysfs_path: sysfs,
            capacity: 0,
            status: BatteryStatus::Unknown,
            energy_now_uwh: 0,
            energy_full_uwh: 0,
            power_now_uw: 0,
            charge_limit: 100,
            has_charge_limit,
            last_poll: std::time::Instant::now() - std::time::Duration::from_secs(60),
            icons_loaded: false,
            open: false,
            toggle_anim: 0.0,
        };
        bat.poll_sysfs();
        bat.toggle_anim = if bat.charge_limit <= 80 { 1.0 } else { 0.0 };
        Some(bat)
    }

    /// Re-read sysfs if enough time has passed. Returns `true` if new data was read.
    pub fn tick(&mut self) -> bool {
        if self.last_poll.elapsed().as_millis() >= POLL_INTERVAL_MS as u128 {
            self.poll_sysfs();
            true
        } else {
            false
        }
    }

    /// Animate the toggle switch.
    pub fn update_anim(&mut self, dt: f32) -> bool {
        let target = if self.charge_limit <= 80 { 1.0 } else { 0.0 };
        if (self.toggle_anim - target).abs() < 0.001 {
            self.toggle_anim = target;
            return false;
        }
        let step = dt / 0.2;
        if self.toggle_anim < target {
            self.toggle_anim = (self.toggle_anim + step).min(1.0);
        } else {
            self.toggle_anim = (self.toggle_anim - step).max(0.0);
        }
        true
    }

    fn poll_sysfs(&mut self) {
        self.last_poll = std::time::Instant::now();
        if let Ok(s) = std::fs::read_to_string(self.sysfs_path.join("capacity")) {
            self.capacity = s.trim().parse().unwrap_or(0);
        }
        if let Ok(s) = std::fs::read_to_string(self.sysfs_path.join("status")) {
            self.status = match s.trim() {
                "Charging" => BatteryStatus::Charging,
                "Discharging" => BatteryStatus::Discharging,
                "Full" => BatteryStatus::Full,
                "Not charging" => BatteryStatus::NotCharging,
                _ => BatteryStatus::Unknown,
            };
        }
        if let Ok(s) = std::fs::read_to_string(self.sysfs_path.join("energy_now")) {
            self.energy_now_uwh = s.trim().parse().unwrap_or(0);
        }
        if let Ok(s) = std::fs::read_to_string(self.sysfs_path.join("energy_full")) {
            self.energy_full_uwh = s.trim().parse().unwrap_or(0);
        }
        if let Ok(s) = std::fs::read_to_string(self.sysfs_path.join("power_now")) {
            self.power_now_uw = s.trim().parse().unwrap_or(0);
        }
        if self.has_charge_limit {
            if let Ok(s) = std::fs::read_to_string(
                self.sysfs_path.join("charge_control_end_threshold"),
            ) {
                self.charge_limit = s.trim().parse().unwrap_or(100);
            }
        }
    }

    /// Toggle the charge limit between 80% and 100%.
    pub fn toggle_charge_limit(&mut self) {
        if !self.has_charge_limit {
            return;
        }
        let new_limit = if self.charge_limit <= 80 { 100 } else { 80 };
        let path = self.sysfs_path.join("charge_control_end_threshold");
        if std::fs::write(&path, format!("{}", new_limit)).is_ok() {
            self.charge_limit = new_limit;
            tracing::info!(limit = new_limit, "charge limit updated");
        } else {
            tracing::warn!("failed to write charge limit — check udev rule permissions");
        }
    }

    /// Time remaining as a human-friendly string, or None.
    fn time_remaining(&self) -> Option<String> {
        if self.power_now_uw == 0 {
            return None;
        }
        let hours = match self.status {
            BatteryStatus::Discharging => {
                self.energy_now_uwh as f64 / self.power_now_uw as f64
            }
            BatteryStatus::Charging => {
                let remaining = self.energy_full_uwh.saturating_sub(self.energy_now_uwh);
                if remaining == 0 { return None; }
                remaining as f64 / self.power_now_uw as f64
            }
            _ => return None,
        };
        let h = hours as u32;
        let m = ((hours - h as f64) * 60.0) as u32;
        if h > 0 {
            Some(format!("{}h {}m remaining", h, m))
        } else {
            Some(format!("{}m remaining", m))
        }
    }

    fn icon_key(&self) -> &'static str {
        if self.status == BatteryStatus::Charging {
            return "battery-charging";
        }
        match self.capacity {
            0..=LOW_THRESH => "battery-low",
            21..=MED_THRESH => "battery-medium",
            _ => "battery-high",
        }
    }

    /// Ensure all battery icons are loaded into the cache.
    pub fn load_icons(
        &mut self,
        icons: &mut IconCache,
        tex_pass: &TexturePass,
        gpu: &GpuContext,
        icon_w: u32,
        icon_h: u32,
    ) {
        if self.icons_loaded { return; }
        let pairs = [
            ("battery-high", "spark-battery-high.svg"),
            ("battery-medium", "spark-battery-medium.svg"),
            ("battery-low", "spark-battery-low.svg"),
            ("battery-charging", "spark-battery-charging(1).svg"),
        ];
        for (key, file) in pairs {
            icons.load_embedded(tex_pass, gpu, key, file, icon_w, icon_h);
        }
        self.icons_loaded = true;
    }

    /// Measure total width without drawing (for right-to-left layout).
    pub fn measure(&self, bar_h: f32, scale: f32) -> f32 {
        let pad = 5.0 * scale;
        let usable = bar_h - pad * 2.0;
        let icon_h = usable * 0.60;
        let leading_pad = 4.0 * scale;
        icon_h * 1.5 + leading_pad
    }

    /// Draw the battery: percentage text centered over the icon body.
    /// Returns (total_width, Vec<TextureDraw>).
    pub fn draw<'a>(
        &self,
        _painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        icons: &'a IconCache,
        palette: &FoxPalette,
        x: f32,
        bar_y: f32,
        bar_h: f32,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
    ) -> (f32, Vec<TextureDraw<'a>>) {
        let pad = 5.0 * scale;
        let usable = bar_h - pad * 2.0;
        let icon_h = usable * 0.60;
        let icon_w = icon_h * 1.5;
        let leading_pad = 4.0 * scale;
        let total_w = icon_w + leading_pad;

        // Icon, vertically centered in bar, offset right by leading_pad so the
        // visible battery body doesn't crowd the adjacent widget.
        let icon_x = x + leading_pad;
        let icon_y = bar_y + (bar_h - icon_h) / 2.0;

        let mut tex_draws = Vec::new();
        let key = self.icon_key();
        if let Some(tex) = icons.get(key) {
            tex_draws.push(TextureDraw::new(tex, icon_x, icon_y, icon_w, icon_h));
        }

        let _ = (text, palette, screen_w, screen_h);

        // Hit zone
        let zone_rect = Rect::new(x, bar_y + pad, total_w, usable);
        ix.add_zone(ZONE_BATTERY, zone_rect);

        (total_w, tex_draws)
    }

    /// Draw the battery popup above/below the bar.
    pub fn draw_popup(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        // X position of battery widget in bar (for alignment)
        bat_x: f32,
        bat_w: f32,
        bar_y: f32,
        bar_h: f32,
        position_top: bool,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        if !self.open { return; }

        let pad = 20.0 * scale;
        let corner_r = 12.0 * scale;
        let gap = 8.0 * scale;
        let popup_w = 320.0 * scale;

        // ── Compute popup height ──
        let title_font = 28.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let bar_track_h = 10.0 * scale;
        let section_gap = 16.0 * scale;
        let toggle_h = 28.0 * scale;

        // Sections: title, progress bar, status, time remaining, separator, charge limit
        let mut content_h = title_font;             // "34%"
        content_h += section_gap * 0.5;
        content_h += bar_track_h;                    // progress bar
        content_h += section_gap;
        content_h += body_font;                      // status text
        if self.time_remaining().is_some() {
            content_h += 6.0 * scale;
            content_h += small_font;                 // time remaining
        }
        if self.has_charge_limit {
            content_h += section_gap;
            content_h += 1.0 * scale;               // separator line
            content_h += section_gap;
            content_h += toggle_h.max(body_font);    // toggle row
        }

        let popup_h = pad * 2.0 + content_h;

        // Position: centered above/below battery widget, clamped to screen
        let popup_x = (bat_x + bat_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = if position_top {
            bar_y + bar_h + gap
        } else {
            (bar_y - popup_h - gap).max(0.0)
        };

        // Shadow
        let shadow_expand = 3.0 * scale;
        let shadow_rect = Rect::new(
            popup_x - shadow_expand,
            popup_y + shadow_expand,
            popup_w + shadow_expand * 2.0,
            popup_h + shadow_expand,
        );
        painter.rect_filled(shadow_rect, corner_r + 2.0, Color::BLACK.with_alpha(0.35));

        // Background
        let bg_rect = Rect::new(popup_x, popup_y, popup_w, popup_h);
        painter.rect_filled(bg_rect, corner_r, palette.bg);
        painter.rect_stroke_sdf(bg_rect, corner_r, 3.0 * scale, crate::theme_state::popup_border());

        let cx = popup_x + pad; // content left edge
        let cw = popup_w - pad * 2.0; // content width
        let mut y = popup_y + pad;

        // ── Big percentage ──
        let pct_text = format!("{}%", self.capacity);
        let pct_color = if self.capacity <= LOW_THRESH && self.status == BatteryStatus::Discharging {
            Color::rgba(1.0, 0.2, 0.2, 1.0)
        } else {
            palette.text
        };
        text.queue(&pct_text, title_font, cx, y, pct_color, cw, screen_w, screen_h);
        y += title_font + section_gap * 0.5;

        // ── Progress bar ──
        let track_r = bar_track_h / 2.0;
        let track_rect = Rect::new(cx, y, cw, bar_track_h);
        painter.rect_filled(track_rect, track_r, palette.surface);

        let fill_frac = (self.capacity as f32 / 100.0).clamp(0.0, 1.0);
        if fill_frac > 0.0 {
            let fill_w = (cw * fill_frac).max(bar_track_h);
            let fill_rect = Rect::new(cx, y, fill_w, bar_track_h);
            let fill_color = if self.capacity <= LOW_THRESH {
                palette.danger
            } else if self.capacity <= MED_THRESH {
                palette.warning
            } else {
                palette.accent
            };
            painter.rect_filled(fill_rect, track_r, fill_color);
        }

        // Charge limit marker (thin vertical line at 80%)
        if self.has_charge_limit && self.charge_limit <= 80 {
            let marker_x = cx + cw * 0.8;
            let marker_rect = Rect::new(
                marker_x - 1.0 * scale,
                y - 3.0 * scale,
                2.0 * scale,
                bar_track_h + 6.0 * scale,
            );
            painter.rect_filled(marker_rect, 1.0 * scale, palette.text.with_alpha(0.6));
        }
        y += bar_track_h + section_gap;

        // ── Status text ──
        let status_text = self.status.label();
        text.queue(status_text, body_font, cx, y, palette.text_secondary, cw, screen_w, screen_h);
        y += body_font;

        // ── Time remaining ──
        if let Some(time_str) = self.time_remaining() {
            y += 6.0 * scale;
            text.queue(&time_str, small_font, cx, y, palette.muted, cw, screen_w, screen_h);
            y += small_font;
        }

        // ── Charge limit toggle ──
        if self.has_charge_limit {
            y += section_gap;
            // Separator
            let sep_rect = Rect::new(cx, y, cw, 1.0 * scale);
            painter.rect_filled(sep_rect, 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            let row_h = toggle_h.max(body_font);
            let is_on = self.charge_limit <= 80;
            let toggle_rect = Rect::new(cx, y, cw, row_h);

            // Label on the right of the toggle
            let toggle_w = 52.0 * scale;
            let label_x = cx + toggle_w + 12.0 * scale;

            // Check hover
            let track_zone = Rect::new(cx, y, toggle_w, row_h);
            ix.add_zone(ZONE_CHARGE_LIMIT_TOGGLE, track_zone);
            let hovered = ix.is_hovered(&track_zone);

            Toggle::new(toggle_rect, is_on)
                .hovered(hovered)
                .scale(scale)
                .transition(self.toggle_anim)
                .draw(painter, text, palette, screen_w, screen_h);

            // Override the label since Toggle puts it after the track — we want custom text
            let label_y = y + (row_h - body_font) / 2.0;
            text.queue(
                "Limit to 80%", body_font,
                label_x, label_y, palette.text,
                cw - toggle_w - 12.0 * scale, screen_w, screen_h,
            );
        }
    }

    /// Returns the popup bounding rect (for contains() checks), or None if closed.
    pub fn popup_rect(&self, bat_x: f32, bat_w: f32, bar_y: f32, bar_h: f32, position_top: bool, scale: f32, screen_w: u32) -> Option<Rect> {
        if !self.open { return None; }
        let pad = 20.0 * scale;
        let gap = 8.0 * scale;
        let popup_w = 320.0 * scale;

        let title_font = 28.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let bar_track_h = 10.0 * scale;
        let section_gap = 16.0 * scale;
        let toggle_h = 28.0 * scale;

        let mut content_h = title_font + section_gap * 0.5 + bar_track_h + section_gap + body_font;
        if self.time_remaining().is_some() {
            content_h += 6.0 * scale + small_font;
        }
        if self.has_charge_limit {
            content_h += section_gap + 1.0 * scale + section_gap + toggle_h.max(body_font);
        }
        let popup_h = pad * 2.0 + content_h;

        let popup_x = (bat_x + bat_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = if position_top {
            bar_y + bar_h + gap
        } else {
            (bar_y - popup_h - gap).max(0.0)
        };

        Some(Rect::new(popup_x, popup_y, popup_w, popup_h))
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn find_battery() -> Option<PathBuf> {
    let base = Path::new("/sys/class/power_supply");
    for entry in std::fs::read_dir(base).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if let Ok(typ) = std::fs::read_to_string(path.join("type")) {
            if typ.trim() == "Battery" {
                // Skip peripheral batteries (mice, headsets, etc.) —
                // only show the system/laptop battery.
                if let Ok(scope) = std::fs::read_to_string(path.join("scope")) {
                    if scope.trim() == "Device" {
                        continue;
                    }
                }
                if path.join("capacity").exists() {
                    return Some(path);
                }
            }
        }
    }
    None
}
