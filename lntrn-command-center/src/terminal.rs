//! Mini terminal view — embeds a real PTY + VTE state grid from the
//! `lntrn-terminal` crate, so it behaves like a proper terminal
//! emulator: interactive programs, ANSI colors, persistent shell
//! state, the works.
//!
//! Layout:
//! - Header strip (in the controls row) shows a one-line "command
//!   bar" preview — actually just text that lives in the body.
//!   Typing goes to the PTY directly.
//! - Body is a `cols × rows` grid of cells drawn cell-by-cell.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_terminal::clipboard::WaylandClipboard;
use lntrn_terminal::pty::Pty;
use lntrn_terminal::terminal::{Color8, TerminalState as Grid, Wide};

pub const INPUT_FONT: f32 = 22.0;
const SHELL_FALLBACK: &str = "/bin/bash";
pub const BODY_PAD: f32 = 16.0;

/// Per-cell logical-px width/height as a function of the unified text
/// size. Both layershell (PTY sizing) and `draw` (rendering) must use
/// this so the shell's wrap column matches the visible column count.
pub fn cell_size_logical(text_size: f32) -> (f32, f32) {
    (text_size * 0.60, text_size * 1.30)
}

/// Compute the physical-pixel body rect + cell metrics + grid (cols,
/// rows) given the current panel dimensions. Single source of truth
/// shared between PTY resize and the painter pass.
pub fn body_metrics(
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
) -> (Rect, f32, f32, usize, usize) {
    let pad = BODY_PAD * scale;
    let body_x = panel.x + pad;
    let body_y = top_y + pad;
    let body_w = (panel.x + panel.w - pad - body_x).max(0.0);
    let body_h = (panel.y + panel.h - pad - body_y).max(0.0);
    let (cw, ch) = cell_size_logical(text_size);
    let cell_w = cw * scale;
    let cell_h = ch * scale;
    let cols = if cell_w > 0.0 {
        (body_w / cell_w).floor() as usize
    } else {
        0
    };
    let rows = if cell_h > 0.0 {
        (body_h / cell_h).floor() as usize
    } else {
        0
    };
    (
        Rect::new(body_x, body_y, body_w, body_h),
        cell_w,
        cell_h,
        cols.max(1),
        rows.max(1),
    )
}

const STATUS_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);

