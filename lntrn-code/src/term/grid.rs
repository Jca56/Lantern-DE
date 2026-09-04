//! The terminal screen: a grid of styled cells, a scrollback of the rows
//! that left the top, the alternate screen, a scroll region, tab stops,
//! and the cursor. Erasing, inserting and the alternate screen live in
//! `grid_edit.rs`; what the escape sequences do lives in [`super::csi`].

use std::collections::VecDeque;

use crate::charwidth::char_cells;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TermColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const UNDERLINE: u8 = 4;
pub const INVERSE: u8 = 8;
pub const DIM: u8 = 16;
pub const STRIKE: u8 = 32;
pub const HIDDEN: u8 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: TermColor,
    pub bg: TermColor,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
    /// The first half of a two-cell character.
    pub wide: bool,
    /// The second half of one: drawn as nothing.
    pub spacer: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self { ch: ' ', style: Style::default(), wide: false, spacer: false }
    }
}

impl Cell {
    pub(super) fn blank(style: Style) -> Self {
        Self { ch: ' ', style: Style { fg: TermColor::Default, bg: style.bg, flags: 0 }, wide: false, spacer: false }
    }
}

pub type Row = Vec<Cell>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub x: usize,
    pub y: usize,
}

/// The DEC special graphics set (`ESC ( 0`), for box drawing.
fn graphics(c: char) -> char {
    match c {
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'q' => '─',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'a' => '▒',
        '`' => '◆',
        '~' => '·',
        'f' => '°',
        'g' => '±',
        'o' => '⎺',
        'p' => '⎻',
        'r' => '⎼',
        's' => '⎽',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        _ => c,
    }
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub(super) lines: Vec<Row>,
    /// The main screen and its cursor while the alternate one shows.
    pub(super) saved_main: Option<(Vec<Row>, Cursor)>,
    pub scrollback: VecDeque<Row>,
    pub(super) scrollback_cap: usize,
    pub cursor: Cursor,
    pub(super) saved_cursor: (Cursor, Style),
    pub pen: Style,
    /// Inclusive scroll region.
    pub(super) top: usize,
    pub(super) bottom: usize,
    tabs: Vec<bool>,
    pub cursor_visible: bool,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub origin_mode: bool,
    pub autowrap: bool,
    pub insert_mode: bool,
    pub mouse_reporting: bool,
    /// Mouse reports in the SGR form (`CSI < b;x;y M`), else X10 bytes.
    pub mouse_sgr: bool,
    pub graphics_charset: bool,
    pub(super) pending_wrap: bool,
    pub title: String,
    /// Bytes to send back to the child (cursor position reports, DA).
    pub replies: Vec<u8>,
    /// Lines the view is scrolled up into the scrollback.
    pub view_offset: usize,
    /// Rows that fell off the front of the scrollback, so a row keeps its
    /// absolute number (see [`Grid::abs_row`]) while output scrolls.
    pub scrolled_off: u64,
    pub bell: bool,
}

impl Grid {
    pub fn new(cols: usize, rows: usize, scrollback_cap: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let mut g = Self {
            cols,
            rows,
            lines: vec![vec![Cell::default(); cols]; rows],
            saved_main: None,
            scrollback: VecDeque::new(),
            scrollback_cap,
            cursor: Cursor::default(),
            saved_cursor: (Cursor::default(), Style::default()),
            pen: Style::default(),
            top: 0,
            bottom: rows - 1,
            tabs: Vec::new(),
            cursor_visible: true,
            app_cursor: false,
            bracketed_paste: false,
            origin_mode: false,
            autowrap: true,
            insert_mode: false,
            mouse_reporting: false,
            mouse_sgr: false,
            graphics_charset: false,
            pending_wrap: false,
            title: String::new(),
            replies: Vec::new(),
            view_offset: 0,
            scrolled_off: 0,
            bell: false,
        };
        g.reset_tabs();
        g
    }

    /// The scroll region's first row.
    pub fn top(&self) -> usize {
        self.top
    }

    pub fn alt_screen(&self) -> bool {
        self.saved_main.is_some()
    }

    fn reset_tabs(&mut self) {
        self.tabs = (0..self.cols).map(|i| i % 8 == 0).collect();
    }

    /// The screen row `i`, or a scrollback row when the view is scrolled up.
    pub fn viewed_row(&self, i: usize) -> &Row {
        let back = self.view_offset.min(self.scrollback.len());
        if i < back {
            let idx = self.scrollback.len() - back + i;
            &self.scrollback[idx]
        } else {
            &self.lines[(i - back).min(self.rows - 1)]
        }
    }

