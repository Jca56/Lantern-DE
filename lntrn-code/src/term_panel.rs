//! Terminal panel — an embedded terminal emulator that lives below the editor.
//! Toggle with Ctrl+`. The panel spawns a PTY running the user's shell and
//! renders the grid using the same GPU pipeline as the editor.

use winit::event_loop::EventLoopProxy;
use winit::keyboard::{Key, ModifiersState};

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::FoxPalette;

use crate::term::grid::TerminalState;
use crate::term::pty::Pty;
use crate::term::render::{draw_terminal_ex, measure_cell};
use crate::UserEvent;

/// Logical height of the terminal panel as a fraction of the editor area.
const PANEL_FRACTION: f32 = 0.35;
/// Font size for the terminal (logical px, scaled at draw time).
const FONT_SIZE: f32 = 20.0;
/// Padding above the terminal grid.
const PAD_TOP: f32 = 6.0;
/// Padding left of the terminal grid.
const PAD_LEFT: f32 = 8.0;

pub struct TermPanel {
    pub terminal: TerminalState,
    pub pty: Pty,
    /// Whether the panel is visible.
    pub visible: bool,
    /// Whether the terminal has keyboard focus (vs the editor).
    pub focused: bool,
    /// Cursor blink state for the terminal.
    pub cursor_visible: bool,
    /// Current grid dimensions (updated on resize).
    cols: usize,
    rows: usize,
}

impl TermPanel {
    /// Spawn a new terminal panel. The PTY reader thread sends `UserEvent::PtyOutput`
    /// via the proxy to wake the event loop when new output arrives.
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let repaint = Box::new(move || {
            let _ = proxy.send_event(UserEvent::PtyOutput);
        });
        let pty = Pty::spawn(&shell, cwd.as_deref(), repaint)?;

        // Start with a small default; resize() is called on the first frame.
        let cols = 80;
        let rows = 24;
        let mut terminal = TerminalState::new(cols, rows);
        terminal.default_fg = crate::term::Color8 { r: 220, g: 225, b: 240, a: 255 };
        terminal.default_bg = crate::term::Color8 { r: 0, g: 0, b: 0, a: 0 };
        terminal.attr_fg = terminal.default_fg;
        terminal.attr_bg = terminal.default_bg;
        pty.resize(cols as u16, rows as u16);

        Ok(Self {
            terminal,
            pty,
            visible: true,
            focused: true,
            cursor_visible: true,
            cols,
            rows,
        })
    }

    /// Drain pending PTY output and feed it to the terminal state.
    pub fn drain(&mut self) {
        while let Some((data, _more)) = self.pty.read(65536) {
            self.terminal.process(&data);
            // Write any pending responses (e.g. cursor position reports) back.
            for resp in self.terminal.pending_responses.drain(..) {
                self.pty.write(&resp);
            }
        }
    }

    /// Recompute grid size from the panel rect. Only resizes the PTY if the
    /// dimensions actually changed.
    pub fn update_size(&mut self, rect: Rect, scale: f32) {
        let font_px = FONT_SIZE * scale;
        let (cell_w, cell_h) = measure_cell(font_px);
        let pad_x = PAD_LEFT * scale;
        let pad_y = PAD_TOP * scale;
        let cols = ((rect.w - pad_x * 2.0) / cell_w).floor().max(1.0) as usize;
        let rows = ((rect.h - pad_y) / cell_h).floor().max(1.0) as usize;
        if cols != self.cols || rows != self.rows {
            self.cols = cols;
            self.rows = rows;
            self.terminal.resize(cols, rows);
            self.pty.resize(cols as u16, rows as u16);
        }
    }

    /// Handle a keyboard event. Returns true if consumed.
    pub fn handle_key(&mut self, key: &Key, mods: ModifiersState) -> bool {
        crate::term::input::handle_key(key, winit::event::ElementState::Pressed, mods, &mut self.terminal, &self.pty)
    }

    /// Compute the panel rect given the full editor area and scale.
    pub fn panel_rect(full_area: Rect, scale: f32) -> Rect {
        let h = (full_area.h * PANEL_FRACTION).max(60.0 * scale);
        Rect::new(full_area.x, full_area.y + full_area.h - h, full_area.w, h)
    }

    /// Compute the editor rect (above the panel) given the full area.
    pub fn editor_rect_above(full_area: Rect, scale: f32) -> Rect {
        let panel_h = (full_area.h * PANEL_FRACTION).max(60.0 * scale);
        Rect::new(full_area.x, full_area.y, full_area.w, (full_area.h - panel_h).max(0.0))
    }

    /// Draw the terminal panel.
    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        rect: Rect,
        palette: &FoxPalette,
        scale: f32,
        sw: u32,
        sh: u32,
    ) {
        let s = scale;

        // Panel background.
        painter.rect_filled(rect, 0.0, palette.surface);

        // Top separator line.
        painter.line(rect.x, rect.y, rect.x + rect.w, rect.y, 1.0 * s, palette.surface_2);

        let font_px = FONT_SIZE * s;
        let pad_x = PAD_LEFT * s;
        let pad_y = PAD_TOP * s;
        let origin = (rect.x + pad_x, rect.y + pad_y);
        let bg = palette.surface;
        let cursor_color = palette.accent;

        painter.push_clip(rect);
        draw_terminal_ex(
            painter,
            text,
            &self.terminal,
            font_px,
            origin,
            sw,
            sh,
            self.cursor_visible,
            bg,
            cursor_color,
            0,
        );
        painter.pop_clip();
    }
}
