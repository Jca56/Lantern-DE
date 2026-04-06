//! Temperature widget — reads /sys/class/thermal and /sys/class/hwmon for CPU temp,
//! per-core temps, NVMe temp, and fan speeds. Click to open a detailed popup.

use std::path::{Path, PathBuf};

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::svg_icon::IconCache;

fn icon_dir() -> std::path::PathBuf { crate::lantern_icons_dir() }
const POLL_INTERVAL_MS: u64 = 5_000; // read temps every 5s

/// Temperature thresholds (°C).
const COOL_THRESH: u32 = 70;
const WARM_THRESH: u32 = 85;

// Hit-test zone IDs
pub const ZONE_TEMP: u32 = 0xCB_0000;

// ── Sensor data ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CoreTemp {
    label: String,
    temp_c: u32,
}

#[derive(Debug, Clone)]
struct FanReading {
    label: String,
    rpm: u32,
}

pub struct Temperature {
    /// Path to the x86_pkg_temp thermal zone (or fallback).
    thermal_zone: Option<PathBuf>,
    /// Path to the coretemp hwmon dir for per-core readings.
    coretemp_dir: Option<PathBuf>,
    /// Paths to NVMe hwmon temp files.
    nvme_temps: Vec<(String, PathBuf)>,
    /// Paths to fan RPM input files.
    fan_paths: Vec<(String, PathBuf)>,
    /// Current CPU package temp in °C.
    cpu_temp: u32,
    /// Per-core temps.
    core_temps: Vec<CoreTemp>,
    /// Fan readings.
    fans: Vec<FanReading>,
    /// NVMe temps.
    nvme_temp_readings: Vec<(String, u32)>,

    last_poll: std::time::Instant,
    icons_loaded: bool,
    pub open: bool,
}

impl Temperature {
    pub fn new() -> Self {
        let thermal_zone = find_cpu_thermal_zone();
        let coretemp_dir = find_hwmon_by_name("coretemp");
        let nvme_temps = find_nvme_temps();
        let fan_paths = find_fans();

        let mut t = Self {
            thermal_zone,
            coretemp_dir,
            nvme_temps,
            fan_paths,
            cpu_temp: 0,
            core_temps: Vec::new(),
            fans: Vec::new(),
            nvme_temp_readings: Vec::new(),
            last_poll: std::time::Instant::now() - std::time::Duration::from_secs(60),
            icons_loaded: false,
            open: false,
        };
        t.poll_sensors();
        t
    }

    /// Poll sensors if the interval has elapsed. Returns `true` if new data was read.
    pub fn tick(&mut self) -> bool {
        if self.last_poll.elapsed().as_millis() >= POLL_INTERVAL_MS as u128 {
            self.poll_sensors();
            true
        } else {
            false
        }
    }

    fn poll_sensors(&mut self) {
        self.last_poll = std::time::Instant::now();

        // CPU package temp from thermal zone
        if let Some(ref tz) = self.thermal_zone {
            if let Ok(s) = std::fs::read_to_string(tz.join("temp")) {
                self.cpu_temp = s.trim().parse::<u32>().unwrap_or(0) / 1000;
            }
        }

        // Per-core temps from coretemp hwmon
        if let Some(ref dir) = self.coretemp_dir {
            self.core_temps.clear();
            // Find all temp*_input files
            if let Ok(entries) = std::fs::read_dir(dir) {
                let mut pairs: Vec<(u32, String, PathBuf)> = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("temp") && name.ends_with("_input") {
                        let idx_str = &name[4..name.len() - 6];
                        if let Ok(idx) = idx_str.parse::<u32>() {
                            let label_path = dir.join(format!("temp{}_label", idx));
                            let label = std::fs::read_to_string(&label_path)
                                .map(|s| s.trim().to_string())
                                .unwrap_or_else(|_| format!("Sensor {}", idx));
                            // Skip the package sensor — we already show it as the main temp
                            if label.starts_with("Package") {
                                continue;
                            }
                            pairs.push((idx, label, entry.path()));
                        }
                    }
                }
                pairs.sort_by_key(|(idx, _, _)| *idx);
                for (_, label, path) in pairs {
                    if let Ok(s) = std::fs::read_to_string(&path) {
                        let temp_c = s.trim().parse::<u32>().unwrap_or(0) / 1000;
                        self.core_temps.push(CoreTemp { label, temp_c });
                    }
                }
            }
        }