    pub fn scroll_view(&mut self, by: isize) {
        let max = self.scrollback.len() as isize;
        self.view_offset = (self.view_offset as isize + by).clamp(0, max) as usize;
    }

    /// The absolute number of viewed row `y`: stable while output scrolls,
    /// so a selection survives new lines.
    pub fn abs_row(&self, y: usize) -> u64 {
        let back = self.view_offset.min(self.scrollback.len());
        self.scrolled_off + (self.scrollback.len() - back + y) as u64
    }

    /// The row with absolute number `abs`, while it still exists.
    pub fn row_by_abs(&self, abs: u64) -> Option<&Row> {
        let i = usize::try_from(abs.checked_sub(self.scrolled_off)?).ok()?;
        if i < self.scrollback.len() { Some(&self.scrollback[i]) } else { self.lines.get(i - self.scrollback.len()) }
    }

    /// The text between two cell boundaries `(absolute row, column)`, in
    /// either order, rows joined by newlines and trailing blanks dropped.
    pub fn text_between(&self, a: (u64, usize), b: (u64, usize)) -> String {
        let (s, e) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        for abs in s.0..=e.0 {
            if abs > s.0 {
                out.push('\n');
            }
            let Some(row) = self.row_by_abs(abs) else {
                continue;
            };
            let from = if abs == s.0 { s.1 } else { 0 };
            let to = if abs == e.0 { e.1 } else { row.len() };
            let line: String = row.iter().take(to.min(row.len())).skip(from).filter(|c| !c.spacer).map(|c| c.ch).collect();
            out.push_str(line.trim_end());
        }
        out
    }

