//! The power buttons along the bottom of the menu: lock, sleep, restart
//! and shut down, with Lantern's own pictures. All but lock ask first: the
//! row turns into the question and its two answers. At the row's other
//! end, Quick Kill starts the bar itself over, at once.

use lntrn_apps::Icons;
use lntrn_core::{log_info, log_warn};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::icons::{self, Icon};
use lntrn_ui::{Sense, Ui};

/// Logical sizes.
const ICON: f64 = 28.0;
const LABEL: f64 = 18.0;
const QUESTION: f64 = 20.0;
const PAD: f64 = 14.0;
const QUICK_KILL: &str = "Quick Kill";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Lock,
    Sleep,
    Restart,
    ShutDown,
}

impl Action {
    const ALL: [Action; 4] = [Action::Lock, Action::Sleep, Action::Restart, Action::ShutDown];

    fn label(self) -> &'static str {
        match self {
            Action::Lock => "Lock",
            Action::Sleep => "Sleep",
            Action::Restart => "Restart",
            Action::ShutDown => "Shut Down",
        }
    }

    /// Its picture: one of Lantern's own, by file name.
    fn icon(self) -> &'static str {
        match self {
            Action::Lock => "spark-menu-lockscreen.svg",
            Action::Sleep => "spark-menu-sleep.svg",
            Action::Restart => "spark-menu-restart.svg",
            Action::ShutDown => "spark-menu-shutdown.svg",
        }
    }

    fn question(self) -> Option<&'static str> {
        match self {
            Action::Lock => None,
            Action::Sleep => Some("Sleep now?"),
            Action::Restart => Some("Restart now?"),
            Action::ShutDown => Some("Shut down now?"),
        }
    }

    /// What does it: the lock screen, and the rest through logind, which
    /// systemd answers on `systemctl` and elogind on `loginctl` (the
    /// Command Center's choice, by `sd_booted`'s test).
    fn command(self) -> (&'static str, &'static [&'static str]) {
        let tool = if std::path::Path::new("/run/systemd/system").is_dir() { "systemctl" } else { "loginctl" };
        match self {
            Action::Lock => ("lntrn-lockscreen", &[]),
            Action::Sleep => (tool, &["suspend"]),
            Action::Restart => (tool, &["reboot"]),
            Action::ShutDown => (tool, &["poweroff"]),
        }
    }

    pub fn run(self) {
        let (program, args) = self.command();
        match lntrn_apps::launch::run_args(program, args) {
            Ok(()) => log_info!("launcher: {program} {args:?}"),
            Err(e) => log_warn!("launcher: {program} {args:?}: {e}"),
        }
    }
}

/// A button of the row: its face, and whether it was clicked.
fn face(ui: &mut Ui, id: &str, rect: Rect, fill: Option<Color>) -> bool {
    let r = ui.interact(ui.id(id), rect, Sense::CLICK);
    let lit = if r.held { 0.14 } else if r.hovered { 0.08 } else { 0.0 };
    let fill = fill.map(|f| f.lerp(Color::WHITE, lit)).or((lit > 0.0).then(|| Color::WHITE.fade(lit)));
    if let Some(fill) = fill {
        ui.draw.rounded_rect(rect, ui.m.px(10.0), fill);
    }
    r.clicked
}

/// Quick Kill at the right end of `row`, outlined so it reads apart from
/// the computer's own power buttons. Returns whether it was clicked, and
/// the rest of the row.
pub fn quick_kill(ui: &mut Ui, row: Rect) -> (bool, Rect) {
    let m = ui.m;
    let ink = ui.theme.text;
    let label = TextStyle::new(m.px(LABEL) as f32);
    let (pad, icon) = (m.px(PAD), m.px(ICON));
    let w = icon + m.px(10.0) + ui.measure(QUICK_KILL, &label) + pad * 2.0;
    let b = Rect::new(Vec2::new(row.max.x - w, row.min.y), row.max);
    let clicked = face(ui, QUICK_KILL, b, None);
    ui.draw.stroke_rect(b, m.px(1.0), m.px(10.0), ink.fade(0.22));
    let pic = Rect::from_min_size(Vec2::new(b.min.x + pad, (b.center().y - icon * 0.5).round()), Vec2::new(icon, icon));
    icons::draw(ui.draw, pic, Icon::Redo, ink, m.px(2.0));
    ui.text_in_rect(QUICK_KILL, &label, Rect::new(Vec2::new(pic.max.x + m.px(10.0), b.min.y), b.max), ink);
    (clicked, Rect::new(row.min, Vec2::new(b.min.x - m.px(6.0), row.max.y)))
}

/// The row in `rect`. Returns an action to run now: one that needs no
/// asking, or one whose question was just answered yes.
pub fn draw(ui: &mut Ui, icons: &mut Icons, rect: Rect, confirm: &mut Option<Action>) -> Option<Action> {
    let m = ui.m;
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    let label = TextStyle::new(m.px(LABEL) as f32);
    let pad = m.px(PAD);
    if let Some(action) = *confirm {
        let question = TextStyle::new(m.px(QUESTION) as f32).bold();
        ui.text_in_rect(action.question().unwrap_or_default(), &question, Rect::new(Vec2::new(rect.min.x + pad, rect.min.y), rect.max), ink);
        let yes_w = ui.measure(action.label(), &label) + pad * 2.0;
        let no_w = ui.measure("Cancel", &label) + pad * 2.0;
        let yes = Rect::new(Vec2::new(rect.max.x - yes_w, rect.min.y), rect.max);
        let no = Rect::new(Vec2::new(yes.min.x - pad - no_w, rect.min.y), Vec2::new(yes.min.x - pad, rect.max.y));
        let (go, back) = (face(ui, "yes", yes, Some(accent)), face(ui, "no", no, None));
        ui.text_centered(action.label(), &label, yes, lntrn_kit::look::on(accent));
        ui.text_centered("Cancel", &label, no, ink);
        // Either answer puts the question away.
        if back || go {
            *confirm = None;
            ui.state.request_rebuild = true;
        }
        return go.then_some(action);
    }
    let icon = m.px(ICON);
    let mut x = rect.min.x;
    let mut run = None;
    for action in Action::ALL {
        let w = icon + m.px(10.0) + ui.measure(action.label(), &label) + pad * 2.0;
        let b = Rect::new(Vec2::new(x, rect.min.y), Vec2::new(x + w, rect.max.y));
        x += w + m.px(6.0);
        if face(ui, action.label(), b, None) {
            match action.question() {
                Some(_) => {
                    *confirm = Some(action);
                    ui.state.request_rebuild = true;
                }
                None => run = Some(action),
            }
        }
        let pic = Rect::from_min_size(Vec2::new(b.min.x + pad, (b.center().y - icon * 0.5).round()), Vec2::new(icon, icon));
        if let Some(handle) = icons.get(action.icon()) {
            ui.draw.image(pic, handle, 0.0, Color::WHITE);
        }
        ui.text_in_rect(action.label(), &label, Rect::new(Vec2::new(pic.max.x + m.px(10.0), b.min.y), b.max), ink);
    }
    run
}
