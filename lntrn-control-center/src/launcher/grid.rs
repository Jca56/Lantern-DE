//! The grid: each app a tile, its icon above its name (two lines at most,
//! then an ellipsis), as many tiles across as fit the icon size with room
//! for a name, the spare width shared out. The keyboard moves a tile left
//! or right and a row up or down, scrolling to keep the selection in view.

use lntrn_apps::Icons;
use lntrn_math::{Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Metrics, Sense, Ui};

use super::Launcher;
use super::menu::{self, Hit, NAME};

/// Around a tile's icon and name, logical.
const PAD: f64 = 10.0;
/// Between the icon and the name.
const ICON_GAP: f64 = 8.0;
/// The lines a name gets.
const LINES: usize = 2;
/// Kept free at a line's ends so its last glyph isn't clipped.
const SLACK: f64 = 4.0;

/// How the grid fits a width: how many tiles across, and each tile's size,
/// window px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub cols: usize,
    pub tile: Vec2,
}

/// As many tiles across `width` as each keeps 1.4 icons' width and its
/// padding, for a name to wrap in; tall enough for the icon and two lines
/// of `line_h`. `icon` and `width` are window px.
pub fn fit(width: f64, icon: f64, line_h: f64, m: Metrics) -> Fit {
    let least = icon * 1.4 + m.px(PAD) * 2.0 + m.px(4.0);
    let cols = ((width / least).floor() as usize).max(1);
    let h = m.px(PAD) * 2.0 + icon + m.px(ICON_GAP) + line_h * LINES as f64;
    Fit { cols, tile: Vec2::new(width / cols as f64, h) }
}

/// Where the selection lands from `at`, among `len` tiles `cols` across,
/// `across` tiles on (back when negative, through the rows' ends) and
/// then `rows` rows down (up). Held in the grid: up from the top row stays
/// on it, down onto a last row too short for its column takes its last
/// tile.
pub fn moved(at: usize, len: usize, cols: usize, rows: isize, across: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let (last, cols) = (len - 1, cols.max(1));
    let i = at.min(last).saturating_add_signed(across).min(last);
    let row = (i / cols) as isize + rows;
    let row = row.clamp(0, (last / cols) as isize) as usize;
    (row * cols + i % cols).min(last)
}

/// The apps that show, as tiles, scrolled by the wheel and to keep the
/// lit one in view, the light following a pointer that `pointed`. Returns
/// the app clicked or right-clicked, by index; a right-click lights its
/// tile too.
pub fn draw(l: &mut Launcher, icons: &mut Icons, ui: &mut Ui, rect: Rect, shown: &[usize], fit: Fit, pointed: bool) -> Option<Hit> {
    let m = ui.m;
    let rows = shown.len().div_ceil(fit.cols);
    let content = rows as f64 * fit.tile.y;
    l.selected = l.selected.map(|s| s.min(shown.len() - 1));
    let selected_row = l.selected.map(|s| ((s / fit.cols) as f64 * fit.tile.y, fit.tile.y));
    let max = menu::scroll(l, ui, rect, content, selected_row);

    let name = TextStyle::new(m.px(NAME) as f32);
    let line_h = f64::from(name.line_height());
    let icon = m.px(f64::from(l.options.grid_icon));
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    let first = (l.scroll / fit.tile.y).floor() as usize;
    let last = (((l.scroll + rect.height()) / fit.tile.y).ceil() as usize).min(rows);
    let (mut hit, mut hovered) = (None, None);
    ui.draw.push_clip(rect);
    for row in first..last {
        for col in 0..fit.cols {
            let pos = row * fit.cols + col;
            let Some(&i) = shown.get(pos) else { break };
            let at = Vec2::new(rect.min.x + col as f64 * fit.tile.x, rect.min.y + row as f64 * fit.tile.y - l.scroll);
            let tile = Rect::from_min_size(at, fit.tile);
            let r = ui.interact(ui.id("tile").with_index(i), tile.intersection(&rect), Sense::CLICK);
            if r.clicked {
                hit = Some(Hit::Launch(i));
            }
            if r.hovered {
                hovered = Some(pos);
            }
            if r.hovered && ui.state.right_pressed {
                (l.selected, l.by_pointer) = (Some(pos), true);
                hit = Some(Hit::Menu(i));
            }
            let lit = tile.shrink(m.px(3.0));
            if Some(pos) == l.selected {
                ui.draw.rounded_rect(lit, m.px(12.0), accent.fade(0.20));
            } else if r.hovered {
                ui.draw.rounded_rect(lit, m.px(12.0), lntrn_math::Color::WHITE.fade(0.06));
            }
            let app = &l.apps[i];
            let pic = Rect::from_min_size(Vec2::new((tile.center().x - icon * 0.5).round(), tile.min.y + m.px(PAD)), Vec2::splat(icon));
            menu::picture(ui, icons, app, pic);
            let width = tile.width() - m.px(PAD) * 2.0;
            let lines = wrap(&mut |s| ui.measure(s, &name), &app.name, width - m.px(SLACK), LINES);
            let mut y = pic.max.y + m.px(ICON_GAP);
            for line in &lines {
                let band = Rect::from_min_size(Vec2::new(tile.min.x + m.px(PAD), y), Vec2::new(width, line_h));
                ui.text_centered(line, &name, band, ink);
                y += line_h;
            }
        }
    }
    ui.draw.pop_clip();
    menu::scrollbar(ui, rect, l.scroll, max, content);
    menu::follow(l, ui, pointed, hovered);
    hit
}

