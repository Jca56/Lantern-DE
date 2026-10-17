//! The Control Center page: the bar along the bottom of the screen, its
//! height, corners and gaps, over the bottom of a screen with the bar
//! drawn on it at its real size (the middle of the screen left out), then
//! whether it hides between uses, what its clock shows, and which items its
//! system tray shows. The bar reads `[control_center]` as it changes.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::Ui;

use crate::config::{CC_DATE_STYLES, CC_GAPS_BELOW, CC_GAPS_SIDES, CC_HEIGHTS, CC_HIDE_DELAYS, CC_RADII, CC_SLIDES, CC_VISIBILITIES, Config};
use crate::kit::{self, look, pixels};
use crate::machine;

/// The preview's height, logical: the tallest bar on its biggest gap with
/// room above, so dragging a slider never moves the sliders under it.
const PREVIEW_H: f64 = (CC_HEIGHTS.1 + CC_GAPS_BELOW.1) as f64 + 48.0;

/// Milliseconds as a slider shows them: `0.5 s`, `1 s`, none at all as
/// at once.
fn seconds(ms: f64) -> String {
    if ms <= 0.0 {
        return "At once".to_owned();
    }
    let s = format!("{:.2}", ms / 1000.0);
    format!("{} s", s.trim_end_matches('0').trim_end_matches('.'))
}

/// What the bar is painted with, as it picks it: the window background
/// chosen, else its style's.
fn plate_color(cfg: &Config) -> Color {
    let style = if cfg.appearance.theme.starts_with("fox") { Color::hex(0x181818) } else { look::BG };
    Color::parse_hex(&cfg.appearance.background_color).map_or(style, |c| c.with_alpha(1.0))
}

/// The bottom of a screen, a wallpaper in the accent's shade, with the
/// bar floating on it: `(height, radius, below, sides)` in logical px.
fn preview(ui: &mut Ui, rect: Rect, (height, radius, below, sides): (i64, i64, i64, i64), plate: Color) {
    let m = ui.m;
    let px = |v: i64| m.px(v as f64);
    let screen = rect.shrink(m.px(4.0));
    let corner = m.px(8.0);
    let accent = ui.theme.accent;
    ui.draw.rounded_rect_gradient(screen, corner, accent.lerp(Color::BLACK, 0.72), accent.lerp(Color::BLACK, 0.9));
    let bar = Rect::new(Vec2::new(screen.min.x + px(sides), screen.max.y - px(below + height)), Vec2::new(screen.max.x - px(sides), screen.max.y - px(below)));
    // On the bottom edge it is square, as the bar draws it.
    let r = if below > 0 { px(radius) } else { 0.0 };
    ui.draw.rounded_rect(bar, r, plate);
    ui.draw.stroke_rect(bar, m.px(1.0), r, plate.lerp(Color::WHITE, 0.10));
    ui.draw.stroke_rect(screen, m.px(1.0), corner, look::LINE);
}

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let plate = plate_color(cfg);
    let cc = &mut cfg.control_center;
    let mut changed = false;

    kit::caption(ui, "Shape");
    kit::card(ui, "shape", |c| {
        let h = c.ui.m.px(PREVIEW_H);
        let shape = (cc.height, cc.corner_radius, cc.gap_below, cc.gap_sides);
        c.block(h, |ui, rect| preview(ui, rect, shape, plate));
        changed |= c.slider_int("Height", "", &mut cc.height, CC_HEIGHTS, 1, pixels);
        changed |= c.slider_int("Corner radius", "While it floats. Docked, it is square.", &mut cc.corner_radius, CC_RADII, 1, pixels);
        changed |= c.slider_int("Gap below", "", &mut cc.gap_below, CC_GAPS_BELOW, 1, pixels);
        changed |= c.slider_int("Gap at the sides", "", &mut cc.gap_sides, CC_GAPS_SIDES, 1, pixels);
    });
    kit::note(ui, "It floats this far from the screen's edges, and docks full width and square while a window is maximized or full screen on its monitor. It follows these as they change.");

    kit::caption(ui, "Visibility");
    kit::card(ui, "visibility", |c| {
        let labels: Vec<&str> = CC_VISIBILITIES.iter().map(|(_, shown)| *shown).collect();
        let mut index = CC_VISIBILITIES.iter().position(|(word, _)| *word == cc.visibility).unwrap_or(0);
        if c.dropdown("Visibility", "", &mut index, &labels) {
            cc.visibility = CC_VISIBILITIES[index].0.to_owned();
            changed = true;
        }
        if cc.visibility == "auto_hide" {
            changed |= c.slider_int("Hide after", "Once the pointer leaves the bar.", &mut cc.auto_hide_delay_ms, CC_HIDE_DELAYS, 100, seconds);
            changed |= c.slider_int("Slide time", "Out of sight, or back.", &mut cc.auto_hide_slide_ms, CC_SLIDES, 25, seconds);
        }
    });
    kit::note(ui, "Auto hide keeps the bar out of sight until you push the pointer against the bottom edge of the screen. It never covers a window: windows make room as it slides up, and take the room back as it slides away.");

    kit::caption(ui, "Clock");
    kit::card(ui, "clock", |c| {
        changed |= c.switch("24-hour clock", "18:52 instead of 6:52 PM.", &mut cc.clock_24h);
        changed |= c.switch("Show AM/PM", "6:52 PM, or just 6:52, on the 12-hour clock.", &mut cc.clock_ampm);
        changed |= c.switch("Show seconds", "", &mut cc.clock_seconds);
        changed |= c.switch("Show the date", "Under the time, or beside it on a short bar.", &mut cc.clock_date);
        let labels: Vec<&str> = CC_DATE_STYLES.iter().map(|(_, shown)| *shown).collect();
        let mut index = CC_DATE_STYLES.iter().position(|(word, _)| *word == cc.clock_date_style).unwrap_or(0);
        if c.dropdown("Date style", "", &mut index, &labels) {
            cc.clock_date_style = CC_DATE_STYLES[index].0.to_owned();
            changed = true;
        }
    });

    kit::caption(ui, "System Tray");
    kit::card(ui, "tray", |c| {
        // A desktop has no battery or backlight to show.
        if machine::has_battery() {
            changed |= c.switch("Battery", "Its charge, and a click for the details.", &mut cc.tray_battery);
            if cc.tray_battery {
                changed |= c.switch("Battery percent", "The number beside the battery.", &mut cc.tray_battery_percent);
            }
        }
        changed |= c.switch("WiFi", "Networks, and joining one.", &mut cc.tray_wifi);
        changed |= c.switch("Bluetooth", "Connecting paired devices.", &mut cc.tray_bluetooth);
        changed |= c.switch("Sound", "The volume and the outputs. The wheel turns it up and down, a middle click mutes.", &mut cc.tray_audio);
        if machine::has_backlight() {
            changed |= c.switch("Brightness", "The screen's. The wheel turns it up and down.", &mut cc.tray_brightness);
        }
        changed |= c.switch("Workspace number", "A click goes to the next workspace, the wheel goes through them.", &mut cc.tray_workspace);
        changed |= c.switch("Game Mode", "Native resolution for games, as Super+G toggles it.", &mut cc.tray_game_mode);
    });
    kit::note(ui, "Each shows where the machine has it. Hold Alt over one in the bar to read what it says.");
    changed
}
