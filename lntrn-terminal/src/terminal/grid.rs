use super::performer::Performer;

// ── Framework-agnostic color type ───────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color8 {
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
}

// ── ANSI color palette ──────────────────────────────────────────────────────

pub const ANSI_COLORS: [Color8; 16] = [
    Color8::from_rgb(0, 0, 0),       // 0  Black
    Color8::from_rgb(205, 49, 49),   // 1  Red
    Color8::from_rgb(13, 188, 121),  // 2  Green
    Color8::from_rgb(229, 229, 16),  // 3  Yellow
    Color8::from_rgb(80, 200, 195),  // 4  Blue
    Color8::from_rgb(188, 63, 188),  // 5  Magenta
    Color8::from_rgb(17, 168, 205),  // 6  Cyan
    Color8::from_rgb(229, 229, 229), // 7  White
    Color8::from_rgb(102, 102, 102), // 8  Bright Black
    Color8::from_rgb(241, 76, 76),   // 9  Bright Red
    Color8::from_rgb(35, 209, 139),  // 10 Bright Green
    Color8::from_rgb(245, 245, 67),  // 11 Bright Yellow
    Color8::from_rgb(120, 225, 215), // 12 Bright Blue
    Color8::from_rgb(214, 112, 214), // 13 Bright Magenta
    Color8::from_rgb(41, 184, 219),  // 14 Bright Cyan
    Color8::from_rgb(229, 229, 229), // 15 Bright White
];

// ── Terminal cell ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Cell {
    pub c: char,
    pub fg: Color8,
    pub bg: Color8,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// For wide characters: the first cell has `wide: Wide::Head`,
    /// the continuation cell has `wide: Wide::Tail`.
    /// Normal single-width characters have `wide: Wide::No`.
    pub wide: Wide,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wide {
    No,
    Head,
    Tail,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: Color8::from_rgb(236, 236, 236),
            bg: Color8::TRANSPARENT,
            bold: false,
            italic: false,
            underline: false,
            wide: Wide::No,
        }
    }
}

// ── Terminal grid state ─────────────────────────────────────────────────────

pub struct TerminalState {
    pub cols: usize,
    pub rows: usize,
    pub grid: Vec<Vec<Cell>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scrollback: Vec<Vec<Cell>>,
    pub max_scrollback: usize,
    pub scroll_offset: usize,

    // Theme-aware default colors (updated on theme switch)
    pub default_fg: Color8,
    pub default_bg: Color8,
    pub default_bold: bool,

    // Current text attributes
    pub attr_fg: Color8,
    pub attr_bg: Color8,
    pub attr_bold: bool,
    pub attr_italic: bool,
    pub attr_underline: bool,
    pub attr_reverse: bool,

    // Scroll region (top, bottom) — inclusive, 0-indexed
    pub scroll_top: usize,
    pub scroll_bottom: usize,

    // Saved cursor position (CSI s / ESC 7)
    pub saved_cursor: Option<(usize, usize)>,

    // DEC private modes
    pub cursor_hidden: bool,      // mode 25
    pub application_cursor: bool, // mode 1

    // Cursor shape (DECSCUSR): 0/1=blinking block, 2=steady block,
    // 3=blinking underline, 4=steady underline, 5=blinking beam, 6=steady beam
    pub cursor_shape: u8,

    // Title set by OSC 0/2
    pub title: Option<String>,

    // Working directory reported by OSC 7
    pub osc7_cwd: Option<String>,

    // Alternate screen buffer
    pub alt_grid: Option<Vec<Vec<Cell>>>,
    pub alt_cursor: Option<(usize, usize)>,

    // Responses to write back to the PTY (DA, DSR, etc.)
    pub pending_responses: Vec<Vec<u8>>,

    // BEL (0x07) received — app should fire a desktop notification
    pub bell: bool,

    // OSC 99 (Kitty) desktop notifications — accumulator and queue
    pub osc99_title: String,
    pub osc99_body: String,
    pub pending_notifications: Vec<(String, String)>, // (title, body)

    // Text selection (grid coordinates)
    pub selection_anchor: Option<(usize, usize)>, // (row, col) where drag started
    pub selection_end: Option<(usize, usize)>,    // (row, col) where drag is now

