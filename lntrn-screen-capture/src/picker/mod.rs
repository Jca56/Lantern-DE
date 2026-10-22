//! The picker: a see-through layer over every monitor at once, the area
//! to record dragged out on any of them, or a window or a whole screen
//! picked; Record or Enter starts, Cancel or Escape goes. One app with a
//! window a monitor, sharing what's chosen, so each knows its own monitor
//! without asking the compositor.

mod area;
mod draw;
mod windows;

use std::sync::{Arc, Mutex};

use lntrn_app::{AppHost, Waker};
use lntrn_kit::desktop::Desktop;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Action, AreaCx, CursorIcon, Host, HostCx, Key, NewWindow, Shell, ShellRequest, Ui};

use crate::audio::Sources;
use crate::capture::Monitor;
use crate::control;
use crate::look;
use crate::record::{Area, SMALLEST};
use area::Edge;
use draw::{Bar, Press};

/// What to record: where, on which monitor, and with what sound.
pub struct Choice {
    pub output: String,
    pub area: Area,
    pub sound: Sources,
}

/// Show the picker on every monitor, its sound switches set as `sound`
/// says; `None` when it was cancelled.
pub fn pick(monitors: &[Monitor], desktop: &Desktop, sound: Sources) -> Option<Choice> {
    let first = monitors.first()?;
    let chosen = Arc::new(Mutex::new(None));
    let mut shell = Shell::new(Editor::Screen(0));
    shell.prefs.theme = look::theme(desktop);
    lntrn_app::run(look::config(desktop, look::picker_layer(&first.name)), Picker::new(monitors, sound, chosen.clone()), shell);
    chosen.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// One window a monitor, by its place in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Screen(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Drag out an area; resize and move it by its handles.
    Area,
    /// Click a window.
    Window,
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    New { screen: usize, from: Vec2 },
    Edge { screen: usize, edge: Edge, start: Rect },
    Move { screen: usize, grab: Vec2, start: Rect },
}

impl Drag {
    fn screen(self) -> Option<usize> {
        match self {
            Drag::None => None,
            Drag::New { screen, .. } | Drag::Edge { screen, .. } | Drag::Move { screen, .. } => Some(screen),
        }
    }
}

struct Picker {
    monitors: Vec<Monitor>,
    editors: Vec<Editor>,
    /// Each monitor's surface as last drawn, physical pixels.
    surfaces: Vec<Rect>,
    /// The other monitors' windows were asked for.
    opened: bool,
    mode: Mode,
    /// The area chosen: on which monitor, where on its surface.
    sel: Option<(usize, Rect)>,
    drag: Drag,
    /// Each monitor's windows, bottom to top, while picking one.
    windows: Vec<Vec<windows::Win>>,
    /// The sound switches.
    sound: Sources,
    chosen: Arc<Mutex<Option<Choice>>>,
    /// Quitting: nothing more to draw.
    done: bool,
}

impl Picker {
    fn new(monitors: &[Monitor], sound: Sources, chosen: Arc<Mutex<Option<Choice>>>) -> Picker {
        Picker {
            editors: (0..monitors.len()).map(|i| Editor::Screen(i as u8)).collect(),
            surfaces: monitors.iter().map(|m| Rect::from_xywh(0.0, 0.0, f64::from(m.width), f64::from(m.height))).collect(),
            monitors: monitors.to_vec(),
            opened: false,
            mode: Mode::Area,
            sel: None,
            drag: Drag::None,
            windows: Vec::new(),
            sound,
            chosen,
            done: false,
        }
    }