    pub fn row(&self, y: usize) -> &Row {
        &self.lines[y.min(self.rows - 1)]
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if cols == self.cols && rows == self.rows {
            return;
        }
        for row in self.lines.iter_mut().chain(self.scrollback.iter_mut()) {
            row.resize(cols, Cell::default());
        }
        if let Some((main, _)) = &mut self.saved_main {
            for row in main.iter_mut() {
                row.resize(cols, Cell::default());
            }
        }
        // Rows lost at the bottom of a shrinking main screen go to the scrollback
        // when the cursor sits below the new height.
        while self.lines.len() > rows {
            if self.cursor.y >= self.lines.len() - 1 || self.cursor.y >= rows {
                let row = self.lines.remove(0);
                if !self.alt_screen() {
                    self.push_scrollback(row);
                }
                self.cursor.y = self.cursor.y.saturating_sub(1);
            } else {
                self.lines.pop();
            }
        }
        while self.lines.len() < rows {
            if let Some(row) = self.scrollback.pop_back().filter(|_| !self.alt_screen()) {
                self.lines.insert(0, row);
                self.cursor.y += 1;
            } else {
                self.lines.push(vec![Cell::default(); cols]);
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.cursor.x = self.cursor.x.min(cols - 1);
        self.cursor.y = self.cursor.y.min(rows - 1);
        self.pending_wrap = false;
        self.reset_tabs();
        self.view_offset = self.view_offset.min(self.scrollback.len());
    }

    fn push_scrollback(&mut self, row: Row) {
        if self.scrollback_cap == 0 {
            return;
        }
        if self.scrollback.len() >= self.scrollback_cap {
            self.scrollback.pop_front();
            self.scrolled_off += 1;
        }
        self.scrollback.push_back(row);
    }

    pub(super) fn blank_row(&self) -> Row {
        vec![Cell::blank(self.pen); self.cols]
    }

    /// Scroll the region up `n` lines (new blank lines at the bottom).
    pub fn scroll_up(&mut self, n: usize) {
        let n = n.min(self.bottom + 1 - self.top);
        for _ in 0..n {
            let row = self.lines.remove(self.top);
            if self.top == 0 && !self.alt_screen() {
                self.push_scrollback(row);
            }
            let blank = self.blank_row();
            self.lines.insert(self.bottom, blank);
        }
    }

    /// Scroll the region down `n` lines (new blank lines at the top).
    pub fn scroll_down(&mut self, n: usize) {
        let n = n.min(self.bottom + 1 - self.top);
        for _ in 0..n {
            self.lines.remove(self.bottom);
            let blank = self.blank_row();
            self.lines.insert(self.top, blank);
        }
    }

    pub fn linefeed(&mut self) {
        if self.cursor.y == self.bottom {
            self.scroll_up(1);
        } else if self.cursor.y + 1 < self.rows {
            self.cursor.y += 1;
        }
        self.pending_wrap = false;
    }

    pub fn reverse_index(&mut self) {
        if self.cursor.y == self.top {
            self.scroll_down(1);
        } else if self.cursor.y > 0 {
            self.cursor.y -= 1;
        }
        self.pending_wrap = false;
    }

    pub fn carriage_return(&mut self) {
        self.cursor.x = 0;
        self.pending_wrap = false;
    }

    pub fn backspace(&mut self) {
        self.cursor.x = self.cursor.x.saturating_sub(1);
        self.pending_wrap = false;
    }

    pub fn tab(&mut self) {
        let mut x = self.cursor.x + 1;
        while x < self.cols && !self.tabs[x] {
            x += 1;
        }
        self.cursor.x = x.min(self.cols - 1);
        self.pending_wrap = false;
    }

    pub fn back_tab(&mut self) {
        let mut x = self.cursor.x;
        while x > 0 {
            x -= 1;
            if self.tabs[x] {
                break;
            }
        }
        self.cursor.x = x;
    }

    pub fn set_tab(&mut self) {
        self.tabs[self.cursor.x] = true;
    }

    pub fn clear_tab(&mut self, all: bool) {
        if all {
            self.tabs.iter_mut().for_each(|t| *t = false);
        } else {
            self.tabs[self.cursor.x] = false;
        }
    }

    pub fn print(&mut self, c: char) {
        let c = if self.graphics_charset { graphics(c) } else { c };
        let w = char_cells(c);
        if w == 0 {
            // A combining mark joins the cell before the cursor.
            let x = self.cursor.x.saturating_sub(usize::from(!self.pending_wrap && self.cursor.x > 0));
            let y = self.cursor.y;
            if x < self.cols {
                let cell = &mut self.lines[y][x];
                if cell.ch != ' ' || x > 0 {
                    let mut s = cell.ch.to_string();
                    s.push(c);
                    // Keep the base character; the mark is drawn with it
                    // only when the font can, so store the base alone.
                    cell.ch = s.chars().next().unwrap_or(' ');
                }
            }
            return;
        }
        if self.pending_wrap && self.autowrap {
            self.carriage_return();
            self.linefeed();
        }
        if w == 2 && self.cursor.x + 1 >= self.cols {
            if self.autowrap {
                self.carriage_return();
                self.linefeed();
            } else {
                self.cursor.x = self.cols - 2;
            }
        }
        let (x, y) = (self.cursor.x, self.cursor.y);
        if self.insert_mode {
            let row = &mut self.lines[y];
            for _ in 0..w {
                row.pop();
                row.insert(x, Cell::blank(self.pen));
            }
        }
        let row = &mut self.lines[y];
        // Overwriting half of a wide character clears its other half.
        if row[x].spacer && x > 0 {
            row[x - 1] = Cell::blank(self.pen);
        }
        if row[x].wide && x + 1 < self.cols {
            row[x + 1] = Cell::blank(self.pen);
        }
        row[x] = Cell { ch: c, style: self.pen, wide: w == 2, spacer: false };
        if w == 2 {
            row[x + 1] = Cell { ch: ' ', style: self.pen, wide: false, spacer: true };
        }
        if x + w >= self.cols {
            self.cursor.x = self.cols - 1;
            self.pending_wrap = true;
        } else {
            self.cursor.x = x + w;
            self.pending_wrap = false;
        }
    }

    /// Move the cursor, clamped to the screen (or the region in origin mode).
    pub fn move_to(&mut self, x: usize, y: usize) {
        let (lo, hi) = if self.origin_mode { (self.top, self.bottom) } else { (0, self.rows - 1) };
        self.cursor.x = x.min(self.cols - 1);
        let y = if self.origin_mode { y + self.top } else { y };
        self.cursor.y = y.clamp(lo, hi);
        self.pending_wrap = false;
    }

    pub fn move_by(&mut self, dx: isize, dy: isize) {
        let x = (self.cursor.x as isize + dx).max(0) as usize;
        let (lo, hi) = if self.cursor.y >= self.top && self.cursor.y <= self.bottom { (self.top as isize, self.bottom as isize) } else { (0, self.rows as isize - 1) };
        let y = (self.cursor.y as isize + dy).clamp(lo, hi) as usize;
        self.cursor.x = x.min(self.cols - 1);
        self.cursor.y = y;
        self.pending_wrap = false;
    }

    pub fn set_region(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.rows - 1);
        if top < bottom {
            self.top = top;
            self.bottom = bottom;
        } else {
            self.top = 0;
            self.bottom = self.rows - 1;
        }
        self.move_to(0, 0);
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(g: &Grid, y: usize) -> String {
        g.row(y).iter().filter(|c| !c.spacer).map(|c| c.ch).collect::<String>().trim_end().to_owned()
    }

    #[test]
    fn printing_wraps_and_scrolls() {
        let mut g = Grid::new(5, 2, 10);
        for c in "abcdefg".chars() {
            g.print(c);
        }
        assert_eq!(text(&g, 0), "abcde");
        assert_eq!(text(&g, 1), "fg");
        assert_eq!(g.cursor, Cursor { x: 2, y: 1 });
        g.carriage_return();
        g.linefeed();
        assert_eq!(text(&g, 0), "fg", "the top row scrolled away");
        assert_eq!(g.scrollback.len(), 1);
        assert_eq!(g.scrollback[0].iter().map(|c| c.ch).collect::<String>(), "abcde");
        g.print('日');
        g.print('本');
        g.print('x');
        assert_eq!(text(&g, 1), "日本x");
        assert!(g.row(1)[0].wide && g.row(1)[1].spacer);
        assert_eq!(g.viewed_row(0)[0].ch, 'f');
        g.scroll_view(1);
        assert_eq!(g.viewed_row(0)[0].ch, 'a');
    }

    #[test]
    fn regions_and_erasing() {
        let mut g = Grid::new(4, 4, 0);
        for y in 0..4 {
            g.move_to(0, y);
            g.print(char::from(b'0' + y as u8));
        }
        g.set_region(1, 2);
        g.move_to(0, 2);
        g.linefeed();
        assert_eq!([text(&g, 0), text(&g, 1), text(&g, 2), text(&g, 3)], ["0", "2", "", "3"]);
        g.move_to(0, 1);
        g.reverse_index();
        assert_eq!([text(&g, 1), text(&g, 2)], ["", "2"]);
        g.set_region(0, 3);
        g.move_to(1, 3);
        g.erase_in_line(0);
        assert_eq!(text(&g, 3), "3");
        g.move_to(0, 3);
        g.erase_in_line(2);
        assert_eq!(text(&g, 3), "");
        g.move_to(0, 0);
        g.print('a');
        g.print('b');
        g.move_to(0, 0);
        g.insert_chars(1);
        assert_eq!(text(&g, 0), " ab");
        g.delete_chars(2);
        assert_eq!(text(&g, 0), "b");
        g.erase_in_display(2);
        assert!((0..4).all(|y| text(&g, y).is_empty()));
    }

    #[test]
    fn alt_screen_and_resize() {
        let mut g = Grid::new(3, 2, 5);
        g.print('m');
        g.enter_alt();
        g.print('a');
        assert_eq!(text(&g, 0), "a");
        g.leave_alt();
        assert_eq!(text(&g, 0), "m");
        assert_eq!(g.cursor.x, 1);
        g.resize(5, 3);
        assert_eq!((g.cols, g.rows), (5, 3));
        assert_eq!(g.row(0).len(), 5);
        g.resize(2, 1);
        assert_eq!(g.rows, 1);
        assert_eq!(g.cursor.y, 0);
        // Resized while the alternate screen shows: the main screen comes
        // back at the new size (this took the editor down once).
        let mut g = Grid::new(10, 18, 5);
        g.enter_alt();
        g.resize(12, 19);
        g.leave_alt();
        assert_eq!(g.lines.len(), 19);
        assert_eq!(g.row(18).len(), 12);
        let _ = g.viewed_row(18);
        g.enter_alt();
        g.resize(8, 3);
        g.leave_alt();
        assert_eq!(g.lines.len(), 3);
    }

    #[test]
    fn absolute_rows_and_selection_text() {
        let mut g = Grid::new(6, 2, 2);
        for line in ["one", "two", "three", "four", "five"] {
            for c in line.chars() {
                g.print(c);
            }
            g.carriage_return();
            g.linefeed();
        }
        // Three rows fell off the front of a 2-row scrollback.
        assert_eq!(g.scrollback.len(), 2);
        assert_eq!(g.scrolled_off, 2);
        assert_eq!(g.abs_row(0), 4, "the top screen row");
        assert_eq!(g.row_by_abs(4).map(|r| r[0].ch), Some('f'));
        assert_eq!(g.text_between((3, 1), (4, 3)), "our\nfiv");
        assert_eq!(g.text_between((4, 3), (3, 1)), "our\nfiv", "either order");
        assert_eq!(g.text_between((4, 0), (4, 6)), "five", "trailing blanks dropped");
        g.scroll_view(2);
        assert_eq!(g.abs_row(0), 2, "scrolled up into the scrollback");
        assert!(g.row_by_abs(1).is_none(), "gone from the scrollback");
    }
}