/// `name` in at most `lines` lines no wider than `width` by `measure`:
/// broken between words where one fits, inside a word too long for a line
/// of its own, and the last ending in "…" when the rest won't fit on it.
pub fn wrap(measure: &mut impl FnMut(&str) -> f64, name: &str, width: f64, lines: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = name.trim();
    while !rest.is_empty() && out.len() < lines {
        if measure(rest) <= width {
            out.push(rest.to_owned());
            break;
        }
        if out.len() + 1 == lines {
            let cut = longest(measure, rest, width, "…");
            out.push(format!("{}…", rest[..cut].trim_end()));
            break;
        }
        // As many whole words as fit, else as much of the first.
        let mut cut = None;
        for (at, _) in rest.match_indices(' ') {
            if measure(rest[..at].trim_end()) > width {
                break;
            }
            cut = Some(at);
        }
        let (line, next) = match cut {
            Some(at) => (rest[..at].trim_end(), rest[at..].trim_start()),
            None => {
                let at = longest(measure, rest, width, "");
                (&rest[..at], &rest[at..])
            }
        };
        out.push(line.to_owned());
        rest = next;
    }
    out
}

/// The most of `s` (a byte index at a character's end, one character at
/// least) that still fits `width` with `then` after it.
fn longest(measure: &mut impl FnMut(&str) -> f64, s: &str, width: f64, then: &str) -> usize {
    let mut best = None;
    for end in s.char_indices().map(|(at, c)| at + c.len_utf8()) {
        if measure(&format!("{}{then}", &s[..end])) > width {
            break;
        }
        best = Some(end);
    }
    best.unwrap_or_else(|| s.chars().next().map_or(0, char::len_utf8))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten px a character, standing in for the text engine.
    fn ten(s: &str) -> f64 {
        s.chars().count() as f64 * 10.0
    }

    #[test]
    fn names_break_between_words_into_two_lines_then_an_ellipsis() {
        let w = &mut ten;
        assert_eq!(wrap(w, "Steam", 100.0, 2), ["Steam"]);
        assert_eq!(wrap(w, "Mozilla Firefox", 100.0, 2), ["Mozilla", "Firefox"]);
        assert_eq!(wrap(w, "System Settings", 100.0, 2), ["System", "Settings"]);
        assert_eq!(wrap(w, "GNU Image Manipulation Program", 100.0, 2), ["GNU Image", "Manipulat…"]);
        assert_eq!(wrap(w, "Supercalifragilistic", 100.0, 2), ["Supercalif", "ragilistic"], "a word too long for a line breaks inside");
        assert_eq!(wrap(w, "Mozilla Firefox", 100.0, 1), ["Mozilla F…"]);
        assert!(wrap(w, "  ", 100.0, 2).is_empty());
        assert_eq!(wrap(w, "Code", 5.0, 2), ["C", "o…"], "never stuck: a character a line at least");
    }

    #[test]
    fn the_keyboard_moves_through_the_grid_and_stays_in_it() {
        // Seven tiles, three across:
        // 0 1 2
        // 3 4 5
        // 6
        assert_eq!(moved(0, 7, 3, 0, 1), 1);
        assert_eq!(moved(2, 7, 3, 0, 1), 3, "right off a row's end: the next row's first");
        assert_eq!(moved(3, 7, 3, 0, -1), 2);
        assert_eq!(moved(0, 7, 3, 0, -1), 0, "held at the first");
        assert_eq!(moved(6, 7, 3, 0, 1), 6, "held at the last");
        assert_eq!(moved(1, 7, 3, 1, 0), 4);
        assert_eq!(moved(1, 7, 3, -1, 0), 1, "up from the top row stays");
        assert_eq!(moved(5, 7, 3, 1, 0), 6, "down onto a short row: its last tile");
        assert_eq!(moved(4, 7, 3, 5, 0), 6, "many rows down: the bottom");
        assert_eq!(moved(9, 7, 3, 0, 0), 6, "a selection past the end comes back");
        assert_eq!(moved(0, 0, 3, 1, 1), 0);
    }

    #[test]
    fn as_many_columns_as_the_icon_size_leaves_room_for() {
        let m = lntrn_ui::Theme::default().metrics(1.0);
        let at = |icon: f64| fit(504.0, icon, 24.0, m);
        assert_eq!(at(48.0).cols, 5);
        assert_eq!(at(64.0).cols, 4);
        assert_eq!(at(96.0).cols, 3);
        assert_eq!(at(64.0).tile, Vec2::new(126.0, 20.0 + 64.0 + 8.0 + 48.0), "the spare width shared out; tall for two lines");
        assert_eq!(fit(50.0, 64.0, 24.0, m).cols, 1, "one, however narrow");
    }
}
