//! The few widgets the pages share: section headings, a colour picker
//! over a hex string, a dropdown over a stored word, integer sliders.

use lntrn_math::Color;
use lntrn_ui::Ui;

/// A heading with room above it.
pub fn section(ui: &mut Ui, title: &str) {
    ui.space(ui.m.pad * 1.5);
    ui.heading(title);
}

/// Dim explanatory text under a control.
pub fn note(ui: &mut Ui, text: &str) {
    ui.label_dim(text);
}

/// A colour picker that reads and writes a `#RRGGBB` string. An empty
/// or bad string shows as `fallback`.
pub fn hex_color(ui: &mut Ui, label: &str, hex: &mut String, fallback: Color) -> bool {
    let mut c = Color::parse_hex(hex).unwrap_or(fallback);
    c.a = 1.0;
    if ui.color_picker(label, &mut c) {
        c.a = 1.0;
        *hex = c.to_hex_string();
        return true;
    }
    false
}

/// A labelled dropdown over `(stored word, shown label)` pairs. A stored
/// word not in the list shows the first option.
pub fn choice(ui: &mut Ui, label: &str, value: &mut String, options: &[(&str, &str)]) -> bool {
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let mut index = options.iter().position(|o| o.0 == value.as_str()).unwrap_or(0);
    let mut changed = false;
    ui.labelled(label, |ui| {
        if ui.dropdown(label, &mut index, &labels) {
            *value = options[index].0.to_owned();
            changed = true;
        }
    });
    changed
}

/// A slider over an integer.
pub fn slider_int(ui: &mut Ui, label: &str, value: &mut i64, min: i64, max: i64, step: i64) -> bool {
    let mut f = *value as f64;
    if ui.slider(label, &mut f, min as f64, max as f64, step as f64) {
        *value = f.round() as i64;
        return true;
    }
    false
}

/// A toggle that stands for "this string is set": on seeds it with
/// `seed`, off clears it.
pub fn toggle_string(ui: &mut Ui, label: &str, value: &mut String, seed: &str) -> bool {
    let mut on = !value.is_empty();
    if ui.toggle(label, &mut on) {
        *value = if on { seed.to_owned() } else { String::new() };
        return true;
    }
    false
}
