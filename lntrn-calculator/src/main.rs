mod calc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer};

use calc::{Calculator, Op};

// ── Layout constants (logical pixels, multiplied by scale at render time) ────

const WIN_W: f32 = 340.0;
const WIN_H: f32 = 520.0;

const PADDING: f32 = 16.0;
const DISPLAY_H: f32 = 120.0;
const BTN_GAP: f32 = 10.0;
const BTN_ROWS: usize = 5;
const BTN_COLS: usize = 4;
const CORNER_RADIUS: f32 = 20.0;
const BTN_RADIUS: f32 = 14.0;

// ── Colors — frosted glass + mint/teal accent ───────────────────────────────

const GLASS_BG: Color = Color::rgba(0.10, 0.14, 0.22, 0.92);
const GLASS_SURFACE: Color = Color::rgba(0.5, 0.6, 0.8, 0.08);
const GLASS_BTN: Color = Color::rgba(0.5, 0.6, 0.8, 0.12);
const GLASS_BTN_HOVER: Color = Color::rgba(0.5, 0.65, 0.85, 0.22);
const GLASS_BTN_PRESS: Color = Color::rgba(0.5, 0.65, 0.85, 0.30);
const CLOSE_BTN: Color = Color::rgba(1.0, 1.0, 1.0, 0.0);
const CLOSE_BTN_HOVER: Color = Color::rgba(1.0, 0.35, 0.35, 0.6);

const ACCENT: Color = Color::rgba(0.376, 0.839, 0.757, 1.0); // mint/teal #60D6C1
const ACCENT_HOVER: Color = Color::rgba(0.439, 0.878, 0.800, 1.0);
const ACCENT_PRESS: Color = Color::rgba(0.502, 0.918, 0.843, 1.0);

const TEXT_PRIMARY: Color = Color::rgba(1.0, 1.0, 1.0, 0.95);
const TEXT_SECONDARY: Color = Color::rgba(1.0, 1.0, 1.0, 0.55);
const TEXT_ON_ACCENT: Color = Color::rgba(0.05, 0.15, 0.12, 1.0);

const OP_TEXT: Color = Color::rgba(0.376, 0.839, 0.757, 1.0); // mint for operator labels

const FONT_DISPLAY: f32 = 44.0;
const FONT_EXPR: f32 = 20.0;
const FONT_BTN: f32 = 24.0;

// ── Button definitions ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum BtnKind {
    Digit(char),
    Op(Op),
    Equals,
    Clear,
    Negate,
    Percent,
    Decimal,
}

#[derive(Clone, Copy)]
struct BtnDef {
    label: &'static str,
    kind: BtnKind,
    col_span: u8,
    accent: bool,
}

const fn btn(label: &'static str, kind: BtnKind) -> BtnDef {
    BtnDef { label, kind, col_span: 1, accent: false }
}

const fn btn_accent(label: &'static str, kind: BtnKind) -> BtnDef {
    BtnDef { label, kind, col_span: 1, accent: true }
}

const fn btn_wide(label: &'static str, kind: BtnKind) -> BtnDef {
    BtnDef { label, kind, col_span: 2, accent: false }
}

const BUTTONS: [BtnDef; 19] = [
    // Row 0: AC  ±  %  ÷
    btn("AC", BtnKind::Clear),
    btn("\u{00b1}", BtnKind::Negate),
    btn("%", BtnKind::Percent),
    btn_accent("\u{00f7}", BtnKind::Op(Op::Div)),
    // Row 1: 7  8  9  ×
    btn("7", BtnKind::Digit('7')),
    btn("8", BtnKind::Digit('8')),
    btn("9", BtnKind::Digit('9')),
    btn_accent("\u{00d7}", BtnKind::Op(Op::Mul)),
    // Row 2: 4  5  6  −
    btn("4", BtnKind::Digit('4')),
    btn("5", BtnKind::Digit('5')),
    btn("6", BtnKind::Digit('6')),
    btn_accent("\u{2212}", BtnKind::Op(Op::Sub)),
    // Row 3: 1  2  3  +
    btn("1", BtnKind::Digit('1')),
    btn("2", BtnKind::Digit('2')),
    btn("3", BtnKind::Digit('3')),
    btn_accent("+", BtnKind::Op(Op::Add)),
    // Row 4: 0(wide)  .  =
    btn_wide("0", BtnKind::Digit('0')),
    btn(".", BtnKind::Decimal),
    btn_accent("=", BtnKind::Equals),
];

