use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::images::ImageManager;
use super::mouse::MouseMode;
use super::performer::Performer;

/// Fallback timeout for synchronized output mode 2026 (DEC mode 2026,
/// `CSI ?2026 h` … `CSI ?2026 l`). The deadline is *sliding*: every
/// time `process()` consumes new bytes while sync is active, we push the
/// deadline forward by this much. As long as data is flowing the batch
/// can run as long as it needs to (Claude Code's "thought for 11s + tool
/// call + task widget" redraws routinely take 1–5s). We only fall back
/// when the app stops sending bytes mid-batch — the genuinely-stuck case
/// the deadline is meant to catch.
pub(super) const SYNC_OUTPUT_TIMEOUT: Duration = Duration::from_millis(1000);

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
    /// Set on the LAST cell of a row when text flowed past the row's end
    /// onto the next row (soft wrap). Copy joins such rows into one logical
    /// line instead of inserting a newline. A hard newline (CR/LF) never
    /// sets this, so real line breaks still copy as newlines.
    pub wrapped: bool,
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
            wrapped: false,
        }
    }
}

/// Snapshot of main-screen DEC state taken on alt-screen entry so the
/// shell's prompt isn't poisoned by attributes the TUI set inside the
/// alt buffer.
#[derive(Clone)]
pub struct AltSavedState {
    pub attr_fg: Color8,
    pub attr_bg: Color8,
    pub attr_bold: bool,
    pub attr_italic: bool,
    pub attr_underline: bool,
    pub attr_reverse: bool,
    pub scroll_top: usize,
    pub scroll_bottom: usize,
    pub wrap_next: bool,
    pub auto_wrap: bool,
    pub active_hyperlink: u16,
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
    pub bracketed_paste: bool,    // mode 2004 — wrap pastes in ESC[200~/201~

    // Mouse reporting (DECSET 1000/1002/1003) + SGR encoding (1006).
    // See terminal/mouse.rs for forwarding details.
    pub mouse_mode: MouseMode,
    pub mouse_sgr: bool,

    // Cursor shape (DECSCUSR): 0/1=blinking block, 2=steady block,
    // 3=blinking underline, 4=steady underline, 5=blinking beam, 6=steady beam
    pub cursor_shape: u8,

    // Title set by OSC 0/2
    pub title: Option<String>,

    // Working directory reported by OSC 7
    pub osc7_cwd: Option<String>,

    // Alternate screen buffer.
    //
    // When entering the alt screen (DECSET 1049/1047/47) we snapshot enough
    // of the main-screen DEC state that leaving the alt screen restores a
    // pristine main-screen context. Saving the grid + cursor alone leaks
    // SGR colors, the scroll region, and the deferred-wrap flag back into
    // the shell's prompt — that's how you get "letters in wrong cells"
    // after a TUI exits.
    pub alt_grid: Option<Vec<Vec<Cell>>>,
    pub alt_cursor: Option<(usize, usize)>,
    pub alt_saved_state: Option<AltSavedState>,

    // Scrollback captured while on the alt screen. Lines pushed here when the
    // alt-screen TUI scrolls; cleared on enter/leave so it never leaks into
    // the main shell's history. Apps like Claude Code need this to be
    // scrollable; classic TUIs (vim/less) just redraw and never trigger it.
    pub alt_scrollback: Vec<Vec<Cell>>,

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
    // the application sends CSI ?2026 l to end the synchronized update.
    // `sync_deadline` is a *sliding* fallback: it gets pushed forward in
    // `process()` every time new bytes arrive while sync_update is set, so
    // a long, actively-streaming batch never times out. The fallback only
    // fires when the app genuinely stops sending bytes mid-batch — the
    // hung-app case the deadline exists to recover from. The app loop must
    // check `is_syncing()` before redraw.
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
            alt_scrollback: Vec::new(),
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
            bracketed_paste: false,
            mouse_mode: MouseMode::Off,
            mouse_sgr: false,
            cursor_shape: 0,
            title: None,
            osc7_cwd: None,
            alt_grid: None,
            alt_cursor: None,
            alt_saved_state: None,
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

        // Sliding sync deadline. Whenever we consume bytes while mode 2026
        // is active, push the fallback deadline forward — Claude Code's
        // big redraws (thinking + tool call + task widget) routinely run
        // 1–5s, far longer than any fixed timeout. As long as data keeps
        // arriving, the batch isn't stuck and we must not render the
        // half-written grid. The fixed timeout only fires when the app
        // genuinely stops sending bytes mid-batch.
        if self.sync_update && !data.is_empty() {
            self.sync_deadline = Some(Instant::now() + SYNC_OUTPUT_TIMEOUT);
        }
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
            wrapped: false,
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
                // Push to alt_scrollback when on the alt screen so the
                // history stays isolated from the main shell's scrollback.
                let max = self.max_scrollback;
                let buf = if self.alt_grid.is_some() {
                    &mut self.alt_scrollback
                } else {
                    &mut self.scrollback
                };
                buf.push(removed);
                if buf.len() > max {
                    buf.remove(0);
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
        self.alt_saved_state = Some(AltSavedState {
            attr_fg: self.attr_fg,
            attr_bg: self.attr_bg,
            attr_bold: self.attr_bold,
            attr_italic: self.attr_italic,
            attr_underline: self.attr_underline,
            attr_reverse: self.attr_reverse,
            scroll_top: self.scroll_top,
            scroll_bottom: self.scroll_bottom,
            wrap_next: self.wrap_next,
            auto_wrap: self.auto_wrap,
            active_hyperlink: self.active_hyperlink,
        });

        // Reset DEC state for a fresh alt screen. xterm/contour reset the
        // scroll region, clear the deferred wrap, drop SGR, and home the
        // cursor — anything less leaks state INTO the TUI's first frame.
        let def = self.default_cell();
        self.grid = vec![vec![def; self.cols]; self.rows];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.alt_scrollback.clear();
        self.scroll_offset = 0;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
        self.wrap_next = false;
        self.auto_wrap = true;
        self.attr_fg = self.default_fg;
        self.attr_bg = self.default_bg;
        self.attr_bold = self.default_bold;
        self.attr_italic = false;
        self.attr_underline = false;
        self.attr_reverse = false;
        self.active_hyperlink = 0;
    }

