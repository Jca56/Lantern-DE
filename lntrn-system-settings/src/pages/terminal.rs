//! The Terminal page: how big its text is and what its cursor looks
//! like, over a line of it drawn as the terminal would, and how its
//! window opens. The terminal's own menus change the same keys.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::Ui;

use crate::config::{CURSOR_STYLES, Config, TERMINAL_FONT_SIZES};
use crate::kit::{self, look};

const CURSORS: [(&str, &str); 3] = [(CURSOR_STYLES[0], "Block"), (CURSOR_STYLES[1], "Underline"), (CURSOR_STYLES[2], "Beam")];
/// The tallest the sample line's well gets, in logical pixels.
const SAMPLE_H: f64 = 96.0;

fn size(v: f64) -> String {
    format!("{v:.1} px")
}

/// Text and the well it sits in, as the terminal picks them: by the
/// window style, with the background the user chose over the style's.
fn inks(cfg: &Config) -> (Color, Color) {
    let (text, well) = if cfg.appearance.theme.starts_with("fox") { (Color::hex(0xECECEC), Color::hex(0x181818)) } else { (Color::hex(0xF0E6D2), Color::hex(0x1E1914)) };
    (text, Color::parse_hex(&cfg.appearance.background_color).map_or(well, |c| c.with_alpha(1.0)))
}

/// A prompt and a command at the size and with the cursor chosen.
fn sample(ui: &mut Ui, rect: Rect, font_size: f64, cursor: &str, (text, well): (Color, Color)) {
    let m = ui.m;
    let well_rect = rect.shrink(m.px(4.0));
    ui.draw.rounded_rect(well_rect, m.px(10.0), well);
    ui.draw.stroke_rect(well_rect, m.px(1.0), m.px(10.0), look::LINE);
    let mut style = kit::mono_style(ui);
    style.size = m.px(font_size) as f32;
    let inner = Rect::new(Vec2::new(well_rect.min.x + m.px(18.0), well_rect.min.y), Vec2::new(well_rect.max.x - m.px(18.0), well_rect.max.y));
    let (prompt, typed) = ("~/Projects $ ", "echo hello");
    let accent = ui.theme.accent;
    ui.text_in_rect(prompt, &style, inner, accent);
    let prompt_w = ui.measure(prompt, &style);
    let after = Rect::new(Vec2::new(inner.min.x + prompt_w, inner.min.y), inner.max);
    ui.text_in_rect(typed, &style, after, text);
    // The cursor, in the cell after what was typed.
    let cell_w = ui.measure("M", &style);
    let line_h = f64::from(style.line_height());
    let x = after.min.x + ui.measure(typed, &style);
    if x + cell_w > inner.max.x {
        return;
    }
    let top = (inner.center().y - line_h * 0.5).round();
    let thin = m.px(2.0).max(1.0);
    let cell = match cursor {
        "underline" => Rect::new(Vec2::new(x, top + line_h - thin), Vec2::new(x + cell_w, top + line_h)),
        "beam" => Rect::new(Vec2::new(x, top), Vec2::new(x + thin, top + line_h)),
        _ => Rect::new(Vec2::new(x, top), Vec2::new(x + cell_w, top + line_h)),
    };
    ui.draw.rect(cell, accent);
}

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let inks = inks(cfg);
    let t = &mut cfg.terminal;
    let mut changed = false;

    kit::caption(ui, "Text");
    kit::card(ui, "text", |c| {
        let h = c.ui.m.px(SAMPLE_H);
        let (font_size, cursor) = (t.font_size, t.cursor_style.clone());
        c.block(h, |ui, rect| sample(ui, rect, font_size, &cursor, inks));
        changed |= c.slider("Text size", "", &mut t.font_size, TERMINAL_FONT_SIZES, 0.5, size);
        changed |= c.choice("Cursor", "Until a program asks for another.", &mut t.cursor_style, &CURSORS);
    });
    kit::note(ui, "Its colours follow the window style and background on the Appearance page.");

    kit::caption(ui, "Window");
    kit::card(ui, "window", |c| {
        changed |= c.switch("Open with the title bar hidden", "Just the terminal. Super+F11 shows and hides the bar.", &mut t.open_bar_hidden);
    });
    kit::note(ui, "These are in the terminal's own right-click and View menus too, along with the window's buttons for when its bar is hidden.");
    changed
}
