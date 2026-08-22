//! ANSI render passes — the big clock and the config panel.

use crate::config::{Config, Format, Style, COLOR_CHOICES};
use crate::glyphs::{glyph, GLYPH_H};

// Rainbow palette (digits cycle through these in ColorMode::Rainbow).
const RAINBOW: &[(u8, u8, u8)] = &[
    (255, 105, 180), // pink
    (255, 176, 46),  // amber
    (240, 220, 80),  // yellow
    (120, 220, 130), // green
    (80, 200, 240),  // cyan
    (170, 110, 230), // violet
];

/// Build a frame: HOME, clear, paint clock, optionally overlay the config
/// panel, ANSI reset. Caller writes the bytes in one syscall.
pub fn render(
    buf: &mut Vec<u8>,
    cfg: &Config,
    cols: u16,
    rows: u16,
    time_str: &str,
    show_panel: bool,
    panel: &PanelState,
) {
    buf.extend_from_slice(b"\x1b[H"); // cursor to 0,0
    buf.extend_from_slice(b"\x1b[2J"); // clear
    paint_clock(buf, cfg, cols, rows, time_str);
    if show_panel {
        paint_panel(buf, cols, rows, cfg, panel);
    }
    buf.extend_from_slice(b"\x1b[0m");
}

/// Build the digit string for the current second based on format settings.
pub fn format_time(cfg: &Config, h: u32, m: u32, s: u32) -> String {
    let (hh, _suffix) = match cfg.format {
        Format::H24 => (h, ""),
        Format::H12 => {
            let am = h < 12;
            let twelve = match h % 12 {
                0 => 12,
                n => n,
            };
            (twelve, if am { " AM" } else { " PM" })
        }
    };
    if cfg.show_seconds {
        format!("{:02}:{:02}:{:02}", hh, m, s)
    } else {
        format!("{:02}:{:02}", hh, m)
    }
}

// ── Clock body ──────────────────────────────────────────────────────────

fn paint_clock(buf: &mut Vec<u8>, cfg: &Config, cols: u16, rows: u16, text: &str) {
    let glyph_h = GLYPH_H * cfg.scale_y as usize;
    let gap = cfg.scale_x as usize; // gap between chars scales with width
    let total_w: usize = text
        .chars()
        .map(|c| {
            glyph(c)
                .map(|g| g.width * cfg.scale_x as usize)
                .unwrap_or(0)
        })
        .sum::<usize>()
        + gap * text.chars().count().saturating_sub(1);

    let x0 = ((cols as usize).saturating_sub(total_w)) / 2;
    let y0 = ((rows as usize).saturating_sub(glyph_h)) / 2;

    let on_char: &[u8] = match cfg.style {
        Style::Solid => "█".as_bytes(),
        Style::Shaded => "▓".as_bytes(),
    };

    // Walk each char, lay out at x_cursor, paint scaled cells.
    let mut x_cursor = x0;
    for (idx, ch) in text.chars().enumerate() {
        let g = match glyph(ch) {
            Some(g) => g,
            None => continue,
        };
        let color = pick_color(cfg, idx, ch);
        if let Some((r, g_, b)) = color {
            write_fg_rgb(buf, r, g_, b);
        }
        for (row_i, row) in g.rows.iter().enumerate() {
            for (col_i, &on) in row.iter().enumerate() {
                if !on {
                    continue;
                }
                for sy in 0..cfg.scale_y as usize {
                    let y = y0 + row_i * cfg.scale_y as usize + sy;
                    let x = x_cursor + col_i * cfg.scale_x as usize;
                    move_cursor(buf, x as u16, y as u16);
                    for _ in 0..cfg.scale_x {
                        buf.extend_from_slice(on_char);
                    }
                }
            }
        }
        x_cursor += g.width * cfg.scale_x as usize + gap;
    }
}

fn pick_color(cfg: &Config, char_index: usize, ch: char) -> Option<(u8, u8, u8)> {
    if cfg.is_rainbow() {
        if ch == ':' || ch == ' ' {
            return Some((235, 235, 235));
        }
        return Some(RAINBOW[char_index % RAINBOW.len()]);
    }
    cfg.resolve_color()
}

// ── Config panel ────────────────────────────────────────────────────────

pub struct PanelState {
    pub cursor: usize,
}

impl PanelState {
    pub fn new() -> Self {
        Self { cursor: 0 }
    }
}

/// Setting rows shown in the panel. Order matches the index returned by
/// `panel_apply` so the run loop can mutate the right field.
pub const PANEL_ROWS: &[&str] = &[
    "Format",
    "Seconds",
    "Color",
    "Style",
    "Scale wide",
    "Scale tall",
];

pub const PANEL_ROW_COUNT: usize = PANEL_ROWS.len();

