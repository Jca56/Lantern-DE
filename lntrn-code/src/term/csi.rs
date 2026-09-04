//! What each escape sequence does to the grid: cursor motion, erasing,
//! scrolling, modes, SGR colors and attributes, the replies a program may
//! ask for, and OSC titles.

use super::grid::{BOLD, DIM, Grid, HIDDEN, INVERSE, ITALIC, STRIKE, Style, TermColor, UNDERLINE};
use super::parser::Action;

pub fn dispatch(g: &mut Grid, a: Action) {
    match a {
        Action::Print(c) => g.print(c),
        Action::Execute(b) => execute(g, b),
        Action::Csi { params, private, intermediate, final_byte } => csi(g, &params, private, intermediate, final_byte),
        Action::Esc { intermediate, final_byte } => esc(g, intermediate, final_byte),
        Action::Osc(bytes) => osc(g, &bytes),
    }
}

fn execute(g: &mut Grid, b: u8) {
    match b {
        0x07 => g.bell = true,
        0x08 => g.backspace(),
        0x09 => g.tab(),
        0x0A..=0x0C => g.linefeed(),
        0x0D => g.carriage_return(),
        0x0E => g.graphics_charset = true,
        0x0F => g.graphics_charset = false,
        _ => {}
    }
}

/// Parameter `i` as a count: missing or zero means `default`.
fn count(p: &[u16], i: usize, default: usize) -> usize {
    p.get(i).copied().filter(|&v| v != 0).map_or(default, |v| v as usize)
}

/// Parameter `i` as a selector: missing means zero.
fn sel(p: &[u16], i: usize) -> u16 {
    p.get(i).copied().unwrap_or(0)
}

fn csi(g: &mut Grid, p: &[u16], private: Option<u8>, intermediate: Option<u8>, f: u8) {
    let n = count(p, 0, 1);
    match (private, intermediate, f) {
        (None, None, b'@') => g.insert_chars(n),
        (None, None, b'A') => g.move_by(0, -(n as isize)),
        (None, None, b'B' | b'e') => g.move_by(0, n as isize),
        (None, None, b'C' | b'a') => g.move_by(n as isize, 0),
        (None, None, b'D') => g.move_by(-(n as isize), 0),
        (None, None, b'E') => {
            g.move_by(0, n as isize);
            g.carriage_return();
        }
        (None, None, b'F') => {
            g.move_by(0, -(n as isize));
            g.carriage_return();
        }
        (None, None, b'G' | b'`') => g.move_to(n - 1, g.cursor.y),
        (None, None, b'H' | b'f') => g.move_to(count(p, 1, 1) - 1, n - 1),
        (None, None, b'I') => (0..n).for_each(|_| g.tab()),
        (None, None, b'J') => g.erase_in_display(sel(p, 0)),
        (None, None, b'K') => g.erase_in_line(sel(p, 0)),
        (None, None, b'L') => g.insert_lines(n),
        (None, None, b'M') => g.delete_lines(n),
        (None, None, b'P') => g.delete_chars(n),
        (None, None, b'S') => g.scroll_up(n),
        (None, None, b'T') => g.scroll_down(n),
        (None, None, b'X') => g.erase_chars(n),
        (None, None, b'Z') => (0..n).for_each(|_| g.back_tab()),
        (None, None, b'd') => g.move_to(g.cursor.x, n - 1),
        (None, None, b'g') => g.clear_tab(sel(p, 0) == 3),
        (None, None, b'h' | b'l') => {
            for &m in p {
                if m == 4 {
                    g.insert_mode = f == b'h';
                }
            }
        }
        (Some(b'?'), None, b'h' | b'l') => {
            let on = f == b'h';
            for &m in p {
                private_mode(g, m, on);
            }
        }
        (None, None, b'm') => sgr(g, p),
        (None, None, b'n') => match sel(p, 0) {
            5 => g.replies.extend_from_slice(b"\x1b[0n"),
            6 => {
                let row = g.cursor.y + 1 - if g.origin_mode { g.top() } else { 0 };
                g.replies.extend_from_slice(format!("\x1b[{};{}R", row, g.cursor.x + 1).as_bytes());
            }
            _ => {}
        },
        (None, None, b'r') => {
            let bottom = if p.len() > 1 { count(p, 1, g.rows) - 1 } else { g.rows - 1 };
            g.set_region(n - 1, bottom);
        }
        (None, None, b's') => g.save_cursor(),
        (None, None, b'u') => g.restore_cursor(),
        (None, None, b'c') => g.replies.extend_from_slice(b"\x1b[?62;22c"),
        (Some(b'>'), None, b'c') => g.replies.extend_from_slice(b"\x1b[>0;10;1c"),
        _ => {}
    }
}