// ── GPU resources ───────────────────────────────────────────────────────────

struct Gpu {
    ctx: GpuContext,
    painter: Painter,
    text: TextRenderer,
}

// ── App state ───────────────────────────────────────────────────────────────

struct CalcApp {
    window: Option<Window>,
    gpu: Option<Gpu>,
    calc: Calculator,
    scale: f32,
    needs_redraw: bool,
    cursor: Option<(f32, f32)>,
    pressed_btn: Option<usize>,
    btn_rects: Vec<Rect>,
    close_rect: Rect,
}

impl CalcApp {
    fn new() -> Self {
        Self {
            window: None,
            gpu: None,
            calc: Calculator::new(),
            scale: 1.0,
            needs_redraw: true,
            cursor: None,
            pressed_btn: None,
            btn_rects: Vec::new(),
            close_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    fn btn_at(&self, x: f32, y: f32) -> Option<usize> {
        for (i, r) in self.btn_rects.iter().enumerate() {
            if r.contains(x, y) {
                return Some(i);
            }
        }
        None
    }

    fn activate_btn(&mut self, idx: usize) {
        let def = BUTTONS[idx];
        match def.kind {
            BtnKind::Digit(d) => self.calc.press_digit(d),
            BtnKind::Op(op) => self.calc.press_operator(op),
            BtnKind::Equals => self.calc.press_equals(),
            BtnKind::Clear => self.calc.clear(),
            BtnKind::Negate => self.calc.press_negate(),
            BtnKind::Percent => self.calc.press_percent(),
            BtnKind::Decimal => self.calc.press_digit('.'),
        }
    }

    fn handle_key(&mut self, key: &Key) {
        match key {
            Key::Character(s) => match s.as_str() {
                "0" => self.calc.press_digit('0'),
                "1" => self.calc.press_digit('1'),
                "2" => self.calc.press_digit('2'),
                "3" => self.calc.press_digit('3'),
                "4" => self.calc.press_digit('4'),
                "5" => self.calc.press_digit('5'),
                "6" => self.calc.press_digit('6'),
                "7" => self.calc.press_digit('7'),
                "8" => self.calc.press_digit('8'),
                "9" => self.calc.press_digit('9'),
                "." => self.calc.press_digit('.'),
                "+" => self.calc.press_operator(Op::Add),
                "-" => self.calc.press_operator(Op::Sub),
                "*" => self.calc.press_operator(Op::Mul),
                "/" => self.calc.press_operator(Op::Div),
                "%" => self.calc.press_percent(),
                _ => {}
            },
            Key::Named(NamedKey::Enter) => self.calc.press_equals(),
            Key::Named(NamedKey::Backspace) => self.calc.press_backspace(),
            Key::Named(NamedKey::Escape) => self.calc.clear(),
            Key::Named(NamedKey::Delete) => self.calc.clear(),
            _ => {}
        }
    }

    fn render(&mut self) {
        let gpu = match &mut self.gpu {
            Some(g) => g,
            None => return,
        };

        let s = self.scale;
        let w = gpu.ctx.width() as f32;
        let h = gpu.ctx.height() as f32;
        let sw = gpu.ctx.width();
        let pad = PADDING * s;

        let painter = &mut gpu.painter;
        let text = &mut gpu.text;
        painter.clear();

        // ── Window background — frosted glass panel ──────────────────────
        // Main glass background
        painter.rect_filled(Rect::new(0.0, 0.0, w, h), CORNER_RADIUS * s, GLASS_BG);
        // ── Close button (top-right) ────────────────────────────────────
        let close_size = 24.0 * s;
        let close_margin = 10.0 * s;
        self.close_rect = Rect::new(
            w - close_size - close_margin,
            close_margin,
            close_size,
            close_size,
        );
        let close_hovered = self.cursor.map_or(false, |(cx, cy)| self.close_rect.contains(cx, cy));
        let close_bg = if close_hovered { CLOSE_BTN_HOVER } else { CLOSE_BTN };
        painter.rect_filled(self.close_rect, 6.0 * s, close_bg);
        // Draw X with two lines
        let cx = self.close_rect.x + close_size / 2.0;
        let cy = self.close_rect.y + close_size / 2.0;
        let arm = 5.0 * s;
        let x_color = if close_hovered {
            Color::rgba(1.0, 1.0, 1.0, 0.95)
        } else {
            Color::rgba(1.0, 1.0, 1.0, 0.4)
        };
        painter.line(cx - arm, cy - arm, cx + arm, cy + arm, 1.5 * s, x_color);
        painter.line(cx + arm, cy - arm, cx - arm, cy + arm, 1.5 * s, x_color);

        // ── Display area ─────────────────────────────────────────────────
        let display_rect = Rect::new(pad, pad, w - pad * 2.0, DISPLAY_H * s);
        painter.rect_filled(display_rect, 14.0 * s, GLASS_SURFACE);

        // Expression (top-right of display)
        let expr = &self.calc.expression;
        if !expr.is_empty() {
            let expr_w = text.measure_width(expr, FONT_EXPR * s);
            let expr_x = display_rect.x + display_rect.w - expr_w - 12.0 * s;
            let expr_y = display_rect.y + 14.0 * s;
            text.queue(
                expr, FONT_EXPR * s, expr_x, expr_y,
                TEXT_SECONDARY, display_rect.w, sw, 0,
            );
        }

        // Result (bottom-right of display, big and bold)
        let display_str = &self.calc.display;
        let display_size = if display_str.len() > 12 {
            FONT_DISPLAY * s * 0.7
        } else if display_str.len() > 8 {
            FONT_DISPLAY * s * 0.85
        } else {
            FONT_DISPLAY * s
        };
        let disp_w = text.measure_width(display_str, display_size);
        let disp_x = display_rect.x + display_rect.w - disp_w - 12.0 * s;
        let disp_y = display_rect.y + display_rect.h - display_size - 16.0 * s;
        text.queue(
            display_str, display_size, disp_x, disp_y,
            TEXT_PRIMARY, display_rect.w, sw, 0,
        );

        // ── Buttons grid ─────────────────────────────────────────────────
        let grid_top = display_rect.y + display_rect.h + BTN_GAP * s;
        let grid_w = w - pad * 2.0;
        let grid_h = h - grid_top - pad;
        let btn_w = (grid_w - BTN_GAP * s * (BTN_COLS as f32 - 1.0)) / BTN_COLS as f32;
        let btn_h = (grid_h - BTN_GAP * s * (BTN_ROWS as f32 - 1.0)) / BTN_ROWS as f32;

        self.btn_rects.clear();
        let mut row = 0usize;
        let mut col = 0usize;

        for (i, def) in BUTTONS.iter().enumerate() {
            let x = pad + col as f32 * (btn_w + BTN_GAP * s);
            let y = grid_top + row as f32 * (btn_h + BTN_GAP * s);
            let bw = if def.col_span == 2 {
                btn_w * 2.0 + BTN_GAP * s
            } else {
                btn_w
            };
            let rect = Rect::new(x, y, bw, btn_h);
            self.btn_rects.push(rect);

            // Determine button state
            let hovered = self.cursor.map_or(false, |(cx, cy)| rect.contains(cx, cy));
            let pressed = self.pressed_btn == Some(i);

            // Draw button
            let (bg, text_color) = if def.accent {
                let bg = if pressed {
                    ACCENT_PRESS
                } else if hovered {
                    ACCENT_HOVER
                } else {
                    ACCENT
                };
                (bg, TEXT_ON_ACCENT)
            } else {
                let bg = if pressed {
                    GLASS_BTN_PRESS
                } else if hovered {
                    GLASS_BTN_HOVER
                } else {
                    GLASS_BTN
                };
                // Top row (AC, ±, %) get slightly different text
                let tc = if matches!(def.kind, BtnKind::Clear | BtnKind::Negate | BtnKind::Percent) {
                    OP_TEXT
                } else {
                    TEXT_PRIMARY
                };
                (bg, tc)
            };

            painter.rect_filled(rect, BTN_RADIUS * s, bg);

            // Button label — centered
            let label_w = text.measure_width(def.label, FONT_BTN * s);
            let lx = rect.x + (rect.w - label_w) / 2.0;
            let ly = rect.y + (rect.h - FONT_BTN * s) / 2.0;
            text.queue(
                def.label, FONT_BTN * s, lx, ly,
                text_color, rect.w, sw, 0,
            );

            // Advance grid position
            col += def.col_span as usize;
            if col >= BTN_COLS {
                col = 0;
                row += 1;
            }
        }

        // ── Submit frame ─────────────────────────────────────────────────
        let ctx = &mut gpu.ctx;
        match ctx.begin_frame("lntrn-calculator") {
            Ok(mut frame) => {
                let view = frame.view().clone();
                painter.render_layer(
                    0, ctx, frame.encoder_mut(), &view,
                    Some(Color::rgba(0.0, 0.0, 0.0, 0.0)),
                );
                text.render_queued(ctx, frame.encoder_mut(), &view);
                frame.submit(&ctx.queue);
            }
            Err(e) => eprintln!("[lntrn-calculator] render error: {e}"),
        }
    }
}

// ── Application handler ─────────────────────────────────────────────────────

impl ApplicationHandler for CalcApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("lntrn-calculator")
            .with_inner_size(winit::dpi::LogicalSize::new(WIN_W, WIN_H))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false);

