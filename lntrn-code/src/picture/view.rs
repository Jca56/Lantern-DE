//! A picture in a file tab: a row of what can be done with it and what
//! it is, and under it the picture, fitted to the tab until the wheel
//! zooms it (toward the pointer) and a drag moves it. A double click
//! goes between fitted and one pixel for one. A picture with
//! see-through parts lies on a checkerboard.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{AreaCx, CursorIcon, FILL, Sense, Ui};

use crate::app::{App, TabState};
use crate::doc::DocId;
use crate::picture::{Picture, State};

/// Fitting does not enlarge a small picture past this.
const MAX_FIT: f64 = 8.0;
const MAX_ZOOM: f64 = 64.0;
/// One notch of the wheel, or one Zoom In, scales by this.
const ZOOM_STEP: f64 = 1.2;
/// This much of the picture stays in the tab however far it is dragged,
/// logical px.
const KEEP_IN_VIEW: f64 = 48.0;
/// A checkerboard square, logical px; they double past [`MAX_CHECKS`].
const CHECK: f64 = 12.0;
const MAX_CHECKS: f64 = 12_000.0;
/// Greys both black and white art shows against.
const CHECK_DARK: Color = Color::hex(0x5C5C5C);
const CHECK_LIGHT: Color = Color::hex(0x7D7D7D);

/// How a picture sits in its tab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Screen pixels per picture pixel; `None` fits the picture to the tab.
    pub zoom: Option<f64>,
    /// How far the picture's middle sits from the tab's.
    pub pan: Vec2,
    /// The scale it was last drawn at.
    pub shown: f64,
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: None, pan: Vec2::ZERO, shown: 1.0 }
    }
}

impl View {
    /// Back to fitting the tab.
    pub fn fit(&mut self) {
        (self.zoom, self.pan) = (None, Vec2::ZERO);
    }

    /// One picture pixel to one screen pixel, in the middle.
    pub fn actual(&mut self) {
        (self.zoom, self.pan) = (Some(1.0), Vec2::ZERO);
    }

    /// `by` steps in (or, negative, out) about the middle of the tab.
    pub fn step(&mut self, by: i32) {
        let factor = ZOOM_STEP.powi(by);
        (self.zoom, self.pan) = (Some(self.shown * factor), self.pan * factor);
    }
}

/// What the user asked of a picture's tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PictureOut {
    /// The picture was pressed: its tab is the focused one.
    pub clicked: bool,
    pub focused: bool,
    /// Read the file as text instead.
    pub as_text: bool,
    pub studio: bool,
    pub viewer: bool,
}

/// The squares of a checkerboard over `vis`, counted from `origin` so
/// they move with the picture.
fn checkerboard(ui: &mut Ui, vis: Rect, origin: Vec2) {
    if vis.is_empty() {
        return;
    }
    let mut cell = ui.m.px(CHECK).max(2.0);
    while vis.area() / (cell * cell) > MAX_CHECKS {
        cell *= 2.0;
    }
    ui.draw.rect(vis, CHECK_DARK);
    let first = |lo: f64, o: f64| ((lo - o) / cell).floor() as i64;
    let (i0, mut j) = (first(vis.min.x, origin.x), first(vis.min.y, origin.y));
    while origin.y + j as f64 * cell < vis.max.y {
        let mut i = i0;
        while origin.x + i as f64 * cell < vis.max.x {
            if (i + j).rem_euclid(2) == 1 {
                let min = Vec2::new(origin.x + i as f64 * cell, origin.y + j as f64 * cell);
                ui.draw.rect(Rect::from_min_size(min, Vec2::splat(cell)).intersection(&vis), CHECK_LIGHT);
            }
            i += 1;
        }
        j += 1;
    }
}