        // NVMe temps
        self.nvme_temp_readings.clear();
        for (label, path) in &self.nvme_temps {
            if let Ok(s) = std::fs::read_to_string(path) {
                let temp_c = s.trim().parse::<u32>().unwrap_or(0) / 1000;
                self.nvme_temp_readings.push((label.clone(), temp_c));
            }
        }

        // Fan RPMs
        self.fans.clear();
        for (label, path) in &self.fan_paths {
            if let Ok(s) = std::fs::read_to_string(path) {
                let rpm = s.trim().parse::<u32>().unwrap_or(0);
                self.fans.push(FanReading { label: label.clone(), rpm });
            }
        }
    }

    fn icon_key(&self) -> &'static str {
        match self.cpu_temp {
            0..=COOL_THRESH => "temp-cool",
            t if t <= WARM_THRESH => "temp-warm",
            _ => "temp-hot",
        }
    }

    pub fn load_icons(
        &mut self,
        icons: &mut IconCache,
        tex_pass: &TexturePass,
        gpu: &GpuContext,
        icon_size: u32,
    ) {
        if self.icons_loaded { return; }
        let dir = icon_dir();
        let pairs = [
            ("temp-cool", "spark-temp-cool.svg"),
            ("temp-warm", "spark-temp-warm.svg"),
            ("temp-hot", "spark-temp-hot.svg"),
        ];
        for (key, file) in pairs {
            icons.load(tex_pass, gpu, key, &dir.join(file), icon_size, icon_size);
        }
        self.icons_loaded = true;
    }

    /// Measure total width without drawing.
    pub fn measure(&self, bar_h: f32, scale: f32) -> f32 {
        let pad = 5.0 * scale;
        let usable = bar_h - pad * 2.0;
        let font_size = (usable * 0.50).max(18.0);
        let icon_h = usable * 0.85;
        let icon_w = icon_h; // SVG is pre-cropped, aspect preserved by renderer
        let gap = 1.0 * scale;

        let temp_text = format!("{}", self.cpu_temp);
        let sym_size = font_size * 0.85;
        let num_w = temp_text.len() as f32 * font_size * 0.52;
        let sym_w = sym_size * 0.58;
        let sym_gap = 1.0 * scale;
        let text_w = num_w + sym_gap + sym_w;

        icon_w + gap + text_w
    }

    /// Draw the temperature: icon on left, temp text to the right.
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
        let font_size = (usable * 0.50).max(18.0);
        let icon_h = usable * 0.85;
        let icon_w = icon_h; // SVG is pre-cropped, aspect preserved by renderer
        let gap = 1.0 * scale;

        // Temperature text: numbers + °
        let temp_text = format!("{}", self.cpu_temp);
        let deg_sym = "°";
        let sym_size = font_size * 0.85;
        let num_w = temp_text.len() as f32 * font_size * 0.52;
        let sym_w = sym_size * 0.58;
        let sym_gap = 1.0 * scale;
        let text_w = num_w + sym_gap + sym_w;
        let total_w = icon_w + gap + text_w;

        // Icon on the left, vertically centered
        let icon_x = x;
        let icon_y = bar_y + (bar_h - icon_h) / 2.0;

        let mut tex_draws = Vec::new();
        let key = self.icon_key();
        if let Some(tex) = icons.get(key) {
            tex_draws.push(TextureDraw::new(tex, icon_x, icon_y, icon_w, icon_h));
        }

        // Text to the right of icon, aligned to top of icon
        let text_x = x + icon_w + gap;
        let text_y = icon_y;

        let text_color = if self.cpu_temp > WARM_THRESH {
            Color::rgba(1.0, 0.2, 0.2, 1.0)
        } else if self.cpu_temp > COOL_THRESH {
            palette.warning
        } else {
            palette.text
        };

        text.queue(
            &temp_text, font_size, text_x, text_y, text_color,
            text_w + 10.0, screen_w, screen_h,
        );
        let sym_y = text_y + (font_size - sym_size) / 2.0;
        text.queue(
            deg_sym, sym_size, text_x + num_w + sym_gap, sym_y, text_color,
            sym_size + 10.0, screen_w, screen_h,
        );

        // Hit zone
        let zone_rect = Rect::new(x, bar_y + pad, total_w, usable);
        ix.add_zone(ZONE_TEMP, zone_rect);

        (total_w, tex_draws)
    }

    /// Draw the temperature popup above/below the bar.
    pub fn draw_popup(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        _ix: &mut InteractionContext,
        palette: &FoxPalette,
        widget_x: f32,
        widget_w: f32,
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
        let popup_w = 340.0 * scale;

        let title_font = 28.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let section_gap = 16.0 * scale;
        let row_gap = 6.0 * scale;
        let bar_track_h = 8.0 * scale;

        // ── Compute popup height ──
        let mut content_h = title_font;          // "67°C" big
        content_h += section_gap * 0.5;
        content_h += bar_track_h;                 // temp bar
        content_h += section_gap;
        content_h += body_font;                   // "CPU Package" label

        // Core temps section
        if !self.core_temps.is_empty() {
            content_h += section_gap;
            content_h += 1.0 * scale;            // separator
            content_h += section_gap;
            content_h += body_font;               // "Core Temperatures" header
            content_h += row_gap;
            // Two columns of cores
            let rows = (self.core_temps.len() + 1) / 2;
            content_h += rows as f32 * (small_font + row_gap);
        }

        // NVMe section
        if !self.nvme_temp_readings.is_empty() {
            content_h += section_gap;
            content_h += 1.0 * scale;            // separator
            content_h += section_gap;
            for _ in &self.nvme_temp_readings {
                content_h += small_font + row_gap;
            }
        }

        // Fan section
        if !self.fans.is_empty() {
            content_h += section_gap;
            content_h += 1.0 * scale;            // separator
            content_h += section_gap;
            content_h += body_font;               // "Fans" header
            content_h += row_gap;
            for _ in &self.fans {
                content_h += small_font + row_gap;
            }
        }

        let popup_h = pad * 2.0 + content_h;

        // Position: centered above/below widget, clamped to screen
        let popup_x = (widget_x + widget_w / 2.0 - popup_w / 2.0)
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
        painter.rect_stroke_sdf(bg_rect, corner_r, 1.0 * scale, Color::WHITE.with_alpha(0.08));

        let cx = popup_x + pad;
        let cw = popup_w - pad * 2.0;
        let mut y = popup_y + pad;

        // ── Big temperature ──
        let temp_text = format!("{}°C", self.cpu_temp);
        let cpu_color = temp_color(self.cpu_temp, palette);
        text.queue(&temp_text, title_font, cx, y, cpu_color, cw, screen_w, screen_h);
        y += title_font + section_gap * 0.5;

        // ── Temp bar ──
        let track_r = bar_track_h / 2.0;
        let track_rect = Rect::new(cx, y, cw, bar_track_h);
        painter.rect_filled(track_rect, track_r, palette.surface);

        // Map temp to 0..1 range (30°C..100°C)
        let fill_frac = ((self.cpu_temp as f32 - 30.0) / 70.0).clamp(0.0, 1.0);
        if fill_frac > 0.0 {
            let fill_w = (cw * fill_frac).max(bar_track_h);
            let fill_rect = Rect::new(cx, y, fill_w, bar_track_h);
            painter.rect_filled(fill_rect, track_r, cpu_color);
        }
        y += bar_track_h + section_gap;

        // ── Status label ──
        text.queue("CPU Package", body_font, cx, y, palette.text_secondary, cw, screen_w, screen_h);
        y += body_font;

        // ── Core temperatures ──
        if !self.core_temps.is_empty() {
            y += section_gap;
            let sep_rect = Rect::new(cx, y, cw, 1.0 * scale);
            painter.rect_filled(sep_rect, 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            text.queue("Core Temperatures", body_font, cx, y, palette.text, cw, screen_w, screen_h);
            y += body_font + row_gap;

            // Two-column layout for cores
            let col_w = cw / 2.0;
            let mut col = 0;
            let mut row_y = y;
            for core in &self.core_temps {
                let col_x = cx + col as f32 * col_w;
                let label = format!("{}: {}°C", core.label, core.temp_c);
                let color = temp_color(core.temp_c, palette);
                text.queue(&label, small_font, col_x, row_y, color, col_w, screen_w, screen_h);
                col += 1;
                if col >= 2 {
                    col = 0;
                    row_y += small_font + row_gap;
                }
            }
            // Advance y past all rows
            let rows = (self.core_temps.len() + 1) / 2;
            y += rows as f32 * (small_font + row_gap);
        }

        // ── NVMe temps ──
        if !self.nvme_temp_readings.is_empty() {
            y += section_gap;
            let sep_rect = Rect::new(cx, y, cw, 1.0 * scale);
            painter.rect_filled(sep_rect, 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            for (label, temp_c) in &self.nvme_temp_readings {
                let nvme_text = format!("{}: {}°C", label, temp_c);
                let color = temp_color(*temp_c, palette);
                text.queue(&nvme_text, small_font, cx, y, color, cw, screen_w, screen_h);
                y += small_font + row_gap;
            }
        }

        // ── Fans ──
        if !self.fans.is_empty() {
            y += section_gap;
            let sep_rect = Rect::new(cx, y, cw, 1.0 * scale);
            painter.rect_filled(sep_rect, 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            text.queue("Fans", body_font, cx, y, palette.text, cw, screen_w, screen_h);
            y += body_font + row_gap;

            for fan in &self.fans {
                let fan_text = format!("{}: {} RPM", fan.label, fan.rpm);
                let color = palette.text_secondary;
                text.queue(&fan_text, small_font, cx, y, color, cw, screen_w, screen_h);
                y += small_font + row_gap;
            }
        }
    }

    /// Returns the popup bounding rect, or None if closed.
    pub fn popup_rect(
        &self, widget_x: f32, widget_w: f32, bar_y: f32, bar_h: f32, position_top: bool, scale: f32, screen_w: u32,
    ) -> Option<Rect> {
        if !self.open { return None; }

        let pad = 20.0 * scale;
        let gap = 8.0 * scale;
        let popup_w = 340.0 * scale;

        let title_font = 28.0 * scale;
        let body_font = 20.0 * scale;
        let small_font = 16.0 * scale;
        let section_gap = 16.0 * scale;
        let row_gap = 6.0 * scale;
        let bar_track_h = 8.0 * scale;

        let mut content_h = title_font + section_gap * 0.5 + bar_track_h + section_gap + body_font;

        if !self.core_temps.is_empty() {
            content_h += section_gap + 1.0 * scale + section_gap + body_font + row_gap;
            let rows = (self.core_temps.len() + 1) / 2;
            content_h += rows as f32 * (small_font + row_gap);
        }
        if !self.nvme_temp_readings.is_empty() {
            content_h += section_gap + 1.0 * scale + section_gap;
            content_h += self.nvme_temp_readings.len() as f32 * (small_font + row_gap);
        }
        if !self.fans.is_empty() {
            content_h += section_gap + 1.0 * scale + section_gap + body_font + row_gap;
            content_h += self.fans.len() as f32 * (small_font + row_gap);
        }

        let popup_h = pad * 2.0 + content_h;
        let popup_x = (widget_x + widget_w / 2.0 - popup_w / 2.0)
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

// ── Color helper ────────────────────────────────────────────────────────────

fn temp_color(temp_c: u32, palette: &FoxPalette) -> Color {
    if temp_c > WARM_THRESH {
        palette.danger
    } else if temp_c > COOL_THRESH {
        palette.warning
    } else {
        palette.accent
    }
}

// ── Sensor discovery ────────────────────────────────────────────────────────

/// Find the x86_pkg_temp thermal zone, falling back to TCPU or acpitz.
fn find_cpu_thermal_zone() -> Option<PathBuf> {
    let base = Path::new("/sys/class/thermal");
    let mut fallback: Option<PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.file_name().map_or(false, |n| n.to_string_lossy().starts_with("thermal_zone")) {
                continue;
            }
            if let Ok(typ) = std::fs::read_to_string(path.join("type")) {
                let typ = typ.trim();
                if typ == "x86_pkg_temp" {
                    return Some(path);
                }
                if fallback.is_none() && (typ == "TCPU" || typ == "coretemp" || typ == "acpitz") {
                    fallback = Some(path);
                }
            }
        }
    }
    fallback
}

/// Find a hwmon directory by its `name` file content.
fn find_hwmon_by_name(target: &str) -> Option<PathBuf> {
    let base = Path::new("/sys/class/hwmon");
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(name) = std::fs::read_to_string(path.join("name")) {
                if name.trim() == target {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Find NVMe temperature sensors.
fn find_nvme_temps() -> Vec<(String, PathBuf)> {
    let mut results = Vec::new();
    let base = Path::new("/sys/class/hwmon");
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(name) = std::fs::read_to_string(path.join("name")) {
                if name.trim() == "nvme" {
                    let temp_path = path.join("temp1_input");
                    if temp_path.exists() {
                        results.push(("NVMe".to_string(), temp_path));
                    }
                }
            }
        }
    }
    results
}

/// Find fan RPM sensors.
/// Deduplicates fans that appear under multiple drivers (e.g. acpi_fan vs asus)
/// by preferring platform-specific drivers over generic ACPI ones.
fn find_fans() -> Vec<(String, PathBuf)> {
    let mut results: Vec<(String, PathBuf, bool)> = Vec::new();
    let base = Path::new("/sys/class/hwmon");
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            let hwmon_name = std::fs::read_to_string(path.join("name"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let is_acpi = hwmon_name.starts_with("acpi");
            if let Ok(files) = std::fs::read_dir(&path) {
                for file in files.flatten() {
                    let fname = file.file_name().to_string_lossy().to_string();
                    if fname.starts_with("fan") && fname.ends_with("_input") {
                        // Try to read a label file for a friendly name
                        let idx = &fname[3..fname.len() - 6];
                        let label_path = path.join(format!("fan{}_label", idx));
                        let label = std::fs::read_to_string(&label_path)
                            .map(|s| s.trim().to_string())
                            .unwrap_or_else(|_| {
                                // Use a friendly name based on the fan index
                                let fan_num: u32 = idx.parse().unwrap_or(1);
                                format!("Fan {}", fan_num)
                            });
                        results.push((label, file.path(), is_acpi));
                    }
                }
            }
        }
    }
    // If we have both ACPI and platform fans, drop the ACPI duplicates
    let has_platform = results.iter().any(|(_, _, acpi)| !*acpi);
    if has_platform {
        results.retain(|(_, _, acpi)| !*acpi);
    }
    results.into_iter().map(|(label, path, _)| (label, path)).collect()
}
