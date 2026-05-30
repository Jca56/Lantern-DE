//! Device + lighting panel: pick an RGB color and push it to the selected
//! peripheral's LEDs. Immediate-mode drawing + raw hit-testing, matching
//! the app-template's gallery contract so the window shell stays generic.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use lntrn_gear::caps::{Device, Rgb};

const TEXT_PRIMARY: Color = Color::rgb(0.92, 0.90, 0.96);
const TEXT_SECONDARY: Color = Color::rgb(0.55, 0.50, 0.66);
const SURFACE: Color = Color::rgba(0.10, 0.06, 0.18, 0.55);
const SURFACE_SEL: Color = Color::rgba(0.24, 0.15, 0.04, 0.70);
const WIDGET_BG: Color = Color::rgba(0.06, 0.03, 0.12, 0.60);
const BORDER: Color = Color::rgba(0.40, 0.28, 0.60, 0.30);
const ACCENT: Color = Color::rgb(0.82, 0.50, 0.02);

const PRESETS: &[(&str, u8, u8, u8)] = &[
    ("Gold", 200, 134, 10),
    ("White", 255, 255, 255),
    ("Red", 255, 0, 0),
    ("Green", 0, 220, 60),
    ("Blue", 0, 90, 255),
    ("Cyan", 0, 210, 210),
    ("Pink", 230, 40, 160),
    ("Off", 0, 0, 0),
];

struct Row {
    name: String,
    kind: String,
    has_light: bool,
}

pub struct PanelState {
    devices: Vec<Box<dyn Device>>,
    rows: Vec<Row>,
    selected: usize,
    /// R,G,B channels in 0.0..=1.0.
    chan: [f32; 3],
    drag: Option<usize>,
    status: String,
}

impl PanelState {
    pub fn new() -> Self {
        let mut devices = lntrn_gear::devices::scan();
        let rows: Vec<Row> = devices
            .iter_mut()
            .map(|d| Row {
                name: d.name().to_string(),
                kind: d.kind().label().to_string(),
                has_light: d.lighting().is_some(),
            })
            .collect();
        let status = if rows.is_empty() {
            "No devices found — is the udev rule installed? (then replug)".to_string()
        } else {
            format!("{} device(s) connected", rows.len())
        };
        Self {
            devices,
            rows,
            selected: 0,
            chan: [200.0 / 255.0, 134.0 / 255.0, 10.0 / 255.0], // Lantern gold
            drag: None,
            status,
        }
    }

    fn color(&self) -> Rgb {
        Rgb::new(
            (self.chan[0] * 255.0).round() as u8,
            (self.chan[1] * 255.0).round() as u8,
            (self.chan[2] * 255.0).round() as u8,
        )
    }

    /// Push the current color to the selected device's lighting.
    fn apply(&mut self) {
        let color = self.color();
        let Some(row) = self.rows.get(self.selected) else { return };
        let name = row.name.clone();
        if !row.has_light {
            self.status = format!("{name} has no lighting");
            return;
        }
        if let Some(dev) = self.devices.get_mut(self.selected) {
            if let Some(light) = dev.lighting() {
                self.status = match light.set_all(color) {
                    Ok(()) => format!("{name} → {}", color.hex()),
                    Err(e) => format!("{name}: {e}"),
                };
            }
        }
    }
}

// ── Layout (kept in lock-step between draw and hit-testing) ──────────────────

fn in_rect(cx: f32, cy: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    cx >= x && cx <= x + w && cy >= y && cy <= y + h
}

fn list_x(s: f32) -> f32 {
    32.0 * s
}
fn list_w(s: f32) -> f32 {
    280.0 * s
}
fn row_h(s: f32) -> f32 {
    54.0 * s
}
fn row_y(i: usize, s: f32, top: f32) -> f32 {
    top + 8.0 * s + i as f32 * (row_h(s) + 8.0 * s)
}
fn panel_x(s: f32) -> f32 {
    list_x(s) + list_w(s) + 48.0 * s
}
fn slider_w(s: f32) -> f32 {
    300.0 * s
}
fn slider_rect(i: usize, s: f32, top: f32) -> Rect {
    Rect::new(panel_x(s), top + 86.0 * s + i as f32 * 58.0 * s, slider_w(s), 8.0 * s)
}
fn preset_size(s: f32) -> f32 {
    42.0 * s
}
fn preset_rect(i: usize, s: f32, top: f32) -> Rect {
    let sz = preset_size(s);
    Rect::new(
        panel_x(s) + i as f32 * (sz + 10.0 * s),
        top + 86.0 * s + 3.0 * 58.0 * s + 36.0 * s,
        sz,
        sz,
    )
}

// ── Input ────────────────────────────────────────────────────────────────────

pub fn handle_click(cx: f32, cy: f32, s: f32, top: f32, _wf: f32, _hf: f32, ps: &mut PanelState) -> bool {
    // Device rows
    for i in 0..ps.rows.len() {
        if in_rect(cx, cy, list_x(s), row_y(i, s, top), list_w(s), row_h(s)) {
            ps.selected = i;
            return true;
        }
    }
    // Sliders (generous vertical hit band)
    for i in 0..3usize {
        let r = slider_rect(i, s, top);
        if in_rect(cx, cy, r.x - 12.0 * s, r.y - 16.0 * s, r.w + 24.0 * s, 36.0 * s) {
            ps.drag = Some(i);
            ps.chan[i] = ((cx - r.x) / r.w).clamp(0.0, 1.0);
            return true;
        }
    }
    // Preset swatches — set + apply immediately
    for (i, &(_, r, g, b)) in PRESETS.iter().enumerate() {
        let rc = preset_rect(i, s, top);
        if in_rect(cx, cy, rc.x, rc.y, rc.w, rc.h) {
            ps.chan = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
            ps.apply();
            return true;
        }
    }
    false
}