fn current_value(cfg: &Config, row: usize) -> String {
    match row {
        0 => match cfg.format {
            Format::H24 => "24-hour".into(),
            Format::H12 => "12-hour".into(),
        },
        1 => {
            if cfg.show_seconds {
                "on".into()
            } else {
                "off".into()
            }
        }
        2 => cfg.color.clone(),
        3 => match cfg.style {
            Style::Solid => "solid",
            Style::Shaded => "shaded",
        }
        .into(),
        4 => cfg.scale_x.to_string(),
        5 => cfg.scale_y.to_string(),
        _ => String::new(),
    }
}

fn paint_panel(buf: &mut Vec<u8>, cols: u16, rows: u16, cfg: &Config, panel: &PanelState) {
    let pw: u16 = 42;
    let ph: u16 = (PANEL_ROW_COUNT as u16) + 6;
    let px = cols.saturating_sub(pw) / 2;
    let py = rows.saturating_sub(ph) / 2;

    // Background: just a contrasting block so it reads over the clock.
    for r in 0..ph {
        move_cursor(buf, px, py + r);
        buf.extend_from_slice(b"\x1b[48;5;236m\x1b[38;5;250m");
        for _ in 0..pw {
            buf.push(b' ');
        }
    }

    // Title
    let title = " lclock — configure ";
    move_cursor(
        buf,
        px + (pw.saturating_sub(title.len() as u16)) / 2,
        py + 1,
    );
    buf.extend_from_slice(b"\x1b[1m\x1b[38;5;222m");
    buf.extend_from_slice(title.as_bytes());
    buf.extend_from_slice(b"\x1b[22m\x1b[38;5;250m");

    // Rows
    for (i, label) in PANEL_ROWS.iter().enumerate() {
        let y = py + 3 + i as u16;
        move_cursor(buf, px + 2, y);
        if i == panel.cursor {
            buf.extend_from_slice(b"\x1b[1m\x1b[38;5;222m> ");
        } else {
            buf.extend_from_slice(b"\x1b[38;5;250m  ");
        }
        buf.extend_from_slice(label.as_bytes());
        let value = current_value(cfg, i);
        let label_w = label.len() + 2;
        let pad = (pw as usize).saturating_sub(label_w + value.len() + 5);
        for _ in 0..pad {
            buf.push(b' ');
        }
        // Swatch next to the color row so the choice is visible.
        if i == 2 {
            if let Some((r, g, b)) = cfg.resolve_color() {
                write_fg_rgb(buf, r, g, b);
                buf.extend_from_slice("● ".as_bytes());
                buf.extend_from_slice(b"\x1b[38;5;250m");
            }
        }
        buf.extend_from_slice(value.as_bytes());
        buf.extend_from_slice(b"\x1b[22m");
    }

    // Footer
    let footer = "↑/↓ select  ←/→ change  q/Esc close";
    move_cursor(
        buf,
        px + (pw.saturating_sub(footer.chars().count() as u16)) / 2,
        py + ph - 2,
    );
    buf.extend_from_slice(b"\x1b[2m");
    buf.extend_from_slice(footer.as_bytes());
    buf.extend_from_slice(b"\x1b[22m");
}

// Adjust the setting at `panel.cursor` by `dir` (-1 or +1). Returns true if
// changed so the caller knows to persist.
pub fn panel_adjust(cfg: &mut Config, row: usize, dir: i32) -> bool {
    match row {
        0 => {
            cfg.format = match cfg.format {
                Format::H24 => Format::H12,
                Format::H12 => Format::H24,
            };
        }
        1 => {
            cfg.show_seconds = !cfg.show_seconds;
        }
        2 => {
            let i = COLOR_CHOICES
                .iter()
                .position(|c| *c == cfg.color)
                .unwrap_or(0);
            let n = COLOR_CHOICES.len() as i32;
            let next = (((i as i32) + dir).rem_euclid(n)) as usize;
            cfg.color = COLOR_CHOICES[next].into();
        }
        3 => {
            let order = [Style::Solid, Style::Shaded];
            let i = order.iter().position(|s| *s == cfg.style).unwrap_or(0);
            let n = order.len() as i32;
            let next = (((i as i32) + dir).rem_euclid(n)) as usize;
            cfg.style = order[next];
        }
        4 => {
            let v = (cfg.scale_x as i32 + dir).clamp(1, 6);
            cfg.scale_x = v as u8;
        }
        5 => {
            let v = (cfg.scale_y as i32 + dir).clamp(1, 4);
            cfg.scale_y = v as u8;
        }
        _ => return false,
    }
    true
}

// ── ANSI helpers ────────────────────────────────────────────────────────

fn move_cursor(buf: &mut Vec<u8>, col: u16, row: u16) {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, "\x1b[{};{}H", row + 1, col + 1);
    buf.extend_from_slice(s.as_bytes());
}

fn write_fg_rgb(buf: &mut Vec<u8>, r: u8, g: u8, b: u8) {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, "\x1b[38;2;{};{};{}m", r, g, b);
    buf.extend_from_slice(s.as_bytes());
}
