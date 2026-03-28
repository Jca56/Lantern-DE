use lntrn_render::{
    CellMetrics, Color, CursorShape, CursorState, GridCell, GridCellWide, Painter, Rect,
    TerminalGridRenderer, TextRenderer,
};

use crate::terminal::{Color8, TerminalState, Wide};
use crate::theme;

/// Height of chrome above the terminal grid (title bar + tab bar).
pub fn chrome_height() -> f32 {
    crate::ui_chrome::TITLE_BAR_HEIGHT + crate::tab_bar::TAB_BAR_HEIGHT
}

fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

/// Measure the monospace cell dimensions for the given font size.
pub fn measure_cell(font_size: f32) -> (f32, f32) {
    let cell_w = (font_size * 0.6).ceil();
    let cell_h = (font_size * 1.2).floor();
    (cell_w, cell_h)
}

/// Corner radius — must match the compositor's CORNER_RADIUS (10).
const CORNER_RADIUS: f32 = 10.0;

/// Draw the window background with rounded corners matching the compositor.
/// When maximized, corners are square (radius=0) so the bg doesn't bleed through.
pub fn draw_window_bg(
    painter: &mut Painter,
    title_bar_color: Color,
    bg_color: Color,
    w: f32,
    h: f32,
    maximized: bool,
) {
    let r = if maximized { 0.0 } else { CORNER_RADIUS };
    let title_h = chrome_height();

    // Draw the full window bg as one rounded rect — no overlap artifacts with transparency
    painter.rect_filled(Rect::new(0.0, 0.0, w, h), r, bg_color);

    // Title bar on top (always opaque, covers bg's top area)
    painter.rect_filled(Rect::new(0.0, 0.0, w, title_h), r, title_bar_color);
    // Square off the bottom corners of the title bar
    painter.rect_filled(
        Rect::new(0.0, r, w, (title_h - r).max(0.0)),
        0.0,
        title_bar_color,
    );
}

/// Render the full terminal grid at the given origin.
/// `extra_rows` renders additional rows beyond terminal.rows (for sub-pixel scroll fill).
pub fn draw_terminal(
    painter: &mut Painter,
    text: &mut TextRenderer,
    terminal: &TerminalState,
    font_size: f32,
    origin: (f32, f32),
    screen_w: u32,
    screen_h: u32,
    cursor_visible: bool,
    bg_color: Color,
) {
    draw_terminal_ex(painter, text, terminal, font_size, origin, screen_w, screen_h, cursor_visible, bg_color, 0);
}

/// Extended terminal draw with extra rows for smooth scroll fill.
pub fn draw_terminal_ex(
    painter: &mut Painter,
    text: &mut TextRenderer,
    terminal: &TerminalState,
    font_size: f32,
    origin: (f32, f32),
    screen_w: u32,
    screen_h: u32,
    cursor_visible: bool,
    bg_color: Color,
    extra_rows: usize,
) {
    let (cell_w, cell_h) = measure_cell(font_size);

    let metrics = CellMetrics::new(cell_w, cell_h);
    let render_rows = terminal.rows + extra_rows;

    // ── Cell backgrounds (only non-default) ───────────────────────────
    for row in 0..render_rows {
        let line = terminal.display_line(row);
        for col in 0..terminal.cols {
            if col >= line.len() {
                break;
            }
            let cell = &line[col];
            if cell.bg.a < 2 {
                continue;
            }
            let x = (origin.0 + col as f32 * cell_w).floor();
            let y = (origin.1 + row as f32 * cell_h).floor();
            let nx = (origin.0 + (col + 1) as f32 * cell_w).ceil();
            let ny = (origin.1 + (row + 1) as f32 * cell_h).ceil();
            painter.rect_filled(Rect::new(x, y, nx - x, ny - y), 0.0, c(cell.bg));
        }
    }

    // ── Glyphs ────────────────────────────────────────────────────────
    let grid_renderer = TerminalGridRenderer::new(metrics);
    let mut row_data: Vec<Vec<GridCell>> = Vec::with_capacity(render_rows);
    for row in 0..render_rows {
        let line = terminal.display_line(row);
        let mut cells = Vec::with_capacity(terminal.cols);
        for col in 0..terminal.cols {
            if col < line.len() {
                let cell = &line[col];
                let wide = match cell.wide {
                    Wide::No => GridCellWide::No,
                    Wide::Head => GridCellWide::Head,
                    Wide::Tail => GridCellWide::Tail,
                };
                cells.push(GridCell {
                    c: cell.c,
                    fg: c(cell.fg),
                    bg: Color::TRANSPARENT,
                    bold: cell.bold,
                    wide,
                });
            } else {
                cells.push(GridCell::default());
            }
        }
        row_data.push(cells);
    }

    let row_refs: Vec<&[GridCell]> = row_data.iter().map(|r| r.as_slice()).collect();
    grid_renderer.draw_glyphs(
        text,
        origin,
        &row_refs,
        render_rows,
        terminal.cols,
        font_size,
        screen_w,
        screen_h,
    );

    // ── Selection highlight ───────────────────────────────────────────
    if terminal.selection_range().is_some() {
        let sel_color = Color::from_rgba8(200, 134, 10, 100);
        for row in 0..render_rows {
            for col in 0..terminal.cols {
                if terminal.is_selected(row, col) {
                    let x = (origin.0 + col as f32 * cell_w).floor();
                    let y = (origin.1 + row as f32 * cell_h).floor();
                    let nx = (origin.0 + (col + 1) as f32 * cell_w).ceil();
                    let ny = (origin.1 + (row + 1) as f32 * cell_h).ceil();
                    painter.rect_filled(Rect::new(x, y, nx - x, ny - y), 0.0, sel_color);
                }
            }
        }
    }

    // ── Cursor ────────────────────────────────────────────────────────
    if terminal.scroll_offset == 0 && !terminal.cursor_hidden {
        // Map DECSCUSR value to shape enum
        let shape = match terminal.cursor_shape {
            3 | 4 => CursorShape::Underline,
            5 | 6 => CursorShape::Beam,
            _ => CursorShape::Block, // 0, 1, 2
        };

        // Steady cursors (even values >= 2) don't blink
        let steady = matches!(terminal.cursor_shape, 2 | 4 | 6);
        let visible = if steady { true } else { cursor_visible };

        let cursor = CursorState {
            row: terminal.cursor_row,
            col: terminal.cursor_col,
            visible,
            color: c(theme::CURSOR_COLOR),
            shape,
        };

        if terminal.cursor_row < terminal.rows && terminal.cursor_col < terminal.cols {
            let cell = &terminal.grid[terminal.cursor_row][terminal.cursor_col];
            grid_renderer.draw_cursor_with_char(
                painter, text, origin, &cursor, cell.c, bg_color, font_size, screen_w, screen_h,
            );
        } else {
            grid_renderer.draw_cursor(painter, origin, &cursor);
        }
    }
}
