use std::collections::HashMap;
use std::time::Instant;

use super::images::ImageManager;
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
#[allow(dead_code)]
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
    /// Index into TerminalState::hyperlinks, or 0 for no link.
    pub hyperlink: u16,
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
            hyperlink: 0,
        }
    }
}

/// APC interception state — Kitty graphics uses ESC _ G ... ESC \
#[derive(Clone, Copy, PartialEq)]
enum ApcState {
    Normal,
    Esc,     // Just saw ESC
    Apc,     // Inside APC payload
    ApcEsc,  // Saw ESC inside APC (waiting for \ to end)
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

    // Saved cursor position + wrap_next flag (CSI s / ESC 7)
    pub saved_cursor: Option<(usize, usize, bool)>,

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

    // Text selection — stored in ABSOLUTE line coordinates so the highlight
    // follows the text when the user scrolls the scrollback. Absolute row 0
    // is the oldest line in scrollback; the live grid starts at
    // `scrollback.len()`. Use `set_selection_anchor`/`set_selection_end` to
    // store, and `is_selected` for queries from visible-row code.
    pub selection_anchor: Option<(usize, usize)>, // (absolute_row, col)
    pub selection_end: Option<(usize, usize)>,    // (absolute_row, col)

    // Deferred line-wrap flag (standard terminal "pending wrap" / "wrap_next").
    // When a character fills the last column the cursor stays at cols-1 and this
    // flag is set. The *next* printable character triggers the actual wrap.
    pub wrap_next: bool,

    // Synchronized output (Mode 2026) — when true, suppress rendering until
    // the application sends CSI ? 2026 l to end the synchronized update.
    // `sync_deadline` is a fallback wakeup so a stuck flag doesn't freeze
    // the screen if the closing sequence never arrives (250ms, matching
    // contour/iTerm2). The app loop must check `is_syncing()` before redraw.
    pub sync_update: bool,
    pub sync_deadline: Option<Instant>,

    // DEC autowrap mode (DECAWM, mode 7). When false, writing past the last
    // column overwrites the last cell instead of advancing to the next row.
    pub auto_wrap: bool,

    // Last printed graphic character — used by CSI Pn b (REP) to repeat the
    // previous character. None until the first printable char.
    pub last_print: Option<char>,

    // OSC 8 hyperlinks — registry of URL strings keyed by u16 ID.
    // ID 0 means "no link". Active hyperlink is applied to new cells.
    pub hyperlinks: HashMap<u16, String>,
    pub active_hyperlink: u16,
    pub hyperlink_next_id: u16,

    // Kitty graphics protocol — inline images
    pub image_manager: ImageManager,

