//! The badge: a plate in the top right corner of the monitor being
//! recorded, with a red dot, the time recorded, and Stop. The compositor
//! shows it but leaves it out of the recording. It goes once the file is
//! saved.

use std::sync::Arc;

use lntrn_app::{AppHost, Waker};
use lntrn_kit::controls::{self, Kind};
use lntrn_kit::desktop::Desktop;
use lntrn_kit::look as kit;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Action, AreaCx, Host, HostCx, Shell, ShellRequest, Ui};

use crate::control;
use crate::files;
use crate::look;
use crate::record::{Phase, Status};

/// Logical pixels: room for the dot, an hour's time and Stop, big.
const SIZE: (u32, u32) = (320, 88);
const DOT: Color = Color::hex(0xFF3B30);

/// Show the badge on the monitor called `output` until the recording is
/// saved (or the compositor takes the badge away).
pub fn show(output: &str, desktop: &Desktop, status: Arc<Status>) {
    let mut shell = Shell::new(Editor::Badge);
    shell.prefs.theme = look::theme(desktop);
    lntrn_app::run(look::config(desktop, look::badge_layer(output, SIZE)), Badge { status, editors: [Editor::Badge] }, shell);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Badge,
}

struct Badge {
    status: Arc<Status>,
    editors: [Editor; 1],
}

impl Badge {
    fn frame(&mut self, ui: &mut Ui, cx: &mut AreaCx<()>) {
        let phase = self.status.phase();
        if phase == Phase::Done {
            cx.request(ShellRequest::Quit);
            return;
        }
        let m = ui.m;
        let plate = ui.clip().shrink(m.px(4.0));
        let radius = m.px(18.0);
        ui.draw.rounded_rect(plate, radius, kit::card());
        ui.draw.stroke_rect(plate, m.px(1.0), radius, kit::line());
        let inner = plate.shrink(m.px(14.0));
        let style = ui.text_style();
        match phase {
            Phase::Starting => ui.text_centered("Starting…", &style, inner, kit::text_dim()),
            Phase::Saving | Phase::Done => ui.text_centered("Saving…", &style, inner, kit::text_dim()),
            Phase::Recording => {
                let elapsed = self.status.elapsed();
                // The dot blinks with the seconds.
                let lit = elapsed.as_secs().is_multiple_of(2);
                let d = m.px(20.0);
                let dot = Rect::from_center_size(Vec2::new(inner.min.x + d * 0.5, inner.center().y), Vec2::new(d, d));
                ui.draw.rounded_rect(dot, d * 0.5, if lit { DOT } else { DOT.fade(0.35) });
                let stop_w = controls::button_width(ui, "Stop");
                let stop = Rect::from_min_size(Vec2::new(inner.max.x - stop_w, (inner.center().y - m.widget_h * 0.5).round()), Vec2::new(stop_w, m.widget_h));
                let time = Rect::new(Vec2::new(dot.max.x + m.px(14.0), inner.min.y), Vec2::new(stop.min.x - m.px(12.0), inner.max.y));
                ui.text_in_rect(&files::clock(elapsed.as_secs_f64()), &style, time, kit::text());
                let id = ui.id("stop");
                if controls::button_of(ui, id, stop, "Stop", Kind::Danger, !control::stopped()) {
                    control::stop();
                }
                // Again at the next whole second: the time, and a stop
                // from a signal, which only sets a flag.
                ui.state.request_redraw_after((1.0 - elapsed.as_secs_f64().fract()).max(0.05));
            }
        }
    }
}

impl Host for Badge {
    type Editor = Editor;
    type AreaState = ();

    fn editors(&self) -> &[Editor] {
        &self.editors
    }

    fn editor_label(&self, _editor: Editor) -> &str {
        "Badge"
    }

    fn title(&self) -> String {
        "Recording".into()
    }

    fn shows_header(&self, _editor: Editor) -> bool {
        false
    }

    fn paints_body(&self, _editor: Editor) -> bool {
        true
    }

    fn draw_body(&mut self, _editor: Editor, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
        self.frame(ui, cx);
        false
    }

    fn run(&mut self, _action: &Action, _cx: &mut HostCx) {}
}

impl AppHost for Badge {
    fn waker(&mut self, waker: Waker) {
        control::set_waker(waker);
    }
}