pub struct TerminalState {
    pty: Option<Pty>,
    grid: Option<Grid>,
    cols: usize,
    rows: usize,
    /// Last error from spawning the PTY (so we can show it).
    spawn_error: Option<String>,
    /// Cached buffer used for accumulating partial reads between frames.
    buf: Vec<u8>,
    /// Wayland clipboard handle. Lazily constructed on first copy/paste —
    /// no-op if the compositor doesn't expose wlr-data-control.
    clipboard: Option<WaylandClipboard>,
    /// True while the user is in the middle of a drag-select gesture
    /// (mouse pressed, possibly moving). Pointer-up clears it but the
    /// selection stays visible until the next click or a Copy.
    pub selecting: bool,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            pty: None,
            grid: None,
            cols: 80,
            rows: 24,
            spawn_error: None,
            buf: Vec::with_capacity(8192),
            clipboard: None,
            selecting: false,
        }
    }

    fn ensure_clipboard(&mut self) -> Option<&WaylandClipboard> {
        if self.clipboard.is_none() {
            self.clipboard = WaylandClipboard::new();
        }
        self.clipboard.as_ref()
    }

    /// Begin a drag-selection at the given visible (row, col).
    pub fn begin_selection(&mut self, vrow: usize, col: usize) {
        if let Some(g) = self.grid.as_mut() {
            g.set_selection_anchor(vrow, col);
            g.set_selection_end(vrow, col);
        }
        self.selecting = true;
    }

    /// Extend an in-progress drag-selection to the cell under the cursor.
    pub fn update_selection(&mut self, vrow: usize, col: usize) {
        if !self.selecting {
            return;
        }
        if let Some(g) = self.grid.as_mut() {
            g.set_selection_end(vrow, col);
        }
    }

    /// Finalize the drag-select gesture; the selection stays visible.
    pub fn end_selection(&mut self) {
        self.selecting = false;
    }

    /// Drop the current selection (visual highlight + range).
    pub fn clear_selection(&mut self) {
        if let Some(g) = self.grid.as_mut() {
            g.clear_selection();
        }
        self.selecting = false;
    }

    /// True when a non-empty selection exists.
    pub fn has_selection(&self) -> bool {
        self.grid.as_ref().and_then(|g| g.selected_text()).is_some()
    }

    /// Copy the currently-selected text onto the Wayland clipboard.
    /// No-op when nothing is selected.
    pub fn copy_selection(&mut self) -> bool {
        let Some(text) = self.grid.as_ref().and_then(|g| g.selected_text()) else {
            return false;
        };
        let Some(clip) = self.ensure_clipboard() else {
            return false;
        };
        clip.set_text(&text);
        true
    }

    /// Read the Wayland clipboard and write the bytes to the PTY.
    /// Strips bracketed-paste-unfriendly characters? — we just pass
    /// through as-is for now; the shell handles bracketed paste itself.
    pub fn paste_from_clipboard(&mut self) -> bool {
        let Some(clip) = self.ensure_clipboard() else {
            return false;
        };
        let Some(text) = clip.get_text() else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        self.write(text.as_bytes());
        true
    }

    pub fn is_spawned(&self) -> bool {
        self.pty.is_some()
    }

    /// Lazily spawn the PTY the first time the user opens the Terminal
    /// view. Keeps the cost off the panel-open hot path.
    pub fn ensure_spawned(&mut self, cols: usize, rows: usize) {
        if self.pty.is_some() {
            // Resize on layout change.
            if cols != self.cols || rows != self.rows {
                self.resize(cols, rows);
            }
            return;
        }
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| SHELL_FALLBACK.to_string());
        let cwd = std::env::var("HOME").ok();
        match Pty::spawn(&shell, cwd.as_deref(), Box::new(|| {})) {
            Ok(pty) => {
                pty.resize(self.cols as u16, self.rows as u16);
                self.pty = Some(pty);
                self.grid = Some(Grid::new(self.cols, self.rows));
                self.spawn_error = None;
            }
            Err(e) => {
                self.spawn_error = Some(e);
            }
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        if let Some(pty) = &self.pty {
            pty.resize(self.cols as u16, self.rows as u16);
        }
        if let Some(grid) = self.grid.as_mut() {
            grid.resize(self.cols, self.rows);
        }
    }

    /// Drain bytes from the PTY into the VTE grid. Returns whether any
    /// new bytes were processed.
    pub fn pump(&mut self) -> bool {
        let Some(pty) = self.pty.as_mut() else {
            return false;
        };
        let mut any = false;
        // Burn through what's available, capped per frame to avoid
        // starving the panel render loop on a flood (e.g. `cat` of a
        // huge file).
        for _ in 0..16 {
            match pty.read(8192) {
                Some((data, _eof)) => {
                    if !data.is_empty() {
                        self.buf.extend_from_slice(&data);
                        any = true;
                    } else {
                        break;
                    }
                }
                None => break,
            }
        }
        if any {
            if let Some(grid) = self.grid.as_mut() {
                grid.process(&self.buf);
            }
            self.buf.clear();
        }
        any
    }

    /// Send raw bytes to the PTY (keypresses, paste, etc.).
    pub fn write(&self, data: &[u8]) {
        if let Some(pty) = &self.pty {
            pty.write(data);
        }
    }

    /// Scroll the terminal viewport. Positive `lines` moves toward the
    /// past (older scrollback rows come into view); negative moves
    /// back toward the live shell at the bottom. Clamped to the grid's
    /// scrollback buffer.
    pub fn scroll_by(&mut self, lines: i32) {
        let Some(grid) = self.grid.as_mut() else {
            return;
        };
        if lines > 0 {
            grid.scroll_offset = (grid.scroll_offset + lines as usize).min(grid.scrollback.len());
        } else {
            grid.scroll_offset = grid.scroll_offset.saturating_sub((-lines) as usize);
        }
    }

    pub fn clear(&mut self) {
        // Send the standard "clear screen + home cursor" sequence so
        // the shell agrees on the state.
        self.write(b"\x1b[2J\x1b[H");
    }

    #[allow(dead_code)] // wired to a future Ctrl+C hotkey
    pub fn kill_running(&mut self) -> bool {
        // Send Ctrl+C to the foreground process group.
        self.write(b"\x03");
        true
    }

    #[allow(dead_code)]
    pub fn cols(&self) -> usize {
        self.cols
    }
    #[allow(dead_code)]
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn grid(&self) -> Option<&Grid> {
        self.grid.as_ref()
    }
}

