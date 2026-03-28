//! Terminal grid renderer skeleton.
//!
//! Provides a framework-agnostic API for rendering a monospace terminal grid
//! using `Painter` (cell backgrounds, cursor, selection) and `TextRenderer`
//! (glyph queuing). This is the reusable rendering layer — it knows nothing
//! about PTY, terminal emulation, or windowing.

use lntrn_draw::{Color, Painter, Rect};
use lntrn_text::TextRenderer;

/// Standard ANSI 16-color palette in linear Color space.
pub struct AnsiPalette;

impl AnsiPalette {
    pub const COLORS: [Color; 16] = [
        Color::rgb(0.0, 0.0, 0.0),                                  // 0  Black
        Color::rgba(0.586, 0.023, 0.023, 1.0),                      // 1  Red
        Color::rgba(0.005, 0.476, 0.100, 1.0),                      // 2  Green
        Color::rgba(0.737, 0.737, 0.009, 1.0),                      // 3  Yellow
        Color::rgba(0.052, 0.287, 0.265, 1.0),                      // 4  Blue
        Color::rgba(0.476, 0.031, 0.476, 1.0),                      // 5  Magenta
        Color::rgba(0.010, 0.185, 0.293, 1.0),                      // 6  Cyan
        Color::rgba(0.737, 0.737, 0.737, 1.0),                      // 7  White
        Color::rgba(0.082, 0.082, 0.082, 1.0),                      // 8  Bright Black
        Color::rgba(0.810, 0.044, 0.044, 1.0),                      // 9  Bright Red
        Color::rgba(0.013, 0.620, 0.128, 1.0),                      // 10 Bright Green
        Color::rgba(0.850, 0.850, 0.034, 1.0),                      // 11 Bright Yellow
        Color::rgba(0.098, 0.711, 0.647, 1.0),                      // 12 Bright Blue
        Color::rgba(0.642, 0.091, 0.642, 1.0),                      // 13 Bright Magenta
        Color::rgba(0.015, 0.231, 0.340, 1.0),                      // 14 Bright Cyan
        Color::rgba(0.737, 0.737, 0.737, 1.0),                      // 15 Bright White
    ];

    pub fn color(index: u8) -> Color {
        Self::COLORS[(index as usize) & 0x0F]
    }
}

/// Whether a cell is part of a wide character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridCellWide {
    No,
    Head,
    Tail,
}

/// Cell data for one grid position, framework-agnostic.
#[derive(Clone, Debug)]
pub struct GridCell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub wide: GridCellWide,
}

impl Default for GridCell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: Color::from_rgb8(236, 236, 236),
            bg: Color::TRANSPARENT,
            bold: false,
            wide: GridCellWide::No,
        }
    }
}

/// Fixed-size cell metrics for a monospace font.
#[derive(Clone, Copy, Debug)]
pub struct CellMetrics {
    pub cell_w: f32,
    pub cell_h: f32,
}

impl CellMetrics {
    pub fn new(cell_w: f32, cell_h: f32) -> Self {
        Self { cell_w, cell_h }
    }
}

/// Cursor shape requested by the application via DECSCUSR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Beam,
    Underline,
}

/// Cursor state.
#[derive(Clone, Copy, Debug)]
pub struct CursorState {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
    pub color: Color,
    pub shape: CursorShape,
}

/// Selection range (row/col in visible coordinates).
#[derive(Clone, Copy, Debug)]
pub struct SelectionRange {
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
    pub color: Color,
}

impl SelectionRange {
    pub fn contains(&self, row: usize, col: usize) -> bool {
        let start = (self.start_row, self.start_col);
        let end = (self.end_row, self.end_col);
        let (s, e) = if start <= end { (start, end) } else { (end, start) };
        (row, col) >= s && (row, col) <= e
    }
}

/// Renders a terminal grid using Painter + TextRenderer.
///
/// Usage:
/// ```ignore
/// let grid = TerminalGridRenderer::new(metrics);
/// grid.draw_backgrounds(painter, origin, &rows, visible_rows, visible_cols);
/// grid.draw_selection(painter, origin, &selection);
/// grid.draw_glyphs(text, origin, &rows, visible_rows, visible_cols, screen_w, screen_h);
/// grid.draw_cursor(painter, origin, &cursor);
/// ```
pub struct TerminalGridRenderer {
    pub metrics: CellMetrics,
}

impl TerminalGridRenderer {
    pub fn new(metrics: CellMetrics) -> Self {
        Self { metrics }
    }