    /// One monitor's frame: its keys, its pointer, what it shows.
    fn screen(&mut self, i: usize, ui: &mut Ui, cx: &mut AreaCx<()>) {
        if !self.opened {
            self.opened = true;
            for (j, m) in self.monitors.iter().enumerate().skip(1) {
                cx.request(ShellRequest::OpenWindow(NewWindow::single("Screen Capture", &format!("Screen {j}")).layer(look::picker_layer(&m.name))));
            }
        }
        if self.done {
            return;
        }
        if control::stopped() {
            return self.finish(None, cx);
        }
        // A signal's stop only sets a flag: look again soon.
        ui.state.request_redraw_after(0.5);
        let surface = ui.clip();
        self.surfaces[i] = surface;
        let was = (self.sel.map(|(s, _)| s), self.mode, self.sound);

        if ui.state.take_key(|k| k.key == Key::Escape && k.mods.is_empty()).is_some() {
            return self.finish(None, cx);
        }
        // Enter on a focused button presses that button instead.
        if ui.state.focus.is_none() && self.choice().is_some() && ui.state.take_key(|k| k.key == Key::Enter && k.mods.is_empty()).is_some() {
            return self.finish(self.choice(), cx);
        }

        let bar = Bar::layout(ui, surface);
        let hovered = match self.mode {
            Mode::Area => {
                self.drag_area(i, ui, surface, bar.panel);
                None
            }
            Mode::Window => self.hover_window(i, ui, surface, bar.panel),
        };
        let mine = self.sel.filter(|(s, _)| *s == i).map(|(_, r)| r);
        draw::shade(ui, surface, hovered.as_ref().map(|(r, _)| *r).or(mine));
        match (&hovered, mine) {
            (Some((r, name)), _) => {
                draw::frame(ui, *r, false);
                draw::label(ui, *r, surface, name);
            }
            (None, Some(r)) => {
                draw::frame(ui, r, self.mode == Mode::Area);
                let a = self.on_monitor(i, r);
                draw::label(ui, r, surface, &format!("{} × {}", a.w, a.h));
            }
            (None, None) => {}
        }
        draw::hint(ui, bar.panel, self.hint());
        let mut mode = self.mode as usize;
        let can_record = self.choice().is_some();
        let press = draw::bar(ui, &bar, &mut mode, &mut self.sound, can_record);
        if mode != self.mode as usize {
            self.set_mode(if mode == 1 { Mode::Window } else { Mode::Area });
        }
        match press {
            Some(Press::WholeScreen) => {
                self.sel = Some((i, surface));
                self.mode = Mode::Area;
            }
            Some(Press::Record) => return self.finish(self.choice(), cx),
            Some(Press::Cancel) => return self.finish(None, cx),
            None => {}
        }
        // The other monitors show what changed on this one.
        if (self.sel.map(|(s, _)| s), self.mode, self.sound) != was {
            control::wake();
        }
    }

    /// Area mode's pointer: drag out a new area, or take the one here by
    /// a handle or by its middle.
    fn drag_area(&mut self, i: usize, ui: &mut Ui, surface: Rect, bar: Rect) {
        let p = surface.clamp_point(ui.state.pointer);
        let reach = ui.m.px(draw::REACH);
        let mine = self.sel.filter(|(s, _)| *s == i).map(|(_, r)| r);
        let over_bar = bar.contains(ui.state.pointer);
        if ui.state.pressed && !over_bar {
            self.drag = match mine.map(|r| (r, area::edge_at(r, p, reach))) {
                Some((start, Some(edge))) => Drag::Edge { screen: i, edge, start },
                Some((start, None)) if start.contains(p) => Drag::Move { screen: i, grab: p - start.min, start },
                _ => Drag::New { screen: i, from: p },
            };
            if let Drag::New { .. } = self.drag {
                self.sel = None;
            }
        }
        if ui.state.down && self.drag.screen() == Some(i) {
            let r = match self.drag {
                Drag::New { from, .. } => area::span(from, p),
                Drag::Edge { edge, start, .. } => area::resize(start, edge, p),
                Drag::Move { grab, start, .. } => area::moved(start, p - grab, surface),
                Drag::None => return,
            };
            self.sel = Some((i, r));
        }
        if ui.state.released && self.drag.screen() == Some(i) {
            // A click that dragged nothing out chooses nothing.
            if matches!(self.drag, Drag::New { .. }) && self.sel.is_some_and(|(_, r)| r.width() < 4.0 || r.height() < 4.0) {
                self.sel = None;
            }
            self.drag = Drag::None;
        }
        ui.state.cursor_icon = match self.drag {
            Drag::Edge { edge, .. } => edge.cursor(),
            Drag::Move { .. } => CursorIcon::Grabbing,
            Drag::New { .. } => CursorIcon::Default,
            Drag::None => match mine {
                _ if over_bar => CursorIcon::Default,
                Some(r) => area::edge_at(r, p, reach).map_or(if r.contains(p) { CursorIcon::Grabbing } else { CursorIcon::Default }, Edge::cursor),
                None => CursorIcon::Default,
            },
        };
    }