// ── Keyboard translation ───────────────────────────────────────────────────

/// Translate an evdev keycode (+ shift) into a byte sequence the PTY
/// expects. Returns None for keys we don't handle.
pub fn keycode_to_bytes(key: u32, shift: bool, ctrl: bool, caps_lock: bool) -> Option<Vec<u8>> {
    use crate::search::input::*;
    match key {
        KEY_ENTER | KEY_KP_ENTER => Some(b"\r".to_vec()),
        KEY_BACKSPACE => Some(b"\x7f".to_vec()),
        KEY_DELETE => Some(b"\x1b[3~".to_vec()),
        KEY_LEFT => Some(b"\x1b[D".to_vec()),
        KEY_RIGHT => Some(b"\x1b[C".to_vec()),
        KEY_UP => Some(b"\x1b[A".to_vec()),
        KEY_DOWN => Some(b"\x1b[B".to_vec()),
        KEY_HOME => Some(b"\x1b[H".to_vec()),
        KEY_END => Some(b"\x1b[F".to_vec()),
        KEY_TAB => Some(b"\t".to_vec()),
        other => {
            let ch = keycode_to_char(other, shift, caps_lock)?;
            if ctrl && ch.is_ascii_alphabetic() {
                // Ctrl-letter → control byte.
                let upper = ch.to_ascii_uppercase() as u8;
                let cc = upper.wrapping_sub(b'A').wrapping_add(1);
                Some(vec![cc])
            } else {
                let mut buf = [0u8; 4];
                Some(ch.encode_utf8(&mut buf).as_bytes().to_vec())
            }
        }
    }
}

// ── Drawing ────────────────────────────────────────────────────────────────