        let window = event_loop
            .create_window(attrs)
            .expect("Failed to create window");
        self.scale = window.scale_factor() as f32;

        let size = window.inner_size();
        let gpu_ctx = GpuContext::from_window(&window, size.width, size.height)
            .expect("Failed to create GPU context");

        self.gpu = Some(Gpu {
            painter: Painter::new(&gpu_ctx),
            text: TextRenderer::new(&gpu_ctx),
            ctx: gpu_ctx,
        });
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.gpu = None;
                self.window = None;
                event_loop.exit();
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.needs_redraw = true;
            }

            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.ctx.resize(size.width, size.height);
                }
                self.needs_redraw = true;
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some((position.x as f32, position.y as f32));
                self.needs_redraw = true;
            }

            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.needs_redraw = true;
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    match state {
                        ElementState::Pressed => {
                            if let Some((cx, cy)) = self.cursor {
                                // Close button
                                if self.close_rect.contains(cx, cy) {
                                    self.gpu = None;
                                    self.window = None;
                                    event_loop.exit();
                                    return;
                                }
                                if let Some(idx) = self.btn_at(cx, cy) {
                                    self.pressed_btn = Some(idx);
                                } else {
                                    // Click on background — drag window
                                    if let Some(w) = &self.window {
                                        let _ = w.drag_window();
                                    }
                                    return;
                                }
                            }
                        }
                        ElementState::Released => {
                            if let Some(idx) = self.pressed_btn.take() {
                                if let Some((cx, cy)) = self.cursor {
                                    if self.btn_rects.get(idx).map_or(false, |r| r.contains(cx, cy)) {
                                        self.activate_btn(idx);
                                    }
                                }
                            }
                        }
                    }
                    self.needs_redraw = true;
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    self.handle_key(&event.logical_key);
                    self.needs_redraw = true;
                }
            }

            WindowEvent::RedrawRequested => {
                if self.needs_redraw {
                    self.render();
                    self.needs_redraw = false;
                }
            }

            _ => {}
        }

        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = CalcApp::new();
    event_loop.run_app(&mut app).expect("Event loop failed");
}
