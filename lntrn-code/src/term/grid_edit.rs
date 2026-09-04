//! The grid's editing operations: erasing, inserting and deleting cells
//! and lines, the saved cursor, the alternate screen, and a full reset.

use super::grid::{Cell, Cursor, Grid};

impl Grid {
    pub fn erase_in_line(&mut self, mode: u16) {
        let x = self.cursor.x;
        let blank = Cell::blank(self.pen);
        let row = &mut self.lines[self.cursor.y];
        let range = match mode {
            1 => 0..=x.min(self.cols - 1),
            2 => 0..=self.cols - 1,
            _ => x..=self.cols - 1,
        };
        for i in range {
            row[i] = blank;
        }
        self.pending_wrap = false;
    }

    pub fn erase_in_display(&mut self, mode: u16) {
        let y = self.cursor.y;
        match mode {
            1 => {
                for r in 0..y {
                    self.lines[r] = self.blank_row();
                }
                self.erase_in_line(1);
            }
            2 | 3 => {
                for r in 0..self.rows {
                    self.lines[r] = self.blank_row();
                }
                if mode == 3 {
                    self.scrollback.clear();
                    self.view_offset = 0;
                }
            }
            _ => {
                self.erase_in_line(0);
                for r in y + 1..self.rows {
                    self.lines[r] = self.blank_row();
                }
            }
        }
    }

    pub fn erase_chars(&mut self, n: usize) {
        let x = self.cursor.x;
        let blank = Cell::blank(self.pen);
        let row = &mut self.lines[self.cursor.y];
        let end = (x + n.max(1)).min(self.cols);
        row[x..end].fill(blank);
    }

    pub fn insert_chars(&mut self, n: usize) {
        let x = self.cursor.x;
        let blank = Cell::blank(self.pen);
        let row = &mut self.lines[self.cursor.y];
        for _ in 0..n.max(1).min(self.cols - x) {
            row.pop();
            row.insert(x, blank);
        }
    }

    pub fn delete_chars(&mut self, n: usize) {
        let x = self.cursor.x;
        let blank = Cell::blank(self.pen);
        let row = &mut self.lines[self.cursor.y];
        for _ in 0..n.max(1).min(self.cols - x) {
            row.remove(x);
            row.push(blank);
        }
    }

    pub fn insert_lines(&mut self, n: usize) {
        if self.cursor.y < self.top || self.cursor.y > self.bottom {
            return;
        }
        let n = n.max(1).min(self.bottom + 1 - self.cursor.y);
        for _ in 0..n {
            self.lines.remove(self.bottom);
            let blank = self.blank_row();
            self.lines.insert(self.cursor.y, blank);
        }
        self.cursor.x = 0;
    }

    pub fn delete_lines(&mut self, n: usize) {
        if self.cursor.y < self.top || self.cursor.y > self.bottom {
            return;
        }
        let n = n.max(1).min(self.bottom + 1 - self.cursor.y);
        for _ in 0..n {
            self.lines.remove(self.cursor.y);
            let blank = self.blank_row();
            self.lines.insert(self.bottom, blank);
        }
        self.cursor.x = 0;
    }

    pub fn save_cursor(&mut self) {
        self.saved_cursor = (self.cursor, self.pen);
    }

    pub fn restore_cursor(&mut self) {
        let (c, pen) = self.saved_cursor;
        self.cursor = Cursor { x: c.x.min(self.cols - 1), y: c.y.min(self.rows - 1) };
        self.pen = pen;
        self.pending_wrap = false;
    }

    pub fn enter_alt(&mut self) {
        if self.alt_screen() {
            return;
        }
        let main = std::mem::replace(&mut self.lines, vec![vec![Cell::default(); self.cols]; self.rows]);
        self.saved_main = Some((main, self.cursor));
        self.cursor = Cursor::default();
        self.view_offset = 0;
        self.pending_wrap = false;
    }

    pub fn leave_alt(&mut self) {
        if let Some((mut main, cursor)) = self.saved_main.take() {
            // The grid may have been resized while the alternate screen showed.
            for row in &mut main {
                row.resize(self.cols, Cell::default());
            }
            let blank = vec![Cell::default(); self.cols];
            if main.len() > self.rows {
                main.drain(..main.len() - self.rows);
            }
            main.resize(self.rows, blank);
            self.lines = main;
            self.cursor = Cursor { x: cursor.x.min(self.cols - 1), y: cursor.y.min(self.rows - 1) };
            self.pending_wrap = false;
        }
    }

    /// RIS: everything back to how it started, scrollback included.
    pub fn reset(&mut self) {
        let (cols, rows, cap) = (self.cols, self.rows, self.scrollback_cap);
        *self = Self::new(cols, rows, cap);
    }
}
