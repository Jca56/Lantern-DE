use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

const BACKDROP_ALPHA: f32 = 0.55;
const MODAL_RADIUS: f32 = 12.0;
const MODAL_BORDER: f32 = 1.0;
const TITLE_FONT_SIZE: f32 = 24.0;
const BODY_FONT_SIZE: f32 = 18.0;
const BUTTON_FONT_SIZE: f32 = 18.0;
const PADDING: f32 = 24.0;
const BUTTON_H: f32 = 38.0;
const BUTTON_RADIUS: f32 = 6.0;
const BUTTON_GAP: f32 = 12.0;
const TITLE_BODY_GAP: f32 = 12.0;
const BODY_BUTTONS_GAP: f32 = 24.0;

/// A button shown in the modal footer.
#[derive(Clone, Debug)]
pub struct ModalButton {
    pub id: u32,
    pub label: String,
    pub primary: bool,
}

impl ModalButton {
    pub fn new(id: u32, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            primary: false,
        }
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}

/// A centered modal dialog with backdrop, title, body text, and action buttons.
///
/// ```ignore
/// let clicked = Modal::new(screen_w, screen_h)
///     .title("Delete file?")
///     .body("This action cannot be undone.")
///     .button(ModalButton::new(0, "Cancel"))
///     .button(ModalButton::new(1, "Delete").primary())
///     .hovered_button(hovered_id)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct Modal<'a> {
    screen_w: f32,
    screen_h: f32,
    width: f32,
    title: Option<&'a str>,
    body: Option<&'a str>,
    buttons: Vec<ModalButton>,
    hovered_button: Option<u32>,
    pressed_button: Option<u32>,
}

impl<'a> Modal<'a> {
    pub fn new(screen_w: f32, screen_h: f32) -> Self {
        Self {
            screen_w,
            screen_h,
            width: 420.0,
            title: None,
            body: None,
            buttons: Vec::new(),
            hovered_button: None,
            pressed_button: None,
        }
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    pub fn body(mut self, body: &'a str) -> Self {
        self.body = Some(body);
        self
    }

    pub fn button(mut self, btn: ModalButton) -> Self {
        self.buttons.push(btn);
        self
    }

    pub fn hovered_button(mut self, id: Option<u32>) -> Self {
        self.hovered_button = id;
        self
    }

    pub fn pressed_button(mut self, id: Option<u32>) -> Self {
        self.pressed_button = id;
        self
    }

    /// Returns the rect of the modal panel (for hit-testing backdrop clicks).
    pub fn panel_rect(&self) -> Rect {
        let h = self.compute_height();
        let x = (self.screen_w - self.width) * 0.5;
        let y = (self.screen_h - h) * 0.5;
        Rect::new(x, y, self.width, h)
    }

    /// Returns the rect of a specific button by id.
    pub fn button_rect(&self, id: u32) -> Option<Rect> {
        let panel = self.panel_rect();
        let total_btns = self.buttons.len();
        if total_btns == 0 {
            return None;
        }

        let btn_area_w = panel.w - PADDING * 2.0;
        let btn_w = (btn_area_w - (total_btns as f32 - 1.0) * BUTTON_GAP) / total_btns as f32;
        let btn_y = panel.y + panel.h - PADDING - BUTTON_H;

        self.buttons.iter().position(|b| b.id == id).map(|i| {
            let btn_x = panel.x + PADDING + i as f32 * (btn_w + BUTTON_GAP);
            Rect::new(btn_x, btn_y, btn_w, BUTTON_H)
        })
    }

    /// Backdrop rect (full screen).
    pub fn backdrop_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.screen_w, self.screen_h)
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        // -- Backdrop --
        painter.rect_filled(
            self.backdrop_rect(),
            0.0,
            Color::BLACK.with_alpha(BACKDROP_ALPHA),
        );

        let panel = self.panel_rect();

        // -- Shadow --
        let shadow = panel.expand(8.0);
        painter.rect_filled(shadow, MODAL_RADIUS + 4.0, Color::BLACK.with_alpha(0.3));

        // -- Panel --
        painter.rect_filled(panel, MODAL_RADIUS, palette.surface);
        painter.rect_stroke(panel, MODAL_RADIUS, MODAL_BORDER, palette.muted.with_alpha(0.2));

        let mut cy = panel.y + PADDING;

        // -- Title --
        if let Some(title) = self.title {
            text_renderer.queue(
                title,
                TITLE_FONT_SIZE,
                panel.x + PADDING,
                cy,
                palette.text,
                panel.w - PADDING * 2.0,
                screen_w,
                screen_h,
            );
            cy += TITLE_FONT_SIZE + TITLE_BODY_GAP;
        }

        // -- Body --
        if let Some(body) = self.body {
            text_renderer.queue(
                body,
                BODY_FONT_SIZE,
                panel.x + PADDING,
                cy,
                palette.text_secondary,
                panel.w - PADDING * 2.0,
                screen_w,
                screen_h,
            );
        }

        // -- Buttons --
        if self.buttons.is_empty() {
            return;
        }

        let total_btns = self.buttons.len();
        let btn_area_w = panel.w - PADDING * 2.0;
        let btn_w =
            (btn_area_w - (total_btns as f32 - 1.0) * BUTTON_GAP) / total_btns as f32;
        let btn_y = panel.y + panel.h - PADDING - BUTTON_H;

        for (i, btn) in self.buttons.iter().enumerate() {
            let btn_x = panel.x + PADDING + i as f32 * (btn_w + BUTTON_GAP);
            let btn_rect = Rect::new(btn_x, btn_y, btn_w, BUTTON_H);

            let is_hovered = self.hovered_button == Some(btn.id);
            let is_pressed = self.pressed_button == Some(btn.id);

            let (bg, text_color) = if btn.primary {
                let bg = if is_pressed {
                    Color::from_rgb8(170, 110, 8)
                } else if is_hovered {
                    Color::from_rgb8(220, 150, 15)
                } else {
                    palette.accent
                };
                (bg, Color::from_rgb8(20, 20, 20))
            } else {
                let bg = if is_pressed {
                    palette.bg
                } else if is_hovered {
                    palette.surface_2
                } else {
                    palette.surface_2.with_alpha(0.5)
                };
                (bg, palette.text)
            };

            painter.rect_filled(btn_rect, BUTTON_RADIUS, bg);

            let text_x =
                btn_rect.x + btn_rect.w * 0.5 - btn.label.len() as f32 * BUTTON_FONT_SIZE * 0.3;
            let text_y = btn_rect.y + (BUTTON_H - BUTTON_FONT_SIZE) * 0.5;
            text_renderer.queue(
                &btn.label,
                BUTTON_FONT_SIZE,
                text_x,
                text_y,
                text_color,
                btn_w,
                screen_w,
                screen_h,
            );
        }
    }

    fn compute_height(&self) -> f32 {
        let mut h = PADDING * 2.0;
        if self.title.is_some() {
            h += TITLE_FONT_SIZE + TITLE_BODY_GAP;
        }
        if self.body.is_some() {
            // Rough estimate: 2 lines of body text
            h += BODY_FONT_SIZE * 2.0 + BODY_BUTTONS_GAP;
        }
        if !self.buttons.is_empty() {
            h += BUTTON_H;
        }
        h
    }
}