/// Draw a one-line mirror of the live cursor row into the controls-row
/// input strip. Echoes the shell prompt + whatever the user is typing,
/// even while the panel is collapsed (so the body grid isn't visible).
#[allow(clippy::too_many_arguments)]
pub fn draw_input_strip(
    painter: &mut Painter,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
    term: &TerminalState,
    rect: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    // Plate behind the prompt mirror.
    let bg = Color::from_rgb8(18, 18, 18).with_alpha(0.80 * alpha);
    painter.rect_filled(rect, 12.0 * scale, bg);
    painter.rect_stroke_sdf(
        rect,
        12.0 * scale,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.18 * alpha),
    );

    let Some(grid) = term.grid() else {
        // PTY not spawned yet — show a hint.
        let font = INPUT_FONT * scale;
        text.queue(
            "Starting shell…",
            font,
            rect.x + 16.0 * scale,
            rect.y + (rect.h - font) / 2.0,
            Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.45 * alpha),
            rect.w - 32.0 * scale,
            surface_w,
            surface_h,
        );
        return;
    };

    let cursor_row_idx = grid.cursor_row.min(grid.rows.saturating_sub(1));
    let Some(row) = grid.grid.get(cursor_row_idx) else {
        return;
    };

    // Determine cell metrics from the strip height. Leave a little
    // breathing room top/bottom so the cursor box doesn't bleed past
    // the rounded strip border.
    let inner_inset = 6.0 * scale;
    let cell_h = (rect.h - inner_inset * 2.0).max(8.0);
    let cell_w = (cell_h * 0.50).max(7.0 * scale);
    let font = cell_h * 0.78;
    let pad_left = 12.0 * scale;
    let strip_inner_y = rect.y + (rect.h - cell_h) / 2.0;
    let max_cells = ((rect.w - pad_left * 2.0) / cell_w).floor() as usize;

    // Painter clip so partial cells at the right edge don't bleed.
    painter.push_clip(rect);
    mono_text.push_clip([rect.x, rect.y, rect.w, rect.h]);

    let mut tmp = [0u8; 4];
    for (col, cell) in row.iter().enumerate().take(max_cells) {
        if matches!(cell.wide, Wide::Tail) {
            continue;
        }
        let x = rect.x + pad_left + col as f32 * cell_w;
        // Cell bg (skip default).
        if cell.bg != grid.default_bg {
            painter.rect_filled(
                Rect::new(x, strip_inner_y, cell_w, cell_h),
                0.0,
                color8_to_color(cell.bg, alpha),
            );
        }
        if cell.c != ' ' && cell.c != '\0' {
            let s = cell.c.encode_utf8(&mut tmp);
            mono_text.queue(
                s,
                font,
                x,
                strip_inner_y + (cell_h - font) * 0.5,
                color8_to_color(cell.fg, alpha),
                cell_w
                    * if matches!(cell.wide, Wide::Head) {
                        2.0
                    } else {
                        1.0
                    }
                    + 2.0 * scale,
                surface_w,
                surface_h,
            );
        }
    }

    // Cursor pill.
    if !grid.cursor_hidden && grid.cursor_col < max_cells {
        let x = rect.x + pad_left + grid.cursor_col as f32 * cell_w;
        let col = color8_to_color(grid.default_fg, 0.6 * alpha);
        painter.rect_filled(Rect::new(x, strip_inner_y, cell_w, cell_h), 0.0, col);
    }

    painter.pop_clip();
    mono_text.pop_clip();
    let _ = text;
}

/// Convert a physical-pixel cursor position into a (visible_row, col)
/// pair inside the terminal grid. Returns `None` when the cursor is
/// outside the body region.
pub fn cell_at(
    panel: Rect,
    top_y: f32,
    scale: f32,
    cell_font_logical: f32,
    grid_cols: usize,
    grid_rows: usize,
    phys_x: f32,
    phys_y: f32,
) -> Option<(usize, usize)> {
    let (body, cell_w, cell_h, _, _) = body_metrics(panel, top_y, scale, cell_font_logical);
    if phys_x < body.x || phys_y < body.y || phys_x > body.x + body.w || phys_y > body.y + body.h {
        return None;
    }
    if cell_w <= 0.0 || cell_h <= 0.0 {
        return None;
    }
    let col = (((phys_x - body.x) / cell_w).floor() as usize).min(grid_cols.saturating_sub(1));
    let row = (((phys_y - body.y) / cell_h).floor() as usize).min(grid_rows.saturating_sub(1));
    Some((row, col))
}

