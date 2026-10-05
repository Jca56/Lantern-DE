//! The bundled cursor, recoloured the way the compositor does it, so the
//! preview on the Mouse page tracks what the pointer will look like.
//! Five canonical colours authored into the SVG are swapped for the
//! user's palette and every `stroke-width` is scaled.

use crate::config::Input;

/// The canonical colours in the shipped SVG, in palette order.
const CANONICAL: [&str; 5] = ["#ffffff", "#ababab", "#fab414", "#9a6300", "#0a0a0a"];

/// The bundled cursor's source: the compositor's runtime copy when it
/// exists, else the bytes built into `lntrn-icons`.
pub fn default_source() -> Option<String> {
    let runtime = lntrn_sys::dirs::lantern().map(|l| l.join("icons/cursors/lntrn-cursor.svg"));
    if let Some(p) = runtime
        && let Ok(text) = std::fs::read_to_string(p)
    {
        return Some(text);
    }
    lntrn_icons::get("lntrn-cursor.svg").and_then(|b| String::from_utf8(b.to_vec()).ok())
}

/// A key that changes whenever the recoloured cursor would.
pub fn key(input: &Input) -> String {
    format!("{}|{}|{}|{}|{}|{:.2}", input.cursor_body_light, input.cursor_body_dark, input.cursor_accent_light, input.cursor_accent_dark, input.cursor_outline_color, input.cursor_outline_scale)
}

pub fn customize(svg: &str, input: &Input) -> String {
    let palette = [&input.cursor_body_light, &input.cursor_body_dark, &input.cursor_accent_light, &input.cursor_accent_dark, &input.cursor_outline_color];
    let mut out = svg.to_owned();
    for (from, to) in CANONICAL.iter().zip(palette) {
        out = replace_hex(&out, from, to);
    }
    scale_strokes(&out, input.cursor_outline_scale)
}

fn replace_hex(s: &str, from_lower: &str, to: &str) -> String {
    let out = s.replace(from_lower, to);
    let upper = from_lower.to_uppercase();
    if upper != from_lower { out.replace(&upper, to) } else { out }
}

/// Multiply every `stroke-width="…"` attribute and `stroke-width: …`
/// style by `scale`.
fn scale_strokes(svg: &str, scale: f64) -> String {
    let scale = scale.max(0.0);
    let pass = |text: &str, key: &str, terminators: &[char]| {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find(key) {
            out.push_str(&rest[..at]);
            out.push_str(key);
            let after = &rest[at + key.len()..];
            let trimmed = after.trim_start();
            out.push_str(&after[..after.len() - trimmed.len()]);
            let end = trimmed.find(|c: char| terminators.contains(&c)).unwrap_or(trimmed.len());
            let value = &trimmed[..end];
            let unit_at = value.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(value.len());
            let number: f64 = value[..unit_at].parse().unwrap_or(0.0);
            out.push_str(&format!("{:.2}{}", (number * scale).max(0.0), &value[unit_at..]));
            rest = &trimmed[end..];
        }
        out.push_str(rest);
        out
    };
    let out = pass(svg, "stroke-width=\"", &['"']);
    pass(&out, "stroke-width:", &[';', '"', '}', '\n'])
}