fn private_mode(g: &mut Grid, m: u16, on: bool) {
    match m {
        1 => g.app_cursor = on,
        6 => {
            g.origin_mode = on;
            g.move_to(0, 0);
        }
        7 => g.autowrap = on,
        25 => g.cursor_visible = on,
        47 | 1047 => {
            if on {
                g.enter_alt();
            } else {
                g.leave_alt();
            }
        }
        1048 => {
            if on {
                g.save_cursor();
            } else {
                g.restore_cursor();
            }
        }
        1049 => {
            if on {
                g.save_cursor();
                g.enter_alt();
            } else {
                g.leave_alt();
                g.restore_cursor();
            }
        }
        1000 | 1002 | 1003 | 1005 | 1015 => g.mouse_reporting = on,
        1006 => g.mouse_sgr = on,
        2004 => g.bracketed_paste = on,
        _ => {}
    }
}

/// A `38;5;n` / `38;2;r;g;b` color at `p[i]`, advancing `i` past it.
fn extended(p: &[u16], i: &mut usize) -> Option<TermColor> {
    match p.get(*i + 1) {
        Some(5) => {
            let c = TermColor::Indexed(p.get(*i + 2).copied().unwrap_or(0).min(255) as u8);
            *i += 2;
            Some(c)
        }
        Some(2) => {
            let ch = |k: usize| p.get(*i + k).copied().unwrap_or(0).min(255) as u8;
            let c = TermColor::Rgb(ch(2), ch(3), ch(4));
            *i += 4;
            Some(c)
        }
        _ => None,
    }
}

fn sgr(g: &mut Grid, p: &[u16]) {
    if p.is_empty() {
        g.pen = Style::default();
        return;
    }
    let mut i = 0;
    while i < p.len() {
        let v = p[i];
        let pen = &mut g.pen;
        match v {
            0 => *pen = Style::default(),
            1 => pen.flags |= BOLD,
            2 => pen.flags |= DIM,
            3 => pen.flags |= ITALIC,
            4 | 21 => pen.flags |= UNDERLINE,
            7 => pen.flags |= INVERSE,
            8 => pen.flags |= HIDDEN,
            9 => pen.flags |= STRIKE,
            22 => pen.flags &= !(BOLD | DIM),
            23 => pen.flags &= !ITALIC,
            24 => pen.flags &= !UNDERLINE,
            27 => pen.flags &= !INVERSE,
            28 => pen.flags &= !HIDDEN,
            29 => pen.flags &= !STRIKE,
            30..=37 => pen.fg = TermColor::Indexed((v - 30) as u8),
            38 => {
                if let Some(c) = extended(p, &mut i) {
                    g.pen.fg = c;
                }
            }
            39 => pen.fg = TermColor::Default,
            40..=47 => pen.bg = TermColor::Indexed((v - 40) as u8),
            48 => {
                if let Some(c) = extended(p, &mut i) {
                    g.pen.bg = c;
                }
            }
            49 => pen.bg = TermColor::Default,
            90..=97 => pen.fg = TermColor::Indexed((v - 90 + 8) as u8),
            100..=107 => pen.bg = TermColor::Indexed((v - 100 + 8) as u8),
            _ => {}
        }
        i += 1;
    }
}

