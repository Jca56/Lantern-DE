//! The Effects page: how see-through windows are and the blur behind
//! them, the glow around the focused window, and the glow inside each
//! window's corners, set on a little window that shows it.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, Sense, Ui};

use crate::config::{Appearance, Config, GRADIENT_STOPS};
use crate::kit::{self, percent};
use crate::look;

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let mut changed = false;

    let w = &mut cfg.windows;
    kit::caption(ui, "Transparency and blur");
    kit::card(ui, "blur", |c| {
        changed |= c.slider("Window opacity", "Lower lets the desktop show through.", &mut w.background_opacity, (0.0, 1.0), 0.01, percent);
        changed |= c.slider("Blur", "How soft what shows through is.", &mut w.blur_intensity, (0.0, 1.0), 0.01, percent);
        changed |= c.slider("Darken", "", &mut w.blur_darken, (0.0, 1.0), 0.01, percent);
        changed |= c.slider("Tint", "", &mut w.blur_tint, (0.0, 1.0), 0.01, percent);
        if w.blur_tint > 0.0 {
            changed |= c.color("Tint colour", "", &mut w.blur_tint_color, Color::hex(0x4A9EFF));
        }
    });

    let wm = &mut cfg.window_manager;
    kit::caption(ui, "Focus glow");
    kit::card(ui, "focus", |c| {
        changed |= c.switch("Glow around the focused window", "", &mut wm.focus_glow);
        if wm.focus_glow {
            changed |= c.color("Colour", "", &mut wm.focus_glow_color, Color::hex(0x4A9EFF));
            changed |= c.slider("Strength", "", &mut wm.focus_glow_intensity, (0.0, 0.6), 0.01, percent);
        }
    });

    let a = &mut cfg.appearance;
    a.normalize_gradient();
    kit::caption(ui, "Window glow");
    kit::card(ui, "glow", |c| {
        let seed = shared_color(&a.window_gradient_stops, &a.accent);
        let h = c.ui.m.px(DIAGRAM_H);
        changed |= c.block(h, |ui, rect| glow_diagram(ui, rect, a, &seed));
        if a.window_gradient_stops.iter().any(|s| !s.is_empty()) {
            let mut color = seed.clone();
            if c.color("Colour", "", &mut color, look::GOLD) {
                for s in a.window_gradient_stops.iter_mut().filter(|s| !s.is_empty()) {
                    *s = color.clone();
                }
                changed = true;
            }
            let mut alpha = a.window_gradient_stop_alphas.first().copied().unwrap_or(1.0);
            if c.slider("Strength", "", &mut alpha, (0.0, 1.0), 0.01, percent) {
                a.window_gradient_stop_alphas = vec![alpha; GRADIENT_STOPS];
                changed = true;
            }
            changed |= c.slider("Reach", "How far in from its corner a glow spreads.", &mut a.window_gradient_radius, (0.1, 1.0), 0.05, percent);
        }
    });
    kit::note(ui, "A soft colour inside a window's corners or centre. Click a dot to light it.");
    changed
}

/// The colour the glow shares: the first position that is on, else the
/// accent.
fn shared_color(stops: &[String], accent: &str) -> String {
    stops.iter().find(|s| !s.is_empty()).cloned().unwrap_or_else(|| accent.to_owned())
}

const DIAGRAM_H: f64 = 270.0;
/// The little window, in logical pixels.
const WINDOW: Vec2 = Vec2::new(380.0, 220.0);

/// Where the five glows sit on `window`, in the order they are stored:
/// top-left, top-right, bottom-left, bottom-right, centre.
fn glow_points(window: Rect) -> [Vec2; GRADIENT_STOPS] {
    [window.min, Vec2::new(window.max.x, window.min.y), Vec2::new(window.min.x, window.max.y), window.max, window.center()]
}

/// A little window with the glows as they are set and a dot at each
/// position that turns its glow on and off. Returns `true` when one was
/// toggled.
fn glow_diagram(ui: &mut Ui, rect: Rect, a: &mut Appearance, seed: &str) -> bool {
    let m = ui.m;
    let window = Rect::from_center_size(rect.center(), Vec2::new(m.px(WINDOW.x), m.px(WINDOW.y))).round();
    let radius = m.px(16.0);
    let face = Color::parse_hex(&a.background_color).unwrap_or(Color::hex(0x0E0E0E)).with_alpha(1.0);
    ui.draw.rounded_rect(window, radius, face);

    // The glows: each fades out over `reach` from its point, as far as
    // the desktop draws it (a share of the window's half-diagonal).
    let reach = a.window_gradient_radius * window.size().length() * 0.5;
    let points = glow_points(window);
    ui.draw.push_clip(window.intersection(&ui.clip()));
    for (i, at) in points.iter().enumerate() {
        if let Some(color) = Color::parse_hex(&a.window_gradient_stops[i]) {
            let alpha = a.window_gradient_stop_alphas.get(i).copied().unwrap_or(1.0);
            ui.draw.shadow(Rect::from_center_size(*at, Vec2::splat(2.0)), 1.0, reach, color.with_alpha(alpha));
        }
    }
    ui.draw.pop_clip();
    // The clip is square; a band in the card's colour takes back what
    // spilled past the rounded corners.
    let band = m.px(8.0);
    ui.draw.stroke_rect(window.expand(band), band, radius + band, look::CARD);
    ui.draw.stroke_rect(window, m.px(2.0), radius, look::TRACK);

    let mut changed = false;
    let dot = m.px(13.0);
    let inset = m.px(38.0);
    ui.push_id("dots");
    for (i, at) in points.iter().enumerate() {
        // Corner dots sit in from their corner, toward the middle.
        let toward = window.center() - *at;
        let c = if toward.length() > 0.0 { *at + Vec2::new(toward.x.signum(), toward.y.signum()) * inset } else { *at };
        let id = ui.id("dot").with_index(i);
        let hit = Rect::from_center_size(c, Vec2::splat(m.px(56.0)));
        let mut r = ui.interact(id, hit, Sense::CLICK);
        ui.focusable(id, hit);
        ui.key_click(id, &mut r);
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
            ui.draw.circle(c, dot + m.px(8.0), Color::WHITE.fade(0.12));
        }
        let stop = &mut a.window_gradient_stops[i];
        if r.clicked {
            *stop = if stop.is_empty() { seed.to_owned() } else { String::new() };
            changed = true;
        }
        match Color::parse_hex(stop) {
            Some(color) => {
                ui.draw.circle(c, dot, look::TEXT);
                ui.draw.circle(c, dot - m.px(3.0), color.with_alpha(1.0));
            }
            None => {
                ui.draw.circle(c, dot, look::TEXT_DIM);
                ui.draw.circle(c, dot - m.px(3.0), face);
            }
        }
        ui.focus_ring(id, Rect::from_center_size(c, Vec2::splat(dot * 2.0)));
    }
    ui.pop_id();
    changed
}