    pub fn leave_alt_screen(&mut self) {
        // Only restore if we were actually on the alt screen — otherwise a
        // stray `CSI ? 1049 l` would silently zero the cursor on the main
        // screen and trash the prompt.
        let Some(grid) = self.alt_grid.take() else {
            return;
        };
        self.grid = grid;
        // Drop the alt-screen history and snap back to the live main grid so
        // the user doesn't see stale TUI lines after the app exits.
        self.alt_scrollback.clear();
        self.scroll_offset = 0;
        if let Some((r, c)) = self.alt_cursor.take() {
            self.cursor_row = r.min(self.rows.saturating_sub(1));
            self.cursor_col = c.min(self.cols.saturating_sub(1));
        }
        if let Some(st) = self.alt_saved_state.take() {
            self.attr_fg = st.attr_fg;
            self.attr_bg = st.attr_bg;
            self.attr_bold = st.attr_bold;
            self.attr_italic = st.attr_italic;
            self.attr_underline = st.attr_underline;
            self.attr_reverse = st.attr_reverse;
            self.scroll_top = st.scroll_top.min(self.rows.saturating_sub(1));
            self.scroll_bottom = st
                .scroll_bottom
                .min(self.rows.saturating_sub(1))
                .max(self.scroll_top);
            self.wrap_next = st.wrap_next;
            self.auto_wrap = st.auto_wrap;
            self.active_hyperlink = st.active_hyperlink;
        } else {
            // Fallback if state somehow went missing — at least don't carry
            // alt-screen SGR/scroll-region into the prompt.
            self.scroll_top = 0;
            self.scroll_bottom = self.rows.saturating_sub(1);
            self.wrap_next = false;
            self.auto_wrap = true;
            self.attr_fg = self.default_fg;
            self.attr_bg = self.default_bg;
            self.attr_bold = self.default_bold;
            self.attr_italic = false;
            self.attr_underline = false;
            self.attr_reverse = false;
            self.active_hyperlink = 0;
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
    /// Returns whichever scrollback buffer is currently in use — the alt
    /// buffer while on the alt screen, otherwise the main one. Callers
    /// outside this module should use this rather than reading `.scrollback`
    /// directly so they see the right history while a TUI is running.
    pub fn active_scrollback(&self) -> &Vec<Vec<Cell>> {
        if self.alt_grid.is_some() {
            &self.alt_scrollback
        } else {
            &self.scrollback
        }
    }

    /// True while the app is on the alternate screen (DECSET 1049/1047/47).
    pub fn is_alt_screen(&self) -> bool {
        self.alt_grid.is_some()
    }

    /// Wipe the active scrollback buffer (main or alt, whichever is live).
    /// Selection coordinates are absolute (scrollback-relative) so they'd
    /// dangle after the wipe — clear them too, and snap back to the bottom.
    pub fn clear_scrollback(&mut self) {
        if self.alt_grid.is_some() {
            self.alt_scrollback.clear();
        } else {
            self.scrollback.clear();
        }
        self.scroll_offset = 0;
        self.clear_selection();
    }

    pub fn display_line(&self, row: usize) -> &[Cell] {
        if self.scroll_offset == 0 {
            if row < self.grid.len() {
                return &self.grid[row];
            }
            return &[];
        }

        let sb = self.active_scrollback();
        let scrollback_len = sb.len();
        let scrollback_start = scrollback_len.saturating_sub(self.scroll_offset);
        let line_idx = scrollback_start + row;

        if line_idx < scrollback_len {
            &sb[line_idx]
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
        let scrollback_len = self.active_scrollback().len();
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
        let sb = self.active_scrollback();
        let scrollback_len = sb.len();
        if abs_row < scrollback_len {
            &sb[abs_row]
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
            let row_start = text.len();
            for col in col_start..=col_end.min(line.len().saturating_sub(1)) {
                // Skip wide-char tail cells — the head already has the character
                if line[col].wide == Wide::Tail {
                    continue;
                }
                text.push(line[col].c);
            }
            // A row that soft-wraps continues on the next row: it's one
            // logical line, so no newline — and no trimming, because every
            // cell up to the wrap is real printed content (a command's
            // spaces may land exactly at the row edge).
            let soft_wrapped = line.last().map_or(false, |cell| cell.wrapped);
            if soft_wrapped {
                continue;
            }
            // Trim trailing blank padding cells — but only what THIS row
            // contributed, never content from a previous wrapped row.
            let trimmed = text.trim_end_matches(' ');
            text.truncate(trimmed.len().max(row_start));
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