fn esc(g: &mut Grid, intermediate: Option<u8>, f: u8) {
    match (intermediate, f) {
        (None, b'7') => g.save_cursor(),
        (None, b'8') => g.restore_cursor(),
        (None, b'D') => g.linefeed(),
        (None, b'E') => {
            g.carriage_return();
            g.linefeed();
        }
        (None, b'H') => g.set_tab(),
        (None, b'M') => g.reverse_index(),
        (None, b'c') => g.reset(),
        (Some(b'('), b'0') => g.graphics_charset = true,
        (Some(b'('), _) => g.graphics_charset = false,
        _ => {}
    }
}

fn osc(g: &mut Grid, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    let (code, rest) = text.split_once(';').unwrap_or((&text, ""));
    match code {
        "0" | "2" => g.title = rest.to_owned(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser::Parser;
    use super::*;

    fn run(g: &mut Grid, bytes: &[u8]) {
        let mut p = Parser::new();
        p.feed(bytes, |a| dispatch(g, a));
    }

    fn text(g: &Grid, y: usize) -> String {
        g.row(y).iter().filter(|c| !c.spacer).map(|c| c.ch).collect::<String>().trim_end().to_owned()
    }

    #[test]
    fn cursor_colors_and_modes() {
        let mut g = Grid::new(10, 4, 10);
        run(&mut g, b"\x1b[2;3Hx\x1b[1;31;48;5;22mred\x1b[0m\x1b[38;2;1;2;3mz");
        assert_eq!(text(&g, 1), "  xredz");
        let cell = g.row(1)[3];
        assert_eq!(cell.style.fg, TermColor::Indexed(1));
        assert_eq!(cell.style.bg, TermColor::Indexed(22));
        assert!(cell.style.flags & BOLD != 0);
        assert_eq!(g.row(1)[6].style.fg, TermColor::Rgb(1, 2, 3));
        assert_eq!(g.row(1)[6].style.flags, 0, "reset cleared bold");
        run(&mut g, b"\x1b[?25l\x1b[?1h\x1b[?2004h\x1b[6n");
        assert!(!g.cursor_visible && g.app_cursor && g.bracketed_paste);
        assert_eq!(g.replies, b"\x1b[2;8R");
        g.replies.clear();
        run(&mut g, b"\x1b[?1049h\x1b[Halt\x1b[?1049l");
        assert_eq!(text(&g, 0), "");
        assert_eq!(text(&g, 1), "  xredz", "the main screen came back");
        assert_eq!(g.cursor.x, 7);
        run(&mut g, b"\x1b]2;my title\x07\x1b[c");
        assert_eq!(g.title, "my title");
        assert!(g.replies.starts_with(b"\x1b[?"));
    }

    #[test]
    fn editing_sequences() {
        let mut g = Grid::new(6, 3, 0);
        run(&mut g, b"abcdef\r\nsecond\r\nthird");
        run(&mut g, b"\x1b[1;1H\x1b[2P");
        assert_eq!(text(&g, 0), "cdef");
        run(&mut g, b"\x1b[2@");
        assert_eq!(text(&g, 0), "  cdef");
        run(&mut g, b"\x1b[1M");
        assert_eq!(text(&g, 0), "second");
        run(&mut g, b"\x1b[1L");
        assert_eq!([text(&g, 0), text(&g, 1)], ["", "second"]);
        run(&mut g, b"\x1b[3;1H\x1b[K\x1b[2J");
        assert!((0..3).all(|y| text(&g, y).is_empty()));
        run(&mut g, b"\x1b(0lqk\x1b(B");
        assert_eq!(text(&g, 2), "┌─┐");
    }
}