    /// Draw cell background fills for all non-transparent cells.
    pub fn draw_backgrounds(
        &self,
        painter: &mut Painter,
        origin: (f32, f32),
        rows: &[&[GridCell]],
        visible_rows: usize,
        visible_cols: usize,
    ) {
        for row in 0..visible_rows.min(rows.len()) {
            let line = rows[row];
            for col in 0..visible_cols.min(line.len()) {
                let cell = &line[col];
                if cell.bg.a < 0.001 {
                    continue;
                }
                let rect = self.cell_rect(origin, row, col);
                painter.rect_filled(rect, 0.0, cell.bg);
            }
        }
    }

    /// Draw selection highlight overlay.
    pub fn draw_selection(
        &self,
        painter: &mut Painter,
        origin: (f32, f32),
        selection: &SelectionRange,
        visible_rows: usize,
        visible_cols: usize,
    ) {
        for row in 0..visible_rows {
            for col in 0..visible_cols {
                if selection.contains(row, col) {
                    let rect = self.cell_rect(origin, row, col);
                    painter.rect_filled(rect, 0.0, selection.color);
                }
            }
        }
    }

    /// Queue visible glyphs through the text renderer.
    ///
    /// Each character is rendered individually at its exact grid cell position
    /// so that font shaping/advance widths never cause drift from the grid.
    pub fn draw_glyphs(
        &self,
        text: &mut TextRenderer,
        origin: (f32, f32),
        rows: &[&[GridCell]],
        visible_rows: usize,
        visible_cols: usize,
        font_size: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        let mut char_buf = [0u8; 4];

        for row in 0..visible_rows.min(rows.len()) {
            let line = rows[row];
            let y = (origin.1 + row as f32 * self.metrics.cell_h).floor();

            for col in 0..visible_cols.min(line.len()) {
                let cell = &line[col];

                // Skip tail cells and blanks — nothing to render
                if cell.wide == GridCellWide::Tail || cell.c == ' ' || cell.c == '\0' {
                    continue;
                }

                let x = (origin.0 + col as f32 * self.metrics.cell_w).floor();
                let max_w = if cell.wide == GridCellWide::Head {
                    2.0 * self.metrics.cell_w + 2.0
                } else {
                    self.metrics.cell_w + 2.0
                };

                let s = cell.c.encode_utf8(&mut char_buf);
                text.queue(s, font_size, x, y, cell.fg, max_w, screen_w, screen_h);
            }
        }
    }

    /// Draw the cursor in its current shape (block, beam, or underline).
    pub fn draw_cursor(
        &self,
        painter: &mut Painter,
        origin: (f32, f32),
        cursor: &CursorState,
    ) {
        if !cursor.visible {
            return;
        }
        let cell = self.cell_rect(origin, cursor.row, cursor.col);
        match cursor.shape {
            CursorShape::Block => {
                painter.rect_filled(cell, 0.0, cursor.color);
            }
            CursorShape::Beam => {
                let beam_w = 2.0;
                painter.rect_filled(
                    Rect::new(cell.x, cell.y, beam_w, cell.h),
                    0.0,
                    cursor.color,
                );
            }
            CursorShape::Underline => {
                let underline_h = 3.0;
                painter.rect_filled(
                    Rect::new(cell.x, cell.y + cell.h - underline_h, cell.w, underline_h),
                    0.0,
                    cursor.color,
                );
            }
        }
    }

    /// Draw cursor with the character underneath rendered in a contrasting color.
    /// For block cursors, the char is drawn on top in a contrasting color.
    /// For beam/underline, the char doesn't need re-drawing (glyph pass handles it).
    pub fn draw_cursor_with_char(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        origin: (f32, f32),
        cursor: &CursorState,
        cell_char: char,
        char_color: Color,
        font_size: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        self.draw_cursor(painter, origin, cursor);
        // Only block cursor needs the character re-drawn in contrasting color
        if cursor.shape == CursorShape::Block
            && cursor.visible
            && cell_char != ' '
            && cell_char != '\0'
        {
            let rect = self.cell_rect(origin, cursor.row, cursor.col);
            let s = cell_char.to_string();
            text.queue(
                &s,
                font_size,
                rect.x,
                rect.y,
                char_color,
                self.metrics.cell_w,
                screen_w,
                screen_h,
            );
        }
    }

    fn cell_rect(&self, origin: (f32, f32), row: usize, col: usize) -> Rect {
        let x = (origin.0 + col as f32 * self.metrics.cell_w).floor();
        let y = (origin.1 + row as f32 * self.metrics.cell_h).floor();
        let nx = (origin.0 + (col + 1) as f32 * self.metrics.cell_w).ceil();
        let ny = (origin.1 + (row + 1) as f32 * self.metrics.cell_h).ceil();
        Rect::new(x, y, nx - x, ny - y)
    }

}
