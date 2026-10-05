//! Sections of the Themes page: the per-corner window glow, the focus
//! glow, borders, and the blur behind windows.

use lntrn_math::Color;
use lntrn_ui::Ui;

use crate::config::{Config, GRADIENT_STOPS};
use crate::widgets::{hex_color, note, section, slider_int, toggle_string};

/// Glow positions in the order they are stored, with their labels.
const POSITIONS: [&str; GRADIENT_STOPS] = ["Top left", "Top right", "Bottom left", "Bottom right", "Centre"];

/// The colour the glow shares: the first position that is on, else the accent.
fn shared_color(stops: &[String], accent: &str) -> String {
    stops.iter().find(|s| !s.is_empty()).cloned().unwrap_or_else(|| accent.to_owned())
}

pub fn window_glow(cfg: &mut Config, ui: &mut Ui) -> bool {
    let a = &mut cfg.appearance;
    a.normalize_gradient();
    let mut changed = false;
    section(ui, "Window Glow");
    note(ui, "A soft colour behind each window's corners or centre, fading over the background.");
    let seed = shared_color(&a.window_gradient_stops, &a.accent);
    ui.push_id("glow");
    for (i, label) in POSITIONS.iter().enumerate() {
        changed |= toggle_string(ui, label, &mut a.window_gradient_stops[i], &seed);
    }
    ui.pop_id();
    let any_on = a.window_gradient_stops.iter().any(|s| !s.is_empty());
    if any_on {
        let mut color = seed.clone();
        if hex_color(ui, "Glow colour", &mut color, Color::hex(0xFFC800)) {
            for s in a.window_gradient_stops.iter_mut().filter(|s| !s.is_empty()) {
                *s = color.clone();
            }
            changed = true;
        }
        let mut alpha = a.window_gradient_stop_alphas.first().copied().unwrap_or(1.0);
        if ui.slider("Intensity", &mut alpha, 0.0, 1.0, 0.01) {
            a.window_gradient_stop_alphas = vec![alpha; GRADIENT_STOPS];
            changed = true;
        }
        changed |= ui.slider("Radius", &mut a.window_gradient_radius, 0.1, 1.0, 0.05);
        if ui.button("Turn All Off").clicked {
            a.window_gradient_stops = vec![String::new(); GRADIENT_STOPS];
            changed = true;
        }
    }
    changed
}

pub fn focus_glow(cfg: &mut Config, ui: &mut Ui) -> bool {
    let wm = &mut cfg.window_manager;
    let mut changed = false;
    section(ui, "Focus Glow");
    changed |= ui.toggle("Glow around the focused window", &mut wm.focus_glow);
    if wm.focus_glow {
        changed |= hex_color(ui, "Glow colour", &mut wm.focus_glow_color, Color::hex(0x4A9EFF));
        changed |= ui.slider("Glow intensity", &mut wm.focus_glow_intensity, 0.0, 0.6, 0.01);
    }
    changed
}

pub fn borders(cfg: &mut Config, ui: &mut Ui) -> bool {
    let wm = &mut cfg.window_manager;
    let mut changed = false;
    section(ui, "Borders");
    changed |= slider_int(ui, "Border width", &mut wm.border_width, 0, 10, 1);
    changed |= hex_color(ui, "Border colour", &mut wm.border_color, Color::hex(0x4A9EFF));
    changed |= slider_int(ui, "Title bar height", &mut wm.titlebar_height, 20, 60, 1);
    changed |= slider_int(ui, "Corner radius", &mut wm.corner_radius, 0, 20, 1);
    changed |= slider_int(ui, "Gap between windows", &mut wm.gap, 0, 32, 1);
    note(ui, "Title bar, corners and borders apply to server-decorated windows; Lantern apps draw their own.");
    changed
}

pub fn blur(cfg: &mut Config, ui: &mut Ui) -> bool {
    let w = &mut cfg.windows;
    let mut changed = false;
    section(ui, "Blur & Effects");
    changed |= ui.slider("Background opacity", &mut w.background_opacity, 0.0, 1.0, 0.01);
    changed |= ui.slider("Blur intensity", &mut w.blur_intensity, 0.0, 1.0, 0.01);
    changed |= ui.slider("Blur tint", &mut w.blur_tint, 0.0, 1.0, 0.01);
    changed |= hex_color(ui, "Tint colour", &mut w.blur_tint_color, Color::hex(0x4A9EFF));
    changed |= ui.slider("Blur darken", &mut w.blur_darken, 0.0, 1.0, 0.01);
    changed
}