    // APC sequence state machine (for Kitty graphics: ESC _ G ... ESC \)
    apc_state: ApcState,
    apc_buf: Vec<u8>,

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
            sync_update: false,
            sync_deadline: None,
            auto_wrap: true,
            last_print: None,
            hyperlinks: HashMap::new(),
            active_hyperlink: 0,
            hyperlink_next_id: 1,
            image_manager: ImageManager::new(),
            apc_state: ApcState::Normal,
            apc_buf: Vec::new(),
            parser: vte::Parser::new(),
        }
    }

    /// Process raw bytes from the PTY through the VTE parser.
    /// APC sequences (ESC _ ... ESC \) are detected in parallel for Kitty
    /// graphics — vte silently discards them but we capture the payload
    /// without interfering with vte's state machine.
    pub fn process(&mut self, data: &[u8]) {
        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());

        for &byte in data {
            // Always forward every byte to vte — never intercept
            let mut performer = Performer { state: self };
            parser.advance(&mut performer, byte);

            // Parallel APC detection (read-only sniffer, doesn't eat bytes)
            match self.apc_state {
                ApcState::Normal => {
                    if byte == 0x1B {
                        self.apc_state = ApcState::Esc;
                    }
                }
                ApcState::Esc => {
                    if byte == b'_' {
                        self.apc_state = ApcState::Apc;
                        self.apc_buf.clear();
                    } else {
                        self.apc_state = ApcState::Normal;
                    }
                }
                ApcState::Apc => {
                    if byte == 0x1B {
                        self.apc_state = ApcState::ApcEsc;
                    } else if byte == 0x07 {
                        self.apc_state = ApcState::Normal;
                        self.dispatch_apc();
                    } else {
                        self.apc_buf.push(byte);
                    }
                }
                ApcState::ApcEsc => {
                    if byte == b'\\' {
                        self.apc_state = ApcState::Normal;
                        self.dispatch_apc();
                    } else {
                        self.apc_buf.push(0x1B);
                        self.apc_buf.push(byte);
                        self.apc_state = ApcState::Apc;
                    }
                }
            }
        }

        self.parser = parser;
    }

    /// Dispatch a completed APC payload.
    fn dispatch_apc(&mut self) {
        if self.apc_buf.is_empty() {
            return;
        }
        // Kitty graphics: first byte is 'G' (or 'g')
        if self.apc_buf[0] == b'G' || self.apc_buf[0] == b'g' {
            let payload = &self.apc_buf[1..];
            let row = self.cursor_row;
            let col = self.cursor_col;
            self.image_manager.process_kitty(payload, row, col);
        }
        self.apc_buf.clear();
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
            hyperlink: 0,
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
            // Don't leak alt-screen content into the main scrollback buffer
            if self.scroll_top == 0 && self.alt_grid.is_none() {
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
        // Guard against re-entry — a second `CSI ? 1049 h` while already on
        // the alt screen would otherwise overwrite the saved main grid with
        // the alt grid and lose the main screen permanently.
        if self.alt_grid.is_some() {
            return;
        }
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

    /// True iff the app is mid synchronized-update batch (mode 2026) and the
    /// fallback deadline has not yet passed. The render loop should suppress
    /// redraws while this is true to avoid showing the partial frame.
    pub fn is_syncing(&self) -> bool {
        if !self.sync_update {
            return false;
        }
        match self.sync_deadline {
            Some(d) => Instant::now() < d,
            None => false,
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

    /// Convert a visible row index (0..rows) into an absolute row index that
    /// is stable across scrolling. Absolute 0 is the oldest line in scrollback;
    /// the current live grid starts at `scrollback.len()`.
    pub fn visible_to_absolute(&self, vrow: usize) -> usize {
        let scrollback_len = self.scrollback.len();
        let start = if self.scroll_offset == 0 {
            scrollback_len
        } else {
            scrollback_len.saturating_sub(self.scroll_offset)
        };
        start + vrow
    }

    /// Get a line by absolute row index. Returns an empty slice if the index
    /// is out of range (e.g. the line was evicted from scrollback).
    pub fn absolute_line(&self, abs_row: usize) -> &[Cell] {
        let scrollback_len = self.scrollback.len();
        if abs_row < scrollback_len {
            &self.scrollback[abs_row]
        } else {
            let grid_row = abs_row - scrollback_len;
            if grid_row < self.grid.len() {
                &self.grid[grid_row]
            } else {
                &[]
            }
        }
    }

    /// Set the selection anchor from a visible (row, col).
    pub fn set_selection_anchor(&mut self, vrow: usize, col: usize) {
        self.selection_anchor = Some((self.visible_to_absolute(vrow), col));
    }

    /// Set the selection end from a visible (row, col).
    pub fn set_selection_end(&mut self, vrow: usize, col: usize) {
        self.selection_end = Some((self.visible_to_absolute(vrow), col));
    }

    /// Returns the normalized selection range in ABSOLUTE coordinates:
    /// (start_abs_row, start_col, end_abs_row, end_col). Start is before end
    /// in reading order.
    pub fn selection_range(&self) -> Option<(usize, usize, usize, usize)> {
        let (ar, ac) = self.selection_anchor?;
        let (er, ec) = self.selection_end?;
        if (ar, ac) <= (er, ec) {
            Some((ar, ac, er, ec))
        } else {
            Some((er, ec, ar, ac))
        }
    }

    /// Check if the cell at the given VISIBLE (row, col) is inside the
    /// current selection. Converts the visible row to its absolute index
    /// internally so the highlight follows the text when scrolling.
    pub fn is_selected(&self, vrow: usize, col: usize) -> bool {
        let abs = self.visible_to_absolute(vrow);
        if let Some((sr, sc, er, ec)) = self.selection_range() {
            if abs < sr || abs > er {
                return false;
            }
            if abs == sr && abs == er {
                return col >= sc && col <= ec;
            }
            if abs == sr {
                return col >= sc;
            }
            if abs == er {
                return col <= ec;
            }
            true
        } else {
            false
        }
    }

    /// Extract selected text as a string. Iterates over absolute rows so
    /// scrollback content is included even when not currently visible.
    pub fn selected_text(&self) -> Option<String> {
        let (sr, sc, er, ec) = self.selection_range()?;
        let mut text = String::new();
        for abs_row in sr..=er {
            let line = self.absolute_line(abs_row);
            if line.is_empty() {
                if abs_row < er {
                    text.push('\n');
                }
                continue;
            }
            let col_start = if abs_row == sr { sc } else { 0 };
            let col_end = if abs_row == er {
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
            if abs_row < er {
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

    /// Get the hyperlink URL at the given grid cell, if any.
    pub fn hyperlink_at(&self, row: usize, col: usize) -> Option<&str> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let id = self.grid[row][col].hyperlink;
        if id == 0 {
            return None;
        }
        self.hyperlinks.get(&id).map(|s| s.as_str())
    }
}