    /// Window mode's pointer: the window under it lights up, and a click
    /// chooses it. Returns it and its name.
    fn hover_window(&mut self, i: usize, ui: &mut Ui, surface: Rect, bar: Rect) -> Option<(Rect, String)> {
        let p = ui.state.pointer;
        if !ui.state.pointer_in_window || bar.contains(p) {
            return None;
        }
        let (r, name) = self.windows.get(i)?.iter().rev().map(|w| (self.on_surface(i, w.rect), &w.label)).find(|(r, _)| r.contains(p))?;
        let (r, name) = (r.intersection(&surface), name.clone());
        ui.state.cursor_icon = CursorIcon::Pointer;
        if ui.state.pressed {
            self.sel = Some((i, r));
        }
        Some((r, name))
    }

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.drag = Drag::None;
        if mode == Mode::Window {
            self.windows = self.monitors.iter().map(|m| windows::on(&m.name)).collect();
        }
    }

    fn hint(&self) -> &'static str {
        let chosen = self.sel.is_some();
        match (self.mode, chosen, self.choice().is_some()) {
            (_, true, false) => "Too small to record: make it bigger",
            (Mode::Area, false, _) => "Drag out the area to record",
            (Mode::Area, true, _) => "Drag its edges to adjust  ·  Enter records",
            (Mode::Window, false, _) => "Click the window to record",
            (Mode::Window, true, _) => "Enter records, or click another window",
        }
    }

    /// `r` on monitor `i`'s surface, in the monitor's own pixels.
    fn on_monitor(&self, i: usize, r: Rect) -> Area {
        let (m, s) = (&self.monitors[i], self.surfaces[i]);
        let (kx, ky) = (f64::from(m.width) / s.width().max(1.0), f64::from(m.height) / s.height().max(1.0));
        let x = |v: f64| ((v - s.min.x) * kx).round().clamp(0.0, f64::from(m.width)) as u32;
        let y = |v: f64| ((v - s.min.y) * ky).round().clamp(0.0, f64::from(m.height)) as u32;
        let (x0, y0) = (x(r.min.x), y(r.min.y));
        Area { x: x0, y: y0, w: x(r.max.x) - x0, h: y(r.max.y) - y0 }
    }

    /// A rect in monitor `i`'s own pixels, on its surface.
    fn on_surface(&self, i: usize, r: Rect) -> Rect {
        let (m, s) = (&self.monitors[i], self.surfaces[i]);
        let k = Vec2::new(s.width() / f64::from(m.width.max(1)), s.height() / f64::from(m.height.max(1)));
        let at = |p: Vec2| Vec2::new(s.min.x + p.x * k.x, s.min.y + p.y * k.y);
        Rect::new(at(r.min), at(r.max))
    }

    /// What Record would record: the area, if it is big enough.
    fn choice(&self) -> Option<Choice> {
        let (i, r) = self.sel?;
        let area = self.on_monitor(i, r);
        (area.w >= SMALLEST && area.h >= SMALLEST).then(|| Choice { output: self.monitors[i].name.clone(), area, sound: self.sound })
    }

    fn finish(&mut self, choice: Option<Choice>, cx: &mut AreaCx<()>) {
        *self.chosen.lock().unwrap_or_else(|e| e.into_inner()) = choice;
        self.done = true;
        cx.request(ShellRequest::Quit);
    }
}

impl Host for Picker {
    type Editor = Editor;
    type AreaState = ();

    fn editors(&self) -> &[Editor] {
        &self.editors
    }

    fn editor_label(&self, _editor: Editor) -> &str {
        "Screen"
    }

    fn editor_id(&self, editor: Editor) -> String {
        let Editor::Screen(i) = editor;
        format!("Screen {i}")
    }

    fn editor_from_id(&self, id: &str) -> Option<Editor> {
        id.strip_prefix("Screen ")?.parse().ok().map(Editor::Screen)
    }

    fn title(&self) -> String {
        "Screen Capture".into()
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        false
    }

    fn paints_body(&self, _editor: Editor) -> bool {
        true
    }

    fn draw_body(&mut self, editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        let Editor::Screen(i) = editor;
        if usize::from(i) < self.monitors.len() {
            self.screen(usize::from(i), ui, cx);
        }
        false
    }

    fn run(&mut self, _action: &Action, _cx: &mut HostCx) {}

    /// A monitor going takes its window. The first monitor's window is
    /// the app's: its going quits, as Cancel does.
    fn close_requested(&mut self, _main: bool, _cx: &mut HostCx) -> bool {
        true
    }
}

impl AppHost for Picker {
    fn waker(&mut self, waker: Waker) {
        control::set_waker(waker);
    }
}