pub fn draw_picture(ui: &mut Ui, p: &mut Picture) -> PictureOut {
    let mut out = PictureOut::default();
    let m = ui.m;
    let ready = p.state == State::Ready;
    if let Some(left) = p.tick(ui.state.now) {
        ui.state.request_redraw_after(left);
    }
    // ---- one row: what to do with it, then what it is ----
    ui.row(|ui| {
        if ready {
            if ui.button("Fit").clicked {
                p.view.fit();
                ui.state.request_rebuild = true;
            }
            if ui.button("1:1").clicked {
                p.view.actual();
                ui.state.request_rebuild = true;
            }
            if p.animated() && ui.button(if p.playing { "Pause" } else { "Play" }).clicked {
                p.playing = !p.playing;
                ui.state.request_rebuild = true;
            }
        }
        // An SVG is its source too; a picture that would not read may
        // not be a picture at all.
        if p.is_svg() {
            out.as_text = ui.button("Edit Source").clicked;
        } else if !ready && p.state != State::Loading {
            out.as_text = ui.button("Open as Text").clicked;
        }
        if ready && !p.is_svg() {
            out.studio = ui.button("Edit in Studio").clicked;
        }
        out.viewer = ui.button("Open in Image Viewer").clicked;
        if ready {
            ui.label_dim(&p.info());
        }
    });
    // ---- the picture ----
    let canvas = ui.alloc(Vec2::new(FILL, ui.remaining_height()));
    let id = ui.id("code");
    let r = ui.interact(id, canvas, Sense::FOCUS);
    out.focused = ui.focusable(id, canvas);
    out.clicked = r.pressed;
    let Some(handle) = p.handle.filter(|_| ready) else {
        // Nothing to show yet, or ever: say which, in the middle.
        let text = if ready { "Loading…".to_owned() } else { p.info() };
        let style = ui.text_style();
        let size = Vec2::new((ui.measure(&text, &style) + m.pad).min(canvas.width()), style.line_height() as f64);
        ui.text_in_rect(&text, &style, Rect::from_center_size(canvas.center(), size), ui.theme.text_dim);
        return out;
    };
    let (w, h) = (f64::from(p.width.max(1)), f64::from(p.height.max(1)));
    let room = canvas.shrink(m.pad);
    let fit = (room.width() / w).min(room.height() / h).clamp(1.0e-4, MAX_FIT);
    let (least, most) = (fit.min(1.0) * 0.1, MAX_ZOOM.max(fit));
    let v = &mut p.view;
    if r.double_clicked {
        if v.zoom.is_some() { v.fit() } else { v.actual() }
        ui.state.request_rebuild = true;
    }
    let mut scale = v.zoom.map_or(fit, |z| z.clamp(least, most));
    if r.hovered && ui.state.wheel.y != 0.0 {
        let to = (scale * ZOOM_STEP.powf(ui.state.wheel.y / m.widget_h)).clamp(least, most);
        // The point of the picture under the pointer stays under it.
        let (at, middle) = (ui.state.pointer, canvas.center() + v.pan);
        v.pan = at - (at - middle) * (to / scale) - canvas.center();
        v.zoom = Some(to);
        scale = to;
        ui.state.wheel = Vec2::ZERO;
        ui.state.request_rebuild = true;
    }
    if r.dragging && v.zoom.is_some() {
        v.pan += r.drag_delta;
        ui.state.cursor_icon = CursorIcon::Grabbing;
        ui.state.request_rebuild = true;
    }
    let size = Vec2::new(w * scale, h * scale);
    if v.zoom.is_none() {
        v.pan = Vec2::ZERO;
    }
    let keep = m.px(KEEP_IN_VIEW);
    let reach = |picture: f64, tab: f64| ((picture + tab) * 0.5 - keep.min(picture)).max(0.0);
    let (rx, ry) = (reach(size.x, canvas.width()), reach(size.y, canvas.height()));
    v.pan = Vec2::new(v.pan.x.clamp(-rx, rx), v.pan.y.clamp(-ry, ry));
    // The status bar says the scale: draw again when it is another.
    if (v.shown - scale).abs() > 1.0e-9 {
        v.shown = scale;
        ui.state.request_rebuild = true;
    }
    let rect = Rect::from_center_size(canvas.center() + v.pan, size).round();
    ui.draw.push_clip(canvas);
    if p.translucent {
        checkerboard(ui, rect.intersection(&canvas), rect.min);
    }
    ui.draw.image(rect, handle, 0.0, Color::WHITE);
    ui.draw.stroke_rect(rect.expand(m.border), m.border, 0.0, ui.theme.border_dark);
    ui.draw.pop_clip();
    out
}

impl App {
    /// The body of a file tab that holds a picture.
    pub(crate) fn draw_picture_tab(&mut self, ui: &mut Ui, cx: &mut AreaCx<TabState>, id: DocId) {
        let Some(p) = self.pictures.get_mut(id) else {
            return;
        };
        let out = draw_picture(ui, p);
        let (path, name) = (p.path.clone(), p.title.clone());
        if out.focused {
            self.last_editor_focus = Some(ui.id("code"));
        }
        if out.clicked {
            self.focus_doc = Some(id);
            self.focus_area = Some(cx.area);
        }
        if out.as_text {
            self.pending_text.push(path.clone());
            cx.rebuild();
        }
        if out.studio {
            crate::launch::open_studio(&path);
            cx.toast(&format!("Opening {name} in Lantern Studio"));
        }
        if out.viewer {
            crate::launch::open_image_viewer(&path);
            cx.toast(&format!("Opening {name} in Image Viewer"));
        }
    }
}