pub fn handle_drag(cx: f32, _cy: f32, s: f32, top: f32, _hf: f32, ps: &mut PanelState) {
    if let Some(i) = ps.drag {
        let r = slider_rect(i, s, top);
        ps.chan[i] = ((cx - r.x) / r.w).clamp(0.0, 1.0);
    }
}

pub fn handle_release(ps: &mut PanelState) {
    // Apply once on release so we don't spam HID++ writes every frame.
    if ps.drag.take().is_some() {
        ps.apply();
    }
}

// ── Draw ─────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw(
    p: &mut Painter,
    t: &mut TextRenderer,
    cx: f32,
    cy: f32,
    s: f32,
    top: f32,
    ps: &PanelState,
    wf: f32,
    _hf: f32,
    sw: u32,
    sh: u32,
) {
    // ── Device list ──────────────────────────────────────────────────
    t.queue("Devices", 16.0 * s, list_x(s), top - 16.0 * s, TEXT_SECONDARY, wf, sw, sh);
    if ps.rows.is_empty() {
        t.queue(
            "No devices.",
            18.0 * s,
            list_x(s),
            row_y(0, s, top) + 14.0 * s,
            TEXT_SECONDARY,
            list_w(s),
            sw,
            sh,
        );
    }
    for (i, row) in ps.rows.iter().enumerate() {
        let y = row_y(i, s, top);
        let rect = Rect::new(list_x(s), y, list_w(s), row_h(s));
        let selected = i == ps.selected;
        let hov = in_rect(cx, cy, rect.x, rect.y, rect.w, rect.h);
        p.rect_filled(rect, 8.0 * s, if selected { SURFACE_SEL } else { SURFACE });
        if selected {
            p.rect_stroke_sdf(rect, 8.0 * s, 1.5 * s, ACCENT);
        } else if hov {
            p.rect_stroke_sdf(rect, 8.0 * s, 1.0 * s, BORDER);
        }
        t.queue(&row.name, 17.0 * s, rect.x + 14.0 * s, y + 9.0 * s, TEXT_PRIMARY, list_w(s) - 28.0 * s, sw, sh);
        let sub = if row.has_light {
            format!("{} · lighting", row.kind)
        } else {
            format!("{} · no lighting", row.kind)
        };
        t.queue(&sub, 13.0 * s, rect.x + 14.0 * s, y + 31.0 * s, TEXT_SECONDARY, list_w(s) - 28.0 * s, sw, sh);
    }

    // ── Color editor (right) ─────────────────────────────────────────
    let px = panel_x(s);
    t.queue("Color", 16.0 * s, px, top - 16.0 * s, TEXT_SECONDARY, wf, sw, sh);

    // Live preview swatch (top-right of the sliders)
    let preview = Color::from_rgba8(
        (ps.chan[0] * 255.0) as u8,
        (ps.chan[1] * 255.0) as u8,
        (ps.chan[2] * 255.0) as u8,
        255,
    );
    let prev_rect = Rect::new(px + slider_w(s) + 24.0 * s, top + 8.0 * s, 64.0 * s, 64.0 * s);
    p.rect_filled(prev_rect, 10.0 * s, preview);
    p.rect_stroke_sdf(prev_rect, 10.0 * s, 1.0 * s, BORDER);

    // R / G / B sliders, each tinted by its channel.
    let labels = ["R", "G", "B"];
    let tints = [
        Color::rgb(0.90, 0.25, 0.25),
        Color::rgb(0.30, 0.85, 0.40),
        Color::rgb(0.35, 0.55, 0.95),
    ];
    for i in 0..3usize {
        let r = slider_rect(i, s, top);
        t.queue(labels[i], 16.0 * s, r.x - 24.0 * s, r.y - 7.0 * s, TEXT_SECONDARY, 30.0 * s, sw, sh);
        p.rect_filled(r, 4.0 * s, WIDGET_BG);
        let fill_w = r.w * ps.chan[i];
        p.rect_filled(Rect::new(r.x, r.y, fill_w, r.h), 4.0 * s, tints[i]);
        let thumb_x = r.x + fill_w;
        let thumb_y = r.y + r.h * 0.5;
        p.circle_filled(thumb_x, thumb_y, 10.0 * s, tints[i]);
        p.circle_filled(thumb_x, thumb_y, 5.0 * s, Color::rgb(0.96, 0.94, 0.90));
        let val = format!("{}", (ps.chan[i] * 255.0).round() as u8);
        t.queue(&val, 14.0 * s, r.x + r.w + 16.0 * s, r.y - 6.0 * s, TEXT_SECONDARY, 60.0 * s, sw, sh);
    }

    // Presets
    t.queue(
        "Presets",
        14.0 * s,
        px,
        preset_rect(0, s, top).y - 24.0 * s,
        TEXT_SECONDARY,
        wf,
        sw,
        sh,
    );
    for (i, &(_, r, g, b)) in PRESETS.iter().enumerate() {
        let rc = preset_rect(i, s, top);
        p.rect_filled(rc, 8.0 * s, Color::from_rgba8(r, g, b, 255));
        let hov = in_rect(cx, cy, rc.x, rc.y, rc.w, rc.h);
        p.rect_stroke_sdf(rc, 8.0 * s, if hov { 1.6 * s } else { 1.0 * s }, BORDER);
    }

    // Status line
    t.queue(
        &ps.status,
        14.0 * s,
        px,
        preset_rect(0, s, top).y + preset_size(s) + 18.0 * s,
        TEXT_SECONDARY,
        wf - px - 16.0 * s,
        sw,
        sh,
    );
}