    // Deferred line-wrap flag (standard terminal "pending wrap" / "wrap_next").
    // When a character fills the last column the cursor stays at cols-1 and this
    // flag is set. The *next* printable character triggers the actual wrap.
    pub wrap_next: bool,

    // VTE parser
    parser: vte::Parser,
}

impl TerminalState {
    pub fn new(cols: usize, rows: usize) -> Self {
        let grid = vec![vec![Cell::default(); cols]; rows];
        Self {
            cols,
            rows,
            grid,
            cursor_row: 0,
            cursor_col: 0,
            scrollback: Vec::new(),
            max_scrollback: 5000,
            scroll_offset: 0,
            default_fg: Cell::default().fg,
            default_bg: Cell::default().bg,
            default_bold: false,
            attr_fg: Cell::default().fg,
            attr_bg: Cell::default().bg,
            attr_bold: false,
            attr_italic: false,
            attr_underline: false,
            attr_reverse: false,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            saved_cursor: None,
            cursor_hidden: false,
            application_cursor: false,
            cursor_shape: 0,
            title: None,
            osc7_cwd: None,
            alt_grid: None,
            alt_cursor: None,
            pending_responses: Vec::new(),
            bell: false,
            osc99_title: String::new(),
            osc99_body: String::new(),
            pending_notifications: Vec::new(),
            selection_anchor: None,
            selection_end: None,
            wrap_next: false,
            parser: vte::Parser::new(),
        }
    }

    /// Process raw bytes from the PTY through the VTE parser
    pub fn process(&mut self, data: &[u8]) {
        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
        let mut performer = Performer { state: self };
        for byte in data {
            parser.advance(&mut performer, *byte);
        }
        self.parser = parser;
    }

    /// Resize the terminal grid
    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        if new_cols == self.cols && new_rows == self.rows {
            return;
        }

        let is_alt = self.alt_grid.is_some();

        // Growing: add blank rows at the bottom.
        // Don't pull from scrollback — the shell will redraw via SIGWINCH and
        // pulling old scrollback lines would desync cursor_row vs. what the
        // shell expects, causing content to render in the wrong place.
        while self.grid.len() < new_rows {
            let def = self.default_cell();
            self.grid.push(vec![def; new_cols]);
        }

        // Shrinking: remove rows.
        // Prefer removing from the bottom (empty rows below cursor) first,
        // only pushing top rows to scrollback when the cursor would go out of bounds.
        while self.grid.len() > new_rows {
            if !is_alt {
                if self.cursor_row + 1 < self.grid.len() {
                    // There are rows below cursor — remove from bottom
                    self.grid.pop();
                } else {
                    // Cursor is at/past last row — must remove from top
                    let row = self.grid.remove(0);
                    self.scrollback.push(row);
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                }
            } else {
                self.grid.pop(); // alt screen: trim from bottom
            }
        }

        // Only extend rows that are too short — never truncate.
        // Cells beyond new_cols are preserved (not rendered) so that
        // growing the window back restores the original content.
        let def_cell = self.default_cell();
        for row in &mut self.grid {
            if row.len() < new_cols {
                row.resize(new_cols, def_cell.clone());
            }
        }

        self.cols = new_cols;
        self.rows = new_rows;
        self.scroll_top = 0;
        self.scroll_bottom = new_rows - 1;

        if self.cursor_row >= new_rows {
            self.cursor_row = new_rows - 1;
        }
        if self.cursor_col >= new_cols {
            self.cursor_col = new_cols - 1;
        }

        while self.scrollback.len() > self.max_scrollback {
            self.scrollback.remove(0);
        }