fn color8_to_color(c: Color8, alpha_mul: f32) -> Color {
    if c.a == 0 {
        return Color::TRANSPARENT;
    }
    Color::from_rgb8(c.r, c.g, c.b).with_alpha((c.a as f32 / 255.0) * alpha_mul)
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
    term: &TerminalState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    cell_font_logical: f32,
) {
    let (body, cell_w, cell_h, _cols, _rows) = body_metrics(panel, top_y, scale, cell_font_logical);
    let body_x = body.x;
    let body_y = body.y;
    let body_w = body.w;
    let body_h = body.h;
    if body_w <= 0.0 || body_h <= 0.0 {
        return;
    }

    // Show an error message if the PTY failed to spawn.
    if let Some(err) = &term.spawn_error {
        let s = format!("Terminal unavailable: {}", err);
        let font = INPUT_FONT * scale;
        text.queue(
            &s,
            font,
            body_x,
            body_y,
            Color::from_rgb8(STATUS_RGB.0, STATUS_RGB.1, STATUS_RGB.2).with_alpha(0.75 * alpha),
            body_w,
            surface_w,
            surface_h,
        );
        return;
    }

    let Some(grid) = term.grid() else {
        // PTY hasn't spawned yet; show a "starting" message.
        let font = INPUT_FONT * scale;
        text.queue(
            "Starting shell…",
            font,
            body_x,
            body_y,
            Color::from_rgb8(STATUS_RGB.0, STATUS_RGB.1, STATUS_RGB.2).with_alpha(0.45 * alpha),
            body_w,
            surface_w,
            surface_h,
        );
        return;
    };

    let font = cell_font_logical * scale;

    // Clip the grid output strictly to the body.
    painter.push_clip(Rect::new(body_x, body_y, body_w, body_h));
    mono_text.push_clip([body_x, body_y, body_w, body_h]);

    // Helper: get the i-th visible row taking scrollback + offset
    // into account. Returns None when out of bounds.
    let scroll = grid.scroll_offset;
    let scrollback = &grid.scrollback;
    let live = &grid.grid;
    let row_at = |vis: usize| -> Option<&Vec<lntrn_terminal::terminal::Cell>> {
        let top_abs = scrollback.len().saturating_sub(scroll);
        let abs = top_abs + vis;
        if abs < scrollback.len() {
            scrollback.get(abs)
        } else {
            live.get(abs - scrollback.len())
        }
    };

    // First pass: per-cell background rects (skip default) + selection
    // tint. Selection tint sits ON TOP of the cell bg so it remains
    // readable over ANSI-coloured runs.
    let selection_color = Color::rgba(0.40, 0.75, 1.00, 0.32 * alpha);
    // Skip the live cursor row in the body — it's already shown in the
    // top-strip mirror, so duplicating it here just doubles the line.
    // When scrolled into history we draw every row (the strip still
    // shows the live cursor row from the unscrolled grid).
    let skip_cursor_row = grid.scroll_offset == 0;
    for row in 0..grid.rows {
        if skip_cursor_row && row == grid.cursor_row {
            continue;
        }
        let y = body_y + row as f32 * cell_h;
        if y >= body_y + body_h {
            break;
        }
        if let Some(grid_row) = row_at(row) {
            for (col, cell) in grid_row.iter().enumerate() {
                let x = body_x + col as f32 * cell_w;
                if cell.bg != grid.default_bg {
                    painter.rect_filled(
                        Rect::new(x, y, cell_w, cell_h),
                        0.0,
                        color8_to_color(cell.bg, alpha),
                    );
                }
                if grid.is_selected(row, col) {
                    painter.rect_filled(Rect::new(x, y, cell_w, cell_h), 0.0, selection_color);
                }
            }
        }
    }

    // Second pass: glyphs (one queued string per cell — slow but
    // simple; a future pass can batch contiguous same-color runs).
    let mut tmp = [0u8; 4];
    for row in 0..grid.rows {
        if skip_cursor_row && row == grid.cursor_row {
            continue;
        }
        let y = body_y + row as f32 * cell_h;
        if y >= body_y + body_h {
            break;
        }
        if let Some(grid_row) = row_at(row) {
            for (col, cell) in grid_row.iter().enumerate() {
                if matches!(cell.wide, Wide::Tail) {
                    // Tail of a wide char — head already drew the glyph.
                    continue;
                }
                if cell.c == ' ' || cell.c == '\0' {
                    continue;
                }
                let x = body_x + col as f32 * cell_w;
                let s = cell.c.encode_utf8(&mut tmp);
                mono_text.queue(
                    s,
                    font,
                    x,
                    y + (cell_h - font) * 0.5,
                    color8_to_color(cell.fg, alpha),
                    cell_w
                        * if matches!(cell.wide, Wide::Head) {
                            2.0
                        } else {
                            1.0
                        }
                        + 2.0 * scale,
                    surface_w,
                    surface_h,
                );
            }
        }
    }
    // Cursor pill lives in the top-strip mirror — not drawn in the body.

    painter.pop_clip();
    mono_text.pop_clip();
}