        // Snap to bottom — scroll_offset may now exceed scrollback after rows were pulled back in
        self.scroll_offset = 0;
    }

    pub fn default_cell(&self) -> Cell {
        Cell {
            c: ' ',
            fg: self.default_fg,
            bg: Color8::TRANSPARENT,
            bold: self.default_bold,
            italic: false,
            underline: false,
            wide: Wide::No,
        }
    }

    pub fn set_default_colors(&mut self, fg: Color8, bg: Color8, bold: bool) {
        let old_fg = self.default_fg;
        self.default_fg = fg;
        self.default_bg = bg;
        self.default_bold = bold;
        self.attr_fg = fg;
        self.attr_bg = bg;

        for row in &mut self.grid {
            for cell in row.iter_mut() {
                if cell.fg == old_fg {
                    cell.fg = fg;
                }
            }
        }
    }

    pub fn scroll_up(&mut self) {
        if self.scroll_top < self.scroll_bottom && self.scroll_bottom < self.rows {
            let removed = self.grid.remove(self.scroll_top);
            if self.scroll_top == 0 {
                self.scrollback.push(removed);
                if self.scrollback.len() > self.max_scrollback {
                    self.scrollback.remove(0);
                }
            }
            let def = self.default_cell();
            self.grid.insert(self.scroll_bottom, vec![def; self.cols]);
        }
    }

    pub fn scroll_down(&mut self) {
        if self.scroll_top < self.scroll_bottom && self.scroll_bottom < self.rows {
            self.grid.remove(self.scroll_bottom);
            let def = self.default_cell();
            self.grid.insert(self.scroll_top, vec![def; self.cols]);
        }
    }

    pub fn enter_alt_screen(&mut self) {
        let saved_grid = self.grid.clone();
        self.alt_grid = Some(saved_grid);
        self.alt_cursor = Some((self.cursor_row, self.cursor_col));
        let def = self.default_cell();
        self.grid = vec![vec![def; self.cols]; self.rows];
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    pub fn leave_alt_screen(&mut self) {
        if let Some(grid) = self.alt_grid.take() {
            self.grid = grid;
        }
        if let Some((r, c)) = self.alt_cursor.take() {
            self.cursor_row = r;
            self.cursor_col = c;
        }
    }

    /// Get a display line accounting for scroll offset.
    pub fn display_line(&self, row: usize) -> &[Cell] {
        if self.scroll_offset == 0 {
            if row < self.grid.len() {
                return &self.grid[row];
            }
            return &[];
        }

        let scrollback_len = self.scrollback.len();
        let scrollback_start = scrollback_len.saturating_sub(self.scroll_offset);
        let line_idx = scrollback_start + row;

        if line_idx < scrollback_len {
            &self.scrollback[line_idx]
        } else {
            let grid_row = line_idx - scrollback_len;
            if grid_row < self.grid.len() {
                &self.grid[grid_row]
            } else {
                &[]
            }
        }
    }

    /// Returns the normalized selection range: (start_row, start_col, end_row, end_col).
    /// Start is always before end in reading order.
    pub fn selection_range(&self) -> Option<(usize, usize, usize, usize)> {
        let (ar, ac) = self.selection_anchor?;
        let (er, ec) = self.selection_end?;
        if (ar, ac) <= (er, ec) {
            Some((ar, ac, er, ec))
        } else {
            Some((er, ec, ar, ac))
        }
    }

    /// Check if a cell at (row, col) is inside the current selection.
    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        if let Some((sr, sc, er, ec)) = self.selection_range() {
            if row < sr || row > er {
                return false;
            }
            if row == sr && row == er {
                return col >= sc && col <= ec;
            }
            if row == sr {
                return col >= sc;
            }
            if row == er {
                return col <= ec;
            }
            true
        } else {
            false
        }
    }

    /// Extract selected text as a string.
    pub fn selected_text(&self) -> Option<String> {
        let (sr, sc, er, ec) = self.selection_range()?;
        let mut text = String::new();
        for row in sr..=er {
            let line = self.display_line(row);
            let col_start = if row == sr { sc } else { 0 };
            let col_end = if row == er {
                ec
            } else {
                line.len().saturating_sub(1)
            };
            for col in col_start..=col_end.min(line.len().saturating_sub(1)) {
                // Skip wide-char tail cells — the head already has the character
                if line[col].wide == Wide::Tail {
                    continue;
                }
                text.push(line[col].c);
            }
            // Trim trailing spaces on each line
            let trimmed = text.trim_end_matches(' ');
            text.truncate(trimmed.len());
            if row < er {
                text.push('\n');
            }
        }
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
        self.selection_end = None;
    }
}
